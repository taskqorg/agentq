//! Worked fake runtime: this file is the entire registration, discovered like production entries.
use super::*;

pub(super) const ENTRY: Entry = Entry {
    name:"example-agent",aliases:&["example"],order:0,profile_order:0,worker_order:0,onboard:true,
    profile:Profile {kind:"example-agent",label:"Example Agent",aliases:&["example-client"],operations:&["hooks"]},
    runtime:Some(&FAKE),builtin:None,root:home,files,setup,inspect:no_inspection,hook:no_hook,hook_key:"hooks",fallback:empty,executable:cli,diagnose:no_diagnosis,provider:None,
};
fn home() -> PathBuf {std::env::temp_dir().join(format!("agentq-registry-example-{}",std::process::id()))}
fn files(root: &Path) -> Vec<PathBuf> {vec![root.join("hooks.json")]}
fn setup(root: &Path, _: &Path, exe: &Path) -> Vec<Edit> {
    vec![edit("example-agent",root.join("hooks.json"),&["hooks","Stop"],json!({"hooks":[hook_command(exe,"example-agent","Stop")]}),true)]
}
fn cli() -> Option<PathBuf> {None}

struct Fake;
static FAKE: Fake=Fake;
static DESCRIPTOR: Descriptor=Descriptor {name:"example-agent",label:"Example Agent",brand:"Example",family:"example",models:&[],minimum:"",capabilities:&["turn"]};

#[allow(unused_variables)]
impl Runtime for Fake {
    fn descriptor(&self) -> &Descriptor { &DESCRIPTOR }
    fn launch(&self, launch: &Launch) -> Res<Run> { Err("example: native operation unsupported".into()) }
    fn resume(&self, session: &str, launch: &Launch) -> Res<Run> { Err("example: native operation unsupported".into()) }
    fn input(&self, run: &mut Run, parts: &[Part]) -> Result<(), Failure> { Err(Failure::Refused(Cause("unsupported"))) }
    fn account(&self, run: &mut Run) -> Result<Account, Failure> { Err(Failure::Refused(Cause("unsupported"))) }
    fn catalogue_with_executable(&self, home: Option<&Path>, executable: Option<&Path>) -> Res<Vec<Model>> { Err("example: native operation unsupported".into()) }
    fn log_limits(&self, log: &str, session: Option<&str>, home: Option<&Path>) -> Vec<Window> { vec![] }
    fn account_limits(&self, home: Option<&Path>, now: i64) -> Vec<Window> { vec![] }
    fn log_events(&self, log: &str) -> Vec<Event> { vec![] }
    fn log_records(&self, log: &str) -> Vec<LogRecord> { vec![] }
    fn account_identity(&self, home: Option<&Path>) -> Option<String> { None }
    fn cancel(&self, run: &mut Run) -> Res<bool> { Err("example: native operation unsupported".into()) }
    fn poll(&self, run: &mut Run) -> Res<State> { Err("example: native operation unsupported".into()) }
    fn report(&self, run: &Run) -> Res<Report> { Err("example: native operation unsupported".into()) }
    fn progress(&self, run: &mut Run) -> Res<Vec<Event>> { Err("example: native operation unsupported".into()) }
    fn launcher_output(&self, run: &mut Run) -> Result<Result<Output, crate::process::Failure>, Cause> { Err(Cause("unsupported")) }
    fn session_id(&self, run: &Run) -> Option<String> { None }
    fn session_url(&self, run: &Run) -> Result<String, Cause> { Err(Cause("unsupported")) }
    fn observe(&self, run: &mut Run) -> Observation { Observation {state:State::Unknown,error:Some("example".into()),session:None,url:None,cause:Some(Cause("unsupported")),detail:None} }
    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause> { Err(Cause("unsupported")) }
    fn ready(&self, executable: Option<&Path>, home: Option<&Path>) -> Ready { Ready {install:check("missing","example has no CLI"),version:check("unknown","example"),login:check("unknown","example")} }
    fn quota(&self, accounts: &Accounts, now: i64) -> Quota { Quota {account:None,free:false,off:false,hold:None} }
    fn running(&self) -> usize { 0 }
    fn protected_paths(&self, home: Option<&Path>) -> Vec<PathBuf> { vec![] }
    fn home_env(&self, tool: &str, label: &str) -> Vec<(OsString, OsString)> { vec![] }
    fn checkpoint(&self, run: &Run) -> Result<Value, Cause> { Err(Cause("unsupported")) }
    fn recover(&self, saved: &Value, launch: &Launch) -> Res<Run> { Err("example: native operation unsupported".into()) }
    fn classify(&self, log: &str) -> Report { Report {session:None,usage:Usage::default(),limit:Snapshot {unknown:Some("example"),..Snapshot::default()},limits:vec![],reached:None,background_job:false,live_background:vec![],outcome:"unknown",error:"example",detail:None,text:None,exit:None,started_ms:0,duration_us:0,recorded:Err("example".into()),calls:vec![],process:ProcessUse::default()} }
    fn executable(&self, path: Option<&OsStr>) -> Res<PathBuf> { Err("example: native operation unsupported".into()) }
    fn home(&self) -> Option<PathBuf> { None }
    fn session_transcript(&self, session: &str, home: Option<&Path>) -> Option<PathBuf> { None }
    fn session_link(&self, spec: &NativeSpec, job: Option<&str>) -> Result<String, (Cause, Option<String>)> { Err((Cause("unsupported"),None)) }
    fn prepare(&self, intent: Intent, product: &str, spec: &mut NativeSpec, install: Option<&Path>) -> Optimization { Optimization {intent,effective:"off",reason:"optimization_retired"} }
    fn command(&self, base: Command, spec: &NativeSpec) -> Res<Command> { Err("example: native operation unsupported".into()) }
    fn turn(&self, base: Command, spec: &NativeSpec, log: &Path, input: &str, process: &Process) -> Res<Turn> { Err("example: native operation unsupported".into()) }
    fn background_launch(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Result<Output, crate::process::Failure>> { Err("example: native operation unsupported".into()) }
    fn jobs(&self, base: Command, spec: &NativeSpec, filtered: bool, budget: Duration) -> Res<Vec<Value>> { Err("example: native operation unsupported".into()) }
    fn job(&self, row: &Value, spec: &NativeSpec, id: &str) -> Res<BackgroundJob> { Err("example: native operation unsupported".into()) }
    fn stop_job(&self, base: Command, spec: &NativeSpec, job: &BackgroundJob, budget: Duration) -> Res<Output> { Err("example: native operation unsupported".into()) }
    fn launch_refusal(&self, text: &str) -> Option<Cause> { None }
    fn app_queue(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Output> { Err("example: native operation unsupported".into()) }
    fn app_server_command(&self, base: Command, spec: &NativeSpec) -> Res<Command> { Err("example: native operation unsupported".into()) }
}
