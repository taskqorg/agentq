//! Native records become ordered runtime-neutral events at the adapter boundary.
use super::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ToolKind { Shell, Mcp, Web, Other }

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Event {
    Text { item: Option<String>, delta: String },
    Message { item: Option<String>, text: String },
    ToolStart { id: Option<String>, kind: ToolKind, name: Option<String>, detail: Option<String> },
    ToolEnd { id: Option<String>, status: Option<String> },
    Quota(Vec<Window>),
    Context { used: Option<u64>, window: Option<u64> },
    Activity,
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
}

fn string(v: &Value) -> Option<String> { v.as_str().map(String::from) }
fn tool_kind(kind: &str) -> ToolKind {
    match kind { "command_execution" | "commandExecution" | "Bash" | "run_command" => ToolKind::Shell,
        "mcp_tool_call" | "mcpToolCall" => ToolKind::Mcp,
        "web_search" | "webSearch" => ToolKind::Web, _ if kind.starts_with("mcp__") => ToolKind::Mcp, _ => ToolKind::Other }
}

// Local audit targets only: never serialize arbitrary native tool input or patches.
fn audit_path(value: &Value) -> Option<String> {
    let text=value.as_str()?;
    let lower=text.to_ascii_lowercase();
    if text.is_empty() || text.len()>512 || !text.bytes().all(|b|b.is_ascii_alphanumeric() || b"/\\._-".contains(&b))
        || ["secret", "token", "password", "credential", "auth", ".env", "id_rsa", "id_ed25519", "api_key", "apikey", "ghp_", "github_pat_", "sk-", "bearer"].iter().any(|s|lower.contains(s)) {return None;}
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

fn antigravity_detail(name: Option<&str>, input: &Value) -> Option<String> {
    match name {
        Some("run_command")=>audit_command(&input["CommandLine"]),
        Some("view_file"|"write_to_file"|"replace_file_content"|"multi_replace_file_content")=>audit_path(input.get("Path").or(input.get("TargetFile")).unwrap_or(&Value::Null)),
        _=>None,
    }
}

impl Cursor {
    pub fn new(deltas: bool) -> Self { Self { deltas, ..Self::default() } }
    pub fn read(&mut self, client: Client, values: Vec<Value>) -> Vec<Event> { self.read_owned(client, values, None) }
    pub fn read_owned(&mut self, client: Client, values: Vec<Value>, thread: Option<&str>) -> Vec<Event> {
        let mut out = vec![];
        for v in values {
            let before = out.len();
            if let Some(w) = limits::reported(client, &v, (unix_ms()/1000).max(0) as u64) { out.push(Event::Quota(w)); }
            match client {
                Client::Claude => self.claude(&v, &mut out),
                Client::Codex => self.codex(&v, &mut out, thread),
                Client::Antigravity => self.antigravity(&v, &mut out),
            }
            if out.len() == before { out.push(Event::Activity); }
        }
        out
    }

    fn claude(&mut self, v: &Value, out: &mut Vec<Event>) {
        if let Some(models) = v["modelUsage"].as_object() {
            self.window = self.model.as_ref().and_then(|m|models.get(m)).and_then(|m|m["contextWindow"].as_u64());
            out.push(Event::Context { used: self.used, window: self.window });
        }
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
                        out.push(Event::ToolStart { id, kind, name, detail: match block["name"].as_str() {Some("Bash")=>audit_command(&block["input"]["command"]),Some("Read"|"Edit"|"Write"|"MultiEdit")=>audit_path(&block["input"]["file_path"]),_=>None} });
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
            if start { out.push(Event::ToolStart { id: id.clone(), kind: tool_kind(native), name: string(&i["tool"]).or_else(|| string(&i["name"])), detail: match native {"commandExecution"|"command_execution"=>audit_command(&i["command"]),"fileChange"|"file_change"=>audit_files(&i["changes"]),_=>None} }); }
            if end { out.push(Event::ToolEnd { id, status: string(&i["status"]) }); }
        }
    }

    fn antigravity(&mut self, v: &Value, out: &mut Vec<Event>) {
        if v["event"] == "result" {
            if let Some(text) = string(&v["result"]["response"]) { out.push(Event::Message { item: None, text }); }
        }
        if v["event"] == "step_update" {
            let s = &v["step_update"];
            let id = s["step_index"].as_u64().map(|n| n.to_string());
            if self.deltas { if let Some(delta) = string(&s["text_delta"]) { out.push(Event::Text { item: id.clone(), delta }); } }
            if s["step_type"] == "tool" {
                if s["state"] == "ACTIVE" { out.push(Event::ToolStart { id: id.clone(), kind: ToolKind::Other, name: string(&s["tool_name"]), detail: antigravity_detail(s["tool_name"].as_str(), &s["tool_info"]["parameters"]) }); }
                if matches!(s["state"].as_str(), Some("DONE" | "ERROR" | "CANCELLED")) { out.push(Event::ToolEnd { id, status: string(&s["state"]) }); }
            }
        }
        if v["type"] == "PLANNER_RESPONSE" && v["status"] == "DONE" {
            if let Some(text) = string(&v["content"]) { out.push(Event::Message { item: v["step_index"].as_u64().map(|n| n.to_string()), text }); }
        }
        if v["type"] == "PLANNER_RESPONSE" {
            if let Some(tools)=v["tool_calls"].as_array() {
                for tool in tools {
                    let id=string(&tool["id"]);
                    if id.as_ref().is_none_or(|id|self.seen.insert(format!("tool:{id}"))) {out.push(Event::ToolStart {id,kind:ToolKind::Other,name:string(&tool["name"]),detail:antigravity_detail(tool["name"].as_str(), &tool["arguments"])});}
                }
            }
        }
        if v["type"] == "TOOL_RESPONSE" && matches!(v["status"].as_str(),Some("DONE"|"ERROR"|"CANCELLED")) {
            out.push(Event::ToolEnd {id:string(&v["tool_call_id"]),status:string(&v["status"])});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // #255 A3: another thread must neither replace context nor make a retained measurement stale.
    #[test]
    fn context_requires_owned_thread() {
        let row=|thread|json!({"method":"thread/tokenUsage/updated","params":{"threadId":thread,"tokenUsage":{"last":{"totalTokens":99},"modelContextWindow":100}}});
        let mut cursor=Cursor::default();
        assert_eq!(cursor.read_owned(Client::Codex,vec![row("owned")],Some("owned")),vec![Event::Context{used:Some(99),window:Some(100)}]);
        let saved=serde_json::to_value(&cursor).unwrap();
        let mut cursor:Cursor=serde_json::from_value(saved).unwrap();
        let events=cursor.read_owned(Client::Codex,vec![row("foreign"),row(""),row("owned")],Some("owned"));
        assert_eq!(events,vec![Event::Activity,Event::Activity,Event::Context{used:Some(99),window:Some(100)}]);
        assert_eq!(cursor.read_owned(Client::Codex,vec![json!({"method":"thread/tokenUsage/updated","params":{"threadId":"owned"}})],Some("owned")),vec![Event::Context{used:None,window:None}]);
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
        assert!(matches!(events.last(),Some(Event::Context{used:Some(7),window:Some(200)})));
    }
}
