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
        "claude-stream" | "claude-native" => {
            item = v
                .get("uuid")
                .or_else(|| v["message"].get("id"))
                .cloned()
                .unwrap_or(Value::Null);
            match v["type"].as_str() {
                Some("user" | "assistant") => {
                    author = if v["type"] == "user" {
                        "user"
                    } else {
                        "assistant"
                    };
                    kind = "message";
                    blocks = v["message"]["content"]
                        .as_array()
                        .cloned()
                        .unwrap_or_else(|| text(&v["message"]["content"]));
                    limited = "none";
                }
                Some("control_request" | "control_response") => {
                    author = "system";
                    kind = "lifecycle";
                    item = v
                        .get("request_id")
                        .or_else(|| v["response"].get("request_id"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    blocks.push(v.clone());
                    limited = "none";
                }
                Some("attachment") => {
                    kind = "attachment";
                    blocks.push(v["attachment"].clone());
                    limited = "none";
                }
                Some("result" | "system" | "rate_limit_event") => {
                    author = "system";
                    kind = "lifecycle";
                    if v["type"] == "result" {
                        blocks = text(&v["result"]);
                    }
                    blocks.push(v.clone());
                    limited = "none";
                }
                _ => {}
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
        "text" | "Text" | "input_text" | "output_text" => json!({"type":"text","text":v["text"],"native":v}),
        "image" | "Image" | "input_image" | "image_url" | "local_image" | "LocalImage" => json!({"type":"image","native":v}),
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
        "command_execution" | "commandExecution" | "CommandExecution" => json!({"type":"tool_result","id":v["id"],
            "name":"shell","input":v["command"],"output":v.get("aggregated_output").or_else(||v.get("aggregatedOutput")),"native":v}),
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

pub(super) fn history_session(_format: &str, v: &Value, raw: &str) -> Option<String> {
    session(raw)
        .or_else(|| (v["sessionId"].as_str().or(v["session_id"].as_str())).map(String::from))
}
pub(super) fn validity(v: &Value) -> Value {
    if v["type"].as_str().is_some_and(|s| !s.is_empty()) {
        json!({"status":"valid","reason":null})
    } else {
        json!({"status":"invalid","reason":"missing-or-invalid-record-type"})
    }
}

fn text(s: &Value) -> Vec<Value> {
    s.as_str()
        .map(|s| vec![json!({"type":"text","text":s})])
        .unwrap_or_default()
}
