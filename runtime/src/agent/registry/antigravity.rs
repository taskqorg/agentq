#[path="antigravity/history.rs"]
mod history;
#[path="antigravity/parsing.rs"]
mod parsing;
use self::parsing::*;
// Antigravity registration: native worker and installation behavior.
use super::super::super::events::{string,tool_kind,audit_path,audit_command,native_timeout};
use super::*;
use super::super::super::limits::{window,unknown,reset,https,token_text,oauth_account};

static ADAPTER: Adapter = Adapter {client: Client::Antigravity, live: AtomicUsize::new(0), descriptor: Descriptor { name: "antigravity", label: "Antigravity", brand: "Google", family: "gemini", models: &[], minimum: "",
        capabilities: &["history", "turn", "resume", "title", "account-limits"] } , ops: NativeOps {catalogue:|_,_|Err("catalogue unsupported".into()),background_launch:|_,_,_,_|Err("background unsupported".into()),background_jobs:|_,_,_,_|Err("background unsupported".into()),background_job:|_,_,_|Err("background unsupported".into()),background_stop:|_,_,_,_|Err("background unsupported".into()),background_id:|_|None,app_queue:|_,_,_,_|Err("app-queue unsupported".into()),app_command:|_,_|Err("app-server unsupported".into()), protocol:&PROTOCOL, home_var:"ANTIGRAVITY_HOME",alternate_home_var:Some("ANTIGRAVITY_APP_DATA_DIR"),session_vars:&["ANTIGRAVITY_CONVERSATION_ID"],auth_vars:&["GOOGLE_API_KEY", "GEMINI_API_KEY", "GOOGLE_APPLICATION_CREDENTIALS"],home:native_home,executable:native_executable,new_session,args:native_args,mapped, extras, environment:env, settings, login, preparation, restore_preparation:None, sandbox:None,hook_status:None, limits_for, validate, protected, refusal, transcript, background_transcript, link, session_link } };

pub(super) const ENTRY: Entry = Entry { name:"antigravity", aliases:&["agy"], order:4, profile_order:4, worker_order:2, onboard:true,
    profile:Profile {kind:"antigravity",label:"Antigravity",aliases:&["agy"],operations:&["hooks"]},
    runtime:Some(&ADAPTER), builtin:Some(&ADAPTER),
    root:config_root,files,setup,inspect,hook,hook_key:"agentq",fallback,executable:cli,diagnose:no_diagnosis,provider:None,
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
fn login(_: Command,_:bool) -> Check {check("unknown","no login status command")}
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

#[allow(non_upper_case_globals)]
impl Client { pub(crate) const Antigravity: Self=Self("antigravity"); }

fn parse_event(cursor: &mut events::Cursor,v: &Value,out: &mut Vec<Event>,_thread: Option<&str>) {
        if v["event"] == "init" || v["type"] == "init" { out.push(Event::TurnStart); }
        if v["event"] == "result" {
            if let Some(text) = string(&v["result"]["response"]) { out.push(Event::Message { item: None, text }); }
        }
        if v["event"] == "step_update" {
            let s = &v["step_update"];
            let id = s["step_index"].as_u64().map(|n| n.to_string());
            if cursor.deltas { if let Some(delta) = string(&s["text_delta"]) { out.push(Event::Text { item: id.clone(), delta }); } }
            if s["step_type"] == "tool" {
                match s["state"].as_str() {
                    Some("ACTIVE") => out.push(Event::ToolStart { id: id.clone(), kind: tool_kind(s["tool_name"].as_str().unwrap_or("")), timeout_ms:native_timeout(&s["tool_info"]["parameters"]), name: string(&s["tool_name"]), detail: antigravity_detail(s["tool_name"].as_str(), &s["tool_info"]["parameters"]) }),
                    Some("DONE" | "ERROR" | "FAILED" | "CANCELLED") => out.push(Event::ToolEnd { id, status: string(&s["state"]) }),
                    _ => out.push(Event::Unsupported { reason: "native tool state unsupported" }),
                }
            }
        }
        if v["type"] == "PLANNER_RESPONSE" && v["status"] == "DONE" {
            if let Some(text) = string(&v["content"]) { out.push(Event::Message { item: v["step_index"].as_u64().map(|n| n.to_string()), text }); }
        }
        if v["type"] == "PLANNER_RESPONSE" {
            if let Some(tools)=v["tool_calls"].as_array() {
                for tool in tools {
                    let id=string(&tool["id"]);
                    if id.as_ref().is_none_or(|id|cursor.seen.insert(format!("tool:{id}"))) {
                        let input=tool.get("arguments").or(tool.get("args")).unwrap_or(&Value::Null);
                        out.push(Event::ToolStart {id,kind:tool_kind(tool["name"].as_str().unwrap_or("")),timeout_ms:native_timeout(input),name:string(&tool["name"]),detail:antigravity_detail(tool["name"].as_str(),input)});
                    }
                }
            }
        }
        if v["type"] == "TOOL_RESPONSE" && matches!(v["status"].as_str(),Some("DONE"|"ERROR"|"CANCELLED")) {
            out.push(Event::ToolEnd {id:string(&v["tool_call_id"]),status:string(&v["status"])});
        }
    }

fn terminal(cursor: &events::Cursor,v: &Value,out: &mut Vec<Event>,_thread: Option<&str>,_turn: Option<&str>) -> Option<&'static str> {
    if !ends(v) && v["event"]=="result" {out.push(Event::Unsupported{reason:"native turn status unsupported"});}
    events::default_terminal(Client::Antigravity,cursor,v).or_else(||session_end(v))
}

fn session_end(_: &Value) -> Option<&'static str> {None}

fn assistant_failure(_: &Value) -> Option<events::AssistantFailure> {None}

fn permission_mode(_: &Value) -> Option<String> {None}

fn lifecycle(v:&Value)->Option<String> {
 let name:String={if matches!(v["step_update"]["step_type"].as_str(),Some("system_message"|"checkpoint")) {v["step_update"]["step_type"].as_str()?.into()} else if matches!(v["event"].as_str(),Some("init"|"result")) {v["event"].as_str()?.into()} else {return None;}};
    let name = name.replace("task_", "turn_").replace(['_', '.', '/'], " ");
    Some(if name == "init" {"session started".into()} else {name})
}

fn advances(v:&Value)->bool {v["event"]=="step_update" && matches!(v["step_update"]["step_type"].as_str(),Some("user_input"|"agent_response"|"system_message"|"checkpoint")) && v["step_update"]["state"]=="DONE"}

fn antigravity_detail(name: Option<&str>, input: &Value) -> Option<String> {
    match name {
        Some("run_command")=>audit_command(&input["CommandLine"]),
        Some("view_file"|"write_to_file"|"replace_file_content"|"multi_replace_file_content")=>audit_path(input.get("Path").or(input.get("TargetFile")).unwrap_or(&Value::Null)),
        _=>None,
    }
}

/// Antigravity's conversation ID from `agentapi` JSON output or a stream event.
pub(crate) fn session(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text).into_iter().chain(text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())).find_map(|v| {
        v.pointer("/response/newConversation/conversationId").and_then(Value::as_str)
            .or_else(|| v.pointer("/response/sendMessage/recipientId").and_then(Value::as_str))
            .or_else(|| v["conversationId"].as_str())
            .or_else(|| v["conversation_id"].as_str())
            .or_else(|| v["result"]["conversation_id"].as_str())
            .or_else(|| (matches!(v["type"].as_str(), Some("conversation.started" | "system" | "result"))).then(|| v["session_id"].as_str()).flatten())
            .filter(|s| native::plain_id(s) && *s != "-")
            .map(String::from)
    })
}

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

fn read_report(text:&str,events:&[Value],rollout:Option<&Path>)->Report {let end_index=events.iter().rposition(|v|v["event"]=="result" || matches!(v["type"].as_str(),Some("PLANNER_RESPONSE"|"TOOL_RESPONSE"|"turn.failed"|"turn.cancelled"))).filter(|i|ends(&events[*i])); classify_report(text,events,rollout,end_index)}

static PROTOCOL: Protocol=Protocol {stream_history:"agy-stream",native_history:"antigravity-sidecar",history_alias:false,history_provenance:|_,_,_|false,history_parent:|_|None,rollout_report:|_,_|{},history_formats:&["agy-stream","antigravity-sidecar"],history_session:history::history_session,history_normalize:history::normalize,history_validity:history::validity,ends,start_limit,read_calls,final_text,turn_ended:native_turn_ended,parse_event,terminal,assistant_failure,permission_mode,lifecycle,advances,session,snapshot:read_snapshot,report:read_report,prepare_command:prepare_command,input_bytes:input_bytes,native_record,native_metadata:None,finish_native_report,session_report,reported,update_limits,account_limits,identity:account_identity,credentials,oauth_token,oauth_usage};

fn code_assist_call(token: &str, host: &str, method: &str, body: Value) -> String {
    format!("url = \"https://{host}/v1internal:{method}\"\nheader = \"Authorization: Bearer {token}\"\nheader = \"Content-Type: application/json\"\nheader = \"User-Agent: antigravity/hub/2.28.2\"\ndata = {}\n",json!(body.to_string()))
}

pub(crate) fn code_assist(token: &str, mut read: impl FnMut(&str)->Res<Value>) -> Res<Vec<Window>> {
    let load=read(&code_assist_call(token,"cloudcode-pa.googleapis.com","loadCodeAssist",json!({"metadata":{"ideType":"ANTIGRAVITY"}})))?;
    let host=match load.pointer("/currentTier/usesGcpTos") {Some(Value::Bool(true))=>"cloudcode-pa.googleapis.com",None|Some(Value::Bool(false))=>"daily-cloudcode-pa.googleapis.com",_=>return Err("quota-tier-unknown".into())};
    let body=match load.get("cloudaicompanionProject").filter(|p|!p.is_null()) {Some(Value::String(p))=>json!({"project":p}),None=>json!({}),_=>return Err("quota-project-unknown".into())};
    let quota=read(&code_assist_call(token,host,"retrieveUserQuotaSummary",body))?;
    let mut windows=vec![];
    for group in quota["groups"].as_array().ok_or("quota-groups-unreadable")? {
        let Some(buckets)=group["buckets"].as_array().filter(|b|!b.is_empty()) else {windows.extend(unknown("quota-buckets-unreadable"));continue;};
        for b in buckets {
            windows.push(window(b["bucketId"].as_str().map(String::from),b["remainingFraction"].as_f64().filter(|f|(0.0..=1.0).contains(f)).map(|n|1.0-n),reset(&b["resetTime"]),None));
        }
    }
    Ok(if windows.is_empty() {unknown("not-reported")} else {windows})
}

pub(crate) fn project_identity(load: &Value) -> Option<String> {
    load["cloudaicompanionProject"].as_str().filter(|s|!s.is_empty()).map(String::from)
}

fn reported(_: &Value,_:u64)->Option<Vec<Window>> {None}

fn update_limits(v:&Value,now:u64,all:&mut BTreeMap<String,Vec<Window>>)->bool {
    let Some(windows) = reported(v, now) else { return false; };
    for w in windows { all.insert(w.window.clone().unwrap_or_default(), vec![w]); }
    true
}

fn account_limits(home:Option<&Path>,_executable:Option<&Path>,now:i64)->Vec<Window> {let client=Client::Antigravity;
    let explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(client)) else { return unknown("home-unavailable"); };
 oauth_account(client,&home,explicit,now)
}

fn account_identity(home:Option<&Path>)->Option<String> {let client=Client::Antigravity;
    let explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let home=home.map(Path::to_path_buf).or_else(||base_home(client))?;

            let before=credentials(&home,explicit).ok()?;
            let credential=token_text(&before).ok()?;
            let oauth=credential.get("token").filter(|v|v.is_object()).unwrap_or(&credential);
            let token=access_token(oauth,(unix_ms()/1000).max(0)).ok()?;
            let load=https(&code_assist_call(token,"cloudcode-pa.googleapis.com","loadCodeAssist",json!({"metadata":{"ideType":"ANTIGRAVITY"}}))).ok()?;
            if credentials(&home,explicit).ok()?.as_str()!=before {return None;}
            project_identity(&load)
}

fn credentials(home:&Path,explicit:bool)->Res<String> {
 #[cfg(target_os="macos")] {if !explicit {if let Some(text)=limits::keychain("gemini",Some("antigravity"))? {return Ok(text);}}}
 // shortcut: Windows keyring-only logins read unknown; add its native reader when qualified.
 if let Ok(text)=std::fs::read_to_string(home.join("antigravity-oauth-token")) {return Ok(text);}
 if !explicit {if let Some(parent)=home.parent() {if let Ok(text)=std::fs::read_to_string(parent.join("oauth_creds.json")) {return Ok(text);}}} Err("credential-unreadable".into())
}

fn access_token(oauth:&Value,now:i64)->Res<&str> {
 let token=oauth["access_token"].as_str().filter(|s|!s.is_empty() && s.bytes().all(|b|b.is_ascii_alphanumeric() || b"_.-".contains(&b))).ok_or("oauth-unavailable")?;
 let expiry=reset(&oauth["expiry"]).and_then(|n|i64::try_from(n).ok()).or_else(||oauth["expiry_date"].as_i64().map(|n|n/1000));
 if expiry.is_some_and(|end|end<=now) {return Err("credential-expired".into());} Ok(token)
}
fn oauth_token(credential:&Value,now:i64)->Res<&str> {access_token(credential.get("token").filter(|v|v.is_object()).unwrap_or(credential),now)}

fn oauth_usage(token:&str)->Res<Vec<Window>> {code_assist(token,https)}

fn native_record(turn:&mut native::NativeTurn,v:&Value)->Res<()> {

                    if matches!(v["type"].as_str(), Some("USER_INPUT")) {
                        let content = v["content"].as_str().unwrap_or_default();
                        if content == turn.input {
                            if turn.accepted { return Err("native input appears more than once".into()); }
                            turn.accepted = true;
                        } else if !content.is_empty() && turn.accepted && !turn.ended {
                            return Err("another input superseded the native turn".into());
                        }
                    } else if turn.accepted {
                        if v["type"] == "PLANNER_RESPONSE" {
                            turn.events.push(v.clone());
                            if v["status"] == "CANCELLED" {
                                turn.ended = true;
                                turn.aborted = true;
                            } else if ends(v) {
                                turn.ended = true;
                            } else {
                                turn.ended = false;
                            }
                        } else if v["type"] == "TOOL_RESPONSE" {
                            turn.ended = false;
                        }
                    }
 Ok(())
}

fn finish_native_report(_: &native::NativeTurn,_:&mut Report) {}

fn session_report(_: &str,_:&mut Report) {}

#[derive(Clone, Debug, Default)]
struct AntigravityTranscript {
    native_agentapi: bool,
    session: Option<String>,
    cursor: u64,
    identity: Option<String>,
    prefix: Option<String>,
    partial: Vec<u8>,
    pending: std::collections::VecDeque<Value>,
    full_step: Option<u64>,
    full_cursor: u64,
    full_partial: Vec<u8>,
    full_identity: Option<String>,
    full_prefix: Option<String>,
}

fn is_native_agentapi(cmd: &Command, spec: &Spec) -> bool {
    if spec.client != Client::Antigravity {
        return false;
    }
    std::iter::once(cmd.get_program()).chain(cmd.get_args()).any(|arg| {
        matches!(Path::new(arg).file_name().and_then(|n| n.to_str()),
            Some("agentapi" | "agentapi.exe" | "agentapi.cmd" | "language_server" | "language_server.exe"))
    })
}

fn transcript_paths(home: &Path, session: &str) -> (PathBuf, PathBuf) {
    let logs = home.join("brain").join(session).join(".system_generated").join("logs");
    (logs.join("transcript.jsonl"), logs.join("transcript_full.jsonl"))
}

impl AntigravityTranscript {
    fn baseline(home: Option<&Path>, session: Option<String>) -> Res<Self> {
        let mut state = Self { native_agentapi: true, session, ..Self::default() };
        if let Some(session) = state.session.as_deref() {
            if !native::plain_id(session) || session == "-" { return Err("antigravity session invalid".into()); }
            let home = home.ok_or("antigravity home unavailable")?;
            let mut file = File::open(transcript_paths(home, session).0).map_err(|e| format!("antigravity baseline: {e}"))?;
            state.cursor = file.metadata().map_err(|e| e.to_string())?.len();
            if state.cursor > 0 {
                file.seek(SeekFrom::Start(state.cursor-1)).map_err(|e| e.to_string())?;
                let mut last = [0]; file.read_exact(&mut last).map_err(|e| e.to_string())?;
                if last != [b'\n'] { return Err("antigravity baseline incomplete".into()); }
            }
            state.identity = Some(native::file_id(&file)?);
            state.prefix = Some(native::digest_prefix(&mut file, state.cursor)?);
        }
        Ok(state)
    }

    fn full_record(&mut self, path: &Path, step: u64) -> Res<Option<Value>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.full_identity.is_none() => return Ok(None),
            Err(e) => return Err(format!("antigravity full transcript: {e}")),
        };
        if self.full_step != Some(step) {
            self.full_step = Some(step); self.full_cursor = 0; self.full_partial.clear();
            self.full_identity = None; self.full_prefix = None;
        }
        let identity = native::file_id(&file)?;
        if self.full_identity.as_ref().is_some_and(|id| *id != identity) { return Err("antigravity full transcript replaced".into()); }
        let prefix = native::digest_prefix(&mut file, self.full_cursor)?;
        if self.full_prefix.as_ref().is_some_and(|p| *p != prefix) { return Err("antigravity full transcript rewritten".into()); }
        file.seek(SeekFrom::Start(self.full_cursor)).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        (&mut file).take(8*1024*1024).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        self.full_cursor += bytes.len() as u64;
        self.full_partial.extend(bytes);
        self.full_identity = Some(identity);
        self.full_prefix = Some(native::digest_prefix(&mut file, self.full_cursor)?);
        let mut consumed = 0;
        let mut found = None;
        for row in self.full_partial.split_inclusive(|b| *b == b'\n') {
            if !row.ends_with(b"\n") { break; }
            consumed += row.len();
            let v: Value = serde_json::from_slice(row).map_err(|_| "antigravity full transcript invalid JSON")?;
            if !v.is_object() { return Err("antigravity full transcript invalid record".into()); }
            if v["step_index"].as_u64() == Some(step) && v.get("truncated_fields").is_none() { found = Some(v); }
        }
        self.full_partial.drain(..consumed);
        Ok(found)
    }

    fn append(&mut self, home: &Path, log: &Path) -> Res<()> {
        let Some(session) = self.session.as_deref() else { return Ok(()); };
        if !native::plain_id(session) || session == "-" { return Err("antigravity session invalid".into()); }
        let (compact, full) = transcript_paths(home, session);
        let mut file = match File::open(compact) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.identity.is_none() => return Ok(()),
            Err(e) => return Err(format!("antigravity transcript: {e}")),
        };
        let identity = native::file_id(&file)?;
        if self.identity.as_ref().is_some_and(|id| *id != identity) { return Err("antigravity transcript replaced".into()); }
        let prefix = native::digest_prefix(&mut file, self.cursor)?;
        if self.prefix.as_ref().is_some_and(|p| *p != prefix) { return Err("antigravity transcript rewritten".into()); }
        file.seek(SeekFrom::Start(self.cursor)).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        (&mut file).take(8*1024*1024).read_to_end(&mut bytes).map_err(|e| format!("antigravity transcript: {e}"))?;
        self.cursor += bytes.len() as u64;
        self.partial.extend(bytes);
        let mut consumed = 0;
        for row in self.partial.split_inclusive(|b| *b == b'\n') {
            if !row.ends_with(b"\n") { break; }
            consumed += row.len();
            let value: Value = serde_json::from_slice(row).map_err(|_| "antigravity transcript invalid JSON")?;
            if !value.is_object() { return Err("antigravity transcript invalid record".into()); }
            self.pending.push_back(value);
        }
        self.partial.drain(..consumed);
        self.identity = Some(identity);
        self.prefix = Some(native::digest_prefix(&mut file, self.cursor)?);
        let mut output = File::options().append(true).open(log).map_err(|e| e.to_string())?;
        while let Some(mut value) = self.pending.front().cloned() {
            if value.get("truncated_fields").is_some() {
                let step = value["step_index"].as_u64().ok_or("antigravity truncated step missing index")?;
                let Some(v) = self.full_record(&full, step)? else { break; };
                if v["type"] != value["type"] || v["status"] != value["status"] { break; }
                value = v;
            }
            writeln!(output, "{value}").map_err(|e| format!("agent log: {e}"))?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

/// The turn's client process: the owned child, or after recovery the child observed by its native identity.

fn prepare_command(mut base:Command,spec:&Spec,text:&str)->Res<(Command,Box<dyn TurnExtension>)> {let home=spec_home(spec); if !is_native_agentapi(&base,spec) {return stdio_command(base,spec,text);}
            if std::iter::once(base.get_program()).chain(base.get_args()).any(|arg| Path::new(arg).file_stem().is_some_and(|n| n == "language_server")) {
                base.arg("agentapi");
            }
            base.arg(if spec.resume {"send-message"} else {"new-conversation"}).args(&spec.args);
            if spec.resume { base.arg(&spec.session); }
            base.arg(text);
            base = environment(base, spec);
            let session = spec.resume.then(|| spec.session.clone());
            return Ok((base,Box::new(AntigravityTranscript::baseline(home.as_deref(),session)?)));

}

impl TurnExtension for AntigravityTranscript {
 fn is_submission(&self)->bool {self.native_agentapi}
 fn session(&self)->Option<&str> {self.session.as_deref()}
 fn synchronize(&mut self,exit:Option<i32>,receipt:&mut Option<std::sync::mpsc::Receiver<Res<String>>>,resumed:&Option<String>,home:Option<&Path>,log:&Path)->Res<()> {
        if !self.native_agentapi {
            return Ok(());
        }
        if exit.is_none() { return Ok(()); }
        if let Some(receive) = receipt.as_ref() {
            let text = match receive.try_recv() {
                Ok(result) => result?,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
                Err(_) => return Err("agentapi receipt reader disconnected".into()),
            };
            let session = session_of(Client::Antigravity, &text).or_else(|| resumed.clone());
            if resumed.is_some() && session.as_ref() != resumed.as_ref() { return Err("antigravity receipt session differs".into()); }
            self.session = session;
            let mut log = File::options().append(true).open(log).map_err(|e| e.to_string())?;
            let encoded = serde_json::from_str::<Value>(&text).map(|v| v.to_string()).unwrap_or(text);
            writeln!(log, "\n{encoded}").map_err(|e| e.to_string())?;
            *receipt=None;
        }
        if exit == Some(0) {
            if let Some(home) = home { self.append(home, log)?; }
        }
        Ok(())
    }
}

fn input_bytes(text:&str)->Vec<u8> {format!("{}\n",json!({"event":"user","message":{"content":text}})).into_bytes()}

#[cfg(test)]
mod transcript_tests {
use super::*;
fn events(text:&str)->Vec<Value> {text.lines().filter_map(|s|serde_json::from_str(s).ok()).collect()}
    #[test]
    fn antigravity_full_history_progresses_past_budget() {
        let root = crate::scratch("large-full-tail");
        let (path, full) = transcript_paths(&root, "conv-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let log = root.join("turn.log"); std::fs::write(&log, "").unwrap();
        let compact = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","truncated_fields":["content"]});
        std::fs::write(path, format!("{compact}\n")).unwrap();
        let old = json!({"step_index":0,"content":"x".repeat(9*1024*1024)});
        let complete = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","content":"full"});
        std::fs::write(full, format!("{old}\n{complete}\n")).unwrap();
        let mut state = AntigravityTranscript {native_agentapi:true,session:Some("conv-1".into()),..Default::default()};
        for _ in 0..4 {state.append(&root, &log).unwrap();}
        assert_eq!(final_text(&events(&std::fs::read_to_string(log).unwrap())).as_deref(), Some("full"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn antigravity_tail_preserves_boundaries_and_unknowns() {
        let root = crate::scratch("antigravity-tail");
        let (path, full) = transcript_paths(&root, "conv-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let log = root.join("turn.log"); std::fs::write(&log, "").unwrap();
        let old = json!({"type":"PLANNER_RESPONSE","step_index":0,"status":"DONE","content":"old"});
        std::fs::write(&path, format!("{old}\n")).unwrap();
        let mut state = AntigravityTranscript::baseline(Some(&root), Some("conv-1".into())).unwrap();
        let event = json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","content":"new","tool_calls":[]}).to_string();
        let half = event.len()/2;
        let mut writer = File::options().append(true).open(&path).unwrap();
        writer.write_all(event[..half].as_bytes()).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
        writeln!(writer, "{}", &event[half..]).unwrap();
        state.append(&root, &log).unwrap(); state.append(&root, &log).unwrap();
        assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1, "new complete record only, once");
        let compact = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","truncated_fields":["content"]});
        writeln!(writer, "{compact}").unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1);
        let complete = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","content":"full"});
        std::fs::write(&full, format!("{old}\n{complete}")).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1, "partial full record waits");
        std::fs::write(&full, format!("{old}\n{complete}\n")).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 2);
        std::fs::write(&path, "").unwrap(); assert!(state.append(&root, &log).is_err(), "truncation retains unknown ownership");
        let failed = events(r#"{"type":"PLANNER_RESPONSE","step_index":1,"status":"ERROR","content":"quota exceeded"}"#);
        assert_eq!(Report::read(Client::Antigravity, "", &failed, None).outcome, "refused");
        let cancelled = events(r#"{"type":"PLANNER_RESPONSE","step_index":1,"status":"CANCELLED"}"#);
        assert_eq!(Report::read(Client::Antigravity, "", &cancelled, None).outcome, "cancelled");
        assert_eq!(snapshot(Client::Antigravity, &[], None).unknown, Some("not-reported"));
        assert_eq!(antigravity_session(r#"{"conversationId":"../foreign"}"#), None);
        std::fs::remove_dir_all(root).unwrap();
    }

}

fn preparation(_: &Launch, _: &Repair) -> (LaunchCheck, LaunchCheck) {
    (LaunchCheck::new("workspace","unknown",Some("workspace-trust-unknown"),"No native trust probe","check native workspace trust"),
     LaunchCheck::new("evidence","degraded",Some("hooks-unknown"),"No native hook trust probe","check AgentQ hooks"))
}

fn limits_for(l:&Launch,now:i64)->Vec<Window> {super::super::super::limits::account(Client::Antigravity,preflight::home(&ADAPTER,l).as_deref(),None,now)}

fn validate(args:&[OsString],_:&[(String,String)],_:&Policy)->Res<()> {validate_arguments(args)}
