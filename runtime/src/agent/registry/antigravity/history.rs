use super::*;
pub(in crate::agent) fn input_state(format:&str,record:&Value)->Option<&'static str> {
    if format!="agy-stream" || record["event"]!="step_update" || record["step_update"]["step_type"]!="user_input" {return None;}
    Some(match record["step_update"]["state"].as_str() {Some("ACTIVE")=>"started",Some("DONE")=>"completed",_=>"unknown"})
}

pub(super) fn normalize(
    format: &str,
    v: &Value,
) -> (&'static str, &'static str, Value, Vec<Value>, &'static str) {
    if format=="acp" {return crate::agent::acp::normalize(v);}
    let mut author = "unknown";
    let mut kind = "unknown";
    let mut blocks = vec![];
    let mut item = Value::Null;
    let mut limited = "unsupported-source-shape";
    match format {
        "agy-stream" => match v["event"].as_str() {
            Some("step_update") => {
                let step = &v["step_update"];
                item = step["step_index"].clone();
                match step["step_type"].as_str() {
                    Some("agent_response") => {
                        author = "assistant";
                        kind = "message";
                        blocks = text(&step["text_delta"]);
                        limited = if blocks.is_empty() {
                            "text-not-emitted"
                        } else {
                            "none"
                        };
                    }
                    Some("user_input") => {
                        author = "user";
                        kind = "message";
                        limited = "text-not-emitted";
                    }
                    Some("tool") => {
                        author = "tool";
                        kind = "tool";
                        blocks.push(step["tool_info"].clone());
                        limited = "none";
                    }
                    Some("system_message" | "checkpoint") => {
                        author = "system";
                        kind = "lifecycle";
                        blocks.push(step.clone());
                        limited = "none";
                    }
                    _ => {}
                }
            }
            Some("user") if v["message"]["content"].is_string() => {
                author="user";kind="message";blocks=text(&v["message"]["content"]);limited="none";
            }
            Some("init") => {
                author = "system";
                kind = "lifecycle";
                blocks.push(v.clone());
                limited = "none";
            }
            Some("result") => {
                author = "assistant";
                kind = "result";
                blocks = text(&v["result"]["response"]);
                limited = "none";
            }
            _ => {}
        },
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
                    author = if v["type"] == "USER_INPUT" {
                        "user"
                    } else {
                        "assistant"
                    };
                    kind = "message";
                    blocks = text(&v["content"]);
                    if let Some(calls) = v["tool_calls"].as_array() {
                        blocks.extend(calls.iter().cloned());
                    }
                    limited = if truncation != "none" {
                        truncation
                    } else if v.get("attachments").is_some() {
                        "attachments-raw-only"
                    } else {
                        "none"
                    };
                }
                Some("TOOL_RESPONSE") => {
                    author = "tool";
                    kind = "tool";
                    blocks.push(v.clone());
                    limited = truncation;
                }
                Some("SYSTEM_MESSAGE") => {
                    author = "system";
                    kind = "lifecycle";
                    blocks = text(&v["content"]);
                    limited = truncation;
                }
                Some("GENERIC") => {
                    blocks = text(&v["content"]);
                    limited = if truncation != "none" {
                        truncation
                    } else {
                        "generic-author-unknown"
                    };
                }
                _ => {}
            }
        }
        _ => {}
    }
    let mut blocks: Vec<Value> = blocks.into_iter().map(|v| block(format, v)).collect();
    if format=="agy-stream" {
        for block in &mut blocks {if block["type"]=="text" {
            if v["event"]=="user" {block["delivery"]=json!("intent");}
            if v["event"]=="result" {block["text_mode"]=json!("complete");}
            if v["event"]=="step_update" && v["step_update"]["step_type"]=="agent_response" {block["text_mode"]=json!("delta");}
        }}
    }
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
        "image" | "Image" | "input_image" | "output_image" | "image_url" | "local_image" | "LocalImage" => {
            json!({"type":"image","image":crate::agent::history::media::recorded_image(&v),"native":v})
        }
        "document" | "attachment" | "file" | "input_file" => {
            json!({"type":"attachment","native":v})
        }
        "tool_use" | "function_call" | "custom_tool_call" => {
            let raw=v.get("input").or_else(||v.get("arguments")).cloned().unwrap_or(Value::Null);
            let free=kind=="custom_tool_call" && raw.is_string();
            let input=if free {raw.clone()}else{raw.as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or(raw.clone())};
            let input_kind=if kind=="custom_tool_call" {if free {"free_text"}else{"unknown"}}else if input.is_object() || input.is_array() {"structured"}else{"unknown"};
            let summary=None::<String>;
            json!({"type":"tool_call","id":v.get("call_id").or_else(||v.get("id")),"name":v["name"],
                "input":input,"input_kind":input_kind,"summary":summary,"native":v})
        },
        "tool_result" | "function_call_output" | "custom_tool_call_output" => {
            let output=v.get("content").or_else(||v.get("output")).cloned().unwrap_or(Value::Null);
            let parts=output.as_array().map(|parts|parts.iter().cloned().map(|part|block(format,part)).collect::<Vec<_>>())
                .or_else(||output.as_str().map(|_|text(&output).into_iter().map(|part|block(format,part)).collect::<Vec<_>>()));
            json!({"type":"tool_result","id":v.get("tool_use_id").or_else(||v.get("call_id")),
                "output":output,"parts":parts,"unknown":if parts.is_none(){json!({"parts":"native result content missing or unsupported"})}else{json!({})},"native":v})
        }
        "command_execution" | "commandExecution" | "CommandExecution" => {
            let output=v.get("aggregated_output").or_else(||v.get("aggregatedOutput")).cloned().unwrap_or(Value::Null);
            let parts=output.as_array().map(|parts|parts.iter().cloned().map(|part|block(format,part)).collect::<Vec<_>>())
                .or_else(||output.as_str().map(|_|text(&output).into_iter().map(|part|block(format,part)).collect::<Vec<_>>()));
            json!({"type":"tool_result","id":v["id"],"name":"shell","input":v["command"],"output":output,"parts":parts,"unknown":if parts.is_none(){json!({"parts":"native result content missing or unsupported"})}else{json!({})},"native":v})
        }
        "mcp_tool_call" | "mcpToolCall" | "McpToolCall" | "dynamicToolCall" | "TOOL_RESPONSE" => {
            json!({"type":"tool_result","parts":null,"unknown":{"parts":"native result content not decoded"},"native":v})
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
        _ => json!({"type":"unknown","name":v["name"].as_str(),"native":v}),
    }
}

pub(super) fn history_session(format: &str, v: &Value, raw: &str) -> Option<String> {
    if format=="acp" {return v["message"]["params"]["sessionId"].as_str().or(v["message"]["result"]["sessionId"].as_str()).map(String::from);}
    session(raw).or_else(|| {
        (if format == "agy-stream" {
            v["step_update"]["conversation_id"].as_str()
        } else {
            None
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
