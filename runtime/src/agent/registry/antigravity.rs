//! Antigravity registration: native worker and installation behavior.
use super::*;

static ADAPTER: Adapter = Adapter {client: Client::Antigravity, live: AtomicUsize::new(0), descriptor: Descriptor { name: "antigravity", label: "Antigravity", brand: "Google", family: "gemini", models: &[], minimum: "",
        capabilities: &["turn", "resume", "title", "account-limits"] } , ops: NativeOps { home_var:"ANTIGRAVITY_HOME",alternate_home_var:Some("ANTIGRAVITY_APP_DATA_DIR"),session_var:"ANTIGRAVITY_CONVERSATION_ID",home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, protected, refusal, transcript, background_transcript, link, session_link } };

pub(super) const ENTRY: Entry = Entry { name:"antigravity", aliases:&["agy"], order:4, profile_order:4, worker_order:2, onboard:true,
    profile:Profile {kind:"antigravity",label:"Antigravity",aliases:&["agy"],operations:&["hooks"]},
    runtime:Some(&ADAPTER), builtin:Some(&ADAPTER),
    root:config_root,files,setup,inspect,hook,fallback,executable:cli,diagnose:no_diagnosis,provider:None,
};

fn mapped(l: &Launch) -> Res<Vec<OsString>> {
    let name = "antigravity";
    let p = &l.policy;
    let refuse = |part: &str| Err(format!("policy {part} unsupported by {name}"));
    let mut a: Vec<OsString> = vec![];

            for (part, native) in [("read", p.read == Scope::Native), ("write", p.write == Scope::Native), ("network", p.network == Network::Native), ("tools", p.tools.is_none()),
                ("roots", p.roots == Roots::Native), ("unelevated", !p.unelevated), ("builtin", p.builtin == Builtin::Native), ("isolated", !p.isolated),
                ("subscription", !p.subscription)] {
                if !native { return refuse(part); }
            }
            match p.approval { Approval::Native => {}, Approval::Bypass => a.push("--dangerously-skip-permissions".into()), _ => return refuse("approval") }
    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        a.push(format!("--model={m}").into());
    }
    if l.effort.is_some() { return Err(format!("effort unsupported by {name}")); }
    if let Some(t) = &l.title { a.push(format!("--title={t}").into()); }
    if l.compact_hook.is_some() { return Err(format!("compact-hook unsupported by {name}")); }
    Ok(a)
}
fn extras(_: &Policy) -> Vec<OsString> { vec![] }
fn env(l: &Launch, spec: &mut Spec) { spec.env.extend(l.settings_env.iter().map(|(k,v)| (k.into(),v.into()))); }
fn settings(_: &Launch, _: &mut Spec) {}
fn login(_: &Path, _: Option<&Path>) -> Check {check("unknown","no login status command")}
fn protected(home: PathBuf) -> Vec<PathBuf> { let mut v=vec![home.clone()]; if let Some(p)=home.parent() {v.push(p.join("config"));} v }
fn refusal(_: &str) -> Option<Cause> { None }
fn transcript(_: &str, _: Option<&Path>) -> Option<PathBuf> { None }
fn link(_: &Run) -> Result<String,native::Absence> { Err((Cause("no-session-url"),None)) }
fn session_link(_: &NativeSpec, _: Option<&str>) -> Result<String,native::Absence> { Err((Cause("no-session-url"),None)) }
fn config_root() -> PathBuf { base_home(Client::Antigravity).and_then(|h|h.parent().map(Path::to_path_buf)).unwrap_or_else(||host_home().join(".gemini")) }
fn files(root: &Path) -> Vec<PathBuf> { vec![root.join("config/hooks.json")] }
fn setup(root: &Path, _: &Path, exe: &Path) -> Vec<Edit> {
    vec![edit("antigravity",root.join("config/hooks.json"),&["agentq","enabled"],json!(true),false),
        edit("antigravity",root.join("config/hooks.json"),&["agentq","Stop"],hook_command(exe,"antigravity","Stop"),true)]
}
fn inspect(_: &[Option<String>], docs: &[Option<Value>], _: &[bool]) -> ProfileState {
    let hooks=docs[0].as_ref().map_or("unknown",|d|hook_state([d["agentq"]["enabled"]!=false && d["agentq"]["Stop"].as_array().into_iter().flatten()
        .filter(|m|m["matcher"].as_str().is_none_or(|m|matches!(m,""|"*"|"run_command")))
        .any(|h|h["command"].as_str().is_some_and(|c|c.ends_with(" hook antigravity Stop")))].into_iter()));
    ProfileState {hooks,plugin:"unsupported",adapter:"hook",overrides:None}
}
fn hook(event: &str) -> Hook {if event=="Stop" {Hook::Stop {model:false}} else {Hook::None}}
fn fallback(event: &str) -> Value {if matches!(event,"PreToolUse"|"Stop") {json!({"decision":"allow"})} else {json!({})}}
fn cli() -> Option<PathBuf> {executable(Client::Antigravity).ok()}
fn background_transcript(_: &Spec, _: &str) -> Res<PathBuf> {Err("background unsupported".into())}

fn antigravity_executable(path: &OsStr) -> PathBuf {
    if let Some(path) = std::env::var_os("ANTIGRAVITY_AGENTAPI_EXE").filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    let path = path.to_os_string();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let path = user_bin_path(&path, home.as_deref()).unwrap_or(path);
    let cli = which_in("agy", &path);
    if cli.is_absolute() { return cli; }
    let api = which_in("agentapi", &path);
    if api.is_absolute() { return api; }
    if let Some(home) = base_home(Client::Antigravity) {
        let p = home.join("bin/agentapi");
        if p.is_file() {
            return p;
        }
    }
    which_in("agy", &path)
}

fn native_home() -> Option<PathBuf> {
    std::env::var_os("ANTIGRAVITY_HOME").filter(|v|!v.is_empty()).or_else(||std::env::var_os("ANTIGRAVITY_APP_DATA_DIR").filter(|v|!v.is_empty())).map(PathBuf::from)
        .or_else(||std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")).map(|h|{let cli=Path::new(&h).join(".gemini/antigravity-cli");if cli.is_dir() {cli} else {Path::new(&h).join(".gemini/antigravity")}}))
}
fn native_executable(path: &OsStr) -> Res<PathBuf> {Ok(antigravity_executable(path))}
fn new_session() -> String {"-".into()}
fn native_args(spec: &Spec) -> Vec<OsString> {
    let mut a=vec![];let s=|v: &str|OsString::from(v);

            a.extend(["--input-format", "stream-json", "--output-format", "stream-json"].map(s));
            a.extend(spec.args.iter().cloned());
            if spec.resume { a.extend([s("--conversation"), s(&spec.session)]); }
    a
}
