//! Claude and Antigravity native hooks; keep permission classification, input fields and command semantics.
use serde_json::{Value, json};
use std::path::Path;

fn quote(s: &str) -> String { format!("'{}'", s.replace('\'', "'\"'\"'")) }

pub fn response(agent: &str, event: &str, input: &str, exe: &Path) -> Result<Value, String> {
    let empty = || json!({});
    // The client passes its own environment and working directory to the hook (M6.9, M6.10).
    let launched = || crate::model::launched(|k| std::env::var(k).ok());
    let data = || exe.parent().map(|p| p.join("data")).ok_or("hook launcher has no install root");
    if agent == "claude" && event == "Stop" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::observe(&data()?, &v, launched())?;
        return Ok(empty());
    }
    if agent == "hermes" && event == "post_api_request" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::observe_hermes(&data()?, &v, launched(), std::env::current_dir().ok().as_deref())?;
        return Ok(empty());
    }
    if agent == "codex" && event == "SessionStart" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::session_start(&data()?, &v, launched(), std::env::current_dir().ok().as_deref())?;
        return Ok(empty());
    }
    if agent == "antigravity" && event == "PreToolUse" {
        if cfg!(windows) { return Ok(empty()); }
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        let tool = v["toolCall"]["name"].as_str().or(v["tool_name"].as_str()).unwrap_or_default();
        if tool != "run_command" { return Ok(empty()); }
        let Some(command) = v["toolCall"]["args"]["CommandLine"].as_str().or(v["tool_input"]["CommandLine"].as_str()) else {
            return Ok(empty());
        };
        if command.contains([';', '&', '|', '<', '>', '\n', '$', '`']) { return Ok(empty()); }
        let words: Vec<String> = command.split_ascii_whitespace().map(String::from).collect();
        if !crate::engines::qualified(&words) { return Ok(empty()); }
        let mut env = String::from("AGENTQ_AGENT=antigravity ");
        if let Some(session) = v["conversationId"].as_str().or(v["session_id"].as_str()) {
            env += &format!("AGENTQ_SESSION={} ", quote(session));
        }
        if let Some(step) = v["stepIdx"].as_u64() {
            env += &format!("AGENTQ_TOOL_CALL='step-{step}' ");
        }
        let path = exe.display().to_string();
        let path = if cfg!(windows) { path.replace('\\', "/") } else { path };
        return Ok(json!({"decision": "allow", "overwrite": {"CommandLine": format!("{env}{} run -- {command}", quote(&path))}}));
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
        let compressed = crate::compression::compress(&data()?, "claude", &json!({
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
        if cfg!(windows) { return; }
        let ag = json!({"toolCall":{"name":"run_command","args":{"CommandLine":"cargo test --color never"}},"conversationId":"ag-conv-1","stepIdx":19});
        let r_ag = response("antigravity", "PreToolUse", &ag.to_string(), Path::new("/a b/agentq")).unwrap();
        assert!(r_ag["overwrite"]["CommandLine"].as_str().unwrap().ends_with("'/a b/agentq' run -- cargo test --color never"));
        assert!(r_ag["overwrite"]["CommandLine"].as_str().unwrap().contains("AGENTQ_SESSION='ag-conv-1'"));
        assert_eq!(r_ag["decision"], "allow", "native permission policy still decides");
        assert!(r_ag["overwrite"]["CommandLine"].as_str().unwrap().contains("AGENTQ_TOOL_CALL='step-19'"));
        for command in ["git status", "cargo test && echo no", "cargo test $(touch no)"] {
            let mut ag = ag.clone(); ag["toolCall"]["args"]["CommandLine"] = json!(command);
            assert_eq!(response("antigravity", "PreToolUse", &ag.to_string(), Path::new("agentq")).unwrap(), json!({}));
        }
    }
}
