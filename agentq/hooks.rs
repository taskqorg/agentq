//! Claude's native hooks; keep permission classification, input fields and command semantics.
use serde_json::{Value, json};
use std::path::Path;

fn quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\"'\"'")) }

pub fn response(agent: &str, event: &str, input: &str, exe: &Path) -> Result<Value, String> {
    let empty = || json!({});
    if agent == "claude" && event == "Stop" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::observe(&exe.parent().ok_or("hook launcher has no install root")?.join("data"), &v)?;
        return Ok(empty());
    }
    if agent == "hermes" && event == "post_api_request" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::observe_hermes(&exe.parent().ok_or("hook launcher has no install root")?.join("data"), &v)?;
        return Ok(empty());
    }
    if agent != "claude" || !["PreToolUse", "PostToolUse"].contains(&event) { return Ok(empty()); }
    let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
    if v["tool_name"] != "Bash" { return Ok(empty()); }
    let Some(command) = v["tool_input"]["command"].as_str() else { return Ok(empty()); };
    // Shell composition is not a qualified native argv; leave it to the client.
    if command.contains([';', '&', '|', '<', '>', '\n', '$', '`']) { return Ok(empty()); }
    let words: Vec<String> = command.split_ascii_whitespace().map(String::from).collect();
    if !crate::engines::qualified(&words) { return Ok(empty()); }
    if event == "PostToolUse" {
        if v["permission_mode"] != "auto" { return Ok(empty()); }
        let mut result = v["tool_response"].clone();
        if result["interrupted"] != false || result["isImage"] != false || !result["stderr"].is_string() { return Ok(empty()); }
        let Some(text) = result["stdout"].as_str() else { return Ok(empty()); };
        let data = exe.parent().ok_or("hook launcher has no install root")?.join("data");
        let compressed = crate::compression::compress(&data, "claude", &json!({
            "adapter": "hook", "tool": "Bash", "command": command, "text": text, "exit_code": 0,
            "session": v["session_id"], "tool_call": v["tool_use_id"],
        }).to_string())?;
        if !compressed["ref"].is_string() { return Ok(empty()); }
        result["stdout"] = compressed["text"].clone();
        return Ok(json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "updatedToolOutput": result}}));
    }
    // Auto mode must classify the native command; a wrapper is a different permission request.
    if v["permission_mode"] == "auto" { return Ok(empty()); }
    let mut env = String::from("AGENTQ_AGENT=claude ");
    for (key, field) in [("AGENTQ_SESSION", "session_id"), ("AGENTQ_TOOL_CALL", "tool_use_id")] {
        if let Some(value) = v[field].as_str() { env += &format!("{key}={} ", quote(value)); }
    }
    let path = exe.display().to_string();
    let path = if cfg!(windows) { path.replace('\\', "/") } else { path };
    let mut updated = v["tool_input"].clone();
    updated["command"] = json!(format!("{env}{} run -- {command}", quote(&path)));
    Ok(json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":updated}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_claude_rewrite_preserves_input_and_ids() {
        let v = json!({"tool_name":"Bash","tool_input":{"command":"cargo test --color never","description":"keep"},"session_id":"native-session","tool_use_id":"native-call"});
        let r = response("claude","PreToolUse", &v.to_string(), Path::new("/a b/agentq")).unwrap();
        let u = &r["hookSpecificOutput"]["updatedInput"];
        assert_eq!(u["description"], "keep");
        assert!(u["command"].as_str().unwrap().ends_with("'/a b/agentq' run -- cargo test --color never"));
        assert!(u["command"].as_str().unwrap().contains("AGENTQ_SESSION='native-session'"));
        let mut auto = v.clone(); auto["permission_mode"] = "auto".into();
        assert_eq!(response("claude", "PreToolUse", &auto.to_string(), Path::new("/a b/agentq")).unwrap(), json!({}), "auto mode must classify the original native command");
        for command in ["git status", "cargo test && echo no", "cargo test; echo no", "cargo test $(touch no)"] {
            let mut v=v.clone();v["tool_input"]["command"]=json!(command);
            assert_eq!(response("claude","PreToolUse",&v.to_string(),Path::new("agentq")).unwrap(),json!({}));
        }
    }
}
