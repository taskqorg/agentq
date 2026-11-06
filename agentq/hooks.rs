//! Claude and Antigravity native hooks; keep permission classification, input fields and command semantics.
use serde_json::{Value, json};
use std::path::Path;


pub fn default_response(agent: &str, event: &str) -> Value {
    if agent == "antigravity" && matches!(event, "PreToolUse" | "Stop") { json!({"decision": "allow"}) } else { json!({}) }
}

pub fn response(agent: &str, event: &str, input: &str, exe: &Path) -> Result<Value, String> {
    let empty = || default_response(agent, event);
    // The client passes its own environment and working directory to the hook (M6.9, M6.10).
    let launched = || crate::model::launched(|k| std::env::var(k).ok());
    let data = || exe.parent().map(|p| p.join("data")).ok_or("hook launcher has no install root");
    if agent == "claude" && event == "Stop" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        let started = std::time::Instant::now();
        let spans = crate::model::observe(&data()?, &v, launched());
        // The old path and the new records fail independently (M6.11).
        let records = crate::observe::stop(&data()?, "claude", &v, launched(), started + crate::observe::STOP_PASS);
        spans.and(records)?;
        return Ok(empty());
    }
    if agent == "hermes" && event == "post_api_request" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        let spans = crate::model::observe_hermes(&data()?, &v, launched(), std::env::current_dir().ok().as_deref());
        let records = crate::observe::hermes_call(&data()?, &v, launched());
        spans.and(records)?;
        return Ok(empty());
    }
    if agent == "hermes" && event == "on_session_end" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::observe::hermes_turn_end(&data()?, &v, launched())?;
        return Ok(empty());
    }
    if matches!(agent, "codex" | "antigravity") && event == "Stop" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::observe::stop(&data()?, agent, &v, launched(), std::time::Instant::now() + crate::observe::STOP_PASS)?;
        return Ok(empty());
    }
    if agent == "codex" && event == "SessionStart" {
        let v: Value = serde_json::from_str(input).map_err(|e| format!("hook JSON: {e}"))?;
        crate::model::session_start(&data()?, &v, launched(), std::env::current_dir().ok().as_deref())?;
        return Ok(empty());
    }
    Ok(empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Chain: M1.2, M6.11. Risk: a Stop answer or fallback that lacks the required `decision`; `continue` would re-enter the loop.
    #[test]
    fn antigravity_stop_always_allows_the_stop() {
        let d = std::env::temp_dir().join(format!("agentq-hook-stop-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(default_response("antigravity", "Stop"), json!({"decision": "allow"}));
        let input = json!({"conversationId": "c", "fullyIdle": false}).to_string();
        assert_eq!(response("antigravity", "Stop", &input, &d.join("agentq")).unwrap(), json!({"decision": "allow"}));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn retired_wrapping_hooks_leave_native_input_unchanged() {
        for agent in ["claude", "antigravity"] {
            for event in ["PreToolUse", "PostToolUse"] {
                assert_eq!(response(agent, event, "invalid old payload", Path::new("/tmp/agentq")).unwrap(), default_response(agent, event));
            }
        }
    }
}
