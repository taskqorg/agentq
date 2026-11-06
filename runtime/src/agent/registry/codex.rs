//! Codex registration: native worker and installation behavior.
#[path="codex/app.rs"]
pub(crate) mod app;
use super::super::super::events::{string,tool_kind,audit_command,audit_files,native_timeout};
use super::*;
use super::super::super::native::plain_id;
use super::super::super::limits::{window,unknown};

static ADAPTER: Adapter = Adapter {client: Client::Codex, live: AtomicUsize::new(0), descriptor: Descriptor { name: "codex", label: "Codex CLI", brand: "OpenAI", family: "gpt", models: &[], minimum: "0.162.0",
        capabilities: &["history", "turn", "resume", "quota-snapshot", "account-limits", "catalogue", "app-queue", "app-server", "effort", "image-input", "account", "host-args", "settings-env", "mcp-config", "instructions-file", "turn-parts", "turn-deltas"] } , ops: NativeOps {catalogue:catalogue,background_launch:|_,_,_,_|Err("background unsupported".into()),background_jobs:|_,_,_,_|Err("background unsupported".into()),background_job:|_,_,_|Err("background unsupported".into()),background_stop:|_,_,_,_|Err("background unsupported".into()),background_id:|_|None,app_queue:app_queue,app_command:app::app_server_command, protocol:&PROTOCOL, home_var:"CODEX_HOME",alternate_home_var:None,session_var:"CODEX_THREAD_ID",home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, protected, refusal, transcript, background_transcript, link, session_link } };

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

#[allow(non_upper_case_globals)]
impl Client { pub(crate) const Codex: Self=Self("codex"); }

fn parse_event(cursor: &mut events::Cursor,v: &Value,out: &mut Vec<Event>,thread: Option<&str>) {
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        if v["type"] == "turn.started" || (method == "turn/started" && thread.is_none_or(|t|p["threadId"].as_str()==Some(t))) { out.push(Event::TurnStart); }
        if cursor.deltas && method == "item/agentMessage/delta" {
            if let Some(delta) = string(&p["delta"]) { out.push(Event::Text { item: string(&p["itemId"]), delta }); }
        }
        if method == "thread/tokenUsage/updated" && thread.is_some_and(|t|!t.is_empty() && p["threadId"].as_str()==Some(t)) {
            out.push(Event::Context { used: p["tokenUsage"]["last"]["totalTokens"].as_u64(), window: p["tokenUsage"]["modelContextWindow"].as_u64() });
        }
        let payload = v.get("payload").or(v.get("msg")).unwrap_or(v);
        if payload["type"] == "token_count" {
            out.push(Event::Context { used: payload["info"]["last_token_usage"]["total_tokens"].as_u64(), window: payload["info"]["model_context_window"].as_u64() });
        }
        // A rollout call exposes only its identity, name and deadline, never its arguments or output.
        match payload["type"].as_str() {
            Some("function_call"|"custom_tool_call") => {
                let input=payload["arguments"].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).unwrap_or(Value::Null);
                out.push(Event::ToolStart { id: string(&payload["call_id"]), kind: tool_kind(payload["name"].as_str().unwrap_or("")), name: string(&payload["name"]), detail: None, timeout_ms: native_timeout(&input) });
            }
            Some("function_call_output"|"custom_tool_call_output") => out.push(Event::ToolEnd { id: string(&payload["call_id"]), status: None }),
            _ => {}
        }
        let kind = v["type"].as_str().unwrap_or("");
        let start = method == "item/started" || kind == "item.started";
        let end = method == "item/completed" || kind == "item.completed";
        if !start && !end { return; }
        let i = if method.is_empty() { &v["item"] } else { &p["item"] };
        let native = i["type"].as_str().unwrap_or("");
        let id = string(&i["id"]);
        if matches!(native, "agentMessage" | "agent_message") {
            if end { if let Some(text) = string(&i["text"]) { out.push(Event::Message { item: id, text }); } }
        } else if matches!(native, "commandExecution" | "command_execution" | "mcpToolCall" | "mcp_tool_call" | "webSearch" | "web_search" | "fileChange" | "file_change" | "dynamicToolCall" | "dynamic_tool_call" | "collabAgentToolCall" | "collab_tool_call" | "imageView" | "image_view" | "imageGeneration" | "image_generation") {
            if start { out.push(Event::ToolStart { id: id.clone(), kind: tool_kind(native), timeout_ms:native_timeout(i), name: string(&i["tool"]).or_else(|| string(&i["name"])), detail: match native {"commandExecution"|"command_execution"=>audit_command(&i["command"]),"fileChange"|"file_change"=>audit_files(&i["changes"]),_=>None} }); }
            if end { out.push(Event::ToolEnd { id, status: string(&i["status"]) }); }
        }
    }

fn terminal(cursor: &events::Cursor,v: &Value,out: &mut Vec<Event>,thread: Option<&str>,turn: Option<&str>) -> Option<&'static str> {
    if v["method"]=="turn/completed" && (thread.is_none() || thread==v["params"]["threadId"].as_str() && turn.is_some_and(|t|v["params"]["turn"]["id"].as_str()==Some(t))) {
        let outcome=match v["params"]["turn"]["status"].as_str() {Some("completed")=>Some("ok"),Some("failed")=>Some("failed"),Some("interrupted")=>Some("cancelled"),_=>None};
        if outcome.is_none() {out.push(Event::Unsupported{reason:"native turn status unsupported"});}
        return outcome;
    }
    events::default_terminal(Client::Codex,cursor,v).or_else(||session_end(v))
}

fn session_end(v: &Value) -> Option<&'static str> {if v["type"]!="event_msg" {return None;} match v["payload"]["type"].as_str() {Some("task_complete")=>Some("ok"),Some("turn_aborted")=>Some("cancelled"),_=>None}}

fn assistant_failure(_: &Value) -> Option<events::AssistantFailure> {None}

fn permission_mode(_: &Value) -> Option<String> {None}

fn lifecycle(v:&Value)->Option<String> {
 let name:String={let p=v.get("payload").or(v.get("msg")).unwrap_or(v);
if matches!(p["type"].as_str(),Some("task_started"|"task_complete"|"turn_aborted")) {p["type"].as_str()?.into()}
else if matches!(v["type"].as_str(),Some("thread.started"|"turn.started"|"turn.completed"|"turn.failed"|"error")) {v["type"].as_str()?.into()}
else if matches!(v["method"].as_str(),Some("thread/started"|"turn/started"|"turn/completed"|"error")) {v["method"].as_str()?.into()} else {return None;}};
    let name = name.replace("task_", "turn_").replace(['_', '.', '/'], " ");
    Some(if name == "init" {"session started".into()} else {name})
}

fn advances(v:&Value)->bool {matches!(v.get("payload").or(v.get("msg")).unwrap_or(v)["type"].as_str(),Some("message"|"reasoning")) || (v["type"]=="item.completed" && events::completed_non_tool(&v["item"])) || (v["method"]=="item/completed" && events::completed_non_tool(&v["params"]["item"]))}

/// Codex's thread ID: the first `thread.started` line.
pub(crate) fn session(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        (v["type"] == "thread.started").then(|| v["thread_id"].as_str().map(String::from)).flatten()
    })
}

/// The thread's rollout `<home>/sessions/**/rollout-*-<thread>.jsonl`, in `home` or the client's own home.
pub(crate) fn rollout(thread: &str, home: Option<&Path>) -> Option<PathBuf> {
    fn find(dir: &Path, end: &str, depth: u8) -> Option<PathBuf> {
        if depth == 0 { return None; }
        std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
            let p = e.path();
            if p.is_dir() { find(&p, end, depth - 1) } else { p.file_name()?.to_str()?.ends_with(end).then_some(p) }
        })
    }
    if thread.is_empty() || !thread.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return None; }
    let home = home.map(Path::to_path_buf).or_else(|| base_home(Client::Codex))?;
    // sessions/<yyyy>/<mm>/<dd>/<file>
    find(&home.join("sessions"), &format!("{thread}.jsonl"), 4)
}

fn codex_limits(v: &Value) -> Option<Snapshot> {
    let r = [&v["rate_limits"], &v["payload"]["rate_limits"], &v["msg"]["rate_limits"]].into_iter().find(|r| r.is_object())?;
    let (used, w) = [&r["primary"], &r["secondary"]].into_iter().filter_map(|w| w["used_percent"].as_f64().map(|u| (u, w)))
        .fold(None::<(f64, &Value)>, |best, c| if best.is_none_or(|b| c.0 > b.0) { Some(c) } else { best })?;
    let resets = w["resets_at"].as_u64().or_else(|| w["resets_in_seconds"].as_u64().map(|s| (unix_ms() / 1000) as u64 + s));
    Some(Snapshot { used_percent: percent(used), resets_at: resets, window: w["window_minutes"].as_u64().map(|m| format!("{m}m")), unknown: None })
}

fn read_snapshot(events:&[Value],rollout:Option<&Path>)->Snapshot {
 let found={
            let own = events.iter().rev().find_map(|v| codex_limits(v));
            own.or_else(|| {
                let f = File::open(rollout?).ok()?;
                BufReader::new(f).lines().map_while(Result::ok).filter(|l| l.contains("\"rate_limits\""))
                    .filter_map(|l| serde_json::from_str::<Value>(&l).ok()).filter_map(|v| codex_limits(&v)).last()
            })
};
    match found {
        Some(mut s) => {
            if s.used_percent.is_none() || s.resets_at.is_none() { s.unknown = Some("partial-snapshot"); }
            s
        }
        None => Snapshot { unknown: Some(if rollout.is_none() { "no-rollout" } else { "not-reported" }), ..Snapshot::default() },
    }
}

fn read_report(text:&str,events:&[Value],rollout:Option<&Path>)->Report {let end_index=events.iter().rposition(|v|ends(v) || assistant_failure(v).is_some()); let failure=None; classify_report(Client::Codex,text,events,rollout,end_index,failure,vec![])}

static PROTOCOL: Protocol=Protocol {turn_ended:native_turn_ended,parse_event,terminal,assistant_failure,permission_mode,lifecycle,advances,session,snapshot:read_snapshot,report:read_report,prepare_command:stdio_command,input_bytes:plain_input,native_record,native_metadata:Some(verify_meta),finish_native_report,session_report,reported,update_limits,account_limits,identity:account_identity,credentials,oauth_token,oauth_usage};

pub(crate) fn codex_groups(v: &Value) -> Option<Vec<(&str, &Value)>> {
    if let Some(groups) = v["result"].get("rateLimitsByLimitId").or(v["params"].get("rateLimitsByLimitId")).or(v.get("rateLimitsByLimitId")) {
        return Some(match groups.as_object() { Some(g) => g.iter().map(|(id, r)| (id.as_str(), r)).collect(), None => vec![("codex", groups)] });
    }
    let r = [v.get("rate_limits"), v["payload"].get("rate_limits"), v["msg"].get("rate_limits"), v["params"].get("rateLimits"), v["result"].get("rateLimits"), v.get("rateLimits")].into_iter().flatten().next()?;
    Some(vec![(r["limit_id"].as_str().or(r["limitId"].as_str()).unwrap_or("codex"), r)])
}

pub(crate) fn codex_windows(r: &Value, id: Option<String>, now: u64) -> Vec<Window> {
    if !r.is_object() { return unknown("limit-group-unreadable"); }
    let mut windows = vec![];
    for key in ["primary", "secondary"] {
        let Some(w) = r.get(key).filter(|w| !w.is_null()) else { continue; };
        if !w.is_object() { windows.push(window(Some(key.into()),None,None,id.clone())); continue; }
        let minutes = w["window_minutes"].as_u64().or(w["windowDurationMins"].as_u64());
        let used = w["used_percent"].as_f64().or(w["usedPercent"].as_f64()).map(|u| u / 100.0);
        let reset = w["resets_at"].as_u64().or(w["resetsAt"].as_u64()).or_else(|| w["resets_in_seconds"].as_u64().and_then(|s| now.checked_add(s)));
        windows.push(window(minutes.map(|m| format!("{m}m")), used, reset, id.clone()));
    }
    if windows.is_empty() { unknown("partial-snapshot") } else { windows }
}

fn codex_read<T>(home: Option<&Path>, executable: Option<&Path>, read: impl FnOnce(&mut app::AppServer, &mut app::AppSession, Instant)->Res<T>) -> Res<T> {
    let root=std::env::current_dir().map_err(|_|"working directory unavailable")?;
    let spec=Spec {client:Client::Codex,root,session:"-".into(),resume:false,args:vec![],env:vec![],home:home.map(Path::to_path_buf)};
    let log=std::env::temp_dir().join(format!("agentq-read-{}.log",crate::incident::uuid()));
    let exe=match executable {Some(path)=>super::codex_native(path.to_path_buf())?,None=>adapter::runtime("codex").ok_or("runtime missing")?.executable(None)?};
    let mut server=app::AppServer::spawn(app::app_server_command(Command::new(exe),&spec)?,&log,&Process::default())?;
    let result=(|| {
        let until=Instant::now()+Duration::from_secs(30);
        let mut session=app::AppSession::new(&spec)?;
        let initialize=session.initialize(json!({"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}))?;
        exchange(&mut server,&mut session,initialize,until)?;
        server.send(&session.initialized()?,until)?;
        read(&mut server,&mut session,until)
    })();
    let stopped=server.close(); drop(server); let _=std::fs::remove_file(log);
    let result=result?; stopped?; Ok(result)
}

fn exchange(server: &mut app::AppServer, session: &mut app::AppSession, request: Value, until: Instant) -> Res<Value> {
    server.send(&request, until)?;
    loop {
        let m=server.receive(until)?.ok_or("native read deadline")?;
        if m["method"].is_string() && m.get("id").is_some() {
            server.send(&json!({"id":m["id"],"error":{"code":-32601,"message":"read-only session"}}),until)?;
            continue;
        }
        session.observe(&m)?;
        if m["id"]==request["id"] && m.get("method").is_none() {
            return if m.get("error").is_some() {Err("native read refused".into())} else {Ok(m)};
        }
    }
}

pub(crate) fn codex_identity(account: &Value) -> Option<String> {
    ["email","chatgptAccountId"].into_iter().find_map(|key|account[key].as_str().filter(|s|!s.is_empty()).map(String::from))
}

fn credential_snapshot(path: &Path) -> Res<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes)=>Ok(Some(bytes)),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),
        Err(_)=>Err("credential-unreadable".into()),
    }
}

pub(crate) fn catalogue(home: Option<&Path>, executable: Option<&Path>) -> Res<Vec<Model>> {
    codex_read(home, executable, |server,session,until| {
        let mut models=vec![]; let mut cursor=Value::Null;
        for _ in 0..100 {
            let request=session.request("model/list",json!({"cursor":cursor,"limit":100,"includeHidden":false}))?;
            let reply=exchange(server,session,request,until)?;
            for m in reply["result"]["data"].as_array().ok_or("catalogue data missing")? {
                let id=m["model"].as_str().filter(|m|model_code(m)).ok_or("catalogue model invalid")?;
                models.push(Model {id:id.into(),name:m["displayName"].as_str().unwrap_or(id).into()});
            }
            let next=&reply["result"]["nextCursor"];
            if next.is_null() {return Ok(models);}
            if !next.is_string() || *next==cursor {return Err("catalogue cursor invalid".into());}
            cursor=next.clone();
        }
        Err("catalogue page bound exceeded".into())
    })
}

fn reported(v:&Value,now:u64)->Option<Vec<Window>> {let id=v["event_id"].as_str().or(v["uuid"].as_str()).or(v["id"].as_str()).map(String::from);
    let mut windows = vec![];
    for (_, r) in codex_groups(v)? {
        windows.extend(codex_windows(r, id.clone(), now));
    }
    Some(if windows.is_empty() { unknown("partial-snapshot") } else { windows })
}

fn update_limits(v:&Value,now:u64,all:&mut BTreeMap<String,Vec<Window>>)->bool {
        let Some(groups) = codex_groups(v) else { return false; };
        if groups.is_empty() { all.clear(); all.insert("codex".into(), unknown("partial-snapshot")); }
        for (id, r) in groups {
            let event_id = v["event_id"].as_str().or(v["uuid"].as_str()).or(v["id"].as_str()).map(String::from);
            all.insert(id.into(), codex_windows(r, event_id, now));
        }
        return true;

}

fn account_limits(home:Option<&Path>,now:i64)->Vec<Window> {let client=Client::Codex;
    let _explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(client)) else { return unknown("home-unavailable"); };
    let path = home.join("auth.json");
    let before = match credential_snapshot(&path) { Ok(b) => b, Err(_) => return unknown("credential-unreadable") };
    let response = codex_read(Some(&home), None, |server,session,until| {let request=session.request("account/rateLimits/read",json!({}))?;exchange(server,session,request,until)});
    if credential_snapshot(&path).ok().as_ref() != Some(&before) { return unknown("credential-changed"); }
    match response {
        Ok(v) => reported(&v, now.max(0) as u64).unwrap_or_else(|| unknown("not-reported")),
        Err(_) => unknown("native-account-read-failed"),
    }
}

fn account_identity(home:Option<&Path>)->Option<String> {let client=Client::Codex;
    let _explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let home=home.map(Path::to_path_buf).or_else(||base_home(client))?;

            let path=home.join("auth.json"); let before=credential_snapshot(&path).ok()?;
            let reply=codex_read(Some(&home),None,|server,session,until|{
                let request=session.request("account/read",json!({"refreshToken":false}))?;
                exchange(server,session,request,until)
            }).ok()?;
            if credential_snapshot(&path).ok()? != before {return None;}
            codex_identity(&reply["result"]["account"])
}

fn credentials(_: &Path,_:bool)->Res<String> {Err("oauth-unavailable".into())}
fn oauth_token(_: &Value,_:i64)->Res<&str> {Err("oauth-unavailable".into())}
fn oauth_usage(_: &str)->Res<Vec<Window>> {Err("oauth-unavailable".into())}

pub(crate) fn app_queue(mut base: Command, spec: &Spec, reference: &str, budget: Duration) -> Res<std::process::Output> {
    if spec.client != Client::Codex || !plain_id(&spec.session) || spec.session == "-" { return Err("app thread invalid".into()); }
    base.args(["queue", "--thread", &spec.session, "--message", reference]);
    crate::process::output("agent.app-queue", &mut environment(base, spec), budget).map_err(|e| e.to_string())
}

fn native_record(turn:&mut native::NativeTurn,v:&Value)->Res<()> {

                    if v["type"] == "session_meta" { verify_meta(turn,v)?; }
                    if v["type"] != "event_msg" { return Ok(()); }
                    let p = &v["payload"];
                    if !p.is_object() {return Err("native app record shape invalid".into());}
                    match p["type"].as_str() {
                        Some("task_started") => turn.current = p["turn_id"].as_str().filter(|s| plain_id(s)).map(String::from),
                        Some("user_message" | "item_completed") => {
                            let item = p["type"] == "item_completed" && p["item"]["type"] == "UserMessage";
                            let value = if item { p["item"]["content"].as_array().filter(|a| a.len() == 1 && a[0]["type"] == "text").and_then(|a| a[0]["text"].as_str()) } else { p["message"].as_str() };
                            if value.is_some_and(|s| s == turn.input || item && s.strip_suffix('\n') == Some(turn.input.as_str())) {
                                if turn.accepted { return Err("native input appears more than once".into()); }
                                let id = turn.current.clone().ok_or("native input lacks turn identity")?;
                                if item && (p["thread_id"] != turn.spec.session || p["turn_id"].as_str() != Some(&id))
                                    || p["turn_id"].as_str().is_some_and(|s| s != id) { return Err("native input identity differs".into()); }
                                turn.accepted = true; turn.turn = Some(id);
                            }
                        }
                        Some("task_complete" | "turn_aborted") if turn.accepted && p["turn_id"].as_str() == turn.turn.as_deref() => {
                            let aborted = p["type"] == "turn_aborted";
                            if turn.ended && turn.aborted != aborted { return Err("native terminal outcomes conflict".into()); }
                            turn.ended = true; turn.aborted = aborted;
                        }
                        _ => {}
                    }
                    if turn.accepted && !turn.ended && turn.current == turn.turn {
                        if p["type"] == "token_count" && p["turn_id"].as_str() == turn.turn.as_deref() {
                            let u = &p["info"]["last_token_usage"];
                            append_codex_usage(&mut turn.events,u,&p["info"]["total_token_usage"],false);
                        }
                        if p["type"] == "agent_message" && p["turn_id"].as_str().is_none_or(|t|Some(t)==turn.turn.as_deref()) {
                            turn.events.push(json!({"type":"item.completed","item":{"type":"agent_message","text":p["message"]}}));
                        }
                        if !p["rate_limits"].is_null() && p["turn_id"].as_str() == turn.turn.as_deref() {
                            let mut limits=p.clone();
                            // A relative reset without a native timestamp cannot become a stable reset on recovery.
                            if let Some(windows)=limits.get_mut("rate_limits").and_then(Value::as_object_mut) {
                                for key in ["primary","secondary"] {if let Some(w)=windows.get_mut(key).and_then(Value::as_object_mut) {w.remove("resets_in_seconds");}}
                            }
                            turn.events.push(limits);
                        }
                    }
 Ok(())
}

fn finish_native_report(_: &native::NativeTurn,_:&mut Report) {}

fn verify_meta(turn:&native::NativeTurn,v:&Value)->Res<()> {
        let m = &v["payload"];
        let root=turn.spec.root.canonicalize().map_err(|_|"native project identity unavailable")?;
        if v["type"] != "session_meta" || m["id"] != turn.spec.session || !m["forked_from_id"].is_null()
            || m["cwd"].as_str().and_then(|p| Path::new(p).canonicalize().ok()) != Some(root) {
            return Err("native app session identity differs or is copied".into());
        }
        Ok(())
    }

pub(crate) fn append_codex_usage(events:&mut Vec<Value>,last:&Value,total:&Value,camel:bool) {
    let fields=if camel {["inputTokens","cachedInputTokens","outputTokens"]} else {["input_tokens","cached_input_tokens","output_tokens"]};
    let reasoning=&last[if camel {"reasoningOutputTokens"} else {"reasoning_output_tokens"}];
    let totals:Vec<_>=fields.iter().map(|k|total[k].as_u64()).collect();
    let increments:Vec<_>=fields.iter().map(|k|last[k].as_u64()).collect();
    let key=json!(totals);
    if totals.iter().all(Option::is_some) && events.iter().any(|e|e["total_usage"]==key) {return;}
    let prior=events.iter().rev().find_map(|e|e.get("total_usage").and_then(Value::as_array));
    let consistent=totals.iter().all(Option::is_some) && increments.iter().all(Option::is_some)
        && prior.is_none_or(|previous|totals.iter().zip(&increments).zip(previous).all(|((total,increment),old)|
            total.zip(old.as_u64()).and_then(|(t,o)|t.checked_sub(o))==*increment));
    let value=|i|if consistent {json!(increments[i])} else {Value::Null};
    events.push(json!({"type":"turn.completed","call":true,"total_usage":key,"usage":{"input_tokens":value(0),"cached_input_tokens":value(1),"output_tokens":value(2),"reasoning_output_tokens":reasoning}}));
}

fn session_report(text:&str,r:&mut Report) {
        r.calls = capture::rollout_calls(text, false);
        let known = !r.calls.is_empty();
        r.usage = sum_usage(&r.calls, None, known);

}
