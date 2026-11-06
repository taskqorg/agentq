//! Read-only, bounded native history (agentq.md §12; RUNTIME.md history wire contract).
use super::{Res, native};
mod steps;
pub mod sessions;
pub use steps::{Step, StepKind, StepPage, Turn};
use serde_json::{Value, json};
use std::{fs::File, io::{Read, Seek, SeekFrom}, path::Path};

const BUDGET: usize = 16 * 1024 * 1024;
const FORMATS: &[&str] = &["codex-exec", "codex-rollout", "codex-app-server", "claude-stream", "claude-native", "agy-stream", "antigravity-sidecar"];

/// Adapter support is distinct from native qualification. Version and OS are caller readback.
pub fn capabilities(format: &str, native_version: Option<&str>, os: &str) -> Res<Value> {
    if !FORMATS.contains(&format) { return Err("history transport unsupported".into()); }
    let sidecar = format == "antigravity-sidecar";
    let app = format == "codex-app-server";
    let mut features = serde_json::Map::new();
    for feature in ["launch", "resume", "session", "input", "files_images", "ask_reply", "tools_outputs", "heartbeat", "cancel_recovery", "model_effort_accounts_permissions", "usage_errors", "history"] {
        let (status, reason) = match feature {
            "history" if format == "agy-stream" => ("limited", "completed steps may omit text; deltas/result only; unknown shapes retained raw"),
            "history" if sidecar => ("limited", "truncated fields, GENERIC author unknown and raw-only attachments; caller selects full source"),
            "history" => ("supported", "bounded source adapter; unknown fields retained raw"),
            "tools_outputs" if sidecar => ("limited", "native calls retained; GENERIC output has unknown tool/actor/call ID; TOOL_RESPONSE schema fixture only"),
            "heartbeat" => ("limited", "source progress and process poll only; no useful-work inference"),
            "files_images" => ("limited", "caller arguments or native typed references only; no media decoding"),
            "ask_reply" if !app => ("unsupported", "no added interactive reply channel; native caller control only"),
            "cancel_recovery" if sidecar => ("limited", "submission kill is not server cancellation; original ownership retained"),
            "model_effort_accounts_permissions" => ("limited", "caller arguments/environment/parameters; no grant or catalogue inference"),
            _ => ("supported", "existing Turn/NativeTurn/AppSession boundary; not a history receipt"),
        };
        features.insert(feature.into(), json!({"status":status,"reason":reason}));
    }
    Ok(json!({"schema":1,"format":format,"native_version":native_version,"os":os,"features":features,
        "native":{"status":"untested","reason":"history API not qualified on a native version/OS tuple"},
        "evidence":["runtime/tests/history.rs","runtime/tests/agent.rs"],
        "live_output":{"status":if app {"supported"}else{"unsupported"},"reason":if app {"item/commandExecution/outputDelta"}else{"no native live console delta proof"}},
        "limits":{"page_bytes":BUDGET,"record_bytes":BUDGET,"page_events":128}}))
}

/// An observation cursor, never execution or stop authority. Protect its checkpoint as private data.
#[derive(Clone, Debug)]
pub struct History {
    format: String,
    source: String,
    session: String,
    native_session: String,
    native_parent: Option<String>,
    offset: u64,
    identity: Option<String>,
    prefix: String,
}

#[derive(Debug)]
pub struct Page {
    pub events: Vec<Value>,
    /// Forward: unconsumed source bytes. Backward: older bytes remain.
    pub more: bool,
    /// Forward: unconsumed read bytes. Backward: excluded incomplete tail bytes.
    pub pending_bytes: usize,
}

impl History {
    pub fn new(format: &str, source: &str, session: &str) -> Res<Self> {
        if !FORMATS.contains(&format) { return Err("history transport unsupported".into()); }
        if source.is_empty() || session.is_empty() { return Err("history source/session required".into()); }
        Ok(Self {format:format.into(), source:source.into(), session:session.into(), native_session:session.into(), native_parent:None, offset:0,
            identity:None, prefix:crate::sha256_hex(b"")})
    }

    pub fn checkpoint(&self) -> Value {
        json!({"schema":1,"format":self.format,"source":self.source,"session":self.session,
            "native_session":self.native_session,"native_parent":self.native_parent,
            "offset":self.offset,"identity":self.identity,"prefix":self.prefix})
    }

    pub fn recover(v: &Value) -> Res<Self> {
        if v["schema"] != 1 { return Err("history checkpoint schema invalid".into()); }
        let field = |k: &str| v[k].as_str().ok_or_else(|| format!("history checkpoint {k} invalid"));
        let mut h = Self::new(field("format")?, field("source")?, field("session")?)?;
        if let Some(native)=v.get("native_session") {h.native_session=native.as_str().filter(|s|!s.is_empty()).ok_or("history checkpoint native session invalid")?.into();}
        h.native_parent=match &v["native_parent"] {Value::Null=>None,Value::String(s) if !s.is_empty()=>Some(s.clone()),_=>return Err("history checkpoint native parent invalid".into())};
        h.offset = v["offset"].as_u64().ok_or("history checkpoint offset invalid")?;
        h.prefix = field("prefix")?.into();
        if h.prefix.len() != 64 || !h.prefix.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("history checkpoint prefix invalid".into()); }
        h.identity = match &v["identity"] { Value::Null if h.offset == 0 => None,
            Value::String(s) if !s.is_empty() => Some(s.clone()), _ => return Err("history checkpoint identity invalid".into()) };
        Ok(h)
    }

    /// A failed poll leaves the cursor untouched. No source is edited or input submitted.
    pub fn poll(&mut self, path: &Path) -> Res<Page> {
        let (mut file, identity) = self.open(path)?;
        file.seek(SeekFrom::Start(self.offset)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&mut file).take(BUDGET as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let mut consumed = 0;
        let mut events = Vec::new();
        for row in bytes.split_inclusive(|b| *b == b'\n') {
            if events.len() == 128 { break; }
            if row.len() > BUDGET { return Err("history record exceeds 16 MiB".into()); }
            if !row.ends_with(b"\n") || consumed + row.len() > BUDGET { break; }
            let raw = std::str::from_utf8(row).map_err(|_| "history invalid UTF-8")?;
            let v: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !v.is_object() { return Err("history invalid record".into()); }
            events.push(self.event(&v, raw, self.offset + consumed as u64)?);
            consumed += row.len();
        }
        let end = self.offset + consumed as u64;
        let prefix = native::digest_prefix(&mut file, end)?;
        let more = consumed < bytes.len();
        self.offset = end; self.prefix = prefix; self.identity = Some(identity);
        Ok(Page {events, more, pending_bytes:bytes.len()-consumed})
    }

    /// Read older records before this cursor, or from EOF on a fresh cursor.
    /// Save/recover checkpoints exactly as for `poll`; failure never advances the cursor.
    pub fn poll_before(&mut self, path: &Path) -> Res<Page> {
        let (mut file, identity) = self.open(path)?;
        let bound = if self.identity.is_none() { file.metadata().map_err(|e| e.to_string())?.len() } else { self.offset };
        let start = bound.saturating_sub(BUDGET as u64 + 1);
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&mut file).take(bound - start).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != bound - start { return Err("history truncated".into()); }
        let mut end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let pending_bytes = bytes.len() - end;
        let mut events = Vec::new();
        let mut used = 0;
        while end > 0 && events.len() < 128 {
            let row_start = match bytes[..end-1].iter().rposition(|b| *b == b'\n') {
                Some(i) => i + 1,
                None if start == 0 => 0,
                None => break,
            };
            let row = &bytes[row_start..end];
            if row.len() > BUDGET { return Err("history record exceeds 16 MiB".into()); }
            if used + row.len() > BUDGET { break; }
            let raw = std::str::from_utf8(row).map_err(|_| "history invalid UTF-8")?;
            let v: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !v.is_object() { return Err("history invalid record".into()); }
            events.push(self.event(&v, raw, start + row_start as u64)?);
            used += row.len();
            end = row_start;
        }
        if (events.is_empty() && pending_bytes == 0 && bound > 0) || pending_bytes > BUDGET {
            return Err("history record exceeds 16 MiB".into());
        }
        // Skip only the incomplete EOF tail; never start inside a complete record.
        let offset = start + end as u64;
        let prefix = native::digest_prefix(&mut file, offset)?;
        events.reverse();
        self.offset = offset; self.prefix = prefix; self.identity = Some(identity);
        Ok(Page {events, more:offset > 0, pending_bytes})
    }

    fn open(&self, path: &Path) -> Res<(File, String)> {
        let mut file = File::open(path).map_err(|e| format!("history unavailable: {e}"))?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("history not a regular file".into()); }
        let identity = native::file_id(&file)?;
        if self.identity.as_ref().is_some_and(|id| *id != identity) { return Err("history replaced".into()); }
        if native::digest_prefix(&mut file, self.offset)? != self.prefix { return Err("history truncated or rewritten".into()); }
        if self.offset > 0 {
            file.seek(SeekFrom::Start(self.offset - 1)).map_err(|e| e.to_string())?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(|e| e.to_string())?;
            if byte[0] != b'\n' { return Err("history cursor is not a record boundary".into()); }
        }
        Ok((file, identity))
    }

    fn event(&self, v: &Value, raw: &str, offset: u64) -> Res<Value> {
        let client = if self.format.starts_with("codex-") {super::Client::Codex}
            else if self.format.starts_with("claude-") {super::Client::Claude} else {super::Client::Antigravity};
        let session = super::session_of(client, raw);
        let native_session = if self.format.starts_with("claude-") && v["agentId"].is_string() {v["agentId"].as_str()}else{session.as_deref().or_else(|| match self.format.as_str() {
            "claude-native" | "claude-stream" => v["sessionId"].as_str().or(v["session_id"].as_str()),
            "agy-stream" => v["step_update"]["conversation_id"].as_str(),
            "codex-rollout" if v["type"] == "session_meta" => v["payload"]["id"].as_str(),
            "codex-rollout" => v["payload"]["thread_id"].as_str(),
            "codex-app-server" => v["params"]["threadId"].as_str().or(v["params"]["thread"]["id"].as_str()),
            _ => None,
        })};
        if !(self.format == "codex-rollout" && v["type"] == "session_meta" && offset > 0) && native_session.is_some_and(|s| s != self.native_session) { return Err("history native session differs".into()); }
        if self.native_parent.as_ref().is_some_and(|parent|v["sessionId"].as_str().is_some_and(|id|id!=parent)){return Err("history native parent differs".into());}
        let (author, kind, item, blocks, limited) = normalize(&self.format, v);
        let validity = match self.format.as_str() {
            "claude-native" | "claude-stream" if v["type"].as_str().is_some_and(|s| !s.is_empty()) => json!({"status":"valid","reason":null}),
            "claude-native" | "claude-stream" => json!({"status":"invalid","reason":"missing-or-invalid-record-type"}),
            _ => json!({"status":"unknown","reason":"envelope-validation-unsupported"}),
        };
        let hash = crate::sha256_hex(raw.as_bytes());
        let id = crate::sha256_hex(json!([self.format,self.source,self.session,offset,hash]).to_string().as_bytes());
        let time = v.get("timestamp").or_else(|| v.get("time"))
            .or_else(|| v.get("emittedAtMs"))
            .or_else(|| if self.format == "antigravity-sidecar" {v.get("created_at")}else{None})
            .cloned().unwrap_or(Value::Null);
        Ok(json!({"id":id,"source":self.source,"session":self.session,"session_binding":if self.format == "codex-rollout" && v["type"] == "session_meta" && offset > 0 {"caller"}else if native_session.is_some(){"native"}else{"caller"},"provenance":self.format == "codex-rollout" && v["type"] == "session_meta" && offset > 0,"format":self.format,
            "order":offset,"start":offset,"end":offset+raw.len() as u64,"sha256":hash,"raw":raw,
            "validity":validity,"assistant_failure":super::events::assistant_failure(client,v),
            "lifecycle":super::events::lifecycle(client,v),"permission_mode":super::events::permission_mode(client,v),
            "time":time,"author":author,"kind":kind,"item_id":item,"blocks":blocks,"limited":limited}))
    }
}

fn text(s: &Value) -> Vec<Value> {
    s.as_str().map(|s| vec![json!({"type":"text","text":s})]).unwrap_or_default()
}

// Native blocks remain native JSON; only the envelope differs by adapter.
fn normalize(format: &str, v: &Value) -> (&'static str, &'static str, Value, Vec<Value>, &'static str) {
    let mut author = "unknown";
    let mut kind = "unknown";
    let mut blocks = vec![];
    let mut item = Value::Null;
    let mut limited = "unsupported-source-shape";
    match format {
        "claude-stream" | "claude-native" => {
            item = v.get("uuid").or_else(|| v["message"].get("id")).cloned().unwrap_or(Value::Null);
            match v["type"].as_str() {
                Some("user" | "assistant") => {
                    author = if v["type"] == "user" {"user"} else {"assistant"}; kind = "message";
                    blocks = v["message"]["content"].as_array().cloned().unwrap_or_else(|| text(&v["message"]["content"]));
                    limited = "none";
                }
                Some("control_request" | "control_response") => {
                    author="system";kind="lifecycle";
                    item=v.get("request_id").or_else(||v["response"].get("request_id")).cloned().unwrap_or(Value::Null);
                    blocks.push(v.clone());limited="none";
                }
                Some("attachment") => {kind="attachment"; blocks.push(v["attachment"].clone()); limited="none";}
                Some("result" | "system" | "rate_limit_event") => {
                    author="system";kind="lifecycle";
                    if v["type"] == "result" { blocks = text(&v["result"]); }
                    blocks.push(v.clone());limited="none";
                }
                _ => {}
            }
        }
        "codex-exec" | "codex-rollout" | "codex-app-server" => {
            let p = if format == "codex-rollout" {&v["payload"]}
                else if format == "codex-app-server" {&v["params"]} else {v};
            let i = p.get("item").unwrap_or(p);
            item = i.get("id").or_else(||i.get("call_id")).cloned().unwrap_or(Value::Null);
            match i["type"].as_str() {
                Some("message" | "agent_message" | "AgentMessage" | "agentMessage" | "userMessage" | "UserMessage") => {
                    author = match i["role"].as_str() {Some("user")=>"user",Some("assistant")=>"assistant",_ if i["type"]=="userMessage" || i["type"]=="UserMessage"=>"user",_ if i["type"]!="message"=>"assistant",_=>"unknown"};
                    kind="message"; blocks=i["content"].as_array().cloned().unwrap_or_else(||text(&i["text"])); limited="none";
                }
                Some("command_execution" | "commandExecution" | "CommandExecution" | "function_call" | "custom_tool_call" | "function_call_output" | "custom_tool_call_output" | "mcp_tool_call" | "mcpToolCall" | "McpToolCall" | "dynamicToolCall") => {author="tool";kind="tool";blocks.push(i.clone());limited="none";}
                _ => {
                    if matches!(v["type"].as_str(),Some("thread.started"|"turn.started"|"turn.completed"|"turn.failed"|"error"))
                        || matches!(p["type"].as_str(),Some("task_started"|"task_complete"|"turn_aborted"|"token_count"))
                        || matches!(v["method"].as_str(),Some("thread/started"|"turn/started"|"turn/completed"|"error")) {
                        author="system";kind="lifecycle";blocks.push(v.clone());limited="none";
                    }
                }
            }
        }
        "agy-stream" => {
            match v["event"].as_str() {
                Some("step_update") => {
                    let step = &v["step_update"]; item = step["step_index"].clone();
                    match step["step_type"].as_str() {
                        Some("agent_response") => {author="assistant";kind="message";blocks=text(&step["text_delta"]);limited=if blocks.is_empty(){"text-not-emitted"}else{"none"};}
                        Some("user_input") => {author="user";kind="message";limited="text-not-emitted";}
                        Some("tool") => {author="tool";kind="tool";blocks.push(step["tool_info"].clone());limited="none";}
                        Some("system_message" | "checkpoint") => {author="system";kind="lifecycle";blocks.push(step.clone());limited="none";}
                        _ => {}
                    }
                }
                Some("init") => {author="system";kind="lifecycle";blocks.push(v.clone());limited="none";}
                Some("result") => {author="assistant";kind="result";blocks=text(&v["result"]["response"]);limited="none";}
                _ => {}
            }
        }
        "antigravity-sidecar" => {
            item = v["step_index"].clone();
            let truncation = match v.get("truncated_fields") {
                None | Some(Value::Null) => "none",
                Some(Value::Array(fields)) if fields.is_empty() => "none",
                Some(Value::Array(_)) => "truncated-source",
                _ => "unsupported-truncation-field",
            };
            match v["type"].as_str() {
                Some("USER_INPUT" | "PLANNER_RESPONSE") => {
                    author=if v["type"]=="USER_INPUT"{"user"}else{"assistant"};kind="message";
                    blocks=text(&v["content"]);
                    if let Some(calls)=v["tool_calls"].as_array(){blocks.extend(calls.iter().cloned());}
                    limited=if truncation != "none" {truncation}else if v.get("attachments").is_some(){"attachments-raw-only"}else{"none"};
                }
                Some("TOOL_RESPONSE") => {author="tool";kind="tool";blocks.push(v.clone());limited=truncation;}
                Some("SYSTEM_MESSAGE") => {author="system";kind="lifecycle";blocks=text(&v["content"]);limited=truncation;}
                Some("GENERIC") => {blocks=text(&v["content"]);limited=if truncation != "none" {truncation}else{"generic-author-unknown"};}
                _=>{}
            }
        }
        _ => {}
    }
    let blocks: Vec<Value> = blocks.into_iter().map(|v| block(format, v)).collect();
    if limited == "none" && matches!(kind, "message" | "attachment" | "tool")
        && blocks.iter().any(|b| b["type"] == "unknown") { limited = "unsupported-content-block"; }
    (author,kind,item,blocks,limited)
}

fn block(format: &str, v: Value) -> Value {
    let kind = v["type"].as_str().unwrap_or_default();
    match kind {
        "text" | "Text" | "input_text" | "output_text" => json!({"type":"text","text":v["text"],"native":v}),
        "image" | "Image" | "input_image" | "image_url" | "local_image" | "LocalImage" => json!({"type":"image","native":v}),
        "document" | "attachment" | "file" | "input_file" => json!({"type":"attachment","native":v}),
        "tool_use" | "function_call" | "custom_tool_call" => json!({"type":"tool_call",
            "id":v.get("call_id").or_else(||v.get("id")),"name":v["name"],
            "input":v.get("input").or_else(||v.get("arguments")),"native":v}),
        "tool_result" | "function_call_output" | "custom_tool_call_output" => json!({"type":"tool_result",
            "id":v.get("tool_use_id").or_else(||v.get("call_id")),
            "output":v.get("content").or_else(||v.get("output")),"native":v}),
        "command_execution" | "commandExecution" | "CommandExecution" => json!({"type":"tool_result","id":v["id"],
            "name":"shell","input":v["command"],"output":v.get("aggregated_output").or_else(||v.get("aggregatedOutput")),"native":v}),
        "mcp_tool_call" | "mcpToolCall" | "McpToolCall" | "dynamicToolCall" | "TOOL_RESPONSE" => json!({"type":"tool_result","native":v}),
        _ if format == "antigravity-sidecar" && v.get("type").is_none()
            && v.get("args").is_some() && v["name"].as_str().is_some() => {
            json!({"type":"tool_call","id":v.get("id"),"name":v["name"],"input":v["args"],"native":v})
        }
        _ if v.get("parameters").is_some() && v.get("name").is_some() => {
            json!({"type":if v.get("output").is_some() || v.get("error").is_some(){"tool_result"}else{"tool_call"},
                "name":v["name"],"input":v["parameters"],"output":v.get("output"),"error":v.get("error"),"native":v})
        }
        _ => json!({"type":"unknown","native":v}),
    }
}
