use super::*;
pub(super) fn normalize(
    format: &str,
    v: &Value,
) -> (&'static str, &'static str, Value, Vec<Value>, &'static str) {
    let format=crate::agent::history::interpretation(format);
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
    let mut blocks: Vec<Value> = blocks.into_iter().map(|v| block(format, v)).collect();
    if author=="user" {
        let injected=blocks.iter().find_map(|b|b["text"].as_str()).is_some_and(|text|text.starts_with("<environment_context>") || text.starts_with("# AGENTS.md"));
        for block in &mut blocks {if block["type"]=="text" {block["origin"]=json!(if injected {"injected_context"}else{"native_user"});}}
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
            let summary=raw.as_str().filter(|_|free).map(|input|if v["name"]=="apply_patch" {patch_files(input)}else{codex_script(input).unwrap_or_else(||input.into())});
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
    let format=crate::agent::history::interpretation(format);
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

// Native custom-tool summaries moved from TaskQ; consumer masking and display limits stay there.
fn codex_script(js: &str) -> Option<String> {
    let s: Vec<char> = js.chars().collect();
    let (mut i, mut calls, mut key) = (0, Vec::<(String, Option<String>)>::new(), None::<String>);
    let ident = |i: usize| {
        s[i..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '$')
            .collect::<String>()
    };
    let colon = |i: usize| s[i..].iter().find(|c| !c.is_whitespace()) == Some(&':');
    while i < s.len() {
        match s[i] {
            '/' if s.get(i + 1) == Some(&'/') => {
                while i < s.len() && s[i] != '\n' {
                    i += 1
                }
            }
            '/' if s.get(i + 1) == Some(&'*') => {
                i = (i + 3..s.len())
                    .find(|&k| s[k - 1] == '*' && s[k] == '/')
                    .map_or(s.len(), |k| k + 1)
            }
            q @ ('"' | '\'' | '`') => {
                let (lit, end) = literal(&s, i + 1, q);
                i = end;
                if colon(i) {
                    key = Some(lit);
                    continue;
                }
                if let Some((name, target @ None)) = calls.last_mut() {
                    if matches!(
                        (name.as_str(), key.as_deref()),
                        ("exec_command", Some("cmd"))
                            | ("view_image", Some("path"))
                            | ("apply_patch", None)
                    ) {
                        *target = Some(lit);
                    }
                }
                key = None;
            }
            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let word = ident(i);
                i += word.chars().count();
                if word == "tools" && s.get(i) == Some(&'.') {
                    let name = ident(i + 1);
                    i += 1 + name.chars().count();
                    if s.get(i) == Some(&'(') {
                        calls.push((name, None));
                        key = None;
                    }
                } else if colon(i) {
                    key = Some(word);
                }
            }
            _ => i += 1,
        }
    }
    let line: Vec<String> = calls
        .into_iter()
        .map(|(name, target)| match (name.as_str(), target) {
            ("exec_command", Some(cmd)) => cmd,
            ("apply_patch", Some(patch)) => format!("apply_patch {}", patch_files(&patch)),
            (_, Some(target)) => format!("{name} {target}"),
            (_, None) => name,
        })
        .collect();
    (!line.is_empty()).then(|| line.join(" · "))
}

/// The JavaScript string literal that starts at `i`, closed by `q`, and the index after it; an open one ends the script.
fn literal(s: &[char], mut i: usize, q: char) -> (String, usize) {
    let mut out = String::new();
    while i < s.len() && s[i] != q {
        if s[i] == '\\' && i + 1 < s.len() {
            i += 1;
            let hex = |n: usize| {
                s.get(i + 1..i + 1 + n)
                    .and_then(|h| u32::from_str_radix(&h.iter().collect::<String>(), 16).ok())
                    .and_then(char::from_u32)
            };
            match s[i] {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'u' if hex(4).is_some() => {
                    out.push(hex(4).unwrap());
                    i += 4;
                }
                'x' if hex(2).is_some() => {
                    out.push(hex(2).unwrap());
                    i += 2;
                }
                c => out.push(c),
            }
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    (out, (i + 1).min(s.len()))
}

/// The files an `apply_patch` envelope adds, updates or deletes.
fn patch_files(patch: &str) -> String {
    patch
        .lines()
        .filter_map(|l| {
            ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
                .iter()
                .find_map(|p| l.strip_prefix(p))
        })
        .collect::<Vec<_>>()
        .join(" ")
}
