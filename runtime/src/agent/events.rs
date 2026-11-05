//! Native records become ordered runtime-neutral events at the adapter boundary.
use super::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ToolKind { Shell, Mcp, Web, Other }

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Event {
    Text { item: Option<String>, delta: String },
    Message { item: Option<String>, text: String },
    ToolStart { id: Option<String>, kind: ToolKind, name: Option<String> },
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

impl Cursor {
    pub fn new(deltas: bool) -> Self { Self { deltas, ..Self::default() } }
    pub fn read(&mut self, client: Client, values: Vec<Value>) -> Vec<Event> {
        let mut out = vec![];
        for v in values {
            let before = out.len();
            if let Some(w) = limits::reported(client, &v, (unix_ms()/1000).max(0) as u64) { out.push(Event::Quota(w)); }
            match client {
                Client::Claude => self.claude(&v, &mut out),
                Client::Codex => self.codex(&v, &mut out),
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
                        out.push(Event::ToolStart { id, kind, name });
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

    fn codex(&mut self, v: &Value, out: &mut Vec<Event>) {
        let method = v["method"].as_str().unwrap_or("");
        let p = &v["params"];
        if self.deltas && method == "item/agentMessage/delta" {
            if let Some(delta) = string(&p["delta"]) { out.push(Event::Text { item: string(&p["itemId"]), delta }); }
        }
        if method == "thread/tokenUsage/updated" {
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
        } else if !matches!(native, "userMessage" | "reasoning" | "plan" | "todo_list") {
            if start { out.push(Event::ToolStart { id: id.clone(), kind: tool_kind(native), name: string(&i["tool"]).or_else(|| string(&i["name"])) }); }
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
                if s["state"] == "ACTIVE" { out.push(Event::ToolStart { id: id.clone(), kind: ToolKind::Other, name: string(&s["tool_name"]) }); }
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
                    if id.as_ref().is_none_or(|id|self.seen.insert(format!("tool:{id}"))) {out.push(Event::ToolStart {id,kind:ToolKind::Other,name:string(&tool["name"])});}
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
