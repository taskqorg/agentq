//! Observation only: agentq.md § 5 DashQ release observation. No activation or client writes.
use coreq::Tool;
use serde_json::{Value, json};
use std::time::UNIX_EPOCH;

const COREQ_VERSION: &str = "0.15.0";

fn version(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b)) && *s != "." && *s != "..")
}

pub fn observe(t: &Tool, now: u64) -> Value {
    let current = t.current();
    let selected = version(current.as_deref());
    let marker = t.release_exe(t.version).with_file_name("healthy");
    let receipt = std::fs::symlink_metadata(&marker).ok().filter(|m| m.is_file() && m.len() == 0)
        .and_then(|m| m.modified().ok()).and_then(|at| at.duration_since(UNIX_EPOCH).ok()).map(|at| at.as_secs())
        .filter(|at| *at <= now && std::fs::read(&marker).is_ok_and(|b| b.is_empty()));
    let health = match receipt {
        Some(at) => json!({"state": "verified", "scope": "status_written", "version": t.version, "at": at}),
        None => json!({"state": "unknown", "reason": "no_receipt"}),
    };
    let (lifecycle, reason) = match crate::control::host_disabled(&t.install.join("data")) {
        Err(_) => ("unknown", "control_unknown"),
        Ok(true) => ("pending_disable", "host_disabled"),
        Ok(false) => match selected {
            None => ("unknown", "selection_unknown"),
            Some(v) if v != t.version => ("waiting_for_activation", "selection_changed"),
            Some(_) if receipt.is_some() => ("healthy", "status_written"),
            _ => ("starting", "no_receipt"),
        },
    };
    json!({"at": now, "selected": selected, "running": t.version, "coreq_version": COREQ_VERSION,
        "health": health, "lifecycle": lifecycle, "reason": reason})
}

/// Project only bounded identifiers and fixed codes; stale observations retain history, never current health.
pub fn project(v: &Value, now: u64) -> Value {
    if !v.is_object() { return json!({"lifecycle": "unknown", "health": {"state": "unknown"}, "reason": "not_reported"}); }
    let at = v["at"].as_u64();
    let fresh = at.is_some_and(|at| at <= now && now - at <= 90);
    let running = version(v["running"].as_str());
    let selected = version(v["selected"].as_str());
    let health_at = v["health"]["at"].as_u64();
    let verified = fresh && v["health"]["state"] == "verified" && v["health"]["scope"] == "status_written"
        && running.is_some() && v["health"]["version"].as_str() == running && health_at.is_some_and(|h| Some(h) <= at);
    let health = if verified {
        json!({"state": "verified", "scope": "status_written", "version": running, "at": health_at})
    } else { json!({"state": "unknown", "reason": if fresh { "no_receipt" } else { "stale_observation" }, "at": health_at}) };
    let lifecycle = v["lifecycle"].as_str().filter(|s| ["waiting_for_activation", "healthy", "starting", "pending_disable", "unknown"].contains(s)).unwrap_or("unknown");
    let lifecycle = if !fresh || (lifecycle == "healthy" && (!verified || selected.is_none() || selected != running)) { "unknown" } else { lifecycle };
    let reason = v["reason"].as_str().filter(|s| ["control_unknown", "host_disabled", "selection_unknown", "selection_changed", "status_written", "no_receipt"].contains(s)).unwrap_or("unknown");
    json!({"at": at, "fresh": fresh, "selected": selected, "running": running, "coreq_version": version(v["coreq_version"].as_str()),
        "health": health, "lifecycle": lifecycle, "reason": if fresh { reason } else { "stale_observation" }})
}

#[cfg(test)]
mod tests {
    use super::*;
    /// #141 A1/A3: real CoreQ selection and marker, never binary presence alone; pin must match the lockfile.
    #[test]
    fn installation_sources() {
        assert!(include_str!("../Cargo.lock").contains(&format!("name = \"coreq\"\nversion = \"{COREQ_VERSION}\"")));
        let d = std::env::temp_dir().join(format!("agentq-141-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mut t = Tool::new("agentq", "AgentQ", "taskqorg/agentq", "1.0.0");
        t.install = d.clone();
        assert_eq!(observe(&t, crate::now())["reason"], "selection_unknown");
        std::fs::create_dir_all(d.join("releases/1.0.0")).unwrap();
        std::fs::write(d.join("current"), "1.0.0").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "starting");
        std::fs::write(d.join("releases/1.0.0/healthy"), "").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "healthy");
        std::fs::write(d.join("current"), "2.0.0").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "waiting_for_activation");
        std::fs::write(d.join("releases/1.0.0/healthy"), "invalid").unwrap();
        assert_eq!(observe(&t, crate::now())["health"]["state"], "unknown");
        std::fs::create_dir_all(d.join("data")).unwrap();
        std::fs::write(d.join("data/integration.json"), "{").unwrap();
        assert_eq!(observe(&t, crate::now())["reason"], "control_unknown");
        std::fs::remove_dir_all(d).unwrap();
    }
}
