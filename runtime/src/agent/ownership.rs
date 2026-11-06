//! Caller-owned single-writer admission; native identities never become replay authority.
use super::*;
use serde::{Deserialize,Serialize};
use std::sync::{Arc,Mutex};

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct NativeProcess { pub pid:u32, pub started:String }
impl NativeProcess {
    pub fn read(pid:u32)->Res<Option<Self>> {crate::process::identity(pid).map(|p|p.map(|p|Self {pid:p.pid,started:p.started})).map_err(|e|format!("session-owner-unknown: {e}"))}
    fn live(&self)->Res<bool> {Ok(Self::read(self.pid)?.as_ref()==Some(self))}
}
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub struct OwnerObservation {pub state:String,pub reason:Option<String>}
impl Default for OwnerObservation {fn default()->Self {Self {state:"unknown".into(),reason:Some("no-owner-evidence".into())}}}

/// A terminal resume owns admission before it can spawn; it never submits a worker turn.
pub struct ResumeCommand {pub(super) command:Command,pub(super) owner:SessionOwner,pub(super) arguments:Vec<String>,pub(super) process:Process}
impl ResumeCommand {
    pub fn arguments(&self)->&[String] {&self.arguments}
    pub fn owner(&self)->&SessionOwner {&self.owner}
    pub fn spawn(self)->Res<std::process::Child> {self.owner.process_hooks(&self.process).spawn(self.command,"terminal resume")}
}

#[derive(Clone,Debug)]
pub struct SessionStore {pub path:PathBuf}
impl SessionStore {
    pub fn new(path:impl Into<PathBuf>)->Self {Self {path:path.into()}}
    fn directory(&self)->Res<PathBuf> {
        if !self.path.is_absolute() {return Err("session-store-requires-absolute-path".into());}
        let created=!self.path.exists();std::fs::create_dir_all(&self.path).map_err(|e|e.to_string())?;
        #[cfg(unix)] if created {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(&self.path,std::fs::Permissions::from_mode(0o700)).map_err(|e|e.to_string())?;}
        let _=created;self.path.canonicalize().map_err(|e|e.to_string())
    }
    pub(super) fn adopt(&self,spec:&Spec,session:&str,native:&[NativeProcess])->Res<OwnerObservation> {
        valid_session(session)?;
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let expected=binding(spec)?;
        for entry in std::fs::read_dir(&root).map_err(|e|e.to_string())? {
            let pending=entry.map_err(|e|e.to_string())?.path();
            if !pending.file_name().is_some_and(|s|s.to_string_lossy().starts_with("pending-") && pending.extension().is_some_and(|e|e=="json")) {continue;}
            if let Some(row)=read(&pending)? {
                if row["schema"]!="agentq-owner/1" || !row["runtime"].is_string() || !row["root"].is_string() || !(row["home"].is_null() || row["home"].is_string()) {return Err("session-owner-unknown: pending binding unreadable".into());}
                if ["runtime","root","home"].iter().any(|k|row[*k]!=expected[*k]) {continue;}
                scope(&row,spec)?;
                let _pending=lock(&pending.with_extension("lock")).map_err(|e|format!("session-owner-unknown: pending admission: {e}"))?;
                if process(&row["native"])?.map(|n|n.live()).transpose()?.unwrap_or(false) {
                    return Err("session-owner-unknown: pending native session must be bound before adoption".into());
                }
            }
        }
        let path=path(&root,spec.client.code(),session);let _guard=lock(&path.with_extension("lock"))?;
        if let Some(row)=read(&path)? {
            scope(&row,spec)?;
            if row["phase"]!="idle" {
                if let Some(native)=process(&row["native"])? {if native.live()? {return Err("session-busy: live native writer".into());}}
            }
        }
        for process in native {if process.live()? {return Err("session-busy: live native writer".into());}}
        let reason=Some("native-writer-unverifiable".to_string());
        let mut row=binding(spec)?;row["session"]=json!(session);row["phase"]=json!("idle");row["native_reason"]=json!(reason);
        save(&path,&row)?;
        Ok(OwnerObservation {state:"adopted".into(),reason})
    }
    pub(super) fn acquire(&self,spec:&Spec,fresh:bool,saved:Option<&Value>)->Res<SessionOwner> {
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let mut session=saved.and_then(|r|r["session"].as_str()).unwrap_or(&spec.session).to_string();
        let token=saved.and_then(|r|r["token"].as_str()).map(String::from).unwrap_or_else(crate::incident::uuid);
        if !native::plain_id(&token) || token=="-" {return Err("session-owner-unknown: invalid recovery token".into());}
        let mut file=if session=="-" {root.join(format!("pending-{token}.json"))} else {path(&root,spec.client.code(),&session)};
        // A checkpoint taken before discovery keeps its token after the native ID is bound.
        if session=="-" && saved.is_some() {
            if let Some(row)=read(&file)? {if let Some(target)=row["binding"].as_str() {valid_session(target)?;let target_path=path(&root,spec.client.code(),target);if target_path.exists() {session=target.into();file=target_path;}}}
            for entry in std::fs::read_dir(&root).map_err(|e|e.to_string())? {
                let candidate=entry.map_err(|e|e.to_string())?.path();if candidate.extension().is_none_or(|s|s!="json") {continue;}
                if let Some(row)=read(&candidate)? {if row["token"]==saved.unwrap()["token"] && row["token"].is_string() && row["session"]!="-" {
                    scope(&row,spec)?;session=row["session"].as_str().ok_or("session-owner-unknown: native session missing")?.into();file=candidate;break;
                }}
            }
        }
        let unknown=session=="-";
        if !unknown {valid_session(&session)?;}
        let guard=Some(lock(&file.with_extension("lock"))?);
        let old=read(&file)?;
        if let Some(saved)=saved {
            let row=old.as_ref().ok_or("session-owner-unknown: ownership record missing")?;
            scope(row,spec)?;
            if row["token"]!=saved["token"] || row["token"].as_str().is_none() {return Err("session-owner-unknown: recovery token differs".into());}
            if row["phase"]!="idle" {if let Some(native)=process(&row["native"])? {let _=native.live()?;}}
        } else if let Some(row)=&old {scope(row,spec)?;idle(row)?;if fresh {return Err("session-busy: native session already registered".into());}}
        else if !fresh {return Err("session-owner-unknown: explicitly adopt this session".into());}
        let mut row=binding(spec)?;row["session"]=json!(session);
        row["token"]=json!(token);
        row["phase"]=saved.map(|_|old.as_ref().unwrap()["phase"].clone()).unwrap_or(json!("prepared"));
        row["native"]=saved.map(|_|old.as_ref().unwrap()["native"].clone()).unwrap_or(Value::Null);
        row["native_reason"]=old.as_ref().map(|r|r["native_reason"].clone()).unwrap_or(Value::Null);
        row["binding"]=old.as_ref().map(|r|r["binding"].clone()).unwrap_or(Value::Null);
        save(&file,&row)?;
        Ok(SessionOwner(Arc::new(Mutex::new(Owned {root,path:file,row,previous:old,guard,binding:None}))))
    }
}

struct Owned {root:PathBuf,path:PathBuf,row:Value,previous:Option<Value>,guard:Option<coreq::file_lock::Lock>,binding:Option<(PathBuf,coreq::file_lock::Lock)>}
impl Drop for Owned {fn drop(&mut self) {
    if self.row["phase"]=="prepared" && self.row["native"].is_null() {
        if let Some(previous)=&self.previous {let _=save(&self.path,previous);} else {let _=std::fs::remove_file(&self.path);}
    }
}}
#[derive(Clone)]
pub struct SessionOwner(Arc<Mutex<Owned>>);
impl std::fmt::Debug for SessionOwner {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("SessionOwner").finish_non_exhaustive()}}
impl SessionOwner {
    fn with<T>(&self,f:impl FnOnce(&mut Owned)->Res<T>)->Res<T> {{let mut owned=self.0.lock().map_err(|_|"session-owner-unknown: poisoned owner")?;f(&mut owned)}}
    pub fn observation(&self)->OwnerObservation {self.with(|o|Ok(OwnerObservation {state:if o.row["phase"]=="idle" {"idle"} else if o.row["native"].is_null() || o.row["phase"]=="unconfirmed" {"unknown"} else {"owned"}.into(),reason:o.row["native_reason"].as_str().map(String::from).or_else(||o.row["native"].is_null().then(||"native-writer-unidentified".into()))})).unwrap_or_default()}
    pub fn checkpoint(&self)->Res<Value> {self.with(|o|Ok(json!({"session":o.row["session"],"token":o.row["token"]})))}
    pub(super) fn verify(&self,spec:&Spec)->Res<()> {self.with(|o| {scope(&o.row,spec)?;if spec.resume && o.row["session"]!=spec.session {return Err("session-owner-unknown: session differs".into());}Ok(())})}
    pub(super) fn spawned(&self,pid:u32)->Res<()> {self.with(|o| {
        if o.guard.is_none() {return Err("session-owner-unknown: admission released".into());}
        let native=NativeProcess::read(pid)?.ok_or("session-owner-unknown: child identity unavailable")?;
        if let Some(previous)=process(&o.row["native"])? {if previous!=native {return Err("session-owner-unknown: native process identity changed".into());}}
        o.row["native"]=json!(native);save(&o.path,&o.row)})}
    pub(super) fn observe_process(&self,pid:u32)->Res<()> {self.with(|o| {
        if o.guard.is_none() {return Ok(());}
        match NativeProcess::read(pid) {
            Ok(Some(native)) if o.row["native"].is_null()=>{o.row["native"]=json!(native);save(&o.path,&o.row)},
            Ok(Some(native)) if process(&o.row["native"])?.as_ref()==Some(&native)=>Ok(()),
            observed=>{
                let reason=if matches!(observed,Ok(Some(_))) {"native-process-changed"} else {"native-process-unverifiable"};
                if o.row["native_reason"]!=reason {o.row["native_reason"]=json!(reason);o.row["phase"]=json!("unconfirmed");save(&o.path,&o.row)?;}
                Ok(())
            }
        }
    })}
    pub(super) fn submit(&self)->Res<()> {self.with(|o| {
        if o.binding.is_some() {return Err("session-owner-unknown: binding write incomplete".into());}
        if o.guard.is_none() {
            let guard=lock(&o.path.with_extension("lock"))?;
            let row=read(&o.path)?.ok_or("session-owner-unknown: ownership record missing")?;
            if row["token"]!=o.row["token"] {return Err("session-busy: ownership transferred".into());}
            idle(&row)?;o.guard=Some(guard);
        }
        if o.row["phase"]!="prepared" && o.row["phase"]!="idle" {return Err("session-input-in-flight".into());}
        o.row["phase"]=json!("submitted");save(&o.path,&o.row)
    })}
    pub(crate) fn bind(&self,session:&str)->Res<()> {valid_session(session)?;self.with(|o| {
        if o.row["session"]==session {return Ok(());}
        if o.row["session"]!="-" {return Err("session-owner-unknown: native session differs".into());}
        let _admission=lock(&o.root.join("admission.lock"))?;
        let file=path(&o.root,o.row["runtime"].as_str().ok_or("ownership runtime missing")?,session);
        if let Some((target,_))=&o.binding {if *target!=file {return Err("session-owner-unknown: binding transition differs".into());}}
        else {
            let guard=lock(&file.with_extension("lock"))?;
            if let Some(existing)=read(&file)? {if existing["token"]!=o.row["token"] {return Err("session-busy: native session already registered".into());}}
            o.binding=Some((file.clone(),guard));
        }
        // Journal the destination before writing it, so an early checkpoint cannot revive an obsolete alias.
        let mut source=o.row.clone();source["binding"]=json!(session);save(&o.path,&source)?;o.row=source;
        let mut row=o.row.clone();row["session"]=json!(session);row["binding"]=Value::Null;
        let (target,_)=o.binding.as_ref().unwrap();save(target,&row)?;
        match std::fs::remove_file(&o.path) {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.to_string())}
        let (target,guard)=o.binding.take().unwrap();o.path=target;o.row=row;o.guard=Some(guard);Ok(())
    })}
    /// Release after this session's known terminal native report; never use a launcher exit as proof.
    pub fn finish(&self,report:&Report)->Res<()> {self.with(|o| {
        if report.session.as_deref().zip(o.row["session"].as_str()).is_some_and(|(seen,owned)|seen!=owned) {return Err("session-owner-unknown: native session differs".into());}
        if o.guard.is_none() {return Ok(());}
        if report.session.is_none() || report.outcome=="unknown" || report.background_job || matches!(report.error,"interrupted"|"timeout"|"exit"|"no-end") {
            o.row["phase"]=json!("unconfirmed");o.row["native_reason"]=json!("native-completion-unverified");return save(&o.path,&o.row);
        }
        o.row["phase"]=json!("idle");save(&o.path,&o.row)?;o.guard=None;Ok(())
    })}
    pub(super) fn stop_uncertain(&self)->Res<()> {self.with(|o| {
        o.row["phase"]=json!("unconfirmed");o.row["native_reason"]=json!("native-subtree-stop-unverified");save(&o.path,&o.row)
    })}
    pub(crate) fn process_hooks(&self,process:&Process)->Process {
        let owner=self.clone();let previous=process.spawned.clone();
        Process {configure:process.configure.clone(),spawned:Some(Arc::new(move |child| {if let Some(previous)=&previous {previous(child)?;}owner.spawned(child.id())?;owner.submit()}))}
    }
}
fn valid_session(session:&str)->Res<()> {if !session.starts_with('-') && native::plain_id(session) {Ok(())} else {Err("session requires a native session ID".into())}}
fn binding(spec:&Spec)->Res<Value> {
    let home=spec_home(spec).map(|p|p.canonicalize().map_err(|_|"session-owner-unknown: account home unavailable")).transpose()?;
    Ok(json!({"schema":"agentq-owner/1","runtime":spec.client.code(),"root":spec.root.canonicalize().map_err(|e|e.to_string())?,"home":home,
        "controller":NativeProcess::read(std::process::id())?.ok_or("session-owner-unknown: controller identity unavailable")?}))
}
fn scope(row:&Value,spec:&Spec)->Res<()> {let expected=binding(spec)?;if row["schema"]!="agentq-owner/1" || ["runtime","root","home"].iter().any(|k|row[*k]!=expected[*k]) {Err("session-owner-unknown: runtime, project or account differs".into())} else {Ok(())}}
fn path(root:&Path,runtime:&str,session:&str)->PathBuf {root.join(format!("{}.json",crate::sha256_hex(format!("{runtime}/{session}").as_bytes())))}
fn lock(path:&Path)->Res<coreq::file_lock::Lock> {coreq::file_lock::try_lock(path).map_err(|e|match e {std::fs::TryLockError::WouldBlock=>"session-busy: another writer holds admission".into(),std::fs::TryLockError::Error(e)=>format!("session-owner-unknown: {e}")})}
fn read(path:&Path)->Res<Option<Value>> {match std::fs::read(path) {Ok(b)=>serde_json::from_slice(&b).map(Some).map_err(|_|"session-owner-unknown: unreadable record".into()),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(e)=>Err(format!("session-owner-unknown: {e}"))}}
fn save(path:&Path,row:&Value)->Res<()> {coreq::write_durable(path,serde_json::to_vec(row).map_err(|e|e.to_string())?)}
fn process(value:&Value)->Res<Option<NativeProcess>> {if value.is_null() {Ok(None)} else {serde_json::from_value(value.clone()).map(Some).map_err(|_|"session-owner-unknown: invalid native identity".into())}}
fn idle(row:&Value)->Res<()> {
    if row["phase"]=="idle" {return Ok(());}
    match process(&row["native"])? {Some(native) if !native.live()?=>Err("session-owner-unknown: native writer ended without completion; adopt explicitly".into()),Some(_)=>Err("session-busy: native writer alive".into()),None=>Err("session-owner-unknown: native writer not identified".into())}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_runs_are_independent_and_early_checkpoints_follow_native_binding() {
        let root=crate::scratch("pending-owners");let home=root.join("home");std::fs::create_dir_all(&home).unwrap();
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:Some(home)};
        let store=SessionStore::new(root.join("owners"));let first=store.acquire(&spec,true,None).unwrap();let second=store.acquire(&spec,true,None).unwrap();
        assert_ne!(first.checkpoint().unwrap()["token"],second.checkpoint().unwrap()["token"]);drop(second);
        first.spawned(std::process::id()).unwrap();first.submit().unwrap();let early=first.checkpoint().unwrap();first.bind("native-1").unwrap();drop(first);
        let recovered=store.acquire(&spec,false,Some(&early)).unwrap();assert_eq!(recovered.checkpoint().unwrap()["session"],"native-1");
        let mut malformed=early;malformed["token"]=json!("../../foreign");assert!(store.acquire(&spec,false,Some(&malformed)).unwrap_err().starts_with("session-owner-unknown"));
        drop(recovered);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn admission_requires_adoption_and_refuses_live_reused_and_unknown_owners() {
        let root=crate::scratch("session-owners");let home=root.join("home");std::fs::create_dir_all(&home).unwrap();
        let spec=Spec {client:Client::Claude,root:root.clone(),session:"session-1".into(),resume:true,args:vec![],env:vec![],home:Some(home)};
        let store=SessionStore::new(root.join("owners"));
        assert!(store.acquire(&spec,false,None).unwrap_err().starts_with("session-owner-unknown"));
        let current=NativeProcess::read(std::process::id()).unwrap().unwrap();
        assert!(store.adopt(&spec,&spec.session,std::slice::from_ref(&current)).unwrap_err().starts_with("session-busy"));
        assert_eq!(store.adopt(&spec,&spec.session,&[]).unwrap().reason.as_deref(),Some("native-writer-unverifiable"));
        let owner=store.acquire(&spec,false,None).unwrap();owner.spawned(current.pid).unwrap();owner.submit().unwrap();
        assert!(store.acquire(&spec,false,None).unwrap_err().starts_with("session-busy"));
        let saved=owner.checkpoint().unwrap();assert!(store.acquire(&spec,false,Some(&saved)).unwrap_err().starts_with("session-busy"));drop(owner);
        assert!(store.acquire(&spec,false,None).unwrap_err().starts_with("session-busy"));
        let recovered=store.acquire(&spec,false,Some(&saved)).unwrap();drop(recovered);
        let file=path(&store.directory().unwrap(),spec.client.code(),&spec.session);
        let mut row=read(&file).unwrap().unwrap();row["native"]["started"]=json!("different native incarnation");save(&file,&row).unwrap();
        assert!(store.acquire(&spec,false,None).unwrap_err().starts_with("session-owner-unknown"));
        store.adopt(&spec,&spec.session,&[]).unwrap();let owner=store.acquire(&spec,false,None).unwrap();drop(owner);
        row["native"]=json!({"pid":0,"started":"unreadable"});save(&file,&row).unwrap();
        assert!(store.acquire(&spec,false,None).unwrap_err().starts_with("session-owner-unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_and_uncertain_handles_never_regain_submission_authority() {
        let root=crate::scratch("owner-stale-retry");
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"s".into(),resume:true,args:vec![],env:vec![],home:Some(root.clone())};
        let store=SessionStore::new(root.join("owners"));store.adopt(&spec,"s",&[]).unwrap();
        let stale=store.acquire(&spec,false,None).unwrap();let mut complete=Report::empty();complete.session=Some("s".into());complete.outcome="ok";complete.error="none";
        stale.finish(&complete).unwrap();
        let current=store.acquire(&spec,false,None).unwrap();current.submit().unwrap();let saved=current.checkpoint().unwrap();drop(current);
        for _ in 0..2 {assert!(stale.submit().unwrap_err().starts_with("session-busy"));}
        let current=store.acquire(&spec,false,Some(&saved)).unwrap();assert_eq!(current.checkpoint().unwrap(),saved);
        current.finish(&Report::empty()).unwrap();assert_eq!(current.observation().state,"unknown");
        for _ in 0..2 {assert_eq!(current.submit().unwrap_err(),"session-input-in-flight");}
        drop(current);drop(stale);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_binding_stays_fenced_and_early_recovery_follows_durable_intent() {
        let root=crate::scratch("owner-bind-write");
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let store=SessionStore::new(root.join("owners"));let owner=store.acquire(&spec,true,None).unwrap();
        owner.spawned(std::process::id()).unwrap();owner.submit().unwrap();
        let early=owner.checkpoint().unwrap();let directory=store.directory().unwrap();let target=path(&directory,"codex","native");
        let reserved=target.with_extension("json.pending");std::fs::write(&reserved,"interrupted write").unwrap();
        for _ in 0..2 {
            assert!(owner.bind("native").is_err());assert_eq!(owner.checkpoint().unwrap(),early);assert!(!target.exists());
            assert!(store.adopt(&spec,"native",&[]).unwrap_err().starts_with("session-owner-unknown"));
        }
        let pending=directory.join(format!("pending-{}.json",early["token"].as_str().unwrap()));let intent=read(&pending).unwrap().unwrap();
        std::fs::remove_file(reserved).unwrap();owner.bind("native").unwrap();assert_eq!(owner.checkpoint().unwrap()["session"],"native");drop(owner);
        let recovered=store.acquire(&spec,false,Some(&early)).unwrap();assert_eq!(recovered.checkpoint().unwrap()["session"],"native");
        let mut terminal=Report::empty();terminal.session=Some("native".into());terminal.outcome="ok";terminal.error="none";recovered.finish(&terminal).unwrap();drop(recovered);
        let mut resumed=spec.clone();resumed.session="native".into();resumed.resume=true;
        let newer=store.acquire(&resumed,false,None).unwrap();newer.submit().unwrap();let newer_token=newer.checkpoint().unwrap()["token"].clone();drop(newer);
        save(&pending,&intent).unwrap();assert!(store.acquire(&spec,false,Some(&early)).unwrap_err().starts_with("session-owner-unknown"));
        assert_eq!(read(&target).unwrap().unwrap()["token"],newer_token);
        std::fs::remove_dir_all(root).unwrap();
    }
}
