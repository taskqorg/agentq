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
    pub routing: Option<String>,
    pub agent: String,
    pub instruction: bool,
}

impl Edit {
    pub fn field(&self) -> &'static str {
        if self.instruction { return "instructions"; }
        match (&self.routing, self.key[0].as_str()) {
            (Some(_), _) => "routing",
            (None, "hooks" | "agentq") => "hooks",
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
pub fn entry(name: &str) -> Option<&'static Entry> { ENTRIES.into_iter().find(|e| e.name == name || e.aliases.contains(&name)) }
pub fn profile(kind: &str) -> Option<&'static Entry> { ENTRIES.into_iter().find(|e| e.profile.kind == kind) }
pub(super) fn builtin(client: Client) -> &'static Adapter { ENTRIES.into_iter().filter_map(|e| e.builtin).find(|a| a.client == client).expect("native adapter is registered") }

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
    Edit { file, key:key.iter().map(|k| k.to_string()).collect(), value, append, routing:None, agent:agent.into(), instruction:false }
}
fn hook_command(exe: &Path, agent: &str, event: &str) -> Value { json!({"type":"command","command":format!("\"{}\" hook {agent} {event}",exe.display()),"timeout":5}) }
#[cfg(test)]
fn empty(_: &str) -> Value { json!({}) }
#[cfg(test)]
fn no_hook(_: &str) -> Hook { Hook::None }
fn no_inspection(_: &[Option<String>], _: &[Option<Value>], _: &[bool]) -> ProfileState { ProfileState { hooks:"unsupported",plugin:"unsupported",adapter:"",overrides:None } }

fn check(state: &'static str, detail: &str) -> Check { Check {state,detail:detail.into()} }
fn probe(exe: &Path, args: &[&str], home: Option<&Path>, home_var: &str, budget: u64) -> Result<Output,crate::process::Failure> {
    let mut c=Command::new(exe); c.args(args); if let Some(home)=home {c.env(home_var,home);} crate::quiet(&mut c);
    crate::process::output("agent.ready",&mut c,Duration::from_secs(budget))
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
    for capability in d.capabilities {
        if !matches!(*capability,"turn"|"resume"|"caller-session-id"|"compact-hook"|"session-url"|"quota-snapshot"|"account-limits"|"background"|"title"|"mcp-config"|"instructions-file"|"turn-parts"|"turn-deltas"|"settings-env"|"catalogue"|"app-queue"|"app-server"|"effort"|"image-input"|"account"|"host-args") {
            return Err(format!("capability {capability} has no Runtime method"));
        }
    }
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
        let launch=Launch::new(std::env::temp_dir(),std::env::temp_dir().join("example.log"),"example","input");
        assert!(e.runtime.unwrap().launch(&launch).is_err(),"the fake never starts native work");
        let root=(e.root)();
        let requests=(e.setup)(&root,Path::new(""),Path::new("/agentq"));
        assert_eq!(requests[0].file,root.join("hooks.json"));
        assert!(!root.exists(),"registration and discovery never write setup");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_entries_are_complete_and_capabilities_have_methods() {
        validate().unwrap();
        let broken=Entry {name:"",..*ENTRIES[0]};
        assert!(validate_entries(&[&broken]).is_err());
        let descriptor=Descriptor {name:"broken",label:"Broken",brand:"Test",family:"test",minimum:"",models:&[],capabilities:&["missing-method"]};
        assert!(validate_descriptor(&descriptor).unwrap_err().contains("missing-method"));
        assert!(validate_entries(&[ENTRIES[0],ENTRIES[0]]).is_err());
    }
}

pub(crate) fn native_client(name: &str) -> Option<Client> {entry(name)?.builtin.map(|a|a.client)}
pub(crate) fn native_name(client: Client) -> &'static str {builtin(client).descriptor.name}
pub(crate) fn native_home_var(client: Client) -> &'static str {builtin(client).ops.home_var}
pub(crate) fn alternate_home_var(client: Client) -> Option<&'static str> {builtin(client).ops.alternate_home_var}
pub(crate) fn native_home(client: Client) -> Option<PathBuf> {(builtin(client).ops.home)()}
pub(crate) fn native_executable(client: Client,path: &OsStr) -> Res<PathBuf> {(builtin(client).ops.executable)(path)}
pub(crate) fn native_session(client: Client) -> String {(builtin(client).ops.new_session)()}
pub(crate) fn native_args(spec: &Spec) -> Vec<OsString> {(builtin(spec.client).ops.args)(spec)}
pub(crate) fn session_vars() -> Vec<&'static str> {ENTRIES.into_iter().filter_map(|e|e.builtin.map(|a|a.ops.session_var)).collect()}
pub(crate) use codex::codex_native;

pub fn legacy_provider(agent: &str,root: &Path) -> Result<Provider,String> {
    entry(agent).and_then(|e|e.provider).ok_or("no legacy provider")?(root)
}
