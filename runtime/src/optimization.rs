//! Retired optimization compatibility API and installation lifecycle state (agentq.md M12.5).
use crate::agent::{Client, Spec, child_env};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The caller's request for this launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent { Inherit, Off, On }

impl Intent {
    pub fn parse(code: &str) -> Option<Intent> {
        match code { "inherit" => Some(Intent::Inherit), "off" => Some(Intent::Off), "on" => Some(Intent::On), _ => None }
    }
    pub fn code(self) -> &'static str { match self { Intent::Inherit => "inherit", Intent::Off => "off", Intent::On => "on" } }
}

/// Intent plus the effective state (`on`, `off`, `unknown`) and its reason code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Optimization { pub intent: Intent, pub effective: &'static str, pub reason: &'static str }

impl Optimization {
    pub fn json(&self) -> Value { json!({"intent": self.intent.code(), "effective": self.effective, "reason": self.reason}) }
}

/// The child variable every compression adapter reads; `off` disables compression for that session (M12.7).
pub const ENV: &str = "AGENTQ_OPTIMIZATION";
/// The Codex provider header the loopback provider reads (agentq.md § 3 Codex adapter).
pub const HEADER: &str = "x-agentq-optimization";

/// The default installation (agentq.md § 5): `~/.local/share/agentq`, Windows `%LOCALAPPDATA%\agentq`.
pub fn install_root() -> Option<PathBuf> {
    if cfg!(windows) { std::env::var_os("LOCALAPPDATA").map(|d| Path::new(&d).join("agentq")) }
    else { std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share/agentq")) }
}

pub const AGENTS: [&str; 5] = ["codex", "claude", "hermes", "claude-desktop", "antigravity"];

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

/// Retired optimization intent remains source-compatible for pinned consumers.
pub(crate) fn observe(intent: Intent, _client: Client, _install: Option<&Path>, _home: Option<&Path>) -> Optimization {
    Optimization { intent, effective: "off", reason: "optimization_retired" }
}

/// Launcher attribution is independent of retired optimization.
pub(crate) fn prepare(intent: Intent, product: &str, spec: &mut Spec, install: Option<&Path>) -> Optimization {
    let set = |k: &str| child_env(spec, k).is_some_and(|v| !v.is_empty());
    if !set("TASKQ_TASK") && !set("AGENTQ_LAUNCHER") { spec.env.push(("AGENTQ_LAUNCHER".into(), product.into())); }
    observe(intent, spec.client, install, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    // M12.5-M12.7: old consumers retain their API without altering native launch arguments or configuration.
    #[test]
    fn retired_intents_do_not_change_native_launch() {
        for intent in [Intent::Inherit, Intent::Off, Intent::On] {
            let mut spec = Spec { client: Client::Codex, root: PathBuf::from("."), session: "-".into(), resume: false,
                args: vec!["native".into()], home: None, env: vec![("TASKQ_TASK".into(), "7".into())] };
            let before = spec.clone();
            let state = prepare(intent, "fixture", &mut spec, Some(Path::new("/unreadable")));
            assert_eq!((state.intent, state.effective, state.reason), (intent, "off", "optimization_retired"));
            assert_eq!(spec.args, before.args);
            assert_eq!(spec.env, before.env);
            assert_eq!(Intent::parse(intent.code()), Some(intent));
        }
    }
}
