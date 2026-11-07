//! Native registration catalog shared by worker execution and installation (agentq.md M12.9).
use super::*;

#[derive(Clone, Copy)]
pub struct Profile {
    pub kind: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    pub operations: &'static [&'static str],
}

/// Journal edit request. The installation's journal still validates, applies and restores it.
pub struct Edit {
    pub file: PathBuf,
    pub key: Vec<String>,
    pub value: Value,
    pub append: bool,
    /// An exact owned hook value that may be migrated to `value`.
    pub previous: Option<Value>,
    pub routing: Option<String>,
    pub agent: String,
    pub instruction: bool,
}

impl Edit {
    pub fn field(&self) -> &'static str {
        if self.instruction { return "instructions"; }
        match (&self.routing, self.key[0].as_str()) {
            (Some(_), _) => "routing",
            (None, "hooks") => "hooks",
            (None, key) if entry(&self.agent).is_some_and(|e|e.hook_key==key) => "hooks",
            (None, "plugins" | "$file") => "plugin",
            _ => "mcp",
        }
    }
}

#[derive(Clone, Copy)]
pub enum Hook { None, Stop { model: bool }, SessionStart, ModelCall, TurnEnd }

pub struct Provider {pub selected:bool,pub present:bool,pub loopback:bool}

pub struct ProfileState {
    pub hooks: &'static str,
    pub plugin: &'static str,
    pub adapter: &'static str,
    pub overrides: Option<usize>,
}

/// Every callback is required; an integration without a worker explicitly sets `runtime` to None.
/// ```compile_fail
/// let incomplete = agentq_runtime::agent::registry::Entry { name: "incomplete" };
/// ```
#[derive(Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub order: usize,
    pub profile_order: usize,
    pub worker_order: usize,
    pub onboard: bool,
    pub profile: Profile,
    pub runtime: Option<&'static dyn Runtime>,
    pub unqualified: Option<&'static str>,
    builtin: Option<&'static Adapter>,
    pub root: fn() -> PathBuf,
    pub files: fn(&Path) -> Vec<PathBuf>,
    pub setup: fn(&Path, &Path, &Path) -> Vec<Edit>,
    pub inspect: fn(&[Option<String>], &[Option<Value>], &[bool]) -> ProfileState,
    pub hook: fn(&str) -> Hook,
    pub hook_key: &'static str,
    pub fallback: fn(&str) -> Value,
    pub executable: fn() -> Option<PathBuf>,
    pub diagnose: fn(&Path) -> Value,
    pub provider: Option<fn(&Path) -> Result<Provider,String>>,
}

impl Entry {
    /// Metadata remains visible when native proof is missing; this grants no launch authority.
    pub fn descriptor(&self)->Option<&'static Descriptor> {
        self.runtime.map(|r|r.descriptor()).or_else(||self.builtin.map(|a|&a.descriptor))
    }
    pub fn qualification(&self)->Check {
        match self.unqualified {
            Some(reason)=>check("unqualified",reason),
            None if self.runtime.is_some()=>check("unknown","see version and platform native evidence"),
            None=>check("unsupported","integration only"),
        }
    }
}

include!(concat!(env!("OUT_DIR"), "/registry.rs"));

pub const AGENTS: [&str; ENTRIES.len()] = {
    let mut out = [""; ENTRIES.len()];
    let mut i = 0;
    while i < ENTRIES.len() { out[ENTRIES[i].order] = ENTRIES[i].name; i += 1; }
    out
};
const ONBOARD_COUNT: usize = {
    let mut n = 0; let mut i = 0;
    while i < ENTRIES.len() { if ENTRIES[i].onboard { n += 1; } i += 1; }
    n
};
pub const ONBOARD_AGENTS: [&str; ONBOARD_COUNT] = {
    let mut out = [""; ONBOARD_COUNT]; let mut i = 0; let mut n = 0;
    while i < AGENTS.len() {
        let mut j = 0;
        while j < ENTRIES.len() { if ENTRIES[j].order == i && ENTRIES[j].onboard { out[n] = ENTRIES[j].name; n += 1; } j += 1; }
        i += 1;
    }
    out
};
pub const KINDS: [(&str, &str, &[&str], &[&str]); ENTRIES.len()] = {
    let mut out: [(&str, &str, &[&str], &[&str]); ENTRIES.len()] = [("", "", &[], &[]); ENTRIES.len()];
    let mut i = 0;
    while i < ENTRIES.len() { let e = ENTRIES[i]; out[e.profile_order] = (e.profile.kind, e.profile.label, e.profile.aliases, e.profile.operations); i += 1; }
    out
};

pub fn entries() -> Vec<&'static Entry> { let mut entries = ENTRIES.to_vec(); entries.sort_by_key(|e| e.order); entries }
fn native_entries()->impl Iterator<Item=&'static Entry> {
    #[cfg(test)] let fixtures=examples::ENTRIES.as_slice();
    #[cfg(not(test))] let fixtures:&[&'static Entry]=&[];
    ENTRIES.into_iter().chain(fixtures.iter().copied())
}
pub fn entry(name: &str) -> Option<&'static Entry> { native_entries().find(|e| e.name == name || e.aliases.contains(&name)) }
pub fn profile(kind: &str) -> Option<&'static Entry> { ENTRIES.into_iter().find(|e| e.profile.kind == kind) }
pub(super) fn builtin(client: Client) -> &'static Adapter { native_entries().filter_map(|e| e.builtin).find(|a| a.client == client).expect("native adapter is registered") }

pub fn cli_present(entry: &Entry) -> Result<PathBuf, String> {
    let path = (entry.executable)().ok_or_else(|| format!("{} has no CLI", entry.name))?;
    if path.is_absolute() && path.is_file() { Ok(path) } else { Err(format!("{}: CLI {} not found", entry.name, path.display())) }
}

fn host_home() -> PathBuf { PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default()) }
fn root(var: &str, directory: &str) -> PathBuf { std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| host_home().join(directory)) }
fn grouped_hooks(doc: &Value, agent: &str, events: &[&str]) -> &'static str {
    hook_state(events.iter().map(|event| doc["hooks"][event].as_array().into_iter().flatten()
        .flat_map(|m| m["hooks"].as_array().into_iter().flatten())
        .any(|h| h["command"].as_str().is_some_and(|s| s.ends_with(&format!(" hook {agent} {event}"))))))
}
fn hook_state(values: impl Iterator<Item = bool>) -> &'static str {
    let values: Vec<_> = values.collect();
    match values.iter().filter(|v| **v).count() { 0 => "absent", n if n == values.len() => "registered", _ => "partial" }
}
fn edit(agent: &str, file: PathBuf, key: &[&str], value: Value, append: bool) -> Edit {
    Edit { file, key:key.iter().map(|k| k.to_string()).collect(), value, append, previous:None, routing:None, agent:agent.into(), instruction:false }
}
fn hook_command(exe: &Path, agent: &str, event: &str) -> Value { json!({"type":"command","command":format!("\"{}\" hook {agent} {event}",exe.display()),"timeout":5}) }
#[cfg(test)]
fn empty(_: &str) -> Value { json!({}) }
#[cfg(test)]
fn no_hook(_: &str) -> Hook { Hook::None }
fn no_inspection(_: &[Option<String>], _: &[Option<Value>], _: &[bool]) -> ProfileState { ProfileState { hooks:"unsupported",plugin:"unsupported",adapter:"",overrides:None } }

fn check(state: &'static str, detail: &str) -> Check { Check {state,detail:detail.into()} }
fn probe(mut c: Command, args: &[&str], budget: u64) -> Result<Output,crate::process::Failure> {
    c.args(args); crate::quiet(&mut c);
    crate::process::output("agent.ready", &mut c, Duration::from_secs(budget))
}

pub fn claude_json() -> PathBuf {claude::claude_json()}

fn no_diagnosis(_: &Path) -> Value {json!({"state":"absent"})}

pub(super) fn app_adapter() -> &'static Adapter {ENTRIES.into_iter().filter_map(|e|e.builtin).find(|a|a.descriptor.has("app-server")).expect("app-server adapter is registered")}
#[cfg(test)]
pub(super) fn project_dir(root: &str) -> String {claude::project_dir(root)}

/// Presence only: this starts no native process and never writes a profile.
pub fn require_cli(name: &str) -> Result<PathBuf,String> {
    cli_present(entry(name).ok_or_else(||format!("unknown agent {name}"))?)
}

fn validate_descriptor(d: &Descriptor) -> Result<(),String> {
    if [d.name,d.label,d.brand,d.family].iter().any(|s|s.is_empty()) {return Err("incomplete runtime descriptor".into());}
    let mut capabilities=std::collections::BTreeSet::new();
    if d.capabilities.iter().any(|c|c.is_empty() || !capabilities.insert(c)) {return Err("duplicate or empty capability".into());}
    Ok(())
}

fn validate_entries(entries: &[&Entry]) -> Result<(),String> {
    let mut names=std::collections::BTreeSet::new();
    let mut kinds=std::collections::BTreeSet::new();
    let mut aliases=std::collections::BTreeSet::new();
    let mut order=std::collections::BTreeSet::new();
    let mut profiles=std::collections::BTreeSet::new();
    let mut workers=std::collections::BTreeSet::new();
    for e in entries {
        if e.unqualified.is_some() && (e.runtime.is_some() || e.builtin.is_none()) {return Err("unqualified entry must be hidden and retain its descriptor".into());}
        if e.name.is_empty() || e.profile.kind.is_empty() || e.profile.label.is_empty() || e.profile.aliases.is_empty() {return Err("incomplete registry entry".into());}
        if !kinds.insert(e.profile.kind) || !order.insert(e.order) || !profiles.insert(e.profile_order) {return Err("duplicate registry metadata".into());}
        for name in std::iter::once(e.name).chain(e.aliases.iter().copied()) {
            if name.is_empty() || !names.insert(name) {return Err("duplicate registry identity".into());}
        }
        for alias in e.profile.aliases {if alias.is_empty() || !aliases.insert(*alias) {return Err("duplicate profile alias".into());}}
        if let Some(runtime)=e.runtime {
            if !workers.insert(e.worker_order) {return Err("duplicate runtime order".into());}
            let d=runtime.descriptor(); validate_descriptor(d)?;
            if d.name!=e.name {return Err("runtime identity differs".into());}
        }
    }
    if order.iter().copied().ne(0..entries.len()) || profiles.iter().copied().ne(0..entries.len()) {return Err("registry order has a gap".into());}
    if workers.iter().copied().ne(0..workers.len()) {return Err("runtime order has a gap".into());}
    Ok(())
}

pub fn validate() -> Result<(),String> {validate_entries(&ENTRIES)}

#[cfg(test)]
mod examples {
    use super::*;
    include!(concat!(env!("OUT_DIR"),"/registry-examples.rs"));
    #[test]
    fn adding_one_module_registers_a_fake_without_a_core_edit() {
        validate_entries(&ENTRIES).unwrap();
        let e=ENTRIES[0];
        assert_eq!((e.name,e.profile.operations),("example-agent",&["hooks"][..]));
        assert!(cli_present(e).unwrap_err().contains("example-agent"));

        let root=std::env::temp_dir().join(format!("agentq-registry-setup-{}",crate::incident::uuid()));
        assert!(!root.exists());
        let requests=(e.setup)(&root,Path::new(""),Path::new("/agentq"));
        assert_eq!(requests[0].file,root.join("hooks.json"));
        assert!(!root.exists(),"registration only describes edits; it never writes setup");

    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unqualified_structured_entries_are_visible_but_never_offered() {
        for name in ["grok","opencode","opencode2","omp","pi"] {
            let e=entry(name).unwrap();
            assert_eq!(e.qualification().state,"unqualified");
            assert_eq!(e.descriptor().unwrap().name,name);
            assert!(runtime(name).is_none());
            assert!(!runtimes().iter().any(|r|r.descriptor().name==name));
            assert!(native_client(name).is_none(),"native CLI seams cannot bypass qualification");
            for (capability,flow) in [("login-browser",LoginFlow::Browser),("login-device",LoginFlow::Device)] {
                assert!(!e.descriptor().unwrap().has(capability));
                let options=LoginOptions::new(Some(std::env::temp_dir().join(crate::incident::uuid())),flow);
                assert!(matches!(e.builtin.unwrap().login_start(&options),Err(Failure::Refused(Cause("login-flow-unsupported")))));
            }
        }
        validate().unwrap();
    }

    // An isolated candidate must not inherit the enclosing native account or session; caller overrides still win.
    #[test] fn candidate_environment_isolates_native_auth_and_sessions() {
        let root=crate::scratch("candidate-environment");
        for (name,key) in [("grok","XAI_API_KEY"),("opencode","OPENCODE_API_KEY"),("opencode2","AZURE_OPENAI_API_KEY"),("omp","OMP_AUTH_BROKER_TOKEN"),("pi","ANTHROPIC_OAUTH_TOKEN")] {
            let adapter=entry(name).unwrap().builtin.unwrap();
            let mut launch=Launch::new(&root,root.join("unused.jsonl"),"input","fixtureq");launch.command=vec!["fixture-cli".into()];
            for home in [true,false] {
                launch.home=home.then(||root.clone());
                launch.env=if home {vec![]}else{vec![(adapter.ops.home_var.into(),root.clone().into_os_string())]};
                let command=super::super::preflight::command(adapter,&launch).unwrap();
                for var in [key,"XAI_API_KEY","PI_SESSION_ID","PI_SESSION_FILE","PI_CODING_AGENT_SESSION_DIR"] {
                    assert!(command.get_envs().any(|(k,v)|k==var && v.is_none()),"{name}: inherited {var}");
                }
                for var in [key,"PI_SESSION_ID"] {launch.env.push((var.into(),"fixture-explicit".into()));}
                let command=super::super::preflight::command(adapter,&launch).unwrap();
                for var in [key,"PI_SESSION_ID"] {assert!(command.get_envs().any(|(k,v)|k==var && v==Some(std::ffi::OsStr::new("fixture-explicit"))),"{name}: explicit {var}");}
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structured_candidates_share_launch_resume_cancel_events_and_history() {
        let root=crate::scratch("structured-candidates");
        for name in ["grok","opencode","opencode2","omp","pi"] {
            let runtime=entry(name).unwrap().builtin.unwrap();
            let work=root.join(name);std::fs::create_dir_all(&work).unwrap();
            let mut l=Launch::new(&work,work.join("native.jsonl"),"first","fixtureq");
            l.command=vec![std::env::current_exe().unwrap().into_os_string(),"--exact".into(),format!("{}::structured_server",module_path!().split_once("::").unwrap().1).into(),"--nocapture".into(),"--".into()];
            l.env=vec![("AGENTQ_STRUCTURED_FIXTURE".into(),if runtime.ops.rpc.is_some(){"rpc"}else{"acp"}.into())];
            l.home=Some(work.join("home"));std::fs::create_dir_all(l.home.as_ref().unwrap()).unwrap();l.deadline=Some(Instant::now()+Duration::from_secs(20));l.deltas=true;
            l.process.configure=Some(std::sync::Arc::new(|command|assert!(command.get_envs().any(|(k,v)|k=="XAI_API_KEY" && v.is_none()),"native spawn retains account cleaning")));
            let mut scoped=l.clone();scoped.lifetime=Lifetime::StopWithOwner;
            assert_eq!(runtime.launch(&scoped).err().as_ref().map(|e|e.detail.as_str()),Some("lifetime-unsupported: native submission scope"),"{name}: unsupported owner-death policy refuses before spawn");
            assert!(runtime.resume("untracked",&l).err().unwrap().detail.starts_with("session-owner-unknown"));
            let mut blocked=l.clone();blocked.home=Some(work.join("missing-account"));
            let readiness=runtime.preflight(&blocked,&Repair::default());
            assert!(readiness.checks.iter().any(|c|c.check=="account" && c.state=="blocked"));
            assert!(runtime.launch(&blocked).err().unwrap().detail.contains("account-home-missing"));
            let mut run=runtime.launch(&l).unwrap_or_else(|e|panic!("{name}: launch: {e:?}"));
            let native=runtime.session_id(&run).unwrap();
            assert_eq!(run.owner().unwrap().checkpoint().unwrap()["session"],native,"{name}: owner binds before prompt");
            assert!(runtime.resume(&native,&l).err().unwrap().detail.starts_with("session-busy"),"{name}: one admitted writer");
            while runtime.poll(&mut run).unwrap()==State::Running {std::thread::yield_now();}
            let report=runtime.report(&run).unwrap();
            assert_eq!(report.owner.state,"idle","{name}: known native settlement permits finish");
            assert_eq!((report.outcome,report.text.as_deref()),("ok",Some("answer")),"{name}: {report:?}");
            if runtime.ops.rpc.is_some() {
                assert_eq!((report.usage.input,report.usage.calls),(Some(2),Some(1)));
                let ctx=Context{product:"fixtureq",version:"1",project:"other",task:None,route:None,account:"default"};
                assert_eq!(run.snapshot(&ctx).unwrap().events().unwrap().len(),2,"turn and observed model call");
            }
            let events=runtime.progress(&mut run).unwrap();
            assert!(events.iter().any(|e|matches!(e,Event::ToolStart{..})),"{name}");
            assert!(events.iter().any(|e|matches!(e,Event::ToolEnd{..})),"{name}");
            let session=runtime.session_id(&run).unwrap();l.input="wait".into();
            let mut resumed=runtime.resume(&session,&l).unwrap_or_else(|e|panic!("{name}: resume: {e:?}"));
            if runtime.ops.rpc.is_some() {
                while !runtime.progress(&mut resumed).unwrap().iter().any(|e|matches!(e,Event::Metadata{kind,..} if kind=="agent_end")) {assert!(Instant::now()<l.deadline.unwrap());std::thread::yield_now();}
                assert_eq!(runtime.poll(&mut resumed).unwrap(),State::Running,"agent_end is not settlement");
            }
            runtime.cancel(&mut resumed).unwrap();
            while runtime.poll(&mut resumed).unwrap()==State::Running {std::thread::yield_now();}
            assert_eq!(runtime.report(&resumed).unwrap().outcome,"cancelled","{name}");
            let mut history=runtime.history(&session,None,Some(&l.log),None).unwrap();
            let page=history.steps(std::fs::metadata(&l.log).unwrap().len()).unwrap();
            assert!(page.steps.iter().any(|s|s.native_id.as_deref()==Some("tool")),"{name}: canonical history keeps native tools");
            assert!(page.steps.iter().all(|s|s.source["raw"].is_string()));
            if runtime.ops.rpc.is_some() {
                l.input="eof".into();
                let mut lost=runtime.launch(&l).unwrap();
                while runtime.poll(&mut lost).unwrap()==State::Running {std::thread::yield_now();}
                assert_eq!(runtime.poll(&mut lost).unwrap(),State::Unknown);
                assert!(runtime.report(&lost).unwrap_err().contains("stream ended"));
                let stop=loop {if let Some(stop)=runtime.stop_event(&mut lost,u64::MAX).unwrap() {break stop;}std::thread::yield_now();};
                assert_eq!(stop.cause,super::super::super::history::stop::StopCause::ExitWithoutTurnEnd);
                assert_eq!(stop.exit_code,Some(0));assert_ne!(stop.outcome,"ok");
                assert_eq!(runtime.stop_event(&mut lost,u64::MAX).unwrap().unwrap().id,stop.id);
                l.policy.write=Scope::Project;
                assert!(runtime.launch(&l).err().unwrap().detail.contains("policy unsupported"));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn structured_server() {
        let Ok(protocol)=std::env::var("AGENTQ_STRUCTURED_FIXTURE") else {return;};
        let emit=|v:Value|{println!("\n{v}");std::io::stdout().flush().unwrap();};
        let update=|u:Value|emit(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"candidate-session","update":u}}));
        let mut prompt=Value::Null;
        for line in std::io::stdin().lock().lines() {
            let v:Value=serde_json::from_str(&line.unwrap()).unwrap();
            let method=if protocol=="acp"{v["method"].as_str()}else{v["type"].as_str()};
            if protocol=="acp" {
                let result=match method {
                    Some("initialize")=>json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
                    Some("session/new"|"session/load")=>json!({"sessionId":"candidate-session"}),
                    Some("session/prompt")=>{
                        prompt=v["id"].clone();if v["params"]["prompt"][0]["text"]=="wait"{continue;}
                        update(json!({"sessionUpdate":"tool_call","toolCallId":"tool","kind":"execute","title":"fixture"}));
                        update(json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed"}));
                        update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}));json!({"stopReason":"end_turn"})
                    },
                    Some("session/cancel")=>{emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}}));continue;},
                    _=>panic!("unexpected fixture message {v}"),
                };
                emit(json!({"jsonrpc":"2.0","id":v["id"],"result":result}));
            }else{
                match method {
                    Some("get_state")=>{
                        let args:Vec<_>=std::env::args().collect();
                        let session=args.windows(2).find(|a|a[0]=="--session").map(|a|std::path::PathBuf::from(&a[1])).unwrap_or_else(||std::env::current_dir().unwrap().join(format!("session {} with spaces.jsonl",std::process::id())));
                        let mut state=json!({"sessionFile":session,"isStreaming":false,"isCompacting":false,"pendingMessageCount":0});
                        if let Ok(value)=std::env::var("AGENTQ_RPC_PENDING_COUNT") {
                            if value=="missing" {state.as_object_mut().unwrap().remove("pendingMessageCount");}else{state["pendingMessageCount"]=serde_json::from_str(&value).unwrap();}
                        }
                        emit(json!({"type":"response","id":v["id"],"command":"get_state","success":true,"data":state}));
                    },
                    Some("prompt")=>{
                        if std::env::var_os("AGENTQ_RPC_PENDING_COUNT").is_some() {std::fs::write("submitted.marker",b"submitted").unwrap();}
                        emit(json!({"type":"response","id":v["id"],"command":"prompt","success":true}));emit(json!({"type":"agent_start"}));
                        if v["message"]=="eof"{return;}
                        if v["message"]=="wait"{emit(json!({"type":"agent_end"}));continue;}
                        emit(json!({"type":"tool_execution_start","toolCallId":"tool","toolName":"bash","args":{"command":"fixture"}}));
                        emit(json!({"type":"tool_execution_end","toolCallId":"tool","toolName":"bash","result":{"content":[]},"isError":false}));
                        emit(json!({"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"answer"}],"stopReason":"stop","usage":{"input":2,"output":1,"cacheRead":0,"cacheWrite":0}}}));
                        emit(json!({"type":"agent_end"}));emit(json!({"type":"agent_settled"}));
                    },
                    Some("abort")=>{
                        emit(json!({"type":"response","id":v["id"],"command":"abort","success":true}));
                        emit(json!({"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"aborted"}}));emit(json!({"type":"agent_end"}));emit(json!({"type":"agent_settled"}));
                    },
                    _=>panic!("unexpected fixture message {v}"),
                }
            }
        }
    }
    #[test]
    fn registered_entries_have_complete_metadata() {
        validate().unwrap();
        let broken=Entry {name:"",..*ENTRIES[0]};
        assert!(validate_entries(&[&broken]).is_err());
        let descriptor=Descriptor {name:"broken",label:"Broken",brand:"Test",family:"test",minimum:"",models:&[],capabilities:&["turn","turn"]};
        assert!(validate_descriptor(&descriptor).unwrap_err().contains("capability"));
        assert!(validate_entries(&[ENTRIES[0],ENTRIES[0]]).is_err());
    }
}

pub(crate) fn native_client(name: &str) -> Option<Client> {let e=entry(name)?; if e.unqualified.is_some() {return None;} e.builtin.map(|a|a.client)}

pub(crate) fn native_home_var(client: Client) -> &'static str {builtin(client).ops.home_var}
pub(crate) fn alternate_home_var(client: Client) -> Option<&'static str> {builtin(client).ops.alternate_home_var}
pub(crate) fn native_home(client: Client) -> Option<PathBuf> {(builtin(client).ops.home)()}
pub(crate) fn native_executable(client: Client,path: &OsStr) -> Res<PathBuf> {(builtin(client).ops.executable)(path)}
pub(crate) fn native_session(client: Client) -> String {(builtin(client).ops.new_session)()}
pub(crate) fn native_args(spec: &Spec) -> Vec<OsString> {(builtin(spec.client).ops.args)(spec)}
pub(crate) fn session_vars() -> Vec<&'static str> {ENTRIES.into_iter().filter_map(|e|e.builtin).flat_map(|a|a.ops.session_vars.iter().copied()).collect()}
pub(crate) use codex::codex_native;

pub fn legacy_provider(agent: &str,root: &Path) -> Result<Provider,String> {
    entry(agent).and_then(|e|e.provider).ok_or("no legacy provider")?(root)
}

#[cfg(test)]
pub(super) fn with_test_path<T>(path: &Path,f: impl FnOnce()->T) -> T {
    struct Restore(Option<OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {unsafe {match &self.0 {Some(v)=>std::env::set_var("PATH",v),None=>std::env::remove_var("PATH")}}}
    }
    // .cargo/config.toml serializes the tests; restore even if a probe panics.
    let _restore=Restore(std::env::var_os("PATH"));
    unsafe {std::env::set_var("PATH",path)}
    f()
}

pub(crate) struct Protocol {
    pub stream_history: &'static str,
    pub native_history: &'static str,
    pub history_provenance:fn(&str,&Value,u64)->bool,
    pub history_parent:fn(&Value)->Option<String>,
    pub rollout_report:fn(Option<&Path>,&mut Report),
    pub history_alias: bool,
    pub history_formats: &'static [&'static str],
    pub history_session:fn(&str,&Value,&str)->Option<String>,
    pub history_normalize:fn(&str,&Value)->super::super::history::Normalized,
    pub history_validity:fn(&Value)->Value,
    pub ends:fn(&Value)->bool,
    pub start_limit:fn(&Value)->Option<StartLimit>,
    pub read_calls:fn(&[Value])->(Vec<Call>,Option<u64>,bool),
    pub history_usage:fn(&[Value])->Vec<super::super::history::live::UsageRevision>,
    pub history_context:fn(&Value)->Option<(Option<u64>,Option<u64>)>,
    pub final_text:fn(&[Value])->Option<String>,
    pub prepare_command:fn(Command,&Spec,&str)->Res<(Command,Box<dyn TurnExtension>)>,
    pub input_bytes:fn(&str)->Vec<u8>,
    pub native_record:fn(&mut native::NativeTurn,&Value)->Res<()>,
    pub native_metadata:Option<fn(&native::NativeTurn,&Value)->Res<()>>,
    pub finish_native_report:fn(&native::NativeTurn,&mut Report),
    pub session_report:fn(&str,&mut Report),
    pub reported:fn(&Value,u64)->Option<Vec<Window>>,
    pub update_limits:fn(&Value,u64,&mut BTreeMap<String,Vec<Window>>)->bool,
    pub account_limits:fn(Option<&Path>,Option<&Path>,i64)->Vec<Window>,
    pub identity:fn(Option<&Path>)->Option<String>,
    pub credentials:fn(&Path,bool)->Res<String>,
    pub oauth_token:fn(&Value,i64)->Res<&str>,
    pub oauth_usage:fn(&str)->Res<Vec<Window>>,
    pub turn_ended:fn(bool,&Value)->bool,
    pub parse_event: fn(&mut events::Cursor,&Value,&mut Vec<Event>,Option<&str>),
    pub terminal: fn(&events::Cursor,&Value,&mut Vec<Event>,Option<&str>,Option<&str>)->Option<&'static str>,
    pub assistant_failure: fn(&Value)->Option<events::AssistantFailure>,
    pub permission_mode: fn(&Value)->Option<String>,
    pub lifecycle: fn(&Value)->Option<String>,
    pub advances: fn(&Value)->bool,
    pub session: fn(&str)->Option<String>,
    pub snapshot: fn(&[Value],Option<&Path>)->Snapshot,
    pub report: fn(&str,&[Value],Option<&Path>)->Report,
}
pub(super) fn structured_session(runtime:&str)->bool {entry(runtime).and_then(|e|e.builtin).is_some_and(|a|a.ops.acp.is_some() || a.ops.rpc.is_some())}
pub(crate) fn protocol(client:Client)->&'static Protocol {builtin(client).ops.protocol}
#[cfg(test)]
pub(crate) use antigravity::session as antigravity_session;


#[cfg(test)]
pub(crate) use codex::codex_identity;
#[cfg(test)]
pub(crate) use antigravity::{code_assist,project_identity};

pub(crate) use codex::app;
pub(crate) use codex::{image as codex_image,catalogue_page as codex_catalogue_page};
pub(in crate::agent) use codex::image_kind as codex_image_kind;
pub(in crate::agent) use antigravity::{input_state as antigravity_input_state,retained_source as antigravity_retained_source};

pub(super) use claude::control::Session as ClaudeControl;
pub use claude::{BackgroundJob,BackgroundCandidate,BackgroundKind,BackgroundState,BackgroundActivity,BackgroundPid};
pub(crate) use claude::{background_candidates,background_home_identity};
pub(crate) use claude::{remote_url,claude_link,Absence};

#[cfg(test)]
pub(crate) use claude::{claude_window,claude_windows};

pub(crate) fn native_transcript(client:Client,session:&str,home:Option<&Path>)->Option<PathBuf> {(builtin(client).ops.transcript)(session,home)}

#[path="shared/parsing.rs"]
pub(crate) mod parsing;
#[cfg(test)]
pub(crate) use claude::claude_settings;

#[cfg(test)]
pub(crate) use codex::rollout_calls as codex_rollout_calls;

#[cfg(test)]
pub(crate) use codex::parsing::auth_rejected as codex_auth_rejected;

pub(crate) fn history_runtime(format:&str)->Option<&'static dyn Runtime> {
    let format=crate::agent::history::interpretation(format);
    native_entries().filter_map(|e|e.builtin).find(|a|a.ops.protocol.history_formats.contains(&format)).map(|a|a as &dyn Runtime)
}

pub(crate) fn history_protocol(format:&str)->Option<&'static Protocol> {
    let format=crate::agent::history::interpretation(format);
    native_entries().filter_map(|e|e.builtin).map(|a|a.ops.protocol).find(|p|p.history_formats.contains(&format))
}

/// Restore only an unchanged, journal-owned native preparation value; native CAS holds the client's config lock.
pub fn restore_preparation(entry: &Value, dry: bool) -> Res<()> {
    let adapter=entry["agent"].as_str().and_then(self::entry).and_then(|e|e.builtin).ok_or("preparation runtime unknown")?;
    adapter.ops.restore_preparation.ok_or("preparation restore unsupported")?(entry,dry)
}

/// Inspect or explicitly prepare only the registered runtime's native hook trust.
pub fn hook_trust(agent: &str, home: &Path, journal: Option<&Path>, repair: bool) -> Option<LaunchCheck> {
    let adapter=entry(agent)?.builtin?;
    if adapter.ops.hook_status.is_none() {return None;}
    let root=match std::env::current_dir() {Ok(root)=>root,Err(_)=>return Some(LaunchCheck::new("evidence","degraded",Some("hooks-unknown"),"Working directory unavailable","select an existing working directory"))};
    let mut launch=Launch::new(root,std::env::temp_dir().join("unused-hooks.log"),"","agentq");launch.home=Some(home.into());
    let repair=Repair {workspace:false,hooks:repair,journal:journal.map(Path::to_path_buf)};
    Some((adapter.ops.preparation)(&launch,&repair).1)
}

fn secret_name(name:&str)->bool {
    let name=name.to_ascii_uppercase();
    name.bytes().all(|b|b.is_ascii_alphanumeric() || b"_.-".contains(&b)) && ["API_KEY","TOKEN","SECRET","PASSWORD","CREDENTIAL"].iter().any(|s|name==*s || name.ends_with(&format!("_{s}")) || name.ends_with(&format!(".{s}")))
}
pub(super) fn validate_arguments(args:&[OsString])->Res<()> {
    if args.iter().filter_map(|s|s.to_str()).any(|a|a.split_once('=').is_some_and(|(k,v)|!v.is_empty() && secret_name(k)) || ["--api-key","--token","--password","--secret"].contains(&a)) {return Err("secret-in-argv".into());}
    Ok(())
}
pub(crate) fn validate_native_args(client:Client,args:&[OsString])->Res<()> {(builtin(client).ops.validate)(args,&[],&Policy::default())}

// Provider auth inventory: Pi env-api-keys.ts; OMP v17.0.5 environment-variables.md; OpenCode v1.18.31 provider.ts.
const STRUCTURED_AUTH_VARS:&[&str]=&[
    "AICORE_SERVICE_KEY","AIMLAPI_API_KEY","AI_GATEWAY_API_KEY","ALIBABA_CODING_PLAN_API_KEY",
    "ANTHROPIC_API_KEY","ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_CUSTOM_HEADERS","ANTHROPIC_FOUNDRY_API_KEY","ANTHROPIC_IDENTITY_TOKEN_FILE","ANTHROPIC_OAUTH_TOKEN",
    "ANT_LING_API_KEY","AWS_ACCESS_KEY_ID","AWS_BEARER_TOKEN_BEDROCK","AWS_CONFIG_FILE",
    "AWS_CONTAINER_AUTHORIZATION_TOKEN","AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE","AWS_CONTAINER_CREDENTIALS_FULL_URI","AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_DEFAULT_PROFILE","AWS_PROFILE","AWS_ROLE_ARN","AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN","AWS_SHARED_CREDENTIALS_FILE","AWS_WEB_IDENTITY_TOKEN_FILE","AZURE_OPENAI_API_KEY",
    "BASETEN_API_KEY","CEREBRAS_API_KEY","CF_AIG_TOKEN","CLAUDE_CODE_CLIENT_CERT",
    "CLAUDE_CODE_CLIENT_KEY","CLOUDFLARE_AI_GATEWAY_API_KEY","CLOUDFLARE_API_KEY","CLOUDFLARE_API_TOKEN",
    "COPILOT_GITHUB_TOKEN","CURSOR_ACCESS_TOKEN","DEEPSEEK_API_KEY","FIREPASS_API_KEY",
    "FIREWORKS_API_KEY","GEMINI_API_KEY","GITLAB_TOKEN","GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS","GOOGLE_CLOUD_API_KEY","GROQ_API_KEY","HF_TOKEN",
    "HUGGINGFACE_HUB_TOKEN","KILO_API_KEY","KIMI_API_KEY","LITELLM_API_KEY",
    "LLAMA_CPP_API_KEY","LM_STUDIO_API_KEY","META_API_KEY","MINIMAX_API_KEY",
    "MINIMAX_CN_API_KEY","MINIMAX_CODE_API_KEY","MINIMAX_CODE_CN_API_KEY","MISTRAL_API_KEY",
    "MOONSHOT_API_KEY","NANO_GPT_API_KEY","NOVITA_API_KEY","NVIDIA_API_KEY",
    "OLLAMA_API_KEY","OLLAMA_CLOUD_API_KEY","OMP_AUTH_BROKER_TOKEN","OMP_AUTH_BROKER_URL",
    "OPENAI_API_KEY","OPENAI_CODEX_OAUTH_TOKEN","OPENCODE_API_KEY","OPENROUTER_API_KEY",
    "QIANFAN_API_KEY","QWEN_OAUTH_TOKEN","QWEN_PORTAL_API_KEY","QWEN_TOKEN_PLAN_API_KEY",
    "QWEN_TOKEN_PLAN_CN_API_KEY","RADIUS_API_KEY","SNOWFLAKE_CORTEX_PAT","SNOWFLAKE_CORTEX_TOKEN",
    "SYNTHETIC_API_KEY","TOGETHER_API_KEY","TYPESAFE_API_KEY","UMANS_AI_CODING_PLAN_API_KEY",
    "VENICE_API_KEY","VLLM_API_KEY","WAFER_SERVERLESS_API_KEY","XAI_API_KEY",
    "XAI_OAUTH_TOKEN","XIAOMI_API_KEY","XIAOMI_TOKEN_PLAN_AMS_API_KEY","XIAOMI_TOKEN_PLAN_CN_API_KEY",
    "XIAOMI_TOKEN_PLAN_SGP_API_KEY","ZAI_API_KEY","ZAI_CODING_CN_API_KEY","ZENMUX_API_KEY",
    "ZHIPU_API_KEY",
];

pub(crate) fn auth_vars()->Vec<&'static str> {ENTRIES.into_iter().filter_map(|e|e.builtin).flat_map(|a|a.ops.auth_vars.iter().copied()).collect()}

pub(super) use codex::{codex_read_command,exchange,credential_snapshot};

/// Passive profile/service inspection never starts a native process.
pub fn hook_status(agent:&str,home:&Path,journal:Option<&Path>)->Option<LaunchCheck> {Some((entry(agent)?.builtin?.ops.hook_status?)(home,journal))}

pub(crate) use claude::start_failure as claude_start_failure;

pub(crate) use claude::launch_exchange as claude_launch_exchange;
pub(crate) use codex::parsing::cli_refusal as codex_cli_refusal;
