//! Claude and Antigravity native hooks; keep permission classification, input fields and command semantics.
use serde_json::{Value, json};
use std::path::Path;
use agentq_runtime::agent::registry::{self,Hook};


pub fn default_response(agent: &str, event: &str) -> Value {
    registry::entry(agent).filter(|e|e.name==agent).map(|e|(e.fallback)(event)).unwrap_or_else(||json!({}))
}

pub fn response(agent: &str, event: &str, input: &str, exe: &Path) -> Result<Value, String> {
    let empty = || default_response(agent, event);
    // The client passes its own environment and working directory to the hook (M6.9, M6.10).
    let launched = || crate::session::launched(|k| std::env::var(k).ok());
    let data = || exe.parent().map(|p| p.join("data")).ok_or("hook launcher has no install root");
    let action=registry::entry(agent).filter(|e|e.name==agent).map(|e|(e.hook)(event)).unwrap_or(Hook::None);
    if matches!(action,Hook::None) {return Ok(empty());}
    let v: Value=serde_json::from_str(input).map_err(|e|format!("hook JSON: {e}"))?;
    let project=crate::observe::archive_project(&v);
    if let Ok(data)=data() {agentq_runtime::archive::hook(&data,agent,&v,launched(),&project);}
    match action {
        Hook::Stop {model} => {
            let started=std::time::Instant::now();
            let spans=if model {crate::model::observe(&data()?,&v,launched())} else {Ok(())};
            // The old path and the new records fail independently (M6.11).
            let records=crate::observe::stop(&data()?,agent,&v,launched(),started+crate::observe::STOP_PASS);
            spans.and(records)?;
        }
        Hook::ModelCall => {
            let spans=crate::model::observe_hermes(&data()?,&v,launched(),std::env::current_dir().ok().as_deref());
            let records=crate::observe::hermes_call(&data()?,&v,launched()); spans.and(records)?;
        }
        Hook::TurnEnd => crate::observe::hermes_turn_end(&data()?,&v,launched())?,
        Hook::SessionStart => crate::session::session_start(&data()?,&v,launched(),std::env::current_dir().ok().as_deref())?,
        Hook::None => {},
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
    fn hooks_keep_exact_native_names_without_alias_widening() {
        assert_eq!(default_response("agy","Stop"),json!({}));
        assert_eq!(response("agy","Stop","invalid",Path::new("/agentq")).unwrap(),json!({}));
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
