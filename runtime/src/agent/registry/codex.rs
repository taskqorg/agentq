//! Codex registration: native worker and installation behavior.
use super::*;

static ADAPTER: Adapter = Adapter {client: Client::Codex, live: AtomicUsize::new(0), descriptor: Descriptor { name: "codex", label: "Codex CLI", brand: "OpenAI", family: "gpt", models: &[], minimum: "0.162.0",
        capabilities: &["turn", "resume", "quota-snapshot", "account-limits", "catalogue", "app-queue", "app-server", "effort", "image-input", "account", "host-args", "settings-env", "mcp-config", "instructions-file", "turn-parts", "turn-deltas"] } , ops: NativeOps { home_var:"CODEX_HOME",alternate_home_var:None,session_var:"CODEX_THREAD_ID",home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, protected, refusal, transcript, background_transcript, link, session_link } };

pub(super) const ENTRY: Entry = Entry { name:"codex", aliases:&[], order:0, profile_order:0, worker_order:1, onboard:true,
    profile:Profile {kind:"codex",label:"Codex",aliases:&["codex-cli", "codex-desktop"],operations:&["hooks"]},
    runtime:Some(&ADAPTER), builtin:Some(&ADAPTER),
    root:config_root,files,setup,inspect,hook,hook_key:"hooks",fallback,executable:cli,diagnose:diagnose,provider:Some(provider),
};

fn config_root() -> PathBuf { root("CODEX_HOME", ".codex") }
fn mapped(l: &Launch) -> Res<Vec<OsString>> {
    let name = "codex";
    let p = &l.policy;
    let refuse = |part: &str| Err(format!("policy {part} unsupported by {name}"));
    let mut a: Vec<OsString> = vec![];

            if !matches!(p.read, Scope::Native | Scope::Any) { return refuse("read"); }
            if p.tools.is_some() { return refuse("tools"); }
            match (p.network, p.write) {
                (Network::Native, _) | (Network::On, Scope::Project) | (Network::Off, Scope::None | Scope::Project) => {}
                _ => return refuse("network"),
            }
            let c = |a: &mut Vec<OsString>, v: &str| a.extend(["-c".into(), v.into()]);
            match (p.approval, p.write) {
                (Approval::Bypass, Scope::Any) => a.push("--dangerously-bypass-approvals-and-sandbox".into()),
                (Approval::Auto, Scope::Project) => a.push("--full-auto".into()),
                (Approval::Native | Approval::Deny, w) => {
                    let sandbox = match w { Scope::Native => None, Scope::None => Some("read-only"), Scope::Project => Some("workspace-write"), Scope::Any => Some("danger-full-access") };
                    if let Some(s) = sandbox { a.extend(["--sandbox".into(), s.into()]); }
                    if p.approval == Approval::Deny { c(&mut a, "approval_policy=never"); }
                }
                _ => return refuse("approval"),
            }
            match (p.network, p.write) {
                (Network::On, _) => c(&mut a, "sandbox_workspace_write.network_access=true"),
                (Network::Off, Scope::Project) => c(&mut a, "sandbox_workspace_write.network_access=false"),
                _ => {}
            }
            let roots = match &p.roots {
                Roots::Native => &[][..],
                Roots::Exact(r) => { c(&mut a, "sandbox_workspace_write.writable_roots=[]"); r }
                Roots::Add(r) => r,
            };
            if roots.iter().any(|r| !r.is_absolute()) { return refuse("roots"); }
            for r in roots { a.extend(["--add-dir".into(), r.clone().into_os_string()]); }
            if p.unelevated { c(&mut a, "windows.sandbox=unelevated"); }
            a.extend(extras(p));

    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        a.extend(["--model".into(), m.into()]);
    }
    if let Some(e) = &l.effort {
        if e.is_empty() || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err("effort requires a plain word".into()); }
        a.extend(["-c".into(), format!("model_reasoning_effort={e}").into()]);
    }
    if l.title.is_some() { return Err(format!("title unsupported by {name}")); }
    if l.compact_hook.is_some() { return Err(format!("compact-hook unsupported by {name}")); }
    Ok(a)
}
fn extras(p: &Policy) -> Vec<OsString> {
    let mut a: Vec<&str> = vec![];
    if p.isolated { for f in ["apps", "plugins", "browser_use", "computer_use", "multi_agent", "image_generation"] { a.extend(["--disable", f]); } }
    match p.builtin {
        Builtin::Native => {}
        Builtin::On => a.extend(["--enable", "shell_tool", "-c", "web_search=\"live\""]),
        Builtin::Off => a.extend(["--disable", "shell_tool", "-c", "web_search=\"disabled\""]),
    }
    if p.subscription { a.extend(["-c", "forced_login_method=\"chatgpt\""]); }
    a.into_iter().map(OsString::from).collect()
}

fn env(l: &Launch, spec: &mut Spec) { spec.env.extend(l.settings_env.iter().map(|(k,v)| (k.into(),v.into()))); }
fn settings(_: &Launch, _: &mut Spec) {}
fn login(exe: &Path, home: Option<&Path>) -> Check {
    let status = |args: &[&str]| probe(exe,args,home,"CODEX_HOME",10).map(|o| format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr)));
    match status(&["-c", "forced_login_method=\"chatgpt\"", "login", "status"]) {
                Ok(out) => {
                    let plain: String = out.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
                    if plain.contains("loggedinusingchatgpt") { check("ok", "subscription") }
                    else if plain.contains("notloggedin") { check("missing", "not logged in") }
                    else if plain.contains("loggedin") { check("api-key", "not a ChatGPT login") }
                    else { check("unknown", "login status unreadable") }
                }
                Err(e) => check("unknown", &e.to_string()),
            }
}
fn protected(home: PathBuf) -> Vec<PathBuf> { vec![home] }
fn refusal(text: &str) -> Option<Cause> {
    let lower = text.to_ascii_lowercase();
    Some(Cause(if text.contains("app-server thread/resume refused") { "session-missing" }
    else if ["401", "login", "log in"].iter().any(|m| lower.contains(m)) { "login-required" } else { return None }))
}
fn transcript(session: &str, home: Option<&Path>) -> Option<PathBuf> { rollout(session,home) }
fn link(_: &Run) -> Result<String,native::Absence> { Err((Cause("no-session-url"),None)) }
fn session_link(_: &NativeSpec, _: Option<&str>) -> Result<String,native::Absence> { Err((Cause("no-session-url"),None)) }
fn files(root: &Path) -> Vec<PathBuf> { vec![root.join("config.toml"),root.join("hooks.json"),root.join("AGENTS.md")] }
fn setup(root: &Path, _: &Path, exe: &Path) -> Vec<Edit> {
    ["SessionStart","Stop"].into_iter().map(|event| edit("codex",root.join("hooks.json"),&["hooks",event],json!({"hooks":[hook_command(exe,"codex",event)]}),true)).collect()
}
fn inspect(texts: &[Option<String>], docs: &[Option<Value>], unread: &[bool]) -> ProfileState {
    ProfileState { hooks:docs[1].as_ref().map_or("unknown",|d|grouped_hooks(d,"codex",&["SessionStart","Stop"])),plugin:"unsupported",adapter:"hook",
        overrides:(!unread[0]).then(||texts[0].as_deref().unwrap_or("").lines().filter(|l|l.trim_start().starts_with("[profiles.")).count()) }
}
fn hook(event: &str) -> Hook { match event {"Stop"=>Hook::Stop {model:false},"SessionStart"=>Hook::SessionStart,_=>Hook::None} }
fn fallback(_: &str) -> Value { json!({}) }
fn cli() -> Option<PathBuf> { executable(Client::Codex).ok() }

fn provider(root: &Path) -> Result<Provider,String> {
    let text = match std::fs::read_to_string(root.join("config.toml")) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Provider {selected:false,present:false,loopback:false}),
        Err(_) => return Err("unreadable_config".into()),
    };
    let mut section = "";
    let (mut selected, mut loopback, mut present) = (false, false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') { section = line.split('#').next().unwrap_or("").trim();
            present |= section == "[model_providers.agentq]"; continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches(['"', '\'']).split(['"', '\'']).next().unwrap_or("");
        if section.is_empty() && key.trim() == "model_provider" { selected = value == "agentq"; }
        if section == "[model_providers.agentq]" && key.trim() == "base_url" {
            let host = value.split_once("://").map(|(_, s)| s.split('/').next().unwrap_or(""));
            loopback = host.is_some_and(|s| s == "localhost" || s.starts_with("localhost:") || s.parse::<std::net::SocketAddr>().is_ok_and(|a| a.ip().is_loopback()) || s.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback()));
        }
    }
    Ok(Provider {selected,present,loopback})
}

fn diagnose(root: &Path) -> Value {
    match provider(root) {
        Err(_)=>json!({"state":"unknown","reason":"unreadable_config"}),
        Ok(p) if p.selected && (!p.present || p.loopback)=>json!({"state":"problem","reason":if p.present {"legacy_agentq_route"} else {"legacy_agentq_provider_missing"},"next":"agentq direct"}),
        Ok(_)=>json!({"state":"absent"}),
    }
}
fn background_transcript(_: &Spec, _: &str) -> Res<PathBuf> {Err("background unsupported".into())}

/// The native Codex executor behind an npm `codex.js` launcher: a kill cannot reach the native child through Node.
pub(crate) fn codex_native(bin: PathBuf) -> Res<PathBuf> {
    if cfg!(windows) || !bin.exists() { return Ok(bin); }
    let bin = bin.canonicalize().map_err(|e| format!("Codex executable unavailable: {e}"))?;
    let native = |path: &Path| -> Res<bool> {
        let mut magic = [0; 4];
        File::open(path).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| format!("Codex executable unreadable: {e}"))?;
        Ok(matches!(magic, [0x7f, b'E', b'L', b'F'] | [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf] | [0xce, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xce] | [0xca, 0xfe, 0xba, 0xbe] | [0xbe, 0xba, 0xfe, 0xca]))
    };
    if native(&bin)? { return Ok(bin); }
    let package = bin.parent().filter(|p| p.file_name().is_some_and(|n| n == "bin")).and_then(Path::parent)
        .filter(|_| bin.file_name().is_some_and(|n| n == "codex.js"))
        .ok_or("Codex launcher ownership unsupported; select a native Codex executable")?;
    let metadata: Value = serde_json::from_reader(File::open(package.join("package.json")).map_err(|e| format!("Codex package unavailable: {e}"))?.take(64 * 1024))
        .map_err(|e| format!("Codex package invalid: {e}"))?;
    if metadata["name"] != "@openai/codex" { return Err("Codex launcher package identity differs".into()); }
    let (platform, target) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("darwin-arm64", "aarch64-apple-darwin"),
        ("macos", "x86_64") => ("darwin-x64", "x86_64-apple-darwin"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-musl"),
        _ => return Err("Codex native package unsupported on this platform".into()),
    };
    // Match Node's package lookup along the entrypoint's ancestors; never scan other installations.
    let installed = package.ancestors().map(|p| p.join(format!("node_modules/@openai/codex-{platform}")))
        .find(|p| p.join("package.json").is_file()).unwrap_or_else(|| package.to_path_buf());
    let executable = installed.join(format!("vendor/{target}/bin/codex"));
    if !native(&executable)? { return Err("Codex package executor is not a native binary".into()); }
    executable.canonicalize().map_err(|e| format!("Codex native executable unavailable: {e}"))
}

fn native_home() -> Option<PathBuf> { std::env::var_os("CODEX_HOME").filter(|v|!v.is_empty()).map(PathBuf::from).or_else(||std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")).map(|h|Path::new(&h).join(".codex"))) }
fn native_executable(path: &OsStr) -> Res<PathBuf> {codex_native(which_in("codex",path))}
fn new_session() -> String {"-".into()}
fn native_args(spec: &Spec) -> Vec<OsString> {
    let mut a=vec![];let s=|v: &str|OsString::from(v);

            a.extend(["exec", "--json"].map(s));
            a.extend(spec.args.iter().cloned());
            if spec.resume { a.extend([s("resume"), s(&spec.session), s("-")]); }
            else { a.extend([s("-C"), spec.root.clone().into_os_string(), s("-")]); }

    a
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn npm_presence_and_launch_share_the_native_dependency_lookup() {
        let root=std::env::temp_dir().join(format!("agentq-npm-registry-{}",crate::incident::uuid()));
        let package=root.join("lib/node_modules/@openai/codex");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(package.join("package.json"),r#"{"name":"@openai/codex"}"#).unwrap();
        std::fs::write(package.join("bin/codex.js"),"#!/usr/bin/env node\n").unwrap();
        std::os::unix::fs::symlink(package.join("bin/codex.js"),root.join("bin/codex")).unwrap();
        let (platform,target)=match (std::env::consts::OS,std::env::consts::ARCH) {
            ("macos","aarch64")=>("darwin-arm64","aarch64-apple-darwin"),
            ("macos","x86_64")=>("darwin-x64","x86_64-apple-darwin"),
            ("linux","aarch64")=>("linux-arm64","aarch64-unknown-linux-musl"),
            ("linux","x86_64")=>("linux-x64","x86_64-unknown-linux-musl"),
            _=>return,
        };
        let dependency=root.join(format!("lib/node_modules/@openai/codex-{platform}"));
        std::fs::create_dir_all(dependency.join(format!("vendor/{target}/bin"))).unwrap();
        std::fs::write(dependency.join("package.json"),"{}").unwrap();
        let native=dependency.join(format!("vendor/{target}/bin/codex"));
        std::fs::write(&native,b"\x7fELF").unwrap();
        let launch=ADAPTER.executable(Some(root.join("bin").as_os_str())).unwrap();
        let presence=with_test_path(&root.join("bin"),||cli_present(&ENTRY)).unwrap();
        assert_eq!(launch,native.canonicalize().unwrap());
        assert_eq!(presence,launch);
        // Presence's callback uses the same runtime resolver, including npm's sibling dependency.
        assert_eq!(native_executable(root.join("bin").as_os_str()).unwrap(),launch);
        std::fs::remove_dir_all(root).unwrap();
    }
}
