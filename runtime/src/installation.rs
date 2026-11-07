//! Installation location and retained integration lifecycle state.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The default installation (agentq.md § 5): `~/.local/share/agentq`, Windows `%LOCALAPPDATA%\agentq`.
pub fn install_root() -> Option<PathBuf> {
    if cfg!(windows) { std::env::var_os("LOCALAPPDATA").map(|d| Path::new(&d).join("agentq")) }
    else { std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share/agentq")) }
}

pub use crate::agent::registry::AGENTS;

fn read_json(path: &Path, missing: Value) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => serde_json::from_slice(&bytes).map_err(|_| "invalid control JSON".into()),
        Ok(_) => Err("control state exceeds 1 MiB".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(missing),
        Err(e) => Err(format!("control state unreadable: {}", e.kind())),
    }
}

/// Missing settings take existing defaults; malformed settings are never replaced by a mutation.
pub fn config(data: &Path) -> Result<Value, String> {
    let c = read_json(&data.join("config.json"), json!({}))?;
    if !c.is_object() || (!c["direct"].is_null() && !c["direct"].as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
        return Err("invalid config.json".into());
    }
    Ok(c)
}

pub fn state(data: &Path) -> Result<Value, String> {
    let s = read_json(&data.join("integration.json"), json!({"schema": 1, "generation": 0, "host_disabled": false, "profiles": {}}))?;
    if s["schema"] != 1 || s["generation"].as_u64().is_none() || !s["host_disabled"].is_boolean()
        || (!s["service"].is_null() && !s["service"].as_object().is_some_and(|o| {
            ["registered", "loaded", "active", "foreign"].iter().all(|k| o.get(*k).is_none_or(|v| v.is_null() || v.is_boolean()))
                && o.get("state").is_none_or(Value::is_string) && o.get("owner").is_none_or(|v| v.as_u64().is_some())
        }))
        || !s["profiles"].as_object().is_some_and(|p| p.len() <= AGENTS.len() && p.iter().all(|(agent, r)| {
            AGENTS.contains(&agent.as_str()) && r.is_object() && r["root"].as_str().is_some_and(|p| Path::new(p).is_absolute())
                && matches!(r["phase"].as_str(), Some("detaching" | "detached" | "enabling" | "conflict"))
                && r["at"].as_u64().is_some() && r["direct"].is_boolean()
                && r["entries"].as_array().is_some_and(|a| a.len() <= 128 && a.iter().all(|e| {
                    e.is_object() && e["file"].as_str().is_some_and(|p| Path::new(p).is_absolute())
                        && e["key"].as_array().is_some_and(|ks| !ks.is_empty() && ks.len() <= 8 && ks.iter().all(Value::is_string))
                        && matches!(e["kind"].as_str(), Some("set" | "append" | "text" | "replace" | "file"))
                        && matches!(e["field"].as_str(), Some("routing" | "mcp" | "hooks" | "plugin" | "instructions"))
                        && e["source"].as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                }))
        }))
    {
        return Err("invalid integration.json".into());
    }
    Ok(s)
}
