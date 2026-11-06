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
    fn registered_entries_have_complete_metadata() {
        validate().unwrap();
        let broken=Entry {name:"",..*ENTRIES[0]};
        assert!(validate_entries(&[&broken]).is_err());
        let descriptor=Descriptor {name:"broken",label:"Broken",brand:"Test",family:"test",minimum:"",models:&[],capabilities:&["turn","turn"]};
        assert!(validate_descriptor(&descriptor).unwrap_err().contains("capability"));
        assert!(validate_entries(&[ENTRIES[0],ENTRIES[0]]).is_err());
    }
}

pub(crate) fn native_client(name: &str) -> Option<Client> {entry(name)?.builtin.map(|a|a.client)}

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
pub(crate) fn protocol(client:Client)->&'static Protocol {builtin(client).ops.protocol}
#[cfg(test)]
pub(crate) use antigravity::session as antigravity_session;


#[cfg(test)]
pub(crate) use codex::codex_identity;
#[cfg(test)]
pub(crate) use antigravity::{code_assist,project_identity};

pub(crate) use codex::app;

pub use claude::BackgroundJob;
pub(crate) use claude::{remote_url,claude_link,Absence};
pub(crate) use codex::append_codex_usage;

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

pub(crate) fn history_protocol(format:&str)->Option<&'static Protocol> {
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
fn validate_arguments(args:&[OsString])->Res<()> {
    if args.iter().filter_map(|s|s.to_str()).any(|a|a.split_once('=').is_some_and(|(k,v)|!v.is_empty() && secret_name(k)) || ["--api-key","--token","--password","--secret"].contains(&a)) {return Err("secret-in-argv".into());}
    Ok(())
}
pub(crate) fn validate_native_args(client:Client,args:&[OsString])->Res<()> {(builtin(client).ops.validate)(args,&[],&Policy::default())}

pub(crate) fn auth_vars()->Vec<&'static str> {ENTRIES.into_iter().filter_map(|e|e.builtin).flat_map(|a|a.ops.auth_vars.iter().copied()).collect()}

pub(super) use codex::{codex_read_command,exchange,credential_snapshot};

/// Passive profile/service inspection never starts a native process.
pub fn hook_status(agent:&str,home:&Path,journal:Option<&Path>)->Option<LaunchCheck> {Some((entry(agent)?.builtin?.ops.hook_status?)(home,journal))}
