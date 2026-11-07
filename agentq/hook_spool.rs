//! M6.14: private metadata outbox; the current installation journal, never its writable marker, grants authority.
use agentq_runtime::agent::{Frozen, archive_file_identity};
use serde_json::{Value, json};
use std::{fs::{self, File}, path::{Path, PathBuf}};
type Res<T> = Result<T, String>;
const MARKER: &str = ".agentq-hook-spool.json";
fn read(path: &Path) -> Res<Value> { serde_json::from_slice(&fs::read(path).map_err(|_| "hook authority unreadable")?).map_err(|_| "hook state corrupt".into()) }
fn index_path(data: &Path) -> PathBuf { data.join("observe/hook-spools.json") }
fn index(data: &Path) -> Res<Value> {
    match fs::read(index_path(data)) {
        Ok(bytes) => {
            let v: Value = serde_json::from_slice(&bytes).map_err(|_| "hook authority corrupt")?;
            if v["schema"] != 1 || !v["entries"].is_array() {return Err("hook authority schema unknown".into());} Ok(v)
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"schema":1,"entries":[]})),
        Err(_) => Err("hook authority unreadable".into()),
    }
}
fn canonical(path: &Path) -> Res<PathBuf> { path.canonicalize().map_err(|_| "hook directory unavailable".into()) }
fn identity(path: &Path) -> Res<String> {
    if !fs::symlink_metadata(path).map_err(|_| "hook directory unavailable")?.is_dir() {return Err("hook directory changed".into());}
    let mut options = fs::OpenOptions::new(); options.read(true);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt; options.custom_flags(0x02000000);}
    archive_file_identity(&options.open(path).map_err(|_| "hook directory identity unknown")?)
}
fn regular(path: &Path) -> Res<()> {
    if !fs::symlink_metadata(path).map_err(|_| "hook file unreadable")?.is_file() {return Err("hook file identity unknown".into());} Ok(())
}
fn shared(data: &Path) -> Res<File> {
    let file = File::open(data.join("lifecycle.lock")).map_err(|_| "hook current revision unreadable")?;
    file.try_lock_shared().map_err(|_| "hook authority busy")?; Ok(file)
}
fn binding(data: &Path, entry: &Value) -> Res<PathBuf> {
    let b = &entry["binding"];
    if entry["active"] != true || b["schema"] != "agentq.hook-spool/1" || b["revision"].as_u64().is_none_or(|r| r == 0) {
        return Err("hook registration revoked or invalid".into());
    }
    let installation = fs::read_to_string(data.join("installation")).map_err(|_| "hook installation unknown")?;
    if installation.trim().is_empty() || b["installation"] != installation.trim() {return Err("hook installation changed".into());}
    let root = PathBuf::from(b["directory"].as_str().ok_or("hook directory unknown")?);
    if !root.is_absolute() || canonical(&root)? != root || b["physical"] != identity(&root)? {return Err("hook directory changed".into());}
    let marker = root.join(MARKER); regular(&marker)?;
    if read(&marker)? != *b {return Err("hook registration changed".into());}
    let journal = read(&data.join("owned.json"))?;
    let receipts = journal.as_array().ok_or("hook ownership unknown")?;
    let matching: Vec<_> = receipts.iter().filter(|r| r["file"] == marker.to_string_lossy().as_ref() && r["key"] == json!(["$file"]) && r["kind"] == "file").collect();
    if matching.len() != 1 || matching[0]["after"] != b.to_string() || entry["receipt"] != crate::configure::digest(matching[0]) {
        return Err("hook ownership revision changed".into());
    }
    Ok(root)
}

/// Registration is an explicit local operator operation; it never creates or repairs the selected directory.
pub fn register(data: &Path, runtime: &str, project: &str, directory: &Path, revoke: bool) -> Res<()> {
    if !crate::onboard::AGENTS.contains(&runtime) || !directory.is_absolute() || !project.split_once('/').is_some_and(|(a,b)| [a,b].iter().all(|p|!p.is_empty() && !p.starts_with('.') && p.bytes().all(|c|c.is_ascii_alphanumeric() || b"._-".contains(&c)))) {
        return Err("select a supported runtime, owner/project and absolute existing private directory".into());
    }
    let _lock = crate::executor::lock(data)?;
    let root = canonical(directory)?;
    let profile = canonical(&crate::onboard::root(runtime))?;
    let mut idx = index(data)?;
    let entries = idx["entries"].as_array_mut().ok_or("hook authority corrupt")?;
    if let Some(entry) = entries.iter_mut().find(|e| e["binding"]["directory"] == root.to_string_lossy().as_ref()) {
        if entry["binding"]["runtime"] != runtime || entry["binding"]["project"] != project || entry["binding"]["profile"] != profile.to_string_lossy().as_ref() {return Err("hook registration scope changed; bytes retained".into());}
        if entry["active"] != true {return Err("hook registration revoked; bytes retained".into());}
        binding(data, entry)?;
        if revoke { entry["active"] = json!(false); coreq::write_durable(&index_path(data), idx.to_string())?; }
        return Ok(());
    }
    if revoke {return Err("hook registration absent".into());}
    if entries.iter().any(|e| e["active"]==true && e["binding"]["runtime"]==runtime && e["binding"]["project"]==project && e["binding"]["profile"]==profile.to_string_lossy().as_ref()) {
        return Err("hook scope already registered; revoke its existing directory first".into());
    }
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(&root).map_err(|_| "hook directory unavailable")?;
        if m.mode() & 0o077 != 0 || m.uid() != unsafe {libc::geteuid()} {return Err("hook directory not already private; no permission repair".into());}
    }
    // An existing private directory is not permission to adopt another spool's retained bytes.
    if fs::read_dir(&root).map_err(|_| "hook directory unreadable")?.next().is_some() {return Err("hook directory not empty; retained bytes not adopted".into());}
    let installation = crate::telemetry::installation(data);
    if fs::read_to_string(data.join("installation")).map_err(|_| "hook installation unreadable")?.trim() != installation {return Err("hook installation unknown".into());}
    let b = json!({"schema":"agentq.hook-spool/1","installation":installation,"directory":root,"physical":identity(&root)?,"runtime":runtime,"project":project,"profile":profile,"revision":1});
    let marker = root.join(MARKER);
    let want = crate::onboard::Want {file:marker.clone(),key:vec!["$file".into()],value:json!(b.to_string()),append:false,previous:None,routing:None,agent:runtime.into(),instruction:false};
    crate::onboard::apply(&[want], &data.join("owned.json"))?;
    let journal = read(&data.join("owned.json"))?;
    let receipt = journal.as_array().and_then(|a| a.iter().find(|e| e["file"] == marker.to_string_lossy().as_ref() && e["after"] == b.to_string())).ok_or("hook ownership not recorded")?;
    fs::OpenOptions::new().write(true).open(data.join("owned.json")).and_then(|f| f.sync_all()).map_err(|_| "hook ownership sync unknown")?;
    fs::OpenOptions::new().write(true).open(&marker).and_then(|f| f.sync_all()).map_err(|_| "hook marker sync unknown")?;
    #[cfg(unix)] {
        File::open(data).and_then(|f|f.sync_all()).map_err(|_| "hook ownership directory sync unknown")?;
        File::open(&root).and_then(|f|f.sync_all()).map_err(|_| "hook marker directory sync unknown")?;
    }
    entries.push(json!({"binding":b,"receipt":crate::configure::digest(receipt),"active":true}));
    fs::create_dir_all(data.join("observe")).map_err(|_| "hook index unavailable")?;
    coreq::write_durable(&index_path(data), idx.to_string())
}

pub struct Delivery { data: PathBuf, pub state: PathBuf, entry: Option<Value>, _authority: Option<File> }
impl Delivery {
    pub fn open(data: &Path, runtime: &str, project: &str) -> Res<Self> {
        let idx = index(data)?;
        let profile = crate::onboard::root(runtime);
        let mut found = vec![];
        for entry in idx["entries"].as_array().ok_or("hook authority corrupt")? {
            let b = &entry["binding"];
            if b["runtime"] == runtime && b["project"] == project && b["profile"] == profile.canonicalize().ok().map(|p|p.to_string_lossy().into_owned()).unwrap_or_default() {
                // Revocation must not silently switch a session back to another delivery location.
                found.push(entry.clone());
            }
        }
        if !found.is_empty() && found.iter().all(|e|e["active"]==false) {return Err("hook registration revoked; bytes retained".into());}
        found.retain(|e|e["active"]!=false);
        if found.len() > 1 {return Err("hook registration ambiguous".into());}
        let entry = found.pop();
        let authority = if entry.is_some() {Some(shared(data)?)} else {None};
        let state = if let Some(e) = &entry {binding(data,e)?} else {data.join("observe")};
        let delivery=Self {data:data.into(),state,entry,_authority:authority};
        delivery.current()?;
        Ok(delivery)
    }
    pub(crate) fn scope(&self,runtime:&str,project:&str)->Res<()> {
        self.current()?;
        if crate::control::disabled(&self.data,runtime)? {return Err("hook integration disabled".into());}
        if let Some(entry)=&self.entry {if entry["binding"]["runtime"]!=runtime || entry["binding"]["project"]!=project {return Err("hook continuation scope differs".into());}}
        Ok(())
    }
    fn current(&self) -> Res<()> {
        if let Some(e) = &self.entry {
            let idx = index(&self.data)?;
            if !idx["entries"].as_array().is_some_and(|a|a.contains(e)) {return Err("hook current revision changed".into());}
            binding(&self.data,e)?;
        }
        Ok(())
    }
    pub fn persist(&self, frozen: &Frozen, deadline: std::time::Instant) -> Res<()> {
        self.current()?;
        let events = frozen.events()?;
        if events.is_empty() {return Err("hook records absent".into());}
        // A retried source range may have grown before its checkpoint committed; never replace its earlier batch.
        let batch = crate::configure::digest(&json!(events.iter().map(|e|e.id()).collect::<Vec<_>>()));
        let path = self.state.join(format!("{batch}.hook.json"));
        if path.try_exists().map_err(|_| "hook spool unreadable")? {
            self.load(&path)?; // A retry retains the original frozen timestamps and payload.
        } else {
            fs::create_dir_all(&self.state).map_err(|_| "hook state unavailable")?;
            let envelope = json!({"schema":1,"registration":self.entry.as_ref().map(|e|crate::configure::digest(&e["binding"])),"at":crate::now(),"frozen":frozen.save()});
            coreq::write_durable(&path,envelope.to_string())?;
        }
        self.deliver(&path, Some(deadline)).or_else(|_| Ok(())) // The committed file, not a successful append, acknowledges capture.
    }
    fn load(&self, path: &Path) -> Res<(Frozen,u64)> {
        regular(path)?;
        let v = read(path)?;
        if v.as_object().is_none_or(|o|o.len()!=4) || v["schema"]!=1 || v["registration"]!=json!(self.entry.as_ref().map(|e|crate::configure::digest(&e["binding"]))) {return Err("hook spool binding changed".into());}
        let at = v["at"].as_u64().filter(|at|*at<=crate::now()).ok_or("hook spool age unknown")?;
        let frozen = Frozen::restore(&v["frozen"])?;
        if let Some(e) = &self.entry {
            for r in v["frozen"]["records"].as_array().ok_or("hook records invalid")? {
                if r["observation"]["runtime"]!=e["binding"]["runtime"] || r["observation"]["project"]!=e["binding"]["project"] || r["observation"]["route"]!="hook" {return Err("hook spool scope changed".into());}
            }
        }
        let name = format!("{}.hook.json",crate::configure::digest(&json!(frozen.events()?.iter().map(|e|e.id()).collect::<Vec<_>>())));
        if path.file_name().and_then(|n|n.to_str()) != Some(&name) {return Err("hook spool identity changed".into());}
        Ok((frozen,at))
    }
    fn deliver(&self, path: &Path, deadline: Option<std::time::Instant>) -> Res<()> {
        self.current()?;
        let (frozen,_) = self.load(path)?;
        for r in frozen.save()["records"].as_array().ok_or("hook records invalid")? {
            if crate::control::disabled(&self.data,r["observation"]["runtime"].as_str().ok_or("hook runtime unknown")?)? {return Err("hook integration disabled".into());}
        }
        if deadline.is_some_and(|d|std::time::Instant::now()>=d) {return Err("hook delivery deferred".into());}
        let store = coreq::telemetry::Store::open(&self.data.join("observations.sqlite3"))?;
        for chunk in frozen.events()?.chunks(agentq_runtime::agent::APPEND_CHUNK) {
            if deadline.is_some_and(|d|std::time::Instant::now()>=d) {return Err("hook delivery deferred".into());}
            agentq_runtime::agent::append_records(&store,chunk)?;
        }
        self.current()?;
        fs::remove_file(path).map_err(|_| "hook delivered file removal unknown")?;
        #[cfg(unix)] File::open(&self.state).and_then(|f|f.sync_all()).map_err(|_| "hook delivered directory sync unknown")?;
        Ok(())
    }
    fn files(&self) -> Res<Vec<PathBuf>> {
        self.current()?;
        match fs::read_dir(&self.state) {
            Ok(entries) => entries.map(|e|e.map(|e|e.path()).map_err(|_|"hook spool unreadable".into())).collect::<Res<Vec<_>>>().map(|paths|paths.into_iter().filter(|p|p.file_name().is_some_and(|n|n.to_string_lossy().contains(".hook.json"))).collect()),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound && self.entry.is_none()=>Ok(vec![]),
            Err(_)=>Err("hook spool unreadable".into()),
        }
    }
}
pub(crate) fn locations(data:&Path)->Res<Vec<Delivery>> {
    let idx=index(data)?;
    let mut out=vec![Delivery {data:data.into(),state:data.join("observe"),entry:None,_authority:None}];
    for e in idx["entries"].as_array().ok_or("hook authority corrupt")? {
        if e["active"]==false {continue;}
        let authority=shared(data)?;
        let state=binding(data,e)?;
        let runtime=e["binding"]["runtime"].as_str().ok_or("hook runtime unknown")?;
        if crate::control::disabled(data,runtime)? {continue;}
        out.push(Delivery {data:data.into(),state,entry:Some(e.clone()),_authority:Some(authority)});
    }
    Ok(out)
}
pub(crate) fn continuation_locations(data:&Path)->Res<Vec<Delivery>> {
    for entry in index(data)?["entries"].as_array().ok_or("hook authority corrupt")? {
        let runtime=entry["binding"]["runtime"].as_str().ok_or("hook runtime unknown")?;
        if entry["active"]!=true || crate::control::disabled(data,runtime)? {return Err("hook continuation authority inactive; bytes retained".into());}
    }
    locations(data)
}
/// Called by the existing service pass; no endpoint, network, discovery, timer or process of its own.
pub fn drain(data:&Path)->Res<()> {
    let mut error=None;
    for location in locations(data)? {
        for path in location.files()? {
            if let Err(e)=location.deliver(&path,None) {error.get_or_insert(e);}
        }
    }
    error.map_or(Ok(()),Err)
}
/// Read-only; partial, corrupt, future or inaccessible evidence is never an empty healthy spool.
pub fn status(data:&Path)->Value {
    let result=(||->Res<(u64,Option<u64>)> {
        if index(data)?["entries"].as_array().ok_or("hook authority corrupt")?.iter().any(|e|e["active"]==false) {return Err("hook registration revoked; bytes retained".into());}
        let (mut ids,mut oldest)=(std::collections::HashSet::new(),None::<u64>);
        for location in locations(data)? {
            for path in location.files()? {
                let (frozen,at)=location.load(&path)?;ids.extend(frozen.events()?.iter().map(|e|e.id().to_string()));oldest=Some(oldest.map_or(at,|old|old.min(at)));
            }
        } Ok((ids.len() as u64,oldest))
    })();
    match result {
        Ok((count,oldest))=>json!({"state":if count==0 {"none"} else {"pending"},"pending":count,"oldest_age":oldest.map(|at|crate::now()-at)}),
        Err(_)=>json!({"state":"unknown","pending":null,"oldest_age":null,"reason":"hook_spool_unavailable"}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration,Instant};
    struct Fixture {root:PathBuf,data:PathBuf,spool:PathBuf,old:Option<std::ffi::OsString>}
    impl Fixture {
        fn new(tag:&str)->Self {
            let root=std::env::temp_dir().join(format!("agentq297-{tag}-{}",std::process::id()));
            let _=fs::remove_dir_all(&root);
            let mut dirs=fs::DirBuilder::new();dirs.recursive(true);
            #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;dirs.mode(0o700);}
            for name in ["data","spool","profile"] {dirs.create(root.join(name)).unwrap();}
            fs::write(root.join("taskq.json"),r#"{"repo":"invented/hook"}"#).unwrap();
            let old=std::env::var_os("CODEX_HOME");
            // The existing test runner serializes process-environment fixtures.
            unsafe {std::env::set_var("CODEX_HOME",root.join("profile"));}
            let f=Self {data:root.join("data"),spool:root.join("spool"),root,old};
            register(&f.data,"codex","invented/hook",&f.spool,false).unwrap();f
        }
        fn input(&self)->Value {json!({"session_id":"invented-session","turn_id":"invented-turn","transcript_path":self.root.join("rollout.jsonl"),"cwd":self.root})}
        fn transcript(&self) {
            fs::write(self.root.join("rollout.jsonl"), concat!(
                "{\"type\":\"turn_context\",\"payload\":{\"model\":\"invented-model\"}}\n",
                "{\"type\":\"event_msg\",\"timestamp\":\"2026-10-19T08:32:09.030Z\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":3,\"output_tokens\":2},\"total_token_usage\":{\"input_tokens\":3,\"output_tokens\":2}}}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"SECRET-CONTENT-NEVER-SPOOL\"}]}}\n",
                "{\"partial"
            )).unwrap();
        }
        fn outage(&self) {fs::create_dir(self.data.join("observations.sqlite3")).unwrap();}
        fn capture(&self) {crate::observe::stop(&self.data,"codex",&self.input(),false,Instant::now()+Duration::from_secs(3)).unwrap();}
        fn delivery(&self)->Delivery {Delivery::open(&self.data,"codex","invented/hook").unwrap()}
        fn files(&self)->Vec<PathBuf> {self.delivery().files().unwrap()}
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {if let Some(old)=&self.old {std::env::set_var("CODEX_HOME",old);} else {std::env::remove_var("CODEX_HOME");}}
            let _=fs::remove_dir_all(&self.root);
        }
    }
    // M6.14/#297: no service/store must lose no metadata; restart and a partial append cannot allocate new event IDs.
    #[test]
    fn outage_registered_write_restart_partial_commit_and_replay_are_idempotent() {
        let f=Fixture::new("outage");f.transcript();f.outage();f.capture();
        assert_eq!(status(&f.data)["pending"],2);
        let file=f.files().pop().unwrap();
        let original=fs::read(&file).unwrap();
        let text=String::from_utf8(original.clone()).unwrap();
        for secret in ["SECRET-CONTENT", "rollout.jsonl", "transcript_path", "cwd"] {assert!(!text.contains(secret),"{text}");}
        assert!(fs::read_dir(f.data.join("observe")).unwrap().all(|e|e.unwrap().file_name()=="hook-spools.json"),"sandbox state stays in explicit registered directory");
        f.capture();assert_eq!(fs::read(&file).unwrap(),original,"duplicate Stop keeps original durable payload");
        assert!(drain(&f.data).is_err());assert_eq!(fs::read(&file).unwrap(),original);
        fs::remove_dir(f.data.join("observations.sqlite3")).unwrap();
        let (frozen,_)=f.delivery().load(&file).unwrap();
        let store=coreq::telemetry::Store::open(&f.data.join("observations.sqlite3")).unwrap();
        let events=frozen.events().unwrap();store.append(&events[..1]).unwrap();drop(store);
        // Reconstruction reads only the owned index: no cached endpoint or retained in-memory authority.
        drain(&f.data).unwrap();assert!(!file.exists());
        let store=coreq::telemetry::Store::open(&f.data.join("observations.sqlite3")).unwrap();
        let rows=store.export(0,0,100).unwrap();assert_eq!(rows["events"].as_array().unwrap().len(),2);drop(store);
        fs::write(&file,&original).unwrap();drain(&f.data).unwrap();
        let store=coreq::telemetry::Store::open(&f.data.join("observations.sqlite3")).unwrap();
        assert_eq!(store.export(0,0,100).unwrap()["events"].as_array().unwrap().len(),2);
        assert_eq!(status(&f.data)["state"],"none");
    }
    // M6.14: a retry after checkpoint loss can contain new calls; preserving the old batch must not drop them.
    #[test]
    fn growing_source_after_checkpoint_loss_keeps_new_calls() {
        let f=Fixture::new("growth");f.transcript();f.outage();f.capture();
        let original_file=f.files().pop().unwrap();let original=fs::read(&original_file).unwrap();
        let source=canonical(&f.root.join("rollout.jsonl")).unwrap();
        let name=crate::session::event_id("transcript",&source.to_string_lossy())[..16].to_string();
        let checkpoint=f.delivery().state.join(name);
        assert!(checkpoint.is_file(),"corrupt the checkpoint actually written by capture");
        fs::write(&checkpoint,"corrupt").unwrap();
        assert!(crate::observe::stop(&f.data,"codex",&f.input(),false,Instant::now()+Duration::from_secs(3)).is_err());
        fs::remove_file(checkpoint).unwrap();
        let mut text=fs::read_to_string(&source).unwrap().replace("{\"partial","");
        text.push_str(&json!({"type":"event_msg","timestamp":"2026-10-19T08:33:09.030Z","payload":{"type":"token_count","info":{
            "last_token_usage":{"input_tokens":4,"output_tokens":2},"total_token_usage":{"input_tokens":7,"output_tokens":4}
        }}}).to_string());text.push('\n');fs::write(source,text).unwrap();
        f.capture();assert_eq!(fs::read(&original_file).unwrap(),original);assert_eq!(f.files().len(),2);
        assert_eq!(status(&f.data)["pending"],3,"overlapping immutable batches count unique IDs");
        fs::remove_dir(f.data.join("observations.sqlite3")).unwrap();drain(&f.data).unwrap();
        let store=coreq::telemetry::Store::open(&f.data.join("observations.sqlite3")).unwrap();
        assert_eq!(store.export(0,0,100).unwrap()["events"].as_array().unwrap().len(),3);
    }
    // M6.14/M6.11: Hermes has no transcript retry; its request and terminal turn must survive a stopped store.
    #[test]
    fn hermes_outage_preserves_call_and_turn_and_refuses_corrupt_aggregation() {
        let f=Fixture::new("hermes");f.outage();
        let input=json!({"session_id":"invented-hermes","api_request_id":"invented-api","model":"invented-model","ended_at":1792398729.0,"cwd":f.root,
            "usage":{"input_tokens":3,"output_tokens":2,"raw":"SECRET-CONTENT"},"prompt":"SECRET-CONTENT"});
        crate::observe::hermes_call(&f.data,&input,false).unwrap();
        let pending=f.data.join("observe/hermes").join(format!("{}.jsonl",crate::session::event_id("hermes","invented-hermes")));
        let bytes=fs::read(&pending).unwrap();assert!(!String::from_utf8_lossy(&bytes).contains("SECRET"));
        fs::write(&pending,b"{partial").unwrap();
        assert!(crate::observe::hermes_turn_end(&f.data,&input,false).is_err());assert_eq!(fs::read(&pending).unwrap(),b"{partial");
        fs::write(&pending,bytes).unwrap();
        crate::observe::hermes_turn_end(&f.data,&json!({"session_id":"invented-hermes","completed":true,"cwd":f.root}),false).unwrap();
        assert!(!pending.exists());assert_eq!(status(&f.data)["pending"],2);
        fs::remove_dir(f.data.join("observations.sqlite3")).unwrap();drain(&f.data).unwrap();
        let store=coreq::telemetry::Store::open(&f.data.join("observations.sqlite3")).unwrap();
        assert_eq!(store.export(0,0,100).unwrap()["events"].as_array().unwrap().len(),2);
    }
    // M6.14/P2: current unreadable authority, forged markers, replacement directories and revoke never permit effects.
    #[test]
    fn authority_loss_changed_marker_identity_and_revocation_refuse_without_fallback() {
        let f=Fixture::new("authority");f.transcript();f.outage();
        let index_bytes=fs::read(index_path(&f.data)).unwrap();
        fs::write(index_path(&f.data),"corrupt").unwrap();
        assert!(crate::observe::stop(&f.data,"codex",&f.input(),false,Instant::now()+Duration::from_secs(3)).is_err());
        assert!(drain(&f.data).is_err());assert_eq!(status(&f.data)["pending"],Value::Null);
        assert_eq!(fs::read_dir(&f.spool).unwrap().count(),1);
        fs::write(index_path(&f.data),&index_bytes).unwrap();
        let marker=fs::read(f.spool.join(MARKER)).unwrap();fs::write(f.spool.join(MARKER),"{}").unwrap();
        assert!(Delivery::open(&f.data,"codex","invented/hook").is_err());
        fs::write(f.spool.join(MARKER),marker).unwrap();
        let journal=fs::read(f.data.join("owned.json")).unwrap();fs::write(f.data.join("owned.json"),"[]").unwrap();
        assert!(Delivery::open(&f.data,"codex","invented/hook").is_err());
        fs::write(f.data.join("owned.json"),journal).unwrap();
        let prior=f.root.join("retained");fs::rename(&f.spool,&prior).unwrap();fs::create_dir(&f.spool).unwrap();
        assert!(Delivery::open(&f.data,"codex","invented/hook").is_err());
        fs::remove_dir(&f.spool).unwrap();fs::rename(prior,&f.spool).unwrap();
        f.capture();let bytes=fs::read(f.files().pop().unwrap()).unwrap();
        register(&f.data,"codex","invented/hook",&f.spool,true).unwrap();
        assert!(Delivery::open(&f.data,"codex","invented/hook").is_err());drain(&f.data).unwrap();
        assert_eq!(status(&f.data)["state"],"unknown");
        assert!(fs::read_dir(&f.spool).unwrap().any(|e|fs::read(e.unwrap().path()).ok()==Some(bytes.clone())));
        assert!(register(&f.data,"codex","invented/hook",&f.spool,false).is_err());
    }
    // M6.14/P2: partial and corrupt files remain private and unknown; no skip or partial delivery is success.
    #[test]
    fn corrupt_partial_future_and_scope_mismatch_remain_unknown_and_retained() {
        let f=Fixture::new("corrupt");f.transcript();f.outage();f.capture();
        let file=f.files().pop().unwrap();let original=fs::read(&file).unwrap();
        for edit in ["partial","future","scope","content"] {
            let mut v:Value=serde_json::from_slice(&original).unwrap();
            match edit {
                "partial"=>{fs::write(&file,"{partial").unwrap();},
                "future"=>{v["at"]=json!(u64::MAX);fs::write(&file,v.to_string()).unwrap();},
                "scope"=>{v["frozen"]["records"][0]["observation"]["project"]=json!("other/project");fs::write(&file,v.to_string()).unwrap();},
                _=>{v["frozen"]["records"][0]["observation"]["prompt"]=json!("SECRET");fs::write(&file,v.to_string()).unwrap();},
            }
            let before=fs::read(&file).unwrap();assert_eq!(status(&f.data)["state"],"unknown");assert!(drain(&f.data).is_err());assert_eq!(fs::read(&file).unwrap(),before);
        }
        fs::write(&file,&original).unwrap();
        let partial=file.with_file_name(format!("{}.pending",file.file_name().unwrap().to_string_lossy()));fs::write(&partial,"{partial").unwrap();
        assert_eq!(status(&f.data)["pending"],Value::Null);assert!(partial.exists());
        fs::remove_dir(f.data.join("observations.sqlite3")).unwrap();
        assert!(drain(&f.data).is_err());assert!(partial.exists());assert!(!file.exists(),"a partial record cannot starve a complete record");
    }
}
