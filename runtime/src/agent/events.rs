//! Native records become ordered runtime-neutral events at the adapter boundary.
use super::*;
use std::collections::{BTreeSet,HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ToolKind { Shell, Mcp, Web, Other }

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Event {
    Text { item: Option<String>, delta: String },
    Message { item: Option<String>, text: String },
    /// Optional local audit detail is untrusted; consumers scrub it before storage or output.
    ToolStart { id: Option<String>, kind: ToolKind, name: Option<String>, detail: Option<String>, timeout_ms: Option<u64> },
    ToolEnd { id: Option<String>, status: Option<String> },
    /// A turn boundary: pending tools and earlier progress no longer apply.
    TurnStart,
    TurnEnd { outcome: &'static str },
    /// log_records only: a supported record proved work advanced without a payload to report.
    Progress,
    /// A record the adapter recognizes but cannot read: the observation stays unknown.
    Unsupported { reason: &'static str },
    Quota(Vec<Window>),
    Context { used: Option<u64>, window: Option<u64> },
    Activity,
}

/// The native time of one source record. Null and malformed time stay apart from absent time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum RecordTime { Absent, Null, Malformed, At(i64) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum TimeFault { Null, Malformed, Future }

impl RecordTime {
    fn read(v: &Value) -> Self {
        match v.get("timestamp") {
            None => Self::Absent,
            Some(Value::Null) => Self::Null,
            Some(Value::String(s)) => native_time(s).map_or(Self::Malformed, Self::At),
            Some(_) => Self::Malformed,
        }
    }
    /// The record's Unix seconds against the caller's clock; absent time is `Ok(None)`.
    pub fn observed(self, now: i64) -> Result<Option<i64>, TimeFault> {
        match self {
            Self::Absent => Ok(None),
            Self::Null => Err(TimeFault::Null),
            Self::Malformed => Err(TimeFault::Malformed),
            Self::At(ms) if ms.div_euclid(1000) > now => Err(TimeFault::Future),
            Self::At(ms) => Ok(Some(ms.div_euclid(1000))),
        }
    }
}

/// One source record of a log, in source order. Native parsing stays behind this envelope.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct LogRecord { pub line: usize, pub time: RecordTime, pub events: Vec<Event> }

pub(super) fn log_records(client: Client, log: &str) -> Vec<LogRecord> {
    let mut cursor=Cursor::new(true);
    let mut out=vec![];
    for (line,text) in log.lines().enumerate() {
        match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => {
                let (time,advanced)=(RecordTime::read(&v),advances(client,&v));
                let mut events=cursor.read(client,vec![v]);
                if advanced && !events.iter().any(|e|matches!(e,Event::Message{..}|Event::ToolEnd{..}|Event::TurnEnd{..})) {
                    events.retain(|e|*e!=Event::Activity);
                    events.push(Event::Progress);
                }
                out.push(LogRecord{line,time,events});
            }            Err(_) if text.trim_start().starts_with('{') => out.push(LogRecord{line,time:RecordTime::Malformed,events:vec![Event::Unsupported{reason:"native record invalid"}]}),
            _ => {}
        }
    }
    out
}

/// Whether a record proved work advanced even when it names no typed payload.
fn advances(client: Client, v: &Value) -> bool {
    match client {
        Client::Claude => matches!(v["type"].as_str(),Some("assistant"|"user")) && v["message"]["content"].as_array().is_some_and(|c|c.iter().any(|b|b["type"]=="text")),
        Client::Codex => matches!(v.get("payload").or(v.get("msg")).unwrap_or(v)["type"].as_str(),Some("message"|"reasoning"))
            || (v["type"]=="item.completed" && v["item"]["type"].is_string())
            || (v["method"]=="item/completed" && v["params"]["item"]["type"].is_string()),
        Client::Antigravity => v["event"]=="step_update" && v["step_update"]["step_type"]!="tool" && v["step_update"]["state"]=="DONE",
    }
}

// `capture::time_ms` trusts its source's shape; a health signal must refuse an impossible date.
fn native_time(s: &str) -> Option<i64> {
    let b=s.as_bytes();
    if b.len()<20 || b[4]!=b'-' || b[7]!=b'-' || b[10]!=b'T' || b[13]!=b':' || b[16]!=b':' {return None;}
    let digits=[0..4,5..7,8..10,11..13,14..16,17..19];
    if digits.iter().any(|r|!b[r.clone()].iter().all(u8::is_ascii_digit)) {return None;}
    let fraction=s.get(19..s.len()-1)?;
    if !fraction.is_empty() && (fraction.len()<2 || !fraction.starts_with('.') || !fraction[1..].bytes().all(|c|c.is_ascii_digit())) {return None;}
    let n=|r: std::ops::Range<usize>|s[r].parse::<u32>().ok();
    let (year,month,day)=(n(0..4)?,n(5..7)?,n(8..10)?);
    let leap=year%4==0 && (year%100!=0 || year%400==0);
    let days=match month {1|3|5|7|8|10|12=>31,4|6|9|11=>30,2 if leap=>29,2=>28,_=>return None};
    if day==0 || day>days || n(11..13)?>23 || n(14..16)?>59 || n(17..19)?>59 {return None;}
    capture::time_ms(s).and_then(|ms|i64::try_from(ms).ok()).filter(|ms|*ms>0)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Model { pub id: String, pub name: String }

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Cursor {
    pub deltas: bool,
    message: Option<String>,
    window: Option<u64>,
    used: Option<u64>,
    last_text: Option<String>,
    model: Option<String>,
    seen: HashSet<String>,
    #[serde(default)]
    start_limit: StartLimit,
}

fn string(v: &Value) -> Option<String> { v.as_str().map(String::from) }
fn tool_kind(kind: &str) -> ToolKind {
    match kind { "command_execution" | "commandExecution" | "Bash" | "run_command" => ToolKind::Shell,
        "mcp_tool_call" | "mcpToolCall" => ToolKind::Mcp,
        "web_search" | "webSearch" => ToolKind::Web, _ if kind.starts_with("mcp__") => ToolKind::Mcp, _ => ToolKind::Other }
}

fn credential_like(text: &str) -> bool {
    let lower=text.to_ascii_lowercase();
    ["secret", "token", "password", "credential", "auth", ".env", "id_rsa", "id_ed25519", "api_key", "apikey", "ghp_", "github_pat_", "sk-", "bearer"].iter().any(|s|lower.contains(s))
}

// Local audit targets only: never serialize arbitrary native tool input or patches.
fn audit_path(value: &Value) -> Option<String> {
    let text=value.as_str()?;
    if text.is_empty() || text.len()>512 || !text.bytes().all(|b|b.is_ascii_alphanumeric() || b"/\\._-".contains(&b)) || credential_like(text) {return None;}
    Some(text.into())
}

fn audit_command(value: &Value) -> Option<String> {
    let text=value.as_str()?;
    if text.len()>2048 || text.chars().any(char::is_control) {return None;}
    let mut words=text.split_whitespace();
    let exe=audit_path(&json!(words.next()?))?;
    // Preserve file operands only where they cannot be inline code, content, credentials or URLs.
    let args=words.collect::<Vec<_>>();
    if matches!(exe.as_str(), "touch" | "mkdir" | "rmdir" | "rm" | "ls" | "stat" | "cat" | "head" | "tail" | "wc") {
        let mut detail=exe;
        for word in args { detail.push(' '); detail.push_str(&audit_path(&json!(word))?); }
        Some(detail)
    } else if matches!(exe.as_str(), "cargo"|"git"|"go"|"pytest"|"python"|"python3"|"bash"|"sh"|"curl"|"wget"|"node"|"npm"|"echo"|"printf"|"env") {Some(format!("{exe} [arguments redacted]"))} else {None}
}

fn audit_files(value: &Value) -> Option<String> {
    let paths=value.as_array()?;
    if paths.is_empty() || paths.len()>16 {return None;}
    paths.iter().map(|p|audit_path(&p["path"])).collect::<Option<Vec<_>>>().map(|p|p.join(", "))
}

fn native_timeout(input: &Value) -> Option<u64> {
    ["timeout_ms","timeoutMs","TimeoutMs","timeout"].iter().find_map(|k|input[*k].as_u64())
        .or_else(||input["timeout_seconds"].as_u64()?.checked_mul(1000)).filter(|ms|*ms>0)
}

/// Terminal records of session files, which ends leaves out because a turn report never reads them as its end.
fn session_end(client: Client, v: &Value) -> Option<&'static str> {
    match client {
        Client::Claude if v["type"]=="system" && v["subtype"]=="turn_duration" => Some("unknown"),
        Client::Codex if v["type"]=="event_msg" => match v["payload"]["type"].as_str() {Some("task_complete")=>Some("ok"),Some("turn_aborted")=>Some("cancelled"),_=>None},
        _ => None,
    }
}

fn antigravity_detail(name: Option<&str>, input: &Value) -> Option<String> {
    match name {
        Some("run_command")=>audit_command(&input["CommandLine"]),
        Some("view_file"|"write_to_file"|"replace_file_content"|"multi_replace_file_content")=>audit_path(input.get("Path").or(input.get("TargetFile")).unwrap_or(&Value::Null)),
        _=>None,
    }
}

impl Cursor {
    pub fn new(deltas: bool) -> Self { Self { deltas, ..Self::default() } }
    pub fn read(&mut self, client: Client, values: Vec<Value>) -> Vec<Event> { self.read_owned(client, values, None, None) }
    pub fn read_owned(&mut self, client: Client, values: Vec<Value>, thread: Option<&str>, turn: Option<&str>) -> Vec<Event> {
        let mut out = vec![];
        for v in values {
            if matches!(self.start_limit,StartLimit::Pending) {
                if let Some(start)=StartLimit::read(&v) {self.start_limit=start;}
            }
            let before = out.len();
            if let Some(w) = limits::reported(client, &v, (unix_ms()/1000).max(0) as u64) { out.push(Event::Quota(w)); }
            match client {
                Client::Claude => self.claude(&v, &mut out),
                Client::Codex => self.codex(&v, &mut out, thread),
                Client::Antigravity => self.antigravity(&v, &mut out),
            }
            let terminal=if client==Client::Codex && v["method"]=="turn/completed" && (thread.is_none() || thread==v["params"]["threadId"].as_str() && turn.is_some_and(|t|v["params"]["turn"]["id"].as_str()==Some(t))) {
                let outcome=match v["params"]["turn"]["status"].as_str() {Some("completed")=>Some("ok"),Some("failed")=>Some("failed"),Some("interrupted")=>Some("cancelled"),_=>None};
                if outcome.is_none() {out.push(Event::Unsupported{reason:"native turn status unsupported"});}
                outcome
            } else if ends(&v) {
                let outcome=Report::read(client,&v.to_string(),std::slice::from_ref(&v),None).outcome;
                Some(match (self.start_limit,outcome) { (StartLimit::Reached(_),_)=>"refused",(StartLimit::Working,"refused")=>"failed",_=>outcome })
            } else {
                if client==Client::Antigravity && v["event"]=="result" {out.push(Event::Unsupported{reason:"native turn status unsupported"});}
                session_end(client,&v)
            };
            if let Some(outcome)=terminal {out.push(Event::TurnEnd {outcome});}
            if out.len() == before { out.push(Event::Activity); }
        }
        out
    }

    fn claude(&mut self, v: &Value, out: &mut Vec<Event>) {
        if let Some(models) = v["modelUsage"].as_object() {
            self.window = self.model.as_ref().and_then(|m|models.get(m)).and_then(|m|m["contextWindow"].as_u64());
            out.push(Event::Context { used: self.used, window: self.window });
        }
        if v["type"] == "system" && v["subtype"] == "init" { out.push(Event::TurnStart); }
        let event = &v["event"];
        if v["type"] == "stream_event" {
            if event["type"] == "message_start" { self.message = string(&event["message"]["id"]); }
            if self.deltas && event["type"] == "content_block_delta" && event["delta"]["type"] == "text_delta" {
                if let Some(delta) = string(&event["delta"]["text"]) { out.push(Event::Text { item: self.message.clone(), delta }); }
            }
        }
        if v["type"] == "assistant" {
            let message = &v["message"];
            let model=string(&message["model"]);
            if model != self.model {self.window=None;} self.model=model;
            let item = string(&message["id"]);
            if let Some(blocks) = message["content"].as_array() {
                let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("");
                if !text.is_empty() && item.as_ref().is_none_or(|id| self.seen.insert(format!("message:{id}:{}", crate::sha256_hex(text.as_bytes())))) { self.last_text = Some(text.clone()); out.push(Event::Message { item: item.clone(), text }); }
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let id = string(&block["id"]);
                    if id.as_ref().is_none_or(|id| self.seen.insert(format!("tool:{id}"))) {
                        let name = string(&block["name"]); let kind = tool_kind(name.as_deref().unwrap_or(""));
                        out.push(Event::ToolStart { id, kind, name, timeout_ms:native_timeout(&block["input"]), detail: match block["name"].as_str() {Some("Bash")=>audit_command(&block["input"]["command"]),Some("Read"|"Edit"|"Write"|"NotebookEdit"|"MultiEdit")=>audit_path(&block["input"]["file_path"]),_=>None} });
                    }
                }
            }
            let u = &message["usage"];
            if u.is_object() {
                let used = u["input_tokens"].as_u64().and_then(|i| i.checked_add(u["cache_read_input_tokens"].as_u64()?).and_then(|n| n.checked_add(u["cache_creation_input_tokens"].as_u64()?)));
                self.used = used;
                out.push(Event::Context { used, window: self.window });
            }
        }
        if v["type"] == "user" {
            if let Some(blocks) = v["message"]["content"].as_array() {
                for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    out.push(Event::ToolEnd { id: string(&b["tool_use_id"]), status: Some(if b["is_error"] == true { "failed" } else { "completed" }.into()) });
                }
            }
        }
        if v["type"] == "result" {
            if let Some(text) = v["result"].as_str().filter(|s|self.last_text.as_deref()!=Some(*s)) {
                self.last_text=Some(text.into()); out.push(Event::Message {item:None,text:text.into()});
            }
        }
    }

    fn codex(&mut self, v: &Value, out: &mut Vec<Event>, thread: Option<&str>) {
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        if v["type"] == "turn.started" || (method == "turn/started" && thread.is_none_or(|t|p["threadId"].as_str()==Some(t))) { out.push(Event::TurnStart); }
        if self.deltas && method == "item/agentMessage/delta" {
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

    fn antigravity(&mut self, v: &Value, out: &mut Vec<Event>) {
        if v["event"] == "init" || v["type"] == "init" { out.push(Event::TurnStart); }
        if v["event"] == "result" {
            if let Some(text) = string(&v["result"]["response"]) { out.push(Event::Message { item: None, text }); }
        }
        if v["event"] == "step_update" {
            let s = &v["step_update"];
            let id = s["step_index"].as_u64().map(|n| n.to_string());
            if self.deltas { if let Some(delta) = string(&s["text_delta"]) { out.push(Event::Text { item: id.clone(), delta }); } }
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
                    if id.as_ref().is_none_or(|id|self.seen.insert(format!("tool:{id}"))) {
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
}

/// One Bash or Monitor call; only a confirmed start makes it a job.
struct Job { tool: String, label: String, background: bool, task: Option<String> }

/// Claude background jobs the last turn left running, oldest first. Log evidence only; this grants no process or stop authority.
pub(super) fn live_background(values: &[Value]) -> Vec<String> {
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
                        jobs.push(Job{tool:id.into(),label:job_label(name,input),background,task:None});
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

/// A bounded description or the fixed tool name; anything that could carry the command stays inside.
fn job_label(name: &'static str, input: &Value) -> String {
    let command=input["command"].as_str().unwrap_or("").trim();
    input["description"].as_str().map(str::trim)
        .filter(|d|!d.is_empty() && !d.chars().any(char::is_control) && !d.contains("://") && !credential_like(d)
            && (command.is_empty() || (!d.contains(command) && !command.contains(d))))
        .map_or_else(||name.into(),|d|d.chars().take(80).collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    // #258 A1: a limit after work is failure, while pre-work rejection remains a refusal across reads/recovery.
    #[test]
    fn log_terminal_keeps_prior_work_context() {
        for (name, start, end) in [
            ("codex",json!({"type":"item.started","item":{"id":"tool","type":"command_execution"}}),json!({"type":"turn.failed","error":{"message":"usage limit"}})),
            ("antigravity",json!({"event":"step_update","step_update":{"step_type":"tool"}}),json!({"event":"result","result":{"status":"ERROR","error":"quota exceeded"}})),
            ("antigravity",json!({"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command"}]}),json!({"type":"PLANNER_RESPONSE","status":"ERROR","content":"quota exceeded"})),
        ] {
            let runtime=adapter::runtime(name).unwrap();
            for (rows, expected) in [(vec![end.clone()],"refused"),(vec![start.clone(),end.clone()],"failed")] {
                let log=rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
                assert_eq!(runtime.classify(&log).outcome,expected);
                assert!(matches!(runtime.log_events(&log).last(),Some(Event::TurnEnd{outcome}) if *outcome==expected),"{name}: {log}");
            }
            let client=Client::parse(name).unwrap();
            let mut cursor=Cursor::default();cursor.read(client,vec![start]);
            let mut old=serde_json::to_value(&Cursor::default()).unwrap();old.as_object_mut().unwrap().remove("start_limit");
            let _:Cursor=serde_json::from_value(old).unwrap();
            let mut recovered:Cursor=serde_json::from_value(serde_json::to_value(&cursor).unwrap()).unwrap();
            for c in [&mut cursor,&mut recovered] {
                assert!(matches!(c.read(client,vec![end.clone()]).last(),Some(Event::TurnEnd{outcome:"failed"})));
            }
        }
        let runtime=adapter::runtime("claude").unwrap();
        let log=format!("{}\n{}",json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected"}}),json!({"type":"result","is_error":true}));
        assert_eq!(runtime.classify(&log).outcome,"refused");
        assert!(matches!(runtime.log_events(&log).last(),Some(Event::TurnEnd{outcome:"refused"})));
    }

    // #258 A1/M12.9: callers can read all attempt transports without constructing a Run.
    #[test]
    fn public_log_events_keep_timeout_order_and_terminal_evidence() {
        for (name,start,end) in [
            ("claude",json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"touch x","timeout":600000}}]}}),json!({"type":"result","is_error":false})),
            ("codex",json!({"type":"item.started","item":{"id":"t","type":"command_execution","timeout_ms":600000}}),json!({"type":"turn.completed"})),
            ("codex",json!({"method":"item/started","params":{"item":{"id":"t","type":"commandExecution","timeoutMs":600000}}}),json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}})),
            ("antigravity",json!({"event":"step_update","step_update":{"step_type":"tool","state":"ACTIVE","tool_info":{"parameters":{"timeout_ms":600000}}}}),json!({"event":"result","result":{"status":"SUCCESS"}})),
            ("antigravity",json!({"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"id":"t","name":"run_command","args":{"TimeoutMs":600000}}]}),json!({"type":"PLANNER_RESPONSE","status":"DONE","content":"answer"})),
        ] {
            let runtime=adapter::runtime(name).unwrap();
            let events=runtime.log_events(&format!("native diagnostic\n{start}\n{end}\n{{partial"));
            assert!(matches!(events.first(),Some(Event::ToolStart{timeout_ms:Some(600000),..})),"{name}: {events:?}");
            assert!(matches!(events.last(),Some(Event::TurnEnd{outcome:"ok"})),"{name}: {events:?}");
            assert_eq!(events.iter().filter(|e|matches!(e,Event::TurnEnd{..})).count(),1);
        }
        for (name,row,outcome) in [
            ("claude",json!({"type":"result","is_error":true}),"failed"),
            ("codex",json!({"type":"turn.failed"}),"failed"),
            ("codex",json!({"method":"turn/completed","params":{"turn":{"status":"interrupted"}}}),"cancelled"),
            ("antigravity",json!({"event":"result","result":{"status":"INTERRUPTED"}}),"cancelled"),
            ("antigravity",json!({"type":"PLANNER_RESPONSE","status":"ERROR"}),"failed"),
        ] {assert!(matches!(adapter::runtime(name).unwrap().log_events(&row.to_string()).last(),Some(Event::TurnEnd{outcome:o}) if *o==outcome));}
        let runtime=adapter::runtime("claude").unwrap();
        for timeout in [Value::Null,json!(-1),json!("600000")] {
            let row=json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"timeout":timeout}}]}});
            assert!(matches!(runtime.log_events(&row.to_string())[0],Event::ToolStart{timeout_ms:None,..}));
        }
        let end=|thread,turn|json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":turn,"status":"completed"}}});
        let mut cursor=Cursor::default();
        assert_eq!(cursor.read_owned(Client::Codex,vec![end("foreign","owned-turn"),end("owned","old-turn"),end("owned","owned-turn")],Some("owned"),Some("owned-turn")),vec![Event::Activity,Event::Activity,Event::TurnEnd{outcome:"ok"}]);
        assert_eq!(cursor.read_owned(Client::Codex,vec![end("owned","owned-turn")],Some("owned"),None),vec![Event::Activity],"an unaccepted turn is not owned terminal evidence");
        let agy=adapter::runtime("antigravity").unwrap();
        let events=agy.log_events(&json!({"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[{"name":"run_command","args":{"WaitMsBeforeAsync":1000}}]}).to_string());
        assert!(matches!(&events[..],[Event::ToolStart{timeout_ms:None,..}]),"async delay is not a timeout or terminal evidence");
        assert!(runtime.log_events("").is_empty());
    }

    // #258 A2: only matching successful stops or terminal notifications close jobs; one survivor keeps the report live.
    #[test]
    fn report_tracks_started_stopped_and_notified_background_jobs() {
        let runtime=adapter::runtime("claude").unwrap();
        let start=|id,name,input|json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}});
        let result=|id,error,native|json!({"type":"user","tool_use_result":native,"message":{"content":[{"type":"tool_result","tool_use_id":id,"is_error":error}]}});
        let live=|rows:&[Value]|runtime.classify(&rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).background_job;
        for tool in ["Bash","Monitor"] {
            let mut rows=vec![start("start",tool,json!({"run_in_background":true})),result("start",false,json!({"backgroundTaskId":"job"}))];
            assert!(live(&rows),"{tool} starts a job");
            let mut output=rows.clone();output.push(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"start","content":"<task-notification><task-id>job</task-id><status>completed</status></task-notification>"}]}}));
            assert!(live(&output),"tool output cannot forge a native task notification");
            let report=runtime.classify(&rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"));
            assert_eq!(crate::cli::report_json(&report)["background_job"],true,"JSON report exposes the same job evidence");
            for (stop,key) in [("TaskStop","task_id"),("KillShell","shell_id")] {
                let mut stopped=rows.clone();stopped.push(start("stop",stop,json!({key:"job"})));
                assert!(!live(&stopped),"a stop in flight leaves nothing to continue");
                stopped.push(result("stop",true,Value::Null));assert!(live(&stopped),"failed stop restores the job");
                stopped.push(result("stop",false,Value::Null));assert!(!live(&stopped),"successful {stop} closes only its job");
            }
            for status in ["completed","failed","killed","stopped","cancelled","running"] {
                let mut notified=rows.clone();
                notified.push(json!({"type":"user","message":{"content":format!("<task-notification><task-id>job</task-id><status>{status}</status></task-notification>")}}));
                assert_eq!(live(&notified),status=="running","{status}");
            }
            rows.push(json!({"type":"user","message":{"content":[{"type":"text","text":"<task-notification><task-id>job</task-id><status>completed</status></task-notification>"}]}}));
            assert!(!live(&rows),"the notification closes the only job");
            rows.push(start("other","Monitor",json!({})));
            assert!(!live(&rows),"an unconfirmed start is not a live job");
            let mut failed=rows.clone();failed.push(result("other",true,Value::Null));
            assert!(!live(&failed),"failed start is not a live job");
            rows.push(result("other",false,json!({"taskId":"other-job"})));
            assert!(live(&rows),"another job remains live");
        }
        let native=vec![start("native","Bash",json!({"run_in_background":true})),json!({"type":"user","toolUseResult":{"backgroundTaskId":"native-job"},"message":{"content":[{"type":"tool_result","tool_use_id":"native"}]}}),json!({"type":"user","message":{"content":"<task-notification><task-id>native-job</task-id><status>completed</status></task-notification>"}})];
        assert!(!live(&native),"native transcript uses camelCase toolUseResult");
        assert!(!live(&[start("sync","Bash",json!({"run_in_background":false}))]));
        assert!(!live(&[]));
    }

    // #261 A2 defect: completed, stopped and never-started jobs read live, so a worker was resumed for nothing.
    // Retained risk: the same fixtures as TaskQ's regression keep a worker's own check from ending its attempt.
    #[test]
    fn report_keeps_the_eight_live_background_outcomes() {
        let start=r#"{"type":"taskq.rules","phase":"start"}"#;
        let resume=r#"{"type":"taskq.rules","phase":"resume"}"#;
        let bash=r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"python3 scripts/check.py","description":"Full check","run_in_background":true}}]}}"#;
        let fg=r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"Bash","input":{"command":"git status"}}]}}"#;
        let fg_done=r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"clean"}]}}"#;
        let started=r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"Command running in background with ID: b7x9. Output is being written to: out"}]}}"#;
        let failed=r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":true,"content":"denied"}]}}"#;
        let monitor=r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_3","name":"Monitor","input":{"description":"CI watch","command":"gh pr checks 1"}}]}}"#;
        let monitor_started=r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_3","content":[{"type":"text","text":"Monitor started (task m42, expires in 20m)"}]}]}}"#;
        let done=r#"{"type":"queue-operation","content":"<task-notification>\n<task-id>b7x9</task-id>\n<tool-use-id>toolu_1</tool-use-id>\n<status>completed</status>\n</task-notification>"}"#;
        let running=r#"{"type":"queue-operation","content":"<task-notification>\n<task-id>b7x9</task-id>\n<status>running</status>\n</task-notification>"}"#;
        let killed=r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_4","name":"KillShell","input":{"shell_id":"b7x9"}}]}}"#;
        let runtime=adapter::runtime("claude").unwrap();
        // The caller finds its own markers by line; the adapter never names a caller's record.
        let after_marker=|rows:&[&str]| {
            let log=rows.join("\n");
            let from=rows.iter().rposition(|r|r.contains("taskq.rules")).map_or(0,|i|i+1);
            assert!(runtime.log_records(&log).iter().filter(|r|rows[r.line].contains("taskq.rules")).all(|r|r.events==[Event::Activity]));
            runtime.classify(&rows[from..].join("\n"))
        };
        let cases:[(Vec<&str>,Vec<&str>);8]=[
            (vec![start,fg,fg_done,bash,started],vec!["Full check"]),
            (vec![start,bash,started,running],vec!["Full check"]),
            (vec![start,bash,started,monitor,monitor_started],vec!["Full check","CI watch"]),
            (vec![start,bash,started,done],vec![]),
            (vec![start,bash,started,killed],vec![]),
            (vec![start,bash,failed],vec![]),
            (vec![start,bash,started,resume,fg,fg_done],vec![]),
            (vec![start,fg,fg_done],vec![]),
        ];
        for (rows,want) in cases {
            let report=after_marker(&rows);
            assert_eq!(report.live_background,want,"{rows:?}");
            assert_eq!(report.background_job,!want.is_empty(),"{rows:?}");
        }
        // A native turn boundary needs no caller marker.
        let init=r#"{"type":"system","subtype":"init"}"#;
        assert!(!runtime.classify(&[bash,started,init,fg,fg_done].join("\n")).background_job);
        assert!(runtime.classify(&[init,bash,started,r#"{"type":"result","is_error":false}"#].join("\n")).background_job,"a job outlives its turn's end record");
        // Concurrent jobs stay distinct even under one label, and only the matching one ends.
        let second=[bash.replace("toolu_1","toolu_5"),started.replace("toolu_1","toolu_5").replace("b7x9","c8y0")];
        let rows=[bash,started,&second[0],&second[1],monitor,monitor_started];
        assert_eq!(runtime.classify(&rows.join("\n")).live_background,["Full check","Full check","CI watch"]);
        assert_eq!(runtime.classify(&[&rows[..],&[done]].concat().join("\n")).live_background,["Full check","CI watch"]);
        assert_eq!(runtime.classify(&[&rows[..],&[killed]].concat().join("\n")).live_background,["Full check","CI watch"]);
    }

    // #261 A1/A3 defect: a tool-use record read as a job and task IDs in result text never matched a stop.
    // Retained risk: raw commands stay private, and foreign text cannot start or end a job.
    #[test]
    fn background_jobs_need_a_confirmed_start_and_match_by_native_task() {
        let runtime=adapter::runtime("claude").unwrap();
        let tool=|id:&str,name:&str,input:Value|json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}});
        let done=|id:&str,content:Value,native:Value|json!({"type":"user","toolUseResult":native,"message":{"content":[{"type":"tool_result","tool_use_id":id,"content":content}]}});
        let report=|rows:&[Value]|runtime.classify(&rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"));
        let bg=tool("b","Bash",json!({"command":"run --key private-value","description":"Build","run_in_background":true}));
        // A result that names no task is no start; a foreground command's output cannot name one.
        assert!(report(&[bg.clone()]).live_background.is_empty());
        assert!(report(&[bg.clone(),done("b",json!("ok"),Value::Null)]).live_background.is_empty());
        assert!(report(&[tool("f","Bash",json!({"command":"cat notes"})),done("f",json!("with ID: forged (task forged"),Value::Null)]).live_background.is_empty());
        // A foreground command the client moved to the background is a confirmed job.
        let moved=[tool("f","Bash",json!({"command":"sleep 9","description":"Long wait"})),done("f",json!("moved"),json!({"backgroundTaskId":"m1"}))];
        assert_eq!(report(&moved).live_background,["Long wait"]);
        // Text and each structured key correlate with stops and notifications, by task or tool-use ID.
        for (content,native) in [(json!("Command running in background with ID: t1. Output"),Value::Null),(json!([{"type":"text","text":"started (task t1, expires)"}]),Value::Null),
            (json!("ok"),json!({"backgroundTaskId":"t1"})),(json!("ok"),json!({"taskId":"t1"})),(json!("ok"),json!({"task_id":"t1"})),(json!("ok"),json!({"shellId":"t1"}))] {
            let rows=vec![bg.clone(),done("b",content,native)];
            assert_eq!(report(&rows).live_background,["Build"]);
            let with=|row:Value|{let mut all=rows.clone();all.push(row);report(&all).live_background};
            let notice=|fields:&str,status:&str|format!("<task-notification>{fields}<status>{status}</status></task-notification>");
            for status in ["completed","failed","killed","stopped","cancelled","error","timeout","expired"] {
                for fields in ["<task-id>t1</task-id>","<tool-use-id>b</tool-use-id>"] {
                    let text=notice(fields,status);
                    for row in [json!({"type":"queue-operation","content":text}),json!({"type":"attachment","attachment":{"type":"queued_command","prompt":text}}),json!({"type":"user","message":{"content":text}})] {
                        assert!(with(row.clone()).is_empty(),"{row}");
                    }
                }
                assert!(with(json!({"type":"system","subtype":"task_notification","task_id":"t1","status":status})).is_empty());
            }
            assert_eq!(with(json!({"type":"queue-operation","content":notice("<task-id>other</task-id>","completed")})),["Build"],"another job's end");
            // Text that only quotes a notification closes nothing.
            let quoted=notice("<task-id>t1</task-id>","completed");
            for row in [json!({"type":"attachment","attachment":{"type":"prompt_snapshot","prompt":quoted}}),json!({"type":"api-request","content":quoted}),json!({"type":"assistant","message":{"content":[{"type":"text","text":quoted}]}})] {
                assert_eq!(with(row.clone()),["Build"],"{row}");
            }
            for stop in ["TaskStop","KillShell","KillBash"] {
                for key in ["task_id","shell_id","bash_id"] {assert!(with(tool("s",stop,json!({key:"t1"}))).is_empty(),"{stop} {key}");}
                assert_eq!(with(tool("s",stop,json!({"task_id":"other"}))),["Build"]);
            }
        }
        // A notification before the start result still ends that job.
        let early=json!({"type":"queue-operation","content":"<task-notification><task-id>t1</task-id><status>completed</status></task-notification>"});
        assert!(report(&[bg.clone(),early,done("b",json!("with ID: t1."),Value::Null)]).live_background.is_empty());
        // Only a bounded description or a fixed name leaves the adapter.
        let long="x".repeat(200);
        for (input,want) in [
            (json!({"description":long,"command":"y"}),"x".repeat(80)),
            (json!({"description":"  Spaced  "}),"Spaced".into()),
            (json!({}),"Bash".into()),(json!({"description":" "}),"Bash".into()),(json!({"description":7}),"Bash".into()),
            (json!({"command":"npm test --key v","description":"npm test --key v"}),"Bash".into()),
            (json!({"command":"npm test --key v","description":"Run npm test --key v now"}),"Bash".into()),
            (json!({"description":"two\nlines"}),"Bash".into()),
            (json!({"description":"fetch https://host/path"}),"Bash".into()),
            (json!({"description":"export API_KEY value"}),"Bash".into()),
        ] {
            let mut input=input;input["run_in_background"]=json!(true);
            assert_eq!(report(&[tool("b","Bash",input),done("b",json!("with ID: t1."),Value::Null)]).live_background,[want]);
        }
        assert_eq!(report(&[tool("m","Monitor",json!({"command":"watch"})),done("m",json!("Monitor started (task m1, x)"),Value::Null)]).live_background,["Monitor"]);
        let json=crate::cli::report_json(&report(&[bg.clone(),done("b",json!("with ID: t1."),json!({"command":"run --key private-value"}))]));
        assert_eq!(json["live_background"],json!(["Build"]));
        assert_eq!(json["background_job"],true);
        for private in ["private-value","--key","t1"] {assert!(!json["live_background"].to_string().contains(private));}
        // Other runtimes have no such records.
        assert!(adapter::runtime("codex").unwrap().classify(&bg.to_string()).live_background.is_empty());
    }

    // #261 A4 defect: valid and invalid native timestamps produced identical typed events.
    // Retained risk: a caller's timestamp_fault guard needs absent, null, malformed and future time apart.
    #[test]
    fn log_records_keep_source_order_and_native_time() {
        let row=|time:Value|json!({"timestamp":time,"type":"item.started","item":{"id":"c","type":"command_execution"}}).to_string();
        let runtime=adapter::runtime("codex").unwrap();
        for (time,want) in [
            (json!("2026-11-06T00:00:00Z"),RecordTime::At(1_793_923_200_000)),
            (json!("2026-11-06T00:00:00.250Z"),RecordTime::At(1_793_923_200_250)),
            (json!("2028-02-29T00:00:00Z"),RecordTime::At(1_835_395_200_000)),
            (Value::Null,RecordTime::Null),
            (json!("invalid"),RecordTime::Malformed),(json!(""),RecordTime::Malformed),(json!(1000),RecordTime::Malformed),(json!(false),RecordTime::Malformed),
            (json!("2026-13-06T00:00:00Z"),RecordTime::Malformed),(json!("2026-02-30T00:00:00Z"),RecordTime::Malformed),(json!("2027-02-29T00:00:00Z"),RecordTime::Malformed),
            (json!("2026-11-06T24:00:00Z"),RecordTime::Malformed),(json!("2026-11-06 00:00:00Z"),RecordTime::Malformed),(json!("2026-11-06T00:00:00+01:00"),RecordTime::Malformed),
            (json!("2026-11-06T00:00:00.Z"),RecordTime::Malformed),(json!("1970-01-01T00:00:00Z"),RecordTime::Malformed),(json!("+026-11-06T00:00:00Z"),RecordTime::Malformed),
        ] {
            let records=runtime.log_records(&row(time.clone()));
            assert_eq!(records[0].time,want,"{time}");
            assert!(matches!(records[0].events[0],Event::ToolStart{..}),"the event stays readable: {time}");
        }
        let absent=runtime.log_records(r#"{"type":"turn.completed"}"#);
        assert_eq!(absent[0].time,RecordTime::Absent);
        assert_eq!(absent[0].time.observed(1),Ok(None));
        assert_eq!(RecordTime::Null.observed(1),Err(TimeFault::Null));
        assert_eq!(RecordTime::Malformed.observed(1),Err(TimeFault::Malformed));
        assert_eq!(RecordTime::At(2_999).observed(2),Ok(Some(2)),"a second clock accepts its own second");
        assert_eq!(RecordTime::At(3_000).observed(2),Err(TimeFault::Future));
        // Source order, with diagnostics and caller records keeping their lines.
        let log=format!("diagnostic\n{}\n{{\"type\":\"taskq.input\"}}\n{{broken\n[1]\n7\n{}\n",row(json!("2026-11-06T00:00:00Z")),row(json!("invalid")));
        let records=runtime.log_records(&log);
        assert_eq!(records.iter().map(|r|r.line).collect::<Vec<_>>(),[1,2,3,6]);
        assert_eq!(records[1].events,[Event::Activity]);
        assert_eq!(records[2].time,RecordTime::Malformed);
        assert_eq!(records[2].events,[Event::Unsupported{reason:"native record invalid"}]);
        assert!(runtime.log_records("").is_empty());
        // A record's events keep native order, and log_events reads the same stream without broken lines.
        let claude=adapter::runtime("claude").unwrap();
        let log=json!({"timestamp":"2026-11-06T00:00:00Z","type":"assistant","message":{"id":"m","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t","name":"Read","input":{}}]}}).to_string();
        assert!(matches!(&claude.log_records(&log)[0].events[..],[Event::Message{..},Event::ToolStart{..}]));
        assert_eq!(claude.log_records(&log)[0].events,claude.log_events(&log));
    }

    /// TaskQ's advisory progress reader over typed records only: no native field is read here.
    #[derive(Default)]
    struct Progress { tools: BTreeMap<String,(i64,Option<i64>)>, last: Option<i64>, seen: bool, error: Option<&'static str> }

    impl Progress {
        fn feed(&mut self, runtime: &dyn adapter::Runtime, log: &str, modified: i64, now: i64) {
            for record in runtime.log_records(log) {
                let at=match record.time.observed(now) {
                    Ok(time)=>time.unwrap_or(modified),
                    Err(_)=>{self.error=Some("native progress timestamp invalid or future");continue;}
                };
                for event in record.events {
                    match event {
                        Event::TurnStart=>{*self=Self::default();}
                        Event::ToolStart{id:Some(id),timeout_ms,..}=>{self.seen=true;self.tools.entry(id).or_insert((at,timeout_ms.map(|ms|ms.div_ceil(1000) as i64)));}
                        Event::ToolEnd{id:Some(id),..}=>{self.tools.remove(&id);self.done(at);}
                        Event::ToolStart{id:None,..}|Event::ToolEnd{id:None,..}=>self.error=Some("native tool identity unsupported"),
                        Event::Unsupported{reason}=>self.error=Some(reason),
                        Event::Message{..}|Event::Progress=>self.done(at),
                        Event::TurnEnd{..}=>{self.tools.clear();self.error=None;self.done(at);}
                        Event::Text{..}|Event::Quota(_)|Event::Context{..}|Event::Activity=>{}
                    }
                }
            }
        }
        fn done(&mut self, at: i64) {self.last=Some(at);self.seen=true;}
        fn status(&self, now: i64, poll: i64) -> &'static str {
            if self.error.is_some() || !self.seen {return "unknown";}
            let overdue=self.tools.values().any(|(at,timeout)|now>timeout.map_or_else(||(*at).max(self.last.unwrap_or(*at))+900,|s|at+s+poll*2));
            if overdue {"overdue"} else if self.tools.is_empty() {"quiet"} else {"active"}
        }
    }

    // #261 A4/A5 defect: typed events hid turn starts, payload-free progress and unsupported records.
    // Retained risk: TaskQ's unbounded stall, explicit deadline and timestamp_fault outcomes on neutral data.
    #[test]
    fn typed_records_keep_progress_guards_without_native_parsing() {
        let (codex,claude,agy)=(adapter::runtime("codex").unwrap(),adapter::runtime("claude").unwrap(),adapter::runtime("antigravity").unwrap());
        // A concurrent tool keeps a stall overdue; age alone never fails quiet work.
        let step=|id:u64,state:&str|json!({"event":"step_update","step_update":{"conversation_id":"invented","step_index":id,"state":state,"step_type":"tool"}}).to_string();
        let mut r=Progress::default();
        r.feed(agy,&step(90,"ACTIVE"),1000,1000);
        assert_eq!(r.status(1900,60),"active");
        assert_eq!(r.status(1901,60),"overdue");
        r.feed(agy,&step(91,"ACTIVE"),1902,1902);
        assert_eq!(r.status(7620,60),"overdue");
        r.feed(agy,&step(90,"DONE"),7621,7621);
        assert_eq!((r.tools.len(),r.status(7621,60)),(1,"active"));
        r.feed(agy,&step(91,"FAILED"),7622,7622);
        assert_eq!(r.status(99999,60),"quiet");
        // An unknown tool state stays unknown until a boundary.
        r.feed(agy,&step(92,"PENDING"),99999,99999);
        assert_eq!(r.status(99999,60),"unknown");
        r.feed(agy,&json!({"event":"result","result":{"status":"NEW"}}).to_string(),99999,99999);
        assert_eq!(r.status(99999,60),"unknown","an unknown terminal status proves nothing");
        r.feed(agy,&json!({"event":"result","result":{"status":"SUCCESS"}}).to_string(),99999,99999);
        assert_eq!(r.status(99999,60),"quiet");
        r.feed(agy,r#"{"event":"init"}"#,100000,100000);
        assert_eq!(r.status(100000,60),"unknown","a new turn has seen nothing yet");
        // An explicit deadline keeps its budget; progress does not extend it.
        let mut r=Progress::default();
        r.feed(codex,&json!({"type":"response_item","payload":{"type":"function_call","call_id":"c","name":"exec","arguments":"{\"timeout_ms\":3600000,\"cmd\":\"private\"}"}}).to_string(),1000,1000);
        assert_eq!(r.tools["c"],(1000,Some(3600)));
        r.feed(codex,&json!({"type":"response_item","payload":{"type":"reasoning"}}).to_string(),4500,4500);
        assert_eq!(r.last,Some(4500),"reasoning is progress");
        assert_eq!(r.status(4720,60),"active");
        assert_eq!(r.status(4721,60),"overdue");
        r.feed(codex,&json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"c","output":"private"}}).to_string(),4722,4722);
        assert_eq!(r.status(99999,60),"quiet");
        let rollout=codex.log_events(&json!({"type":"response_item","payload":{"type":"function_call","call_id":"c","name":"exec","arguments":"{\"cmd\":\"private\"}"}}).to_string());
        assert!(!format!("{rollout:?}").contains("private"),"rollout arguments stay private");
        assert!(matches!(claude.log_events(&json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Bash","input":{"timeout":120000}}]}}).to_string())[0],Event::ToolStart{timeout_ms:Some(120000),..}));
        // Bad time stays unknown across time, empty reads and unrelated progress, until a valid boundary.
        let now=1000;
        let plain=json!({"type":"item.completed","item":{"type":"agent_message"}}).to_string();
        let turn_end=json!({"type":"turn.completed"}).to_string();
        for timestamp in [json!("invalid"),json!("2100-01-01T00:00:00Z"),json!(now),json!(false),Value::Null] {
            let started=json!({"timestamp":timestamp,"type":"item.started","item":{"id":"c","type":"command_execution"}}).to_string();
            let mut r=Progress::default();
            r.feed(codex,&plain,now,now);
            assert_eq!(r.status(now,60),"quiet");
            r.feed(codex,&started,now,now+1);
            assert_eq!(r.status(now+1,60),"unknown","{timestamp}");
            r.feed(codex,"",now,now+101);
            assert_eq!(r.status(now+101,60),"unknown","time and an empty read cannot recover");
            r.feed(codex,&plain,now,now+102);
            assert_eq!(r.status(now+102,60),"unknown","unrelated progress cannot prove the missing tool ended");
            r.feed(codex,&turn_end,now,now+103);
            assert_eq!(r.status(now+103,60),"quiet");
            r.feed(codex,&format!("{started}\n{turn_end}"),now,now+104);
            assert_eq!(r.status(now+104,60),"quiet","a valid boundary in the same batch is not discarded");
        }
        // Time after the read accepts a hot log's own second and fences real future data.
        let at=|time:&str|json!({"timestamp":time,"type":"item.started","item":{"id":"c","type":"command_execution"}}).to_string();
        let mut r=Progress::default();
        r.feed(codex,&at("2026-11-06T00:00:01Z"),1_793_923_200,1_793_923_201);
        assert_eq!(r.status(1_793_923_201,60),"active");
        let mut r=Progress::default();
        r.feed(codex,&at("2026-11-06T00:00:02Z"),1_793_923_200,1_793_923_201);
        assert_eq!(r.status(1_793_923_201,60),"unknown");
        // Unsupported and invalid observations read unknown, never healthy.
        for log in ["{}","{\"type\":\"other\"}","{\"type\":\"item.completed\"}"] {
            let mut r=Progress::default();
            r.feed(codex,log,now,now);
            assert_eq!(r.status(now,60),"unknown","{log}");
        }
        let mut r=Progress::default();
        r.feed(codex,&format!("{plain}\n{{broken"),now,now);
        assert_eq!(r.status(now,60),"unknown");
        let mut cursor=Cursor::default();
        let turn=|status:&str|json!({"method":"turn/completed","params":{"threadId":"owned","turn":{"id":"t","status":status}}});
        assert_eq!(cursor.read_owned(Client::Codex,vec![turn("mystery")],Some("owned"),Some("t")),[Event::Unsupported{reason:"native turn status unsupported"}]);
        // Turn boundaries on each transport; a foreign thread is no boundary.
        for (runtime,row) in [(&codex,json!({"type":"turn.started"})),(&codex,json!({"method":"turn/started","params":{}})),(&claude,json!({"type":"system","subtype":"init"})),(&agy,json!({"type":"init"}))] {
            assert_eq!(runtime.log_events(&row.to_string()),[Event::TurnStart],"{row}");
        }
        let started=json!({"method":"turn/started","params":{"threadId":"foreign"}});
        assert_eq!(cursor.read_owned(Client::Codex,vec![started],Some("owned"),None),[Event::Activity]);
        for (runtime,row,outcome) in [
            (&claude,json!({"type":"system","subtype":"turn_duration"}),"unknown"),
            (&codex,json!({"type":"event_msg","payload":{"type":"task_complete"}}),"ok"),
            (&codex,json!({"type":"event_msg","payload":{"type":"turn_aborted"}}),"cancelled"),
        ] {
            assert!(matches!(runtime.log_events(&row.to_string())[..],[Event::TurnEnd{outcome:o}] if o==outcome),"{row}");
            assert_eq!(runtime.classify(&row.to_string()).outcome,"unknown","a session file record is no report end: {row}");
        }
        // Repeated or user text is progress, never a second message.
        let text=json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"same"}]}}).to_string();
        let events=|log:&str|claude.log_records(log).into_iter().map(|r|r.events).collect::<Vec<_>>();
        assert!(matches!(&events(&format!("{text}\n{text}"))[..],[a,b] if matches!(a[..],[Event::Message{..}]) && b==&[Event::Progress]));
        assert_eq!(events(&json!({"type":"user","message":{"content":[{"type":"text","text":"x"}]}}).to_string()),[[Event::Progress]]);
        // Progress stays out of the live stream, where such records keep reading Activity.
        assert_eq!(codex.log_events(&plain),[Event::Activity]);
    }
    // #255 A2: retain the interrupted command target without copying private tool payloads.
    #[test]
    fn audit_detail_preserves_target_and_withholds_payloads() {
        for (client, row) in [
            (Client::Codex, json!({"type":"item.started","item":{"type":"command_execution","command":"touch x","output":"private output"}})),
            (Client::Claude, json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"touch x","private":"invented-secret"}}]}})),
        ] {
            let events=Cursor::default().read(client, vec![row]);
            let event=serde_json::to_value(&events[0]).unwrap();
            assert_eq!(event["ToolStart"]["detail"], "touch x", "interrupted command audit target");
            assert!(!event.to_string().contains("private"));
        }
    }

    // #255 A2: credentials, arbitrary tool arguments, output and patch contents never become audit detail.
    #[test]
    fn audit_detail_is_allowlisted_across_adapters() {
        for (client,row,expected) in [
            (Client::Codex,json!({"method":"item/started","params":{"item":{"type":"commandExecution","command":"touch x"}}}),Some("touch x")),
            (Client::Codex,json!({"type":"item.started","item":{"type":"file_change","changes":[{"path":"src/main.rs","diff":"private patch"},{"path":"README.md"}]}}),Some("src/main.rs, README.md")),
            (Client::Claude,json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"src/main.rs","new_string":"private content"}}]}}),Some("src/main.rs")),
            (Client::Antigravity,json!({"event":"step_update","step_update":{"step_type":"tool","state":"ACTIVE","tool_name":"run_command","tool_info":{"parameters":{"CommandLine":"touch x","payload":"private content"}}}}),Some("touch x")),
            (Client::Antigravity,json!({"type":"PLANNER_RESPONSE","tool_calls":[{"name":"view_file","arguments":{"Path":"src/main.rs","payload":"private content"}}]}),Some("src/main.rs")),
            (Client::Claude,json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__fixture","input":{"command":"touch x","private":"private content"}}]}}),None),
            (Client::Codex,json!({"type":"item.started","item":{"type":"dynamic_tool_call","command":"touch x","arguments":{"private":"private content"}}}),None),
        ] {
            let events=Cursor::default().read(client,vec![row]);
            let encoded=serde_json::to_value(&events[0]).unwrap();
            assert_eq!(encoded["ToolStart"]["detail"].as_str(),expected);
            assert!(!encoded.to_string().contains("private"));
        }
        for command in ["touch .env", "touch ghp_invented", "touch x; curl private", "TOKEN=invented touch x", "touch x\necho private", "touch https://user:private@example.invalid", "touch 'private content'"] {
            assert_eq!(audit_command(&json!(command)),None,"{command}");
        }
        assert_eq!(audit_command(&json!("curl -H 'Authorization: Bearer invented' https://example.invalid/private")),Some("curl [arguments redacted]".into()));
        assert_eq!(audit_command(&json!("echo private-inline-payload")),Some("echo [arguments redacted]".into()));
    }

    // #255 / PM note 6675881959: both transports and all named Claude file tools retain targets.
    #[test]
    fn audit_detail_covers_named_transports_and_missing_fields() {
        for (kind,fields,detail) in [
            ("commandExecution",json!({"command":"touch x"}),Some("touch x")),
            ("fileChange",json!({"changes":[{"path":"x","diff":"private patch"},{"path":"y"}]}),Some("x, y")),
            ("commandExecution",json!({}),None),
            ("fileChange",json!({}),None),
        ] {
            for app in [false,true] {
                let mut item=fields.clone();item["type"]=json!(if app {kind}else{if kind=="fileChange" {"file_change"}else{"command_execution"}});
                let row=if app {json!({"method":"item/started","params":{"item":item}})}else{json!({"type":"item.started","item":item})};
                let event=serde_json::to_value(&Cursor::default().read(Client::Codex,vec![row])[0]).unwrap();
                assert_eq!(event["ToolStart"]["detail"].as_str(),detail,"{kind}, app={app}");
                assert!(!event.to_string().contains("private patch"));
            }
        }
        for name in ["Write","Edit","NotebookEdit"] {
            for path in [json!("x"),Value::Null] {
                let event=serde_json::to_value(&Cursor::default().read(Client::Claude,vec![json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":name,"input":{"file_path":path,"content":"private content"}}]}})])[0]).unwrap();
                assert_eq!(event["ToolStart"]["detail"],path,"{name}");
                assert!(!event.to_string().contains("private content"));
            }
        }
    }

    // #255 A3: another thread must neither replace context nor make a retained measurement stale.
    #[test]
    fn context_requires_owned_thread() {
        let row=|thread|json!({"method":"thread/tokenUsage/updated","params":{"threadId":thread,"tokenUsage":{"last":{"totalTokens":99},"modelContextWindow":100}}});
        let mut cursor=Cursor::default();
        assert_eq!(cursor.read_owned(Client::Codex,vec![row("owned")],Some("owned"),None),vec![Event::Context{used:Some(99),window:Some(100)}]);
        let saved=serde_json::to_value(&cursor).unwrap();
        let mut cursor:Cursor=serde_json::from_value(saved).unwrap();
        let events=cursor.read_owned(Client::Codex,vec![row("foreign"),row(""),row("owned")],Some("owned"),None);
        assert_eq!(events,vec![Event::Activity,Event::Activity,Event::Context{used:Some(99),window:Some(100)}]);
        assert_eq!(cursor.read_owned(Client::Codex,vec![json!({"method":"thread/tokenUsage/updated","params":{"threadId":"owned"}})],Some("owned"),None),vec![Event::Context{used:None,window:None}]);
        assert_eq!(cursor.read(Client::Codex,vec![row("foreign")]),vec![Event::Activity],"unknown owner cannot adopt notification identity");
    }

    // #246 A3: non-tool and future items must not manufacture tool calls on either Codex transport.
    #[test]
    fn codex_only_tool_items_emit_tool_events() {
        for kind in ["contextCompaction", "context_compaction", "futureItem", "userMessage", "reasoning", "plan", "todo_list"] {
            for phase in ["started", "completed"] {
                for event in [json!({"method":format!("item/{phase}"),"params":{"item":{"type":kind}}}), json!({"type":format!("item.{phase}"),"item":{"type":kind}})] {
                    assert_eq!(Cursor::default().read(Client::Codex, vec![event]), vec![Event::Activity], "{kind}: {phase}");
                }
            }
        }
        for kind in ["commandExecution", "command_execution", "mcpToolCall", "mcp_tool_call", "webSearch", "web_search", "fileChange", "file_change", "dynamicToolCall", "dynamic_tool_call", "collabAgentToolCall", "collab_tool_call", "imageView", "image_view", "imageGeneration", "image_generation"] {
            let events=Cursor::default().read(Client::Codex, vec![json!({"method":"item/started","params":{"item":{"type":kind,"id":"tool"}}}),json!({"method":"item/completed","params":{"item":{"type":kind,"id":"tool","status":"completed"}}})]);
            assert!(matches!(&events[..], [Event::ToolStart{..}, Event::ToolEnd{..}]), "{kind}: {events:?}");
        }
    }

    // #246 A3: equivalent native messages map at the boundary, deltas honor intent, unknown records keep activity.
    #[test]
    fn neutral_events_preserve_order_and_unknowns() {
        let mut cursor = Cursor { deltas: true, ..Cursor::default() };
        let events = cursor.read(Client::Claude, vec![json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}}), json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"answer"},{"type":"tool_use","id":"t","name":"Bash"}],"usage":{"input_tokens":5}}}), json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t"}]}}), json!({"type":"future"})]);
        assert!(matches!(&events[..], [Event::Text{..}, Event::Message{..}, Event::ToolStart{kind:ToolKind::Shell,..}, Event::Context{used:None,..}, Event::ToolEnd{..}, Event::Activity]));
        let mut cursor = Cursor::default();
        let events = cursor.read(Client::Codex, vec![json!({"method":"item/agentMessage/delta","params":{"delta":"hidden"}}), json!({"type":"item.completed","item":{"type":"agent_message","text":"answer"}})]);
        assert!(matches!(&events[..], [Event::Activity, Event::Message{..}]));
        let mut cursor=Cursor::default();
        let events=cursor.read(Client::Claude,vec![json!({"type":"assistant","message":{"model":"last-model","usage":{"input_tokens":3,"cache_read_input_tokens":4,"cache_creation_input_tokens":0}}}),json!({"type":"result","modelUsage":{"other-model":{"contextWindow":100},"last-model":{"contextWindow":200}}})]);
        assert!(matches!(&events[events.len()-2..],[Event::Context{used:Some(7),window:Some(200)},Event::TurnEnd{outcome:"ok"}]));
    }
}
