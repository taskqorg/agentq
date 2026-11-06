#[path="claude/history.rs"]
mod history;
#[path="claude/parsing.rs"]
mod parsing;
use self::parsing::*;
// Claude Code registration: native worker and installation behavior.
use super::super::super::events::{string,tool_kind,audit_path,audit_command,native_timeout};
use super::*;
use super::super::super::native::{plain_id,scope};
use super::super::super::limits::{window,unknown,reset,https,oauth_account};
use std::collections::BTreeSet;

static ADAPTER: Adapter = Adapter {client: Client::Claude, live: AtomicUsize::new(0), descriptor: Descriptor { name: "claude", label: "Claude Code", brand: "Anthropic", family: "claude",
        models: &["opus", "sonnet", "haiku"], minimum: "2.1.0",
        capabilities: &["history", "turn", "resume", "caller-session-id", "compact-hook", "session-url", "quota-snapshot", "account-limits", "background", "title", "mcp-config",
            "instructions-file", "turn-parts", "turn-deltas", "settings-env"] } , ops: NativeOps {catalogue:|_,_|Err("catalogue unsupported".into()),background_launch:background_launch,background_jobs:background_jobs,background_job:BackgroundJob::read,background_stop:background_stop,background_id:background_job_id,app_queue:|_,_,_,_|Err("app-queue unsupported".into()),app_command:|_,_|Err("app-server unsupported".into()), protocol:&PROTOCOL, home_var:"CLAUDE_CONFIG_DIR",alternate_home_var:None,session_var:"CLAUDE_CODE_SESSION_ID",home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, protected, refusal, transcript, background_transcript, link, session_link } };

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

#[allow(non_upper_case_globals)]
impl Client { pub(crate) const Claude: Self=Self("claude"); }

fn parse_event(cursor: &mut events::Cursor,v: &Value,out: &mut Vec<Event>,_thread: Option<&str>) {
        if let Some(models) = v["modelUsage"].as_object() {
            cursor.window = cursor.model.as_ref().and_then(|m|models.get(m)).and_then(|m|m["contextWindow"].as_u64());
            out.push(Event::Context { used: cursor.used, window: cursor.window });
        }
        if v["type"] == "system" && v["subtype"] == "init" { out.push(Event::TurnStart); }
        let event = &v["event"];
        if v["type"] == "stream_event" {
            if event["type"] == "message_start" { cursor.message = string(&event["message"]["id"]); }
            if cursor.deltas && event["type"] == "content_block_delta" && event["delta"]["type"] == "text_delta" {
                if let Some(delta) = string(&event["delta"]["text"]) { out.push(Event::Text { item: cursor.message.clone(), delta }); }
            }
        }
        if v["type"] == "assistant" {
            let message = &v["message"];
            let model=string(&message["model"]);
            if model != cursor.model {cursor.window=None;} cursor.model=model;
            let item = string(&message["id"]);
            if let Some(blocks) = message["content"].as_array() {
                let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("");
                // Older checkpoints deduplicated the aggregate message, rather than each block.
                let legacy_seen=item.as_ref().is_some_and(|id|cursor.seen.contains(&format!("message:{id}:{}",crate::sha256_hex(text.as_bytes()))));
                let mut emitted=false;
                for (index,block) in blocks.iter().enumerate() {
                    if block["type"] == "text" {
                        if let Some(text)=block["text"].as_str().filter(|s|!s.is_empty()) {
                            if !legacy_seen && item.as_ref().is_none_or(|id|cursor.seen.insert(format!("message:{id}:{index}:{}",crate::sha256_hex(text.as_bytes())))) {
                                out.push(Event::Message{item:item.clone(),text:text.into()});
                                emitted=true;
                            } else if cursor.records {out.push(Event::Progress);}
                        }
                    } else if block["type"] == "tool_use" {
                        let id = string(&block["id"]);
                        if id.as_ref().is_none_or(|id| cursor.seen.insert(format!("tool:{id}"))) {
                            let name = string(&block["name"]); let kind = tool_kind(name.as_deref().unwrap_or(""));
                            out.push(Event::ToolStart { id, kind, name, timeout_ms:native_timeout(&block["input"]), detail: match block["name"].as_str() {Some("Bash")=>audit_command(&block["input"]["command"]),Some("Read"|"Edit"|"Write"|"NotebookEdit"|"MultiEdit")=>audit_path(&block["input"]["file_path"]),_=>None} });
                        }
                    }
                }
                if emitted {cursor.last_text=Some(text);}
            }
            let u = &message["usage"];
            if u.is_object() {
                let used = u["input_tokens"].as_u64().and_then(|i| i.checked_add(u["cache_read_input_tokens"].as_u64()?).and_then(|n| n.checked_add(u["cache_creation_input_tokens"].as_u64()?)));
                cursor.used = used;
                out.push(Event::Context { used, window: cursor.window });
            }
        }
        if v["type"] == "user" {
            if let Some(blocks) = v["message"]["content"].as_array() {
                for b in blocks {
                    if b["type"] == "tool_result" {
                        out.push(Event::ToolEnd { id: string(&b["tool_use_id"]), status: Some(if b["is_error"] == true { "failed" } else { "completed" }.into()) });
                    } else if cursor.records && b["type"] == "text" && b["text"].as_str().is_some_and(|s|!s.is_empty()) {out.push(Event::Progress);}
                }
            }
        }
        if v["type"] == "result" {
            if let Some(text) = v["result"].as_str().filter(|s|cursor.last_text.as_deref()!=Some(*s)) {
                cursor.last_text=Some(text.into()); out.push(Event::Message {item:None,text:text.into()});
            }
        }
    }

fn terminal(cursor: &events::Cursor,v: &Value,_out: &mut Vec<Event>,_thread: Option<&str>,_turn: Option<&str>) -> Option<&'static str> {
    events::default_terminal(Client::Claude,cursor,v).or_else(||session_end(v))
}

fn session_end(v: &Value) -> Option<&'static str> {(v["type"]=="system" && v["subtype"]=="turn_duration").then_some("unknown")}

fn assistant_failure(v: &Value) -> Option<AssistantFailure> {
    if v["type"] != "assistant" || !(v["isApiErrorMessage"] == true || v["error"].is_string()) { return None; }
    let code = string(&v["error"]);
    let detail = v["message"]["content"].as_array().map(|blocks| blocks.iter()
        .filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join(""))
        .or_else(|| string(&v["message"]["content"])).filter(|s| !s.is_empty());
    Some(AssistantFailure { error: if code.as_deref() == Some("authentication_failed") {"auth-rejected"} else {"client-error"}, code, detail })
}

fn permission_mode(v: &Value) -> Option<String> {v["permissionMode"].as_str().filter(|s|!s.is_empty()).map(String::from)}

fn lifecycle(v:&Value)->Option<String> {
 let name:String=match v["type"].as_str()? {"system"=>[v["type"].as_str(),v["subtype"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" "),"result"|"control_request"|"control_response"=>v["type"].as_str()?.into(),_=>return None};
    let name = name.replace("task_", "turn_").replace(['_', '.', '/'], " ");
    Some(if name == "init" {"session started".into()} else {name})
}

fn advances(_: &Value)->bool {false}

/// One Bash or Monitor call; only a confirmed start makes it a job.
struct Job { tool: String, label: String, background: bool, task: Option<String> }

/// Claude background jobs the last turn left running, oldest first. Log evidence only; this grants no process or stop authority.
pub(crate) fn live_background(values: &[Value]) -> Vec<String> {
    let mut jobs=Vec::<Job>::new();
    let mut stops=BTreeMap::<String,(String,bool)>::new(); // stop tool-use ID -> target, failed
    let mut ended=BTreeSet::<String>::new();
    for v in values {
        match v["type"].as_str() {
            Some("system") if v["subtype"]=="init" => {jobs.clear();stops.clear();ended.clear();}
            Some("system") if v["subtype"]=="task_notification" => {
                if terminal_status(v["status"].as_str()) {ended.extend(["task_id","tool_use_id"].iter().filter_map(|k|string(&v[*k])));}
            }
            Some("queue-operation") => notifications(v["content"].as_str().unwrap_or(""),&mut ended),
            Some("attachment") if v["attachment"]["type"]=="queued_command" => notifications(v["attachment"]["prompt"].as_str().unwrap_or(""),&mut ended),
            Some("assistant") => for b in v["message"]["content"].as_array().into_iter().flatten().filter(|b|b["type"]=="tool_use") {
                let Some(id)=b["id"].as_str() else {continue;};
                let input=&b["input"];
                match b["name"].as_str() {
                    Some(name@("Bash"|"Monitor")) if !jobs.iter().any(|j|j.tool==id) => {
                        let (name,background)=if name=="Bash" {("Bash",input["run_in_background"]==true)} else {("Monitor",true)};
                        jobs.push(Job{tool:id.into(),label:name.into(),background,task:None});
                    }
                    Some("TaskStop"|"KillShell"|"KillBash") => {
                        if let Some(target)=input["task_id"].as_str().or(input["shell_id"].as_str()).or(input["bash_id"].as_str()) {stops.insert(id.into(),(target.into(),false));}
                    }
                    _ => {}
                }
            },
            Some("user") => {
                let content=&v["message"]["content"];
                if let Some(text)=content.as_str() {notifications(text,&mut ended);}
                let blocks=content.as_array().map_or(&[][..],Vec::as_slice);
                let results=blocks.iter().filter(|b|b["type"]=="tool_result").count();
                let native=v.get("tool_use_result").or(v.get("toolUseResult")).filter(|_|results==1).unwrap_or(&Value::Null);
                for b in blocks {
                    if b["type"]=="text" {notifications(b["text"].as_str().unwrap_or(""),&mut ended);}
                    if b["type"]!="tool_result" {continue;}
                    let Some(id)=b["tool_use_id"].as_str() else {continue;};
                    let failed=b["is_error"]==true;
                    if let Some(stop)=stops.get_mut(id) {
                        if failed {stop.1=true;} else {ended.insert(stop.0.clone());}
                    } else if let Some(job)=jobs.iter_mut().find(|j|j.tool==id) {
                        if failed {ended.insert(id.into());}
                        else if job.task.is_none() {job.task=started_task(job.background,native,&b["content"]);}
                    }
                }
            }
            _ => {}
        }
    }
    // A stop in flight already leaves nothing to continue; only its failed result restores the job.
    let stopping=stops.values().filter(|(_,failed)|!failed).map(|(target,_)|target).collect::<BTreeSet<_>>();
    let gone=|id:&String|ended.contains(id) || stopping.contains(id);
    jobs.into_iter().filter(|j|!gone(&j.tool) && j.task.as_ref().is_some_and(|t|!gone(t))).map(|j|j.label).collect()
}

/// The native task a successful start names. Output of a foreground command is never read for one.
fn started_task(background: bool, native: &Value, content: &Value) -> Option<String> {
    let structured=native["backgroundTaskId"].as_str()
        .or_else(||if background {native["taskId"].as_str().or(native["task_id"].as_str()).or(native["shellId"].as_str())} else {None})
        .map(String::from);
    structured.or_else(||{
        if !background {return None;}
        let text=match content {
            Value::String(s)=>s.clone(),
            Value::Array(parts)=>parts.iter().filter_map(|p|p["text"].as_str()).collect::<Vec<_>>().join(" "),
            _=>return None,
        };
        let (_,rest)=["with ID: ","(task "].iter().find_map(|mark|text.split_once(mark))?;
        Some(rest.chars().take_while(|c|c.is_ascii_alphanumeric() || matches!(c,'_'|'-')).collect())
    }).filter(|id:&String|!id.is_empty() && id.len()<=256)
}

fn terminal_status(status: Option<&str>) -> bool {
    matches!(status,Some("completed"|"failed"|"killed"|"stopped"|"cancelled"|"error"|"timeout"|"expired"))
}

fn notifications(text: &str, ended: &mut BTreeSet<String>) {
    for part in text.split("<task-notification>").skip(1) {
        let Some((body,_))=part.split_once("</task-notification>") else {continue;};
        let field=|tag:&str|body.split_once(&format!("<{tag}>"))?.1.split_once(&format!("</{tag}>")).map(|(v,_)|v.trim().to_string());
        if terminal_status(field("status").as_deref()) {ended.extend(field("task-id").into_iter().chain(field("tool-use-id")));}
    }
}

fn session(text:&str)->Option<String> {text.lines().filter_map(|l|serde_json::from_str::<Value>(l).ok()).find_map(|v|(matches!(v["type"].as_str(),Some("system"|"result"))).then(||v["session_id"].as_str().map(String::from)).flatten())}

fn read_snapshot(events:&[Value],_rollout:Option<&Path>)->Snapshot {
 let found=events.iter().rev().find(|v| v["type"] == "rate_limit_event").map(|v| {
            let i = &v["rate_limit_info"];
            Snapshot { used_percent: i["utilization"].as_f64().and_then(|u| percent(u * 100.0)), resets_at: i["resetsAt"].as_u64(),
                window: i["rateLimitType"].as_str().filter(|w| model_code(w)).map(String::from), unknown: None }
        });
    match found {
        Some(mut s) => {
            if s.used_percent.is_none() || s.resets_at.is_none() { s.unknown = Some("partial-snapshot"); }
            s
        }
        None => Snapshot { unknown: Some(if false { "no-rollout" } else { "not-reported" }), ..Snapshot::default() },
    }
}

fn read_report(text:&str,events:&[Value],rollout:Option<&Path>)->Report {let end_index=events.iter().rposition(|v|ends(v) || assistant_failure(v).is_some()); let failure=end_index.and_then(|i|assistant_failure(&events[i])).or_else(|| {
 let end=end_index.map(|i|&events[i]);
 if !end.is_some_and(|v|v["type"]=="result" && v["is_error"]==true) {return None;}
 events[..end_index?].iter().rev().take_while(|v|!ends(v) && !(v["type"]=="system" && matches!(v["subtype"].as_str(),Some("init"|"turn_duration")))).find(|v|v["type"]=="assistant").and_then(assistant_failure)
 }); classify_report(text,events,rollout,end_index,failure,live_background(events))}

static PROTOCOL: Protocol=Protocol {history_provenance:|_,_,_|false,history_parent:|v|v["sessionId"].as_str().map(String::from),rollout_report:|_,_|{},history_formats:&["claude-stream","claude-native"],history_session:history::history_session,history_normalize:history::normalize,history_validity:history::validity,ends,start_limit,read_calls,final_text,turn_ended:native_turn_ended,parse_event,terminal,assistant_failure,permission_mode,lifecycle,advances,session,snapshot:read_snapshot,report:read_report,prepare_command:stdio_command,input_bytes:plain_input,native_record,native_metadata:None,finish_native_report,session_report,reported,update_limits,account_limits,identity:account_identity,credentials,oauth_token,oauth_usage};

pub(crate) fn claude_info(v: &Value) -> Option<Value> {
    let mut info=if v["type"]=="rate_limit_event" {Some(&v["rate_limit_info"])}
        else {v.get("quotaLimits").filter(|v|v.is_object()).or_else(||v.pointer("/apiErrorParams/rate_limit_info").filter(|v|v.is_object()))}?.clone();
    if v["type"]!="rate_limit_event" && info.is_object() {
        let fallback=&v["apiErrorParams"]["rate_limit_info"];
        if info["rateLimitType"].as_str().is_none() {
            info["rateLimitType"]=fallback["rateLimitType"].clone();
            if let Some(native)=info["rateLimitType"].as_str().and_then(|name|info["unifiedWindows"].get(name)).cloned() {
                if info["utilization"].as_f64().is_none() {info["utilization"]=native["utilization"].clone();}
                if info["resetsAt"].as_u64().is_none() {info["resetsAt"]=native["resetsAt"].clone();}
            }
        }
        if info["resetsAt"].as_u64().is_none() {info["resetsAt"]=fallback["resetsAt"].clone();}
    }
    Some(info)
}

pub(crate) fn claude_window(name: Option<String>, info: &Value, id: Option<String>) -> Window {
    let mut w=window(name,info["utilization"].as_f64(),info["resetsAt"].as_u64(),id);
    w.exhausted=match (info["status"].as_str(),w.exhausted) {(Some("rejected"),_)|(_,Some(true))=>Some(true),(Some("allowed"|"allowed_warning"),_)=>Some(false),(_,used)=>used};
    w
}

pub(crate) fn claude_windows(v: &Value) -> Vec<Window> {
    let Some(fields) = v.as_object() else { return unknown("partial-snapshot"); };
    let mut windows = vec![];
    for (name, w) in fields {
        if (name.starts_with("five_hour") || name.starts_with("seven_day")) && name != "seven_day_breakdown" && !w.is_null() {
            windows.push(window(Some(name.clone()), w["utilization"].as_f64().map(|n|n/100.0), reset(&w["resets_at"]), None));
        }
    }
    if let Some(list) = v.get("limits").filter(|l| !l.is_null()) {
        if let Some(list) = list.as_array() {
            for (index, l) in list.iter().enumerate() {
                let mut name = match l["group"].as_str() { Some("session") => "five_hour".to_string(), Some("weekly") => "seven_day".to_string(), _ => format!("limits[{index}]") };
                let scope = l.get("scope").filter(|s|!s.is_null());
                let model = scope.and_then(|s|s.get("model")).filter(|m|!m.is_null());
                let mut invalid = name.starts_with("limits[") || scope.is_some_and(|s|!s.is_object() || s.as_object().is_some_and(|s|s.iter().any(|(k,v)|k!="model" && !v.is_null())));
                if let Some(model) = model {
                    if let Some(model) = model["display_name"].as_str().or(model["id"].as_str()).filter(|m|!m.is_empty()) { name.push('_'); name.push_str(&model.to_ascii_lowercase().replace(' ', "_")); }
                    else { invalid = true; }
                }
                windows.retain(|w|w.window.as_deref()!=Some(&name));
                windows.push(window(Some(name), if invalid {None} else {l["percent"].as_f64().map(|n|n/100.0)}, reset(&l["resets_at"]), None));
            }
        } else { windows.extend(unknown("limits-list-unreadable")); }
    }
    if windows.is_empty() { unknown("not-reported") } else { windows }
}

fn reported(v:&Value,_now:u64)->Option<Vec<Window>> {let id=v["event_id"].as_str().or(v["uuid"].as_str()).or(v["id"].as_str()).map(String::from); let i=claude_info(v)?;
        let mut windows = vec![];
        if let Some(all) = i["unifiedWindows"].as_object() {
            for (name, w) in all {
                windows.push(claude_window(Some(name.clone()), w, id.clone()));
            }
        }
        if let Some(name) = i["rateLimitType"].as_str() {
            let top = claude_window(Some(name.into()), &i, id.clone());
            if let Some(at) = windows.iter().position(|w| w.window.as_deref() == Some(name)) { windows[at] = top; }
            else { windows.push(top); }
        }
        return Some(if windows.is_empty() {vec![claude_window(None,&i,id)]} else {windows});

}

fn update_limits(v:&Value,now:u64,all:&mut BTreeMap<String,Vec<Window>>)->bool {
    let Some(windows) = reported(v, now) else { return false; };
    for w in windows { all.insert(w.window.clone().unwrap_or_default(), vec![w]); }
    true
}

fn account_limits(home:Option<&Path>,now:i64)->Vec<Window> {let client=Client::Claude;
    let explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(client)) else { return unknown("home-unavailable"); };
 oauth_account(client,&home,explicit,now)
}

fn account_identity(_:Option<&Path>)->Option<String> {None}

fn credentials(home:&Path,explicit:bool)->Res<String> {
 #[cfg(target_os="macos")] {let suffix=if explicit {format!("-{}",&crate::sha256_hex(home.to_string_lossy().as_bytes())[..8])} else {String::new()};
 if let Some(text)=limits::keychain(&format!("Claude Code-credentials{suffix}"),None)? {return Ok(text);}}
 // shortcut: Windows keyring-only logins read unknown; add its native reader when qualified.
 if let Ok(text)=std::fs::read_to_string(home.join(".credentials.json")) {return Ok(text);}
  Err("credential-unreadable".into())
}

fn access_token(oauth:&Value,now:i64)->Res<&str> {
 let token=oauth["accessToken"].as_str().filter(|s|!s.is_empty() && s.bytes().all(|b|b.is_ascii_alphanumeric() || b"_.-".contains(&b))).ok_or("oauth-unavailable")?;
 let expiry=oauth["expiresAt"].as_i64().map(|n|n/1000);
 if expiry.is_some_and(|end|end<=now) {return Err("credential-expired".into());} Ok(token)
}
fn oauth_token(credential:&Value,now:i64)->Res<&str> {access_token(&credential["claudeAiOauth"],now)}

fn oauth_usage(token:&str)->Res<Vec<Window>> {
https(&format!("url = \"https://api.anthropic.com/api/oauth/usage\"\nheader = \"Authorization: Bearer {token}\"\nheader = \"anthropic-beta: oauth-2025-04-20\"\n")).map(|v|claude_windows(&v))
}

/// Claude assigns the job/session on a start. Resume never overrides its saved settings or permissions.
pub(crate) fn background_command(mut base: Command, spec: &Spec) -> Res<Command> {
    if spec.client != Client::Claude { return Err("background requires Claude".into()); }
    base.arg("--bg");
    if spec.resume {
        if !plain_id(&spec.session) { return Err("background session invalid".into()); }
        base.args(["--resume", &spec.session]);
    } else { base.args(&spec.args); }
    Ok(environment(base, spec))
}

/// Only the launcher exchange. Any uncertain response requires observation of the original submission, never replay.
/// `reference` points to caller-protected, durably bound input: Claude forwards stdin to child argv.
pub(crate) fn background_launch(base: Command, spec: &Spec, reference: &str, budget: Duration) -> Res<Result<std::process::Output, crate::process::Failure>> {
    Ok(crate::process::exchange("agent.background-launch", &mut background_command(base, spec)?, reference.as_bytes(), budget))
}

/// The job ID from the launcher's first line `backgrounded · <job>[ · <name>]`; the command hints after it are not IDs.
pub(crate) fn background_job_id(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    line.split('·').nth(1).map(str::trim).filter(|t| plain_id(t)).map(String::from)
}

/// `filtered` adds `--cwd <root>`; Claude's filter misses roots under `.claude/worktrees` (#222), so callers retry
/// unfiltered and `BackgroundJob::read` still binds the exact root.
pub(crate) fn background_jobs(mut base: Command, spec: &Spec, filtered: bool, budget: Duration) -> Res<Vec<Value>> {
    if spec.client != Client::Claude { return Err("background requires Claude".into()); }
    base.args(["agents", "--json", "--all"]);
    if filtered { base.arg("--cwd").arg(&spec.root); }
    let output = crate::process::output("agent.background-jobs", &mut environment(base, spec), budget).map_err(|e| e.to_string())?;
    serde_json::from_slice::<Vec<Value>>(&output.stdout).map_err(|e| format!("background jobs invalid: {e}"))
}

/// Validated native readiness. This is neither accepted input nor an execution grant.
#[derive(Clone, Debug)]
pub struct BackgroundJob {
    pub id: String,
    pub session: String,
    pub pid: Option<u32>,
    pub(crate) root: PathBuf,
    pub(crate) scope: String,
}

impl BackgroundJob {
    pub(crate) fn read(row: &Value, spec: &Spec, job: &str) -> Res<Self> {
        if spec.client != Client::Claude || !plain_id(job) || row["id"] != job || row["kind"] != "background" {
            return Err("background job identity differs".into());
        }
        let root = spec.root.canonicalize().map_err(|e| e.to_string())?;
        if row["cwd"].as_str().and_then(|p| Path::new(p).canonicalize().ok()).as_ref() != Some(&root) {
            return Err("background project differs".into());
        }
        let session = row["sessionId"].as_str().filter(|s| plain_id(s)).ok_or("background session unknown")?;
        if spec.resume && session != spec.session { return Err("background resumed another session".into()); }
        row["state"].as_str().filter(|s| matches!(*s, "working" | "idle" | "done" | "blocked" | "failed" | "crashed" | "stopped"))
            .ok_or("background state unknown")?;
        let pid = if row["pid"].is_null() { None } else { Some(row["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0).ok_or("background pid invalid")?) };
        Ok(Self { id: job.into(), session: session.into(), pid, root, scope:scope(spec) })
    }
}

/// Caller must already own this execution. Acknowledges only a stop request; inspect jobs/process identity for exit.
pub(crate) fn background_stop(mut base: Command, spec: &Spec, job: &BackgroundJob, budget: Duration) -> Res<std::process::Output> {
    if spec.client != Client::Claude || spec.root.canonicalize().ok().as_ref() != Some(&job.root) || scope(spec)!=job.scope
        || (spec.resume && spec.session != job.session) { return Err("background stop scope differs".into()); }
    base.args(["stop", &job.id]);
    crate::process::output("agent.background-stop", &mut environment(base, spec), budget).map_err(|e| e.to_string())
}

/// The caller owns app presence/UI and a durable submission binding. A failed response never permits replay.

/// A native Remote Control session URL.
pub(crate) fn remote_url(url: &str) -> bool {
    url.strip_prefix("https://claude.ai/code/session_").is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The Remote Control URL of a native bridge session `cse_<id>`.
fn bridge_url(id: &str) -> Option<String> {
    let url = format!("https://claude.ai/code/session_{}", id.strip_prefix("cse_")?);
    remote_url(&url).then_some(url)
}

/// A Claude link absence: the cause and its native evidence text.
pub(crate) type Absence = (Cause, Option<String>);

/// Claude Remote Control link of `session` from native records only (RUNTIME.md Session observation): `seen` from the
/// transcript, then the background job state, then the debug log. `requested` is the launch's `--remote-control`.
pub(crate) fn claude_link(home: Option<&Path>, session: &str, job: Option<&str>, mut requested: bool, seen: Option<String>) -> Result<String, Absence> {
    if let Some(url) = seen { return Ok(url); }
    let unreadable = |what: &str, e: &dyn std::fmt::Display| (Cause("evidence-unreadable"), Some(format!("{what}: {e}")));
    let read = |path: PathBuf, what: &str| -> Result<Option<Vec<u8>>, Absence> {
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(unreadable(what, &e)),
        };
        // ponytail: last 16 MiB only; a bridge line older than that reads as the newer evidence or `pending`.
        let size = file.metadata().map_err(|e| unreadable(what, &e))?.len();
        file.seek(SeekFrom::Start(size.saturating_sub(16 << 20))).map_err(|e| unreadable(what, &e))?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes).map_err(|e| unreadable(what, &e))?;
        Ok(Some(bytes))
    };
    let Some(home) = home else { return Err((Cause("evidence-unreadable"), Some("no Claude home".into()))) };
    if let Some(job) = job.filter(|j| plain_id(j)) {
        if let Some(bytes) = read(home.join("jobs").join(job).join("state.json"), "job state")? {
            let state: Value = serde_json::from_slice(&bytes).map_err(|e| unreadable("job state", &e))?;
            if let Some(url) = state["bridgeSessionId"].as_str().and_then(bridge_url) { return Ok(url); }
            requested |= state["respawnFlags"].as_array().is_some_and(|a| a.iter().any(|f| f == "--remote-control"));
        }
    }
    if !plain_id(session) { return Err((Cause("session-unknown"), None)); }
    let Some(log) = read(home.join("debug").join(format!("{session}.txt")), "debug log")? else {
        return Err((Cause(if requested { "unlogged" } else { "not-requested" }), None));
    };
    let failure = |detail: &str| {
        let d = detail.to_ascii_lowercase();
        let refused = ["polic", "disabled", "turned off", "subscription", "login", "logged in", "only available", "not available"].iter().any(|w| d.contains(w));
        Err((Cause(if refused { "disabled" } else { "failed" }), Some(detail.trim().to_string()).filter(|d| !d.is_empty())))
    };
    let mut last = None;
    for line in String::from_utf8_lossy(&log).lines() {
        // Only Claude's own logger prefix (`<time> [<LEVEL>] `) may precede the marker, so logged content cannot spoof it.
        let Some(at) = line.find("[bridge:repl] ") else { continue };
        let prefix = &line[..at];
        if !(prefix.is_empty() || prefix.ends_with("] ") && prefix.split_whitespace().count() <= 2) { continue; }
        let rest = &line[at + 14..];
        let found = if let Some(id) = ["Hook initialized, session=", "Mirror initialized, session="].iter().find_map(|p| rest.strip_prefix(p)) {
            bridge_url(id.trim()).map(Ok)
        } else if let Some(d) = rest.strip_prefix("Init failed: ") {
            Some(failure(d.split("; consecutive failures").next().unwrap_or(d)))
        } else if let Some(d) = rest.strip_prefix("notifyBridgeFailed detail=\"") {
            Some(failure(d.split("\" outboundOnly=").next().unwrap_or(d)))
        } else if rest.starts_with("Org policy now refuses the connected bridge") {
            Some(failure(rest))
        } else { None };
        if found.is_some() { last = found; }
    }
    last.unwrap_or(Err((Cause(if requested { "pending" } else { "not-requested" }), None)))
}

fn native_record(turn:&mut native::NativeTurn,v:&Value)->Res<()> {

                    if v["sessionId"].as_str().or(v["session_id"].as_str()).is_some_and(|s| s != turn.spec.session) {
                        return Err("native Claude session differs".into());
                    }
                    if v["subtype"] == "bridge_status" {
                        if let Some(url) = v["url"].as_str().filter(|s| remote_url(s)) { turn.remote = Some(url.into()); }
                    }
                    if v["type"] == "user" {
                        let content = &v["message"]["content"];
                        let input = content.as_str().or_else(|| content.as_array().filter(|a| a.len() == 1 && a[0]["type"] == "text").and_then(|a| a[0]["text"].as_str()));
                        if input == Some(turn.input.as_str()) {
                            if v["sessionId"].as_str().or(v["session_id"].as_str()) != Some(turn.spec.session.as_str()) {
                                return Err("native Claude input session unproved".into());
                            }
                            if turn.accepted { return Err("native input appears more than once".into()); }
                            turn.accepted = true;
                        } else if input.is_some() && turn.accepted && !turn.ended { return Err("another input superseded the native turn".into()); }
                    }
                    if turn.accepted && !turn.ended {
                        if matches!(v["type"].as_str(),Some("assistant"|"rate_limit_event")) {turn.events.push(v.clone());}
                        if v["type"] == "system" && v["subtype"] == "turn_duration" {
                            turn.ended = true;
                            turn.duration = v["durationMs"].as_u64().and_then(|n| n.checked_mul(1000));
                        }
                    }
 Ok(())
}

fn finish_native_report(turn:&native::NativeTurn,report:&mut Report) {
            report.text = turn.events.iter().rev().filter(|v|v["type"] == "assistant").find_map(|v|v["message"]["content"].as_array()?.iter().rev().find_map(|b|(b["type"]=="text").then(||b["text"].as_str().map(String::from)).flatten()));

}

fn session_report(_: &str,_:&mut Report) {}

/// Claude `--settings` JSON: a `SessionStart` hook on `compact` that runs `compact_hook` (rule re-delivery after
/// compaction) and `env` for the session, which survives resume. None omits the hook.
pub(crate) fn claude_settings(compact_hook: Option<&str>, env: &[(&str, &str)]) -> String {
    let env: serde_json::Map<String, Value> = env.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    let mut v = json!({"env": env});
    if let Some(cmd) = compact_hook {
        v = json!({"hooks": {"SessionStart": [{"matcher": "compact", "hooks": [{"type": "command", "command": cmd}]}]}, "env": v["env"]});
    }
    v.to_string()
}
