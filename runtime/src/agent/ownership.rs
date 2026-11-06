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
pub struct ResumeCommand {pub(super) command:Command,pub(super) owner:SessionOwner,pub(super) arguments:Vec<String>,pub(super) process:Process,pub(super) managed:bool}
impl ResumeCommand {
    pub fn arguments(&self)->&[String] {&self.arguments}
    pub fn owner(&self)->&SessionOwner {&self.owner}
    pub fn spawn(self)->Res<std::process::Child> {self.owner.process_hooks(&self.process,self.managed)?.spawn(self.command,"terminal resume")}
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
        valid_session(spec.client.code(),session)?;
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let expected=binding(spec)?;
        for entry in std::fs::read_dir(&root).map_err(|e|e.to_string())? {
            let pending=entry.map_err(|e|e.to_string())?.path();
            if !pending.file_name().is_some_and(|s|s.to_string_lossy().starts_with("pending-") && pending.extension().is_some_and(|e|e=="json")) {continue;}
            if let Some(row)=read(&pending)? {
                if row["schema"]!="agentq-owner/2" || !row["runtime"].is_string() || !row["root"].is_string() || !(row["home"].is_null() || row["home"].is_string()) {return Err("session-owner-unknown: pending binding unreadable".into());}
                if ["runtime","root","home"].iter().any(|k|row[*k]!=expected[*k]) {continue;}
                scope(&row,spec)?;
                if stopped(&root,&row)? {return Err("session-owner-unknown: unresolved native stop".into());}
                let _pending=lock(&pending.with_extension("lock")).map_err(|e|format!("session-owner-unknown: pending admission: {e}"))?;
                require_scope_absent(&root,&row)?;
                if process(&row["native"])?.map(|n|n.live()).transpose()?.unwrap_or(false) {
                    return Err("session-owner-unknown: pending native session must be bound before adoption".into());
                }
            }
        }
        let path=path(&root,spec.client.code(),session);let _guard=lock(&path.with_extension("lock"))?;
        if let Some(row)=read(&path)? {
            scope(&row,spec)?;
            if stopped(&root,&row)? {return Err("session-owner-unknown: unresolved native stop".into());}
            if row["phase"]!="idle" {
                require_scope_absent(&root,&row)?;
                if let Some(native)=process(&row["native"])? {if native.live()? {return Err("session-busy: live native writer".into());}}
            }
            retain_scope(&root,&row)?;
        }
        for process in native {if process.live()? {return Err("session-busy: live native writer".into());}}
        let reason=Some("native-writer-unverifiable".to_string());
        let mut row=binding(spec)?;row["session"]=json!(session);row["phase"]=json!("idle");row["native_reason"]=json!(reason);
        save(&path,&row)?;
        Ok(OwnerObservation {state:"adopted".into(),reason})
    }
    pub(super) fn acquire(&self,spec:&Spec,fresh:bool,saved:Option<&Value>,policy:Lifetime)->Res<SessionOwner> {
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let mut session=saved.and_then(|r|r["session"].as_str()).unwrap_or(&spec.session).to_string();
        let token=saved.and_then(|r|r["token"].as_str()).map(String::from).unwrap_or_else(crate::incident::uuid);
        if !native::plain_id(&token) || token=="-" {return Err("session-owner-unknown: invalid recovery token".into());}
        let mut file=if session=="-" {root.join(format!("pending-{token}.json"))} else {path(&root,spec.client.code(),&session)};
        // A checkpoint taken before discovery keeps its token after the native ID is bound.
        if session=="-" && saved.is_some() {
            if let Some(row)=read(&file)? {if let Some(target)=row["binding"].as_str() {valid_session(spec.client.code(),target)?;let target_path=path(&root,spec.client.code(),target);if target_path.exists() {session=target.into();file=target_path;}}}
            for entry in std::fs::read_dir(&root).map_err(|e|e.to_string())? {
                let candidate=entry.map_err(|e|e.to_string())?.path();if candidate.extension().is_none_or(|s|s!="json") {continue;}
                if let Some(row)=read(&candidate)? {if row["token"]==saved.unwrap()["token"] && row["token"].is_string() && row["session"]!="-" {
                    scope(&row,spec)?;session=row["session"].as_str().ok_or("session-owner-unknown: native session missing")?.into();file=candidate;break;
                }}
            }
        }
        let unknown=session=="-";
        if !unknown {valid_session(spec.client.code(),&session)?;}
        let guard=Some(lock(&file.with_extension("lock"))?);
        let old=read(&file)?;
        if let Some(saved)=saved {
            let row=old.as_ref().ok_or("session-owner-unknown: ownership record missing")?;
            scope(row,spec)?;
            if row["token"]!=saved["token"] || row["token"].as_str().is_none() {return Err("session-owner-unknown: recovery token differs".into());}
            if row["phase"]!="idle" {if let Some(native)=process(&row["native"])? {let _=native.live()?;}}
        } else if let Some(row)=&old {scope(row,spec)?;if stopped(&root,row)? {return Err("session-owner-unknown: unresolved native stop".into());}idle(row)?;if fresh {return Err("session-busy: native session already registered".into());}}
        else if !fresh {return Err("session-owner-unknown: explicitly adopt this session".into());}
        let alias=root.join(format!("pending-{token}.json"));
        let alias_guard=if saved.is_some() && session!="-" {
            if let Some(alias_row)=read(&alias)? {
                let guard=lock(&alias.with_extension("lock"))?;scope(&alias_row,spec)?;
                if alias_row["token"]!=token || alias_row["session"]!="-" || alias_row["binding"]!=session {return Err("session-owner-unknown: binding alias differs".into());}
                Some(guard)
            } else {None}
        } else {None};
        if saved.is_some() && (old.as_ref().unwrap()["lifetime"]!=json!(policy) || saved.unwrap()["lifetime"]!=json!(policy)) {return Err("session-owner-unknown: lifetime differs".into());}
        let mut row=binding(spec)?;row["session"]=json!(session);
        row["lifetime"]=json!(policy);
        row["lifetime_owner"]=if saved.is_some() {old.as_ref().unwrap()["lifetime_owner"].clone()} else {row["controller"].clone()};
        row["process_scope"]=if saved.is_some() {old.as_ref().unwrap()["process_scope"].clone()} else {Value::Null};
        row["token"]=json!(token);
        row["phase"]=saved.map(|_|old.as_ref().unwrap()["phase"].clone()).unwrap_or(json!("prepared"));
        row["native"]=saved.map(|_|old.as_ref().unwrap()["native"].clone()).unwrap_or(Value::Null);
        row["native_reason"]=old.as_ref().map(|r|r["native_reason"].clone()).unwrap_or(Value::Null);
        row["binding"]=old.as_ref().map(|r|r["binding"].clone()).unwrap_or(Value::Null);
        if stopped(&root,&row)? {row["phase"]=json!("stop-unresolved");row["native_reason"]=json!("native-subtree-stop-unverified");}
        if saved.is_none() {if let Some(old)=&old {retain_scope(&root,old)?;}}
        save(&file,&row)?;
        if alias_guard.is_some() {std::fs::remove_file(&alias).map_err(|e|e.to_string())?;}
        Ok(SessionOwner(Arc::new(Mutex::new(Owned {root,path:file,row,previous:old,guard,binding:None}))))
    }
}

/// Evidence from one durable native scope; cleanup rechecks the token and identities.
#[derive(Clone,Debug,Serialize,Deserialize)]
pub struct Orphan {pub session:String,pub token:String,pub state:String,pub controller:String,pub native:String,pub scope:String,pub reason:Option<String>,pub lifetime:Option<Lifetime>,pub controller_identity:Option<NativeProcess>,pub native_identity:Option<NativeProcess>,pub execution_scope:Value}
fn identity(value:&Value)->Res<&'static str> {
    let expected=process(value)?.ok_or("orphan-identity-unknown")?;
    Ok(match NativeProcess::read(expected.pid)? {None=>"absent",Some(current) if current==expected=>"live",Some(_)=>"changed"})
}
fn inventory(root:&Path,row:&Value)->Orphan {
    let controller=identity(&row["lifetime_owner"]);let native=identity(&row["native"]);
    let scope=process(&row["native"]).and_then(|n|lifetime::verified(&row["process_scope"],&n.ok_or("orphan-native-unknown")?));
    let absent=scope_absent(root,row);
    let scope_state=if absent==Ok(true) {"absent"} else if scope.is_ok() {"verified"} else {"unknown"};
    let state=match (&controller,&native,&scope) {(Ok("live"),_,_)=>"owned",(Ok("absent"),Ok("live"),Ok(()))=>"orphan",_=>"unknown"};
    let reason=controller.as_ref().err().or(native.as_ref().err()).or(scope.as_ref().err()).cloned().or_else(||(state=="unknown").then(||"orphan-identity-unverified".into()));
    Orphan {session:row["session"].as_str().unwrap_or("-").into(),token:row["token"].as_str().unwrap_or("").into(),state:state.into(),controller:controller.unwrap_or("unknown").into(),native:native.unwrap_or("unknown").into(),scope:scope_state.into(),reason,lifetime:serde_json::from_value(row["lifetime"].clone()).ok(),controller_identity:process(&row["lifetime_owner"]).ok().flatten(),native_identity:process(&row["native"]).ok().flatten(),execution_scope:row["process_scope"].clone()}
}
impl SessionStore {
    fn entries(&self,root:&Path,spec:&Spec)->Res<Vec<(PathBuf,Value)>> {
        let expected=binding(spec)?;let mut entries=vec![];
        let operations=root.join("operations");
        for directory in [root,operations.as_path()] {
            let listing=match std::fs::read_dir(directory) {Ok(listing)=>listing,Err(e) if directory==operations && e.kind()==std::io::ErrorKind::NotFound=>continue,Err(e)=>return Err(e.to_string())};
            for entry in listing {
                let path=entry.map_err(|e|e.to_string())?.path();if path.extension().is_none_or(|s|s!="json") {continue;}
                let row=read(&path)?.ok_or("orphan-record-disappeared")?;
                if !row["runtime"].is_string() || !row["root"].is_string() || !(row["home"].is_string() || row["home"].is_null()) {return Err("orphan-binding-unreadable".into());}
                if ["runtime","root","home"].iter().any(|k|row[*k]!=expected[*k]) {continue;}
                scope(&row,spec)?;entries.push((path,row));
            }
        }
        Ok(entries)
    }
    pub(super) fn orphans(&self,spec:&Spec)->Res<Vec<Orphan>> {
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let mut rows=std::collections::BTreeMap::<String,Value>::new();
        for (_,row) in self.entries(&root,spec)? {
            let Some(token)=row["token"].as_str() else {if row["phase"]=="idle" && row["native"].is_null() {continue;}return Err("orphan-token-unknown".into());};let token=token.to_string();
            // Binding aliases share a token; the native destination is authoritative.
            if rows.get(&token).is_none_or(|prior|prior["session"]=="-") {rows.insert(token,row);}
        }
        Ok(rows.values().map(|row|inventory(&root,row)).collect())
    }
    pub(super) fn cleanup(&self,spec:&Spec,entry:&Orphan,reconcile:bool)->Res<()> {
        if !native::plain_id(&entry.token) || entry.token.starts_with('-') {return Err("orphan-token-unknown".into());}
        let root=self.directory()?;let _admission=lock(&root.join("admission.lock"))?;
        let rows=self.entries(&root,spec)?.into_iter().filter(|(_,r)|r["token"]==entry.token).collect::<Vec<_>>();
        let (_,row)=rows.iter().find(|(_,r)|r["session"]==entry.session).ok_or("orphan-token-changed")?;
        let mut guards=vec![];for (path,_) in &rows {guards.push(lock(&path.with_extension("lock"))?);}
        let marker=stop_path(&root,row)?.ok_or("orphan-stop-token-unknown")?;
        if reconcile {
            let original=read(&marker)?.ok_or("orphan-stop-not-requested")?;scope(&original,spec)?;
            if original["token"]!=row["token"] || original["lifetime_owner"]!=row["lifetime_owner"] || original["process_scope"]!=row["process_scope"] || original["native"]!=row["native"] {return Err("orphan-stop-scope-changed".into());}
            if !scope_absent(&root,&original)? {return Err("orphan-scope-still-occupied".into());}
            for (path,row) in &rows {
                if row["process_scope"]!=original["process_scope"] || row["lifetime_owner"]!=original["lifetime_owner"] || row["native"]!=original["native"] {return Err("orphan-alias-scope-changed".into());}
                let mut row=row.clone();row["phase"]=json!("unconfirmed");row["native_reason"]=json!("native-completion-unverified");save(path,&row)?;
            }
            std::fs::remove_file(marker).map_err(|e|e.to_string())?;return Ok(());
        }
        if identity(&row["lifetime_owner"])?!="absent" {return Err("orphan-controller-not-absent".into());}
        let native=process(&row["native"])?.ok_or("orphan-native-unknown")?;
        lifetime::verified(&row["process_scope"],&native)?;
        std::fs::create_dir_all(marker.parent().unwrap()).map_err(|e|e.to_string())?;
        if let Some(original)=read(&marker)? {if ["token","native","process_scope","lifetime_owner"].iter().any(|k|original[*k]!=row[*k]) {return Err("orphan-stop-scope-changed".into());}} else {save(&marker,row)?;}
        for (path,row) in &rows {let mut row=row.clone();row["phase"]=json!("stop-unresolved");row["native_reason"]=json!("native-subtree-stop-unverified");save(path,&row)?;}
        request_stop(&root,row,&native)
    }
}

fn empty_proof(row:&Value)->Value {json!({"token":row["token"],"runtime":row["runtime"],"root":row["root"],"home":row["home"],"native":row["native"],"process_scope":row["process_scope"],"lifetime_owner":row["lifetime_owner"]})}
fn require_scope_absent(root:&Path,row:&Value)->Res<()> {
    if !row["process_scope"].is_null() && !scope_absent(root,row).map_err(|e|format!("session-owner-unknown: {e}"))? {return Err("session-owner-unknown: native scope still occupied".into());}Ok(())
}
fn retain_scope(root:&Path,row:&Value)->Res<()> {
    if row["process_scope"].is_null() {return Ok(());}
    let token=row["token"].as_str().filter(|s|native::plain_id(s) && !s.starts_with('-')).ok_or("orphan-token-unknown")?;
    let path=root.join("operations").join(format!("{token}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
    if let Some(existing)=read(&path)? {if empty_proof(&existing)!=empty_proof(row) {return Err("orphan-retained-scope-changed".into());}return Ok(());}
    save(&path,row)
}
fn scope_absent(root:&Path,row:&Value)->Res<bool> {
    let native=process(&row["native"])?.ok_or("orphan-native-unknown")?;
    lifetime::validate(&row["process_scope"],&native)?;
    let proof=empty_proof(row);
    let path=absence_path(root,&proof)?;
    if let Some(receipt)=read(&path)? {if receipt==proof {return Ok(true);}return Err("orphan-absence-scope-changed".into());}
    lifetime::absent(&row["process_scope"],&native)
}
fn absence_path(root:&Path,proof:&Value)->Res<PathBuf> {
    let token=proof["token"].as_str().filter(|s|native::plain_id(s) && !s.starts_with('-')).ok_or("orphan-token-unknown")?;
    Ok(root.join("scopes").join(format!("{token}.json")))
}
fn record_absence(root:&Path,proof:&Value)->Res<()> {
    let path=absence_path(root,proof)?;std::fs::create_dir_all(path.parent().unwrap()).map_err(|e|e.to_string())?;
    let _guard=coreq::file_lock::lock(&path.with_extension("lock")).map_err(|e|e.to_string())?;
    if let Some(existing)=read(&path)? {return if existing==*proof {Ok(())} else {Err("orphan-absence-scope-changed".into())};}save(&path,proof)
}

fn request_stop(root:&Path,row:&Value,native:&NativeProcess)->Res<()> {
    if let Some(stop)=lifetime::stop(&row["process_scope"],native)? {
        let root=root.to_path_buf();let expected=empty_proof(row);
        #[cfg(test)] let tracking_root=root.clone();
        #[cfg(test)] let pause=lifetime::test_receipts::take_stop_pause(&root);
        #[cfg(test)] let (result_send,result_receive)=std::sync::mpsc::channel();
        let writer=std::thread::Builder::new().name("agentq-stop".into()).spawn(move|| {
            #[cfg(test)] if let Some(pause)=pause {pause.wait();}
            let result=stop.wait().and_then(|()|record_absence(&root,&expected));
            // Keep the native Job handle until the durable evidence is written; name disappearance is not absence.
            #[cfg(test)] let _=result_send.send(result.clone());
            if let Err(error)=result {eprintln!("agentq: native scope cleanup unknown: {error}");}
            drop(stop);
        }).map_err(|e|e.to_string())?;
        #[cfg(test)] lifetime::test_receipts::register(&tracking_root,writer,result_receive);
        #[cfg(not(test))] drop(writer);
    }
    Ok(())
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
    pub fn observation(&self)->OwnerObservation {self.with(|o| {if stopped(&o.root,&o.row)? {return Ok(OwnerObservation {state:"unknown".into(),reason:Some("native-subtree-stop-unverified".into())});}Ok(OwnerObservation {state:if o.row["phase"]=="idle" {"idle"} else if o.row["native"].is_null() || o.row["phase"]=="unconfirmed" {"unknown"} else {"owned"}.into(),reason:o.row["native_reason"].as_str().map(String::from).or_else(||o.row["native"].is_null().then(||"native-writer-unidentified".into()))})}).unwrap_or_default()}
    pub fn checkpoint(&self)->Res<Value> {self.with(|o|Ok(json!({"session":o.row["session"],"token":o.row["token"],"lifetime":o.row["lifetime"]})))}
    pub(super) fn verify(&self,spec:&Spec)->Res<()> {self.with(|o| {scope(&o.row,spec)?;if stopped(&o.root,&o.row)? {return Err("session-owner-unknown: unresolved native stop".into());}if spec.resume && o.row["session"]!=spec.session {return Err("session-owner-unknown: session differs".into());}Ok(())})}
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
        if stopped(&o.root,&o.row)? {return Err("session-owner-unknown: unresolved native stop".into());}
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
    pub(crate) fn bind(&self,session:&str)->Res<()> {self.with(|o| {
        valid_session(o.row["runtime"].as_str().ok_or("ownership runtime missing")?,session)?;
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
        if stopped(&o.root,&o.row)? {o.row["phase"]=json!("stop-unresolved");o.row["native_reason"]=json!("native-subtree-stop-unverified");return save(&o.path,&o.row);}
        if report.session.is_none() || report.outcome=="unknown" || report.background_job || matches!(report.error,"interrupted"|"timeout"|"exit"|"no-end") {
            o.row["phase"]=json!("unconfirmed");o.row["native_reason"]=json!("native-completion-unverified");return save(&o.path,&o.row);
        }
        o.row["phase"]=json!("idle");save(&o.path,&o.row)?;o.guard=None;Ok(())
    })}
    pub(super) fn unscoped_submission(&self)->Res<()> {self.with(|o| {if o.row["lifetime"]==json!(Lifetime::StopWithOwner) {Err("lifetime-unsupported: external native submission".into())} else {Ok(())}})}
    pub(super) fn stop_uncertain(&self)->Res<()> {self.with(|o| {
        if o.guard.is_none() {return Err("session-owner-unknown: admission released".into());}
        let marker=stop_path(&o.root,&o.row)?.ok_or("session-owner-unknown: stop token missing")?;
        std::fs::create_dir_all(marker.parent().unwrap()).map_err(|e|e.to_string())?;
        if read(&marker)?.is_none() {save(&marker,&o.row)?;}
        o.row["phase"]=json!("stop-unresolved");o.row["native_reason"]=json!("native-subtree-stop-unverified");save(&o.path,&o.row)
    })}
    pub(crate) fn stop_scope(&self)->Res<bool> {
        self.stop_uncertain()?;
        self.with(|o| {
            if o.row["process_scope"].is_null() {return Ok(false);}
            let native=process(&o.row["native"])?.ok_or("orphan-native-unknown")?;
            request_stop(&o.root,&o.row,&native)?;Ok(true)
        })
    }
    pub(crate) fn process_hooks(&self,process:&Process,managed:bool)->Res<Process> {
        let policy=self.with(|o|serde_json::from_value::<Lifetime>(o.row["lifetime"].clone()).map_err(|_|"lifetime-unknown".into()))?;
        let managed=managed && process.configure.is_none() && process.spawned.is_none();
        if !managed && policy==Lifetime::StopWithOwner {return Err("lifetime-unsupported: caller process setup".into());}
        let setup=managed.then(||lifetime::Setup::new(policy)).transpose()?;
        let configure=process.configure.clone();let configure_setup=setup.clone();
        let owner=self.clone();let previous=process.spawned.clone();
        Ok(Process {configure:Some(Arc::new(move |command| {if let Some(configure)=&configure {configure(command);}if let Some(setup)=&configure_setup {setup.configure(command);}})),
            spawned:Some(Arc::new(move |child| {
                if let Some(previous)=&previous {previous(child)?;}
                owner.spawned(child.id())?;
                if let Some(setup)=&setup {
                    let mut scope=setup.started(child)?;
                    owner.with(|o| {scope["native"]=o.row["native"].clone();let mut row=o.row.clone();row["process_scope"]=scope;save(&o.path,&row)?;o.row=row;Ok(())})?;
                }
                owner.submit()?;
                if let Some(setup)=&setup {
                    let (root,proof)=owner.with(|o|Ok((o.root.clone(),empty_proof(&o.row))))?;
                    #[cfg(test)] let tracking_root=root.clone();
                    setup.release(child,Box::new(move||record_absence(&root,&proof)),#[cfg(test)] &tracking_root)?;
                }Ok(())
            }))})
    }
}
fn stop_path(root:&Path,row:&Value)->Res<Option<PathBuf>> {
    if row["token"].is_null() {return Ok(None);}
    let token=row["token"].as_str().filter(|s|native::plain_id(s) && !s.starts_with('-')).ok_or("session-owner-unknown: stop token invalid")?;
    Ok(Some(root.join("stops").join(format!("{token}.json"))))
}
fn stopped(root:&Path,row:&Value)->Res<bool> {Ok(row["phase"]=="stop-unresolved" || match stop_path(root,row)? {Some(path)=>read(&path)?.is_some(),None=>false})}
fn valid_session(runtime:&str,session:&str)->Res<()> {
    let structured=registry::structured_session(runtime);
    if !session.is_empty() && session!="-" && (structured || !session.starts_with('-') && native::plain_id(session)) {Ok(())} else {Err("session requires a native session ID".into())}
}
fn binding(spec:&Spec)->Res<Value> {
    let home=spec_home(spec).map(|p|p.canonicalize().map_err(|_|"session-owner-unknown: account home unavailable")).transpose()?;
    Ok(json!({"schema":"agentq-owner/2","runtime":spec.client.code(),"root":spec.root.canonicalize().map_err(|e|e.to_string())?,"home":home,
        "lifetime":Lifetime::default(),"lifetime_owner":null,"process_scope":null,
        "controller":NativeProcess::read(std::process::id())?.ok_or("session-owner-unknown: controller identity unavailable")?}))
}
fn scope(row:&Value,spec:&Spec)->Res<()> {let expected=binding(spec)?;if row["schema"]!="agentq-owner/2" || serde_json::from_value::<Lifetime>(row["lifetime"].clone()).is_err() || !(row["process_scope"].is_null() || row["process_scope"].is_object()) || ["runtime","root","home"].iter().any(|k|row[*k]!=expected[*k]) {Err("session-owner-unknown: runtime, project or account differs".into())} else {Ok(())}}
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
        let store=SessionStore::new(root.join("owners"));let first=store.acquire(&spec,true,None,Lifetime::Recoverable).unwrap();let second=store.acquire(&spec,true,None,Lifetime::Recoverable).unwrap();
        assert_ne!(first.checkpoint().unwrap()["token"],second.checkpoint().unwrap()["token"]);drop(second);
        first.spawned(std::process::id()).unwrap();first.submit().unwrap();let early=first.checkpoint().unwrap();first.bind("native-1").unwrap();drop(first);
        let recovered=store.acquire(&spec,false,Some(&early),Lifetime::Recoverable).unwrap();assert_eq!(recovered.checkpoint().unwrap()["session"],"native-1");
        let mut malformed=early;malformed["token"]=json!("../../foreign");assert!(store.acquire(&spec,false,Some(&malformed),Lifetime::Recoverable).unwrap_err().starts_with("session-owner-unknown"));
        drop(recovered);std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn admission_requires_adoption_and_refuses_live_reused_and_unknown_owners() {
        let root=crate::scratch("session-owners");let home=root.join("home");std::fs::create_dir_all(&home).unwrap();
        let spec=Spec {client:Client::Claude,root:root.clone(),session:"session-1".into(),resume:true,args:vec![],env:vec![],home:Some(home)};
        let store=SessionStore::new(root.join("owners"));
        assert!(store.adopt(&spec,"--foreign-flag",&[]).is_err(),"structured IDs do not relax CLI argument validation");
        assert!(store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap_err().starts_with("session-owner-unknown"));
        let current=NativeProcess::read(std::process::id()).unwrap().unwrap();
        assert!(store.adopt(&spec,&spec.session,std::slice::from_ref(&current)).unwrap_err().starts_with("session-busy"));
        assert_eq!(store.adopt(&spec,&spec.session,&[]).unwrap().reason.as_deref(),Some("native-writer-unverifiable"));
        let owner=store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap();owner.spawned(current.pid).unwrap();owner.submit().unwrap();
        assert!(store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap_err().starts_with("session-busy"));
        let saved=owner.checkpoint().unwrap();assert!(store.acquire(&spec,false,Some(&saved),Lifetime::Recoverable).unwrap_err().starts_with("session-busy"));drop(owner);
        assert!(store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap_err().starts_with("session-busy"));
        let recovered=store.acquire(&spec,false,Some(&saved),Lifetime::Recoverable).unwrap();drop(recovered);
        let file=path(&store.directory().unwrap(),spec.client.code(),&spec.session);
        let mut row=read(&file).unwrap().unwrap();row["native"]["started"]=json!("different native incarnation");save(&file,&row).unwrap();
        assert!(store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap_err().starts_with("session-owner-unknown"));
        store.adopt(&spec,&spec.session,&[]).unwrap();let owner=store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap();drop(owner);
        row["native"]=json!({"pid":0,"started":"unreadable"});save(&file,&row).unwrap();
        assert!(store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap_err().starts_with("session-owner-unknown"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_and_uncertain_handles_never_regain_submission_authority() {
        let root=crate::scratch("owner-stale-retry");
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"s".into(),resume:true,args:vec![],env:vec![],home:Some(root.clone())};
        let store=SessionStore::new(root.join("owners"));store.adopt(&spec,"s",&[]).unwrap();
        let stale=store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap();let mut complete=Report::empty();complete.session=Some("s".into());complete.outcome="ok";complete.error="none";
        stale.finish(&complete).unwrap();
        let current=store.acquire(&spec,false,None,Lifetime::Recoverable).unwrap();current.submit().unwrap();let saved=current.checkpoint().unwrap();drop(current);
        for _ in 0..2 {assert!(stale.submit().unwrap_err().starts_with("session-busy"));}
        let current=store.acquire(&spec,false,Some(&saved),Lifetime::Recoverable).unwrap();assert_eq!(current.checkpoint().unwrap(),saved);
        current.finish(&Report::empty()).unwrap();assert_eq!(current.observation().state,"unknown");
        for _ in 0..2 {assert_eq!(current.submit().unwrap_err(),"session-input-in-flight");}
        drop(current);drop(stale);std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_binding_stays_fenced_and_early_recovery_follows_durable_intent() {
        let root=crate::scratch("owner-bind-write");
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let store=SessionStore::new(root.join("owners"));let owner=store.acquire(&spec,true,None,Lifetime::Recoverable).unwrap();
        owner.spawned(std::process::id()).unwrap();owner.submit().unwrap();
        let early=owner.checkpoint().unwrap();let directory=store.directory().unwrap();let target=path(&directory,"codex","native");
        let reserved=target.with_extension("json.pending");std::fs::write(&reserved,"interrupted write").unwrap();
        for _ in 0..2 {
            assert!(owner.bind("native").is_err());assert_eq!(owner.checkpoint().unwrap(),early);assert!(!target.exists());
            assert!(store.adopt(&spec,"native",&[]).unwrap_err().starts_with("session-owner-unknown"));
        }
        let pending=directory.join(format!("pending-{}.json",early["token"].as_str().unwrap()));let intent=read(&pending).unwrap().unwrap();
        std::fs::remove_file(reserved).unwrap();owner.bind("native").unwrap();assert_eq!(owner.checkpoint().unwrap()["session"],"native");drop(owner);
        save(&pending,&intent).unwrap();
        let recovered=store.acquire(&spec,false,Some(&early),Lifetime::Recoverable).unwrap();assert_eq!(recovered.checkpoint().unwrap()["session"],"native");assert!(!pending.exists(),"recovery retires only its validated binding alias");
        let mut terminal=Report::empty();terminal.session=Some("native".into());terminal.outcome="ok";terminal.error="none";recovered.finish(&terminal).unwrap();drop(recovered);
        let mut resumed=spec.clone();resumed.session="native".into();resumed.resume=true;
        let newer=store.acquire(&resumed,false,None,Lifetime::Recoverable).unwrap();newer.submit().unwrap();let newer_token=newer.checkpoint().unwrap()["token"].clone();drop(newer);
        save(&pending,&intent).unwrap();assert!(store.acquire(&spec,false,Some(&early),Lifetime::Recoverable).unwrap_err().starts_with("session-owner-unknown"));
        assert_eq!(read(&target).unwrap().unwrap()["token"],newer_token);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unresolved_stop_survives_binding_alias_cleanup_and_native_finish() {
        let root=crate::scratch("owner-stop-alias");
        let spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let store=SessionStore::new(root.join("owners"));let owner=store.acquire(&spec,true,None,Lifetime::Recoverable).unwrap();
        owner.spawned(std::process::id()).unwrap();owner.submit().unwrap();let early=owner.checkpoint().unwrap();
        let directory=store.directory().unwrap();let target=path(&directory,"codex","native");let reserved=target.with_extension("json.pending");
        std::fs::write(&reserved,"interrupted write").unwrap();assert!(owner.bind("native").is_err());
        owner.stop_uncertain().unwrap();drop(owner);std::fs::remove_file(reserved).unwrap();
        let recovered=store.acquire(&spec,false,Some(&early),Lifetime::Recoverable).unwrap();recovered.bind("native").unwrap();
        let saved=recovered.checkpoint().unwrap();let mut alias=read(&target).unwrap().unwrap();alias["session"]=json!("-");alias["binding"]=json!("native");
        let pending=directory.join(format!("pending-{}.json",early["token"].as_str().unwrap()));save(&pending,&alias).unwrap();drop(recovered);
        let recovered=store.acquire(&spec,false,Some(&saved),Lifetime::Recoverable).unwrap();assert!(!pending.exists());
        let mut terminal=Report::empty();terminal.session=Some("native".into());terminal.outcome="ok";terminal.error="none";
        recovered.finish(&terminal).unwrap();assert_eq!(recovered.observation().reason.as_deref(),Some("native-subtree-stop-unverified"));
        assert!(recovered.submit().unwrap_err().contains("unresolved native stop"));drop(recovered);
        assert!(store.adopt(&spec,"native",&[]).unwrap_err().contains("unresolved native stop"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
