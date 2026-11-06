//! Claude Code registration: native worker and installation behavior.
use super::*;

static ADAPTER: Adapter = Adapter {client: Client::Claude, live: AtomicUsize::new(0), descriptor: Descriptor { name: "claude", label: "Claude Code", brand: "Anthropic", family: "claude",
        models: &["opus", "sonnet", "haiku"], minimum: "2.1.0",
        capabilities: &["turn", "resume", "caller-session-id", "compact-hook", "session-url", "quota-snapshot", "account-limits", "background", "title", "mcp-config",
            "instructions-file", "turn-parts", "turn-deltas", "settings-env"] } , ops: NativeOps { home_var:"CLAUDE_CONFIG_DIR",alternate_home_var:None,session_var:"CLAUDE_CODE_SESSION_ID",home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, protected, refusal, transcript, background_transcript, link, session_link } };

pub(super) const ENTRY: Entry = Entry { name:"claude", aliases:&[], order:1, profile_order:1, worker_order:0, onboard:true,
    profile:Profile {kind:"claude-code",label:"Claude Code",aliases:&["claude-code-cli", "claude-desktop-code"],operations:&["hooks"]},
    runtime:Some(&ADAPTER), builtin:Some(&ADAPTER),
    root:config_root,files,setup,inspect,hook,hook_key:"hooks",fallback,executable:cli,diagnose:no_diagnosis,provider:None,
};

fn config_root() -> PathBuf { root("CLAUDE_CONFIG_DIR", ".claude") }
fn mapped(l: &Launch) -> Res<Vec<OsString>> {
    let name = "claude";
    let p = &l.policy;
    let refuse = |part: &str| Err(format!("policy {part} unsupported by {name}"));
    let mut a: Vec<OsString> = vec![];

            if p.read != Scope::Native { return refuse("read"); }
            // Full write only without prompts. No write stays refused: MCP servers and project hooks can write whatever the
            // built-in tools are, so Claude has no flag that enforces it.
            if !matches!((p.write, p.approval), (Scope::Native, _) | (Scope::Any, Approval::Bypass)) { return refuse("write"); }
            if p.network != Network::Native { return refuse("network"); }
            if p.roots != Roots::Native { return refuse("roots"); }
            if p.unelevated { return refuse("unelevated"); }
            // `on` is Claude's own default tool set.
            if p.builtin == Builtin::Off { a.extend(["--tools".into(), "".into()]); }
            if p.isolated {
                a.extend(["--strict-mcp-config", "--setting-sources", "project", "--disable-slash-commands", "--system-prompt-snapshot", "off"].map(OsString::from));
            }
            let mut tools = p.tools.clone();
            if tools.as_ref().is_some_and(|t| t.is_empty() || t.iter().any(|t| t.is_empty() || t.starts_with('-'))) { return refuse("tools"); }
            if let Some(file) = &l.mcp {
                a.extend(["--mcp-config".into(), file.clone().into_os_string()]);
                tools.get_or_insert_with(Vec::new).extend(mcp_servers(file)?.into_iter().map(|s| format!("mcp__{s}")));
            }
            if let Some(file) = &l.instructions { a.extend(["--append-system-prompt-file".into(), file.clone().into_os_string()]); }
            if !l.parts.is_empty() { a.extend(["--input-format", "stream-json"].map(OsString::from)); }
            if l.deltas { a.push("--include-partial-messages".into()); }
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                a.push("--allowedTools".into());
                a.extend(tools.iter().map(OsString::from));
            }
            let mode = match p.approval { Approval::Native => None, Approval::Plan => Some("plan"), Approval::Edits => Some("acceptEdits"),
                Approval::Auto => Some("auto"), Approval::Deny => Some("dontAsk"), Approval::Bypass => Some("bypassPermissions") };
    if let Some(m) = mode { a.extend(["--permission-mode".into(),m.into()]); }
    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        a.extend(["--model".into(), m.into()]);
    }
    if l.effort.is_some() { return Err(format!("effort unsupported by {name}")); }
    if let Some(t) = &l.title { a.extend(["--name".into(), t.into(), "--remote-control".into(), t.into()]); }
    if let Some(hook) = &l.compact_hook {
        if l.settings_env.is_empty() { a.extend(["--settings".into(), claude_settings(Some(hook), &[]).into()]); }
    }
    Ok(a)
}
fn extras(_: &Policy) -> Vec<OsString> { vec![] }
fn env(l: &Launch, spec: &mut Spec) {
    if l.policy.subscription { spec.env.extend(["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"].map(|k| (k.into(), OsString::new()))); }
}
fn settings(l: &Launch, spec: &mut Spec) {
        if !l.settings_env.is_empty() {
            // A `--bg` session runs under the Claude daemon, whose environment is not this child's: settings `env` reaches it.
            let child: Vec<(String, String)> = ["TASKQ_TASK", "AGENTQ_LAUNCHER"].into_iter()
                .filter_map(|k| child_env(&spec, k).map(|v| (k.into(), v.to_string_lossy().into_owned()))).collect();
            let mut vars: Vec<(&str, &str)> = l.settings_env.iter().chain(&child).map(|(k, v)| (k.as_str(), v.as_str())).collect();
            if l.policy.subscription { vars.extend([("ANTHROPIC_API_KEY", ""), ("ANTHROPIC_AUTH_TOKEN", "")]); }
            spec.args.extend(["--settings".into(), claude_settings(l.compact_hook.as_deref(), &vars).into()]);
        }
}
fn login(exe: &Path, home: Option<&Path>) -> Check {
    let status = |args: &[&str]| probe(exe,args,home,"CLAUDE_CONFIG_DIR",10).map(|o| format!("{}{}",String::from_utf8_lossy(&o.stdout),String::from_utf8_lossy(&o.stderr)));
    match status(&["auth", "status", "--json"]) {
                Ok(out) => match serde_json::from_str::<Value>(out.trim()).ok().or_else(|| out.lines().find_map(|l| serde_json::from_str(l).ok())) {
                    Some(v) if v["loggedIn"] == false => check("missing", "not logged in"),
                    Some(v) if v["loggedIn"] == true && v["authMethod"] == "claude.ai" && v["apiProvider"] == "firstParty" => check("ok", "subscription"),
                    Some(v) if v["loggedIn"] == true => check("api-key", v["authMethod"].as_str().unwrap_or("other login")),
                    _ => check("unknown", "login status unreadable"),
                },
                Err(e) => check("unknown", &e.to_string()),
            }
}
fn protected(home: PathBuf) -> Vec<PathBuf> { vec![home.clone(), if home.file_name().is_some_and(|n| n == ".claude") { home.with_file_name(".claude.json") } else { home.join(".claude.json") }] }
fn refusal(text: &str) -> Option<Cause> {
    Some(Cause(if text.contains("Workspace not trusted") { "workspace-untrusted" }
    else if text.contains("No conversation found with session ID") { "session-missing" }
    else if text.contains("Not logged in") || text.contains("/login") { "login-required" } else { return None }))
}
fn transcript(_: &str, _: Option<&Path>) -> Option<PathBuf> { None }
fn link(run: &Run) -> Result<String, native::Absence> {
        let none = |c| Err((Cause(c), None));
        // An unreadable transcript names its error rather than reading as an unknown session (P2).
        let text = match &run.body {
            Body::Turn(t) => read_log(&t.log, t.start).map_err(|e| (Cause("evidence-unreadable"), Some(format!("transcript: {e}"))))?,
            _ => String::new(),
        };
        let Some(session) = ADAPTER.session_id(run) else { return none("session-unknown") };
        let (home, job, seen) = match &run.body {
            Body::Turn(t) => (t.home.clone(), None, text.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| bridge(&v)).next_back()),
            Body::Native(n) => (spec_home(&n.spec), n.job.as_ref().map(|j| j.id.clone()).or_else(|| n.job_id.clone()), n.turn.remote_url().map(String::from)),
            Body::App(_) => return none("no-session-url"),
        };
        native::claude_link(home.as_deref(), &session, job.as_deref(), run.remote, seen)
    }

fn session_link(spec: &NativeSpec, job: Option<&str>) -> Result<String, native::Absence> {
        if !ADAPTER.descriptor.has("session-url") { return Err((Cause("no-session-url"), None)); }
        let native = ADAPTER.native(spec);
        let requested = spec.args.iter().any(|a| a == "--remote-control");
        // ponytail: whole transcript read; the last bridge_status URL wins, a missing transcript falls to the job and debug log.
        let seen = claude_transcript(&native, &spec.session).ok().and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| bridge(&v)).next_back());
        native::claude_link(spec_home(&native).as_deref(), &spec.session, job, requested, seen)
    }
fn files(root: &Path) -> Vec<PathBuf> { vec![root.join("settings.json"),claude_json(),root.join("CLAUDE.md")] }
pub(crate) fn claude_json() -> PathBuf { std::env::var_os("CLAUDE_CONFIG_DIR").map(|d| PathBuf::from(d).join(".claude.json")).unwrap_or_else(|| host_home().join(".claude.json")) }
fn setup(root: &Path, _: &Path, exe: &Path) -> Vec<Edit> {
    ["SessionStart","Stop"].into_iter().map(|event| edit("claude",root.join("settings.json"),&["hooks",event],json!({"hooks":[hook_command(exe,"claude",event)]}),true)).collect()
}
fn inspect(_: &[Option<String>], docs: &[Option<Value>], _: &[bool]) -> ProfileState {
    let set = |v: &Value| v.as_object().is_some_and(|o|!o.is_empty()) || v.as_array().is_some_and(|a|!a.is_empty());
    ProfileState {hooks:docs[0].as_ref().map_or("unknown",|d|grouped_hooks(d,"claude",&["SessionStart","Stop"])),plugin:"unsupported",adapter:"hook",
        overrides:docs[1].as_ref().map(|d|d["projects"].as_object().into_iter().flatten().filter(|(_,p)|["mcpServers","env","enabledMcpjsonServers"].iter().any(|k|set(&p[*k]))).count())}
}
fn hook(event: &str) -> Hook { if event=="Stop" {Hook::Stop {model:true}} else {Hook::None} }
fn fallback(_: &str) -> Value { json!({}) }
fn cli() -> Option<PathBuf> { executable(Client::Claude).ok() }

/// The Remote Control URL of a `system` `bridge_status` record.
fn bridge(v: &Value) -> Option<String> {
    (v["subtype"] == "bridge_status").then(|| v["url"].as_str().filter(|u| native::remote_url(u)).map(String::from)).flatten()
}

/// Claude's native transcript of `session`: `<home>/projects/<root, each non-alphanumeric character as ->/<session>.jsonl`.
fn claude_transcript(spec: &Spec, session: &str) -> Res<PathBuf> {
    let root = spec.root.canonicalize().map_err(|e| format!("project root: {e}"))?;
    Ok(spec_home(spec).ok_or("no client home")?.join("projects").join(project_dir(&root.to_string_lossy())).join(format!("{session}.jsonl")))
}

/// Claude's project folder name for a canonical root. Windows canonical paths are verbatim (`\\?\C:\…`); Claude names the
/// folder from the plain path (`C--…`).
pub(super) fn project_dir(root: &str) -> String {
    let plain = root.strip_prefix(r"\\?\UNC\").map(|p| format!(r"\\{p}")).or_else(|| root.strip_prefix(r"\\?\").map(String::from));
    plain.as_deref().unwrap_or(root).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn background_transcript(spec: &Spec, session: &str) -> Res<PathBuf> {claude_transcript(spec,session)}
fn native_home() -> Option<PathBuf> { std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v|!v.is_empty()).map(PathBuf::from).or_else(||std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")).map(|h|Path::new(&h).join(".claude"))) }
fn native_executable(path: &OsStr) -> Res<PathBuf> {Ok(which_in("claude",path))}
fn new_session() -> String {crate::incident::uuid()}
fn native_args(spec: &Spec) -> Vec<OsString> {
    let mut a=vec![];let s=|v: &str|OsString::from(v);

            a.extend(["-p", "--output-format", "stream-json", "--verbose"].map(s));
            a.extend(spec.args.iter().cloned());
            a.push(s(if spec.resume { "--resume" } else { "--session-id" }));
            a.push(s(&spec.session));

    a
}
