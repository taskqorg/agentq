use super::*;
pub(super) fn normalize(
    format: &str,
    v: &Value,
) -> (&'static str, &'static str, Value, Vec<Value>, &'static str) {
    let mut author = "unknown";
    let mut kind = "unknown";
    let mut blocks = vec![];
    let mut item = Value::Null;
    let mut limited = "unsupported-source-shape";
    match format {
        "codex-exec" | "codex-rollout" | "codex-app-server" => {
            let p = if format == "codex-rollout" {
                &v["payload"]
            } else if format == "codex-app-server" {
                &v["params"]
            } else {
                v
            };
            let i = p.get("item").unwrap_or(p);
            item = i
                .get("id")
                .or_else(|| i.get("call_id"))
                .cloned()
                .unwrap_or(Value::Null);
            match i["type"].as_str() {
                Some(
                    "message" | "agent_message" | "AgentMessage" | "agentMessage" | "userMessage"
                    | "UserMessage",
                ) => {
                    author = match i["role"].as_str() {
                        Some("user") => "user",
                        Some("assistant") => "assistant",
                        _ if i["type"] == "userMessage" || i["type"] == "UserMessage" => "user",
                        _ if i["type"] != "message" => "assistant",
                        _ => "unknown",
                    };
                    kind = "message";
                    blocks = i["content"]
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| text(&i["text"]));
                    limited = "none";
                }
                Some(
                    "command_execution"
                    | "commandExecution"
                    | "CommandExecution"
                    | "function_call"
                    | "custom_tool_call"
                    | "function_call_output"
                    | "custom_tool_call_output"
                    | "mcp_tool_call"
                    | "mcpToolCall"
                    | "McpToolCall"
                    | "dynamicToolCall",
                ) => {
                    author = "tool";
                    kind = "tool";
                    blocks.push(i.clone());
                    limited = "none";
                }
                _ => {
                    if matches!(
                        v["type"].as_str(),
                        Some(
                            "thread.started"
                                | "turn.started"
                                | "turn.completed"
                                | "turn.failed"
                                | "error"
                        )
                    ) || matches!(
                        p["type"].as_str(),
                        Some("task_started" | "task_complete" | "turn_aborted" | "token_count")
                    ) || matches!(
                        v["method"].as_str(),
                        Some("thread/started" | "turn/started" | "turn/completed" | "error")
                    ) {
                        author = "system";
                        kind = "lifecycle";
                        blocks.push(v.clone());
                        limited = "none";
                    }
                }
            }
        }
        _ => {}
    }
    let blocks: Vec<Value> = blocks.into_iter().map(|v| block(format, v)).collect();
    if limited == "none"
        && matches!(kind, "message" | "attachment" | "tool")
        && blocks.iter().any(|b| b["type"] == "unknown")
    {
        limited = "unsupported-content-block";
    }
    (author, kind, item, blocks, limited)
}

fn block(format: &str, v: Value) -> Value {
    let kind = v["type"].as_str().unwrap_or_default();
    match kind {
        "text" | "Text" | "input_text" | "output_text" => {
            json!({"type":"text","text":v["text"],"native":v})
        }
        "image" | "Image" | "input_image" | "image_url" | "local_image" | "LocalImage" => {
            json!({"type":"image","native":v})
        }
        "document" | "attachment" | "file" | "input_file" => {
            json!({"type":"attachment","native":v})
        }
        "tool_use" | "function_call" | "custom_tool_call" => json!({"type":"tool_call",
            "id":v.get("call_id").or_else(||v.get("id")),"name":v["name"],
            "input":v.get("input").or_else(||v.get("arguments")),"native":v}),
        "tool_result" | "function_call_output" | "custom_tool_call_output" => {
            json!({"type":"tool_result",
            "id":v.get("tool_use_id").or_else(||v.get("call_id")),
            "output":v.get("content").or_else(||v.get("output")),"native":v})
        }
        "command_execution" | "commandExecution" | "CommandExecution" => {
            json!({"type":"tool_result","id":v["id"],
            "name":"shell","input":v["command"],"output":v.get("aggregated_output").or_else(||v.get("aggregatedOutput")),"native":v})
        }
        "mcp_tool_call" | "mcpToolCall" | "McpToolCall" | "dynamicToolCall" | "TOOL_RESPONSE" => {
            json!({"type":"tool_result","native":v})
        }
        _ if format == "antigravity-sidecar"
            && v.get("type").is_none()
            && v.get("args").is_some()
            && v["name"].as_str().is_some() =>
        {
            json!({"type":"tool_call","id":v.get("id"),"name":v["name"],"input":v["args"],"native":v})
        }
        _ if v.get("parameters").is_some() && v.get("name").is_some() => {
            json!({"type":if v.get("output").is_some() || v.get("error").is_some(){"tool_result"}else{"tool_call"},
                "name":v["name"],"input":v["parameters"],"output":v.get("output"),"error":v.get("error"),"native":v})
        }
        _ => json!({"type":"unknown","native":v}),
    }
}

pub(super) fn history_session(format: &str, v: &Value, raw: &str) -> Option<String> {
    session(raw).or_else(|| {
        (match format {
            "codex-rollout" if v["type"] == "session_meta" => v["payload"]["id"].as_str(),
            "codex-rollout" => v["payload"]["thread_id"].as_str(),
            "codex-app-server" => v["params"]["threadId"]
                .as_str()
                .or(v["params"]["thread"]["id"].as_str()),
            _ => None,
        })
        .map(String::from)
    })
}
pub(super) fn validity(_: &Value) -> Value {
    json!({"status":"unknown","reason":"envelope-validation-unsupported"})
}

fn text(s: &Value) -> Vec<Value> {
    s.as_str()
        .map(|s| vec![json!({"type":"text","text":s})])
        .unwrap_or_default()
}
