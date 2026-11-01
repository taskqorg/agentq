//! Optional AgentQ optimization per launch (agentq.md § 12 M12.5-M12.7, docs/spec/RUNTIME.md § Optimization intent).
//! Reads the installation and the client home, never writes either; `off` reaches the optimizer only through the child.
use crate::agent::{Client, Spec, child_env, spec_home};
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

fn read(path: &Path) -> Result<Option<String>, ()> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

pub const AGENTS: [&str; 4] = ["codex", "claude", "hermes", "claude-desktop"];

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
    if let Some(v) = c.get("compression") {
        if !v.is_object()
            || v.get("enabled").is_some_and(|v| !v.is_boolean())
            || v.get("min_bytes").is_some_and(|v| v.as_u64().is_none())
            || v.get("stats").is_some_and(|v| !matches!(v.as_str(), Some("queue" | "off")))
            || v.get("agents").is_some_and(|v| !v.as_object().is_some_and(|a| a.values().all(Value::is_boolean)))
        {
            return Err("invalid compression settings".into());
        }
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
        || !s["profiles"].as_object().is_some_and(|p| p.len() <= 4 && p.iter().all(|(agent, r)| {
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

/// Codex selects the `agentq` provider at the top level of `config.toml`.
fn codex_routed(home: &Path) -> Result<bool, ()> {
    let text = read(&home.join("config.toml"))?.unwrap_or_default();
    // Only the top-level selection is owned by agentq; preserve native profile/model settings.
    Ok(text.lines().map(str::trim).take_while(|l| !l.starts_with('[')).any(|l| {
        l.split_once('=').is_some_and(|(k, v)| {
            let v = v.trim();
            k.trim() == "model_provider" && ["\"agentq\"", "'agentq'"].iter().any(|q| {
                v.strip_prefix(q).is_some_and(|rest| rest.trim().is_empty() || rest.trim().starts_with('#'))
            })
        })
    }))
}

/// Claude Code runs the agentq hooks from this home's `settings.json`.
fn claude_attached(home: &Path) -> Result<bool, ()> {
    let Some(text) = read(&home.join("settings.json"))? else { return Ok(false) };
    let settings: Value = serde_json::from_str(&text).map_err(|_| ())?;
    let object = settings.as_object().ok_or(())?;
    let Some(hooks) = object.get("hooks") else { return Ok(false) };
    let hooks = hooks.as_object().ok_or(())?;
    Ok(["PreToolUse", "PostToolUse"].iter().any(|event| hooks.get(*event).and_then(Value::as_array).is_some_and(|groups| {
        groups.iter().filter_map(|g| g["hooks"].as_array()).flatten().any(|hook| {
            hook["command"].as_str().is_some_and(|s| s.contains(" hook claude "))
        })
    })))
}

/// Antigravity runs the agentq hooks from `hooks.json` in `home` or `<home>/../config/hooks.json`.
fn antigravity_attached(home: &Path) -> Result<bool, ()> {
    let mut paths = vec![home.join("hooks.json")];
    if let Some(parent) = home.parent() {
        paths.push(parent.join("config/hooks.json"));
    }
    for path in paths {
        let Some(text) = read(&path)? else { continue };
        let settings: Value = serde_json::from_str(&text).map_err(|_| ())?;
        let object = settings.as_object().ok_or(())?;
        let attached = object.values().filter(|group| group["enabled"] != false).any(|group| {
            group["PreToolUse"].as_array().is_some_and(|groups| {
                groups.iter().filter(|g| g["matcher"].as_str().is_none_or(|m| matches!(m, "" | "*" | "run_command")))
                    .filter_map(|g| g["hooks"].as_array()).flatten().any(|hook| {
                    hook["command"].as_str().is_some_and(|s| s.contains(" hook antigravity PreToolUse"))
                })
            })
        });
        if attached {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The effective state for `client` in `home` from the read-only installation state (M12.5). Host disable and the
/// host and agent switches win; `on` never enables anything.
pub fn observe(intent: Intent, client: Client, install: Option<&Path>, home: Option<&Path>) -> Optimization {
    observe_state(intent, client, install, home, intent == Intent::Off || std::env::var(ENV).is_ok_and(|v| v == "off"))
}

fn observe_state(intent: Intent, client: Client, install: Option<&Path>, home: Option<&Path>, off: bool) -> Optimization {
    let o = |effective, reason| Optimization { intent, effective, reason };
    if off { return o("off", "launch_off"); }
    let Some(install) = install.filter(|i| i.join("data").is_dir()) else { return o("off", "not_installed") };
    let data = install.join("data");
    let (Ok(control), Ok(config)) = (state(&data), config(&data)) else {
        return o("unknown", "unreadable_state");
    };
    if control["host_disabled"] == true || control["profiles"].get(client.code()).is_some() { return o("off", "integration_disabled"); }
    let c = &config["compression"];
    if c["enabled"] == false { return o("off", "host_off"); }
    if c["agents"][client.code()] == false { return o("off", "agent_off"); }
    let Some(home) = home else { return o("unknown", "no_client_home") };
    let attached = match client {
        Client::Codex => codex_routed(home),
        Client::Claude => claude_attached(home),
        Client::Antigravity => antigravity_attached(home),
    };
    match attached {
        Ok(true) => o("on", "configured"),
        Ok(false) => o("off", "not_attached"),
        Err(()) => o("unknown", "unreadable_client_config"),
    }
}

/// Observe the state for `spec` and carry `off` and the launcher marker into the child (M12.4, M12.6). Adds only child
/// environment and, for a Codex home routed through agentq, one per-launch header override; nothing is written.
pub fn prepare(intent: Intent, product: &str, spec: &mut Spec, install: Option<&Path>) -> Optimization {
    let home = spec_home(spec);
    let off = intent == Intent::Off || child_env(spec, ENV).is_some_and(|v| v == "off");
    let o = observe_state(intent, spec.client, install, home.as_deref(), off);
    let set = |spec: &Spec, k: &str| child_env(spec, k).is_some_and(|v| !v.is_empty());
    if !set(spec, "TASKQ_TASK") && !set(spec, "AGENTQ_LAUNCHER") { spec.env.push(("AGENTQ_LAUNCHER".into(), product.into())); }
    if off {
        spec.env.push((ENV.into(), "off".into()));
        if spec.client == Client::Codex && home.as_deref().is_some_and(|h| codex_routed(h) == Ok(true)) {
            spec.args.extend(["-c".into(), format!("model_providers.agentq.http_headers.{HEADER}=\"off\"").into()]);
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    // Chain: agentq.md M12.5: effective state follows host disable, host and agent switches and attachment; `on`
    // never enables anything. Risk: a launch reports optimization that a host switch or disable turned off.
    #[test]
    fn effective_state_follows_host_state_and_attachment() {
        let root = crate::scratch("optimization");
        let (install, home) = (root.join("install"), root.join("home"));
        let seen = |i, c| { let o = observe(i, c, Some(&install), Some(&home)); (o.effective, o.reason) };
        assert_eq!(seen(Intent::On, Client::Codex), ("off", "not_installed"));
        assert_eq!(observe(Intent::Inherit, Client::Codex, None, None).reason, "not_installed");
        std::fs::create_dir_all(install.join("data")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(seen(Intent::On, Client::Codex), ("off", "not_attached"));
        std::fs::write(home.join("config.toml"), "model = \"x\"\nmodel_provider = \"agentq\"\n[model_providers.agentq]\n").unwrap();
        std::fs::write(home.join("settings.json"), r#"{"hooks":{"PreToolUse":[{"hooks":[{"command":"/i/agentq hook claude PreToolUse"}]}]}}"#).unwrap();
        std::fs::write(home.join("hooks.json"), r#"{"agentq":{"PreToolUse":[{"matcher":"run_command","hooks":[{"command":"/i/agentq hook antigravity PreToolUse"}]}]}}"#).unwrap();
        assert_eq!((seen(Intent::Inherit, Client::Codex), seen(Intent::On, Client::Claude)), (("on", "configured"), ("on", "configured")));
        assert_eq!(seen(Intent::On, Client::Antigravity), ("on", "configured"));
        assert_eq!(seen(Intent::Off, Client::Claude), ("off", "launch_off"));
        assert_eq!(seen(Intent::Off, Client::Antigravity), ("off", "launch_off"));
        std::fs::write(home.join("hooks.json"), r#"{"agentq":{"enabled":false,"PreToolUse":[{"hooks":[{"command":"/i/agentq hook antigravity PreToolUse"}]}]}}"#).unwrap();
        assert_eq!(seen(Intent::On, Client::Antigravity), ("off", "not_attached"), "disabled hooks cannot prove attachment");
        std::fs::write(install.join("data/config.json"), r#"{"compression":{"agents":{"codex":false}}}"#).unwrap();
        assert_eq!((seen(Intent::On, Client::Codex), seen(Intent::On, Client::Claude)), (("off", "agent_off"), ("on", "configured")));
        std::fs::write(install.join("data/config.json"), r#"{"compression":{"enabled":false}}"#).unwrap();
        assert_eq!(seen(Intent::On, Client::Claude), ("off", "host_off"));
        std::fs::write(install.join("data/integration.json"), r#"{"schema":1,"generation":1,"host_disabled":true,"profiles":{}}"#).unwrap();
        assert_eq!(seen(Intent::On, Client::Claude), ("off", "integration_disabled"), "host disable wins over on");
        std::fs::write(install.join("data/integration.json"), "{").unwrap();
        assert_eq!(seen(Intent::Inherit, Client::Claude), ("unknown", "unreadable_state"), "corrupt state is unknown, never on");
        std::fs::remove_dir_all(root).unwrap();
    }

    // Chain: agentq.md M12.6: `off` reaches the optimizer only through the child; a Codex home routed through agentq
    // gets one per-launch header, others none. Risk: a launch edits global config or sends a header Codex rejects.
    #[test]
    fn off_reaches_the_child_without_global_edits() {
        let root = crate::scratch("prepare");
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let config = "model_provider = \"agentq\"\n";
        std::fs::write(home.join("config.toml"), config).unwrap();
        let mut spec = Spec { client: Client::Codex, root: root.clone(), session: "-".into(), resume: false, args: vec![], home: Some(home.clone()), env: vec![] };
        let o = prepare(Intent::Off, "fixtureq", &mut spec, None);
        assert_eq!((o.effective, o.reason), ("off", "launch_off"));
        assert!(spec.env.iter().any(|(k, v)| k == ENV && v == "off"));
        assert_eq!(spec.args, vec![std::ffi::OsString::from("-c"), "model_providers.agentq.http_headers.x-agentq-optimization=\"off\"".into()]);
        assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), config, "client configuration unchanged");
        std::fs::write(home.join("config.toml"), "").unwrap();
        let mut plain = Spec { args: vec![], env: vec![("TASKQ_TASK".into(), "7".into())], ..spec.clone() };
        prepare(Intent::Off, "fixtureq", &mut plain, None);
        assert!(plain.args.is_empty(), "an unrouted home gets no provider override");
        assert!(!plain.env.iter().any(|(k, _)| k == "AGENTQ_LAUNCHER"), "a TaskQ launch keeps its own marker");
        let mut inherit = Spec { args: vec![], env: vec![], ..spec };
        prepare(Intent::Inherit, "fixtureq", &mut inherit, None);
        assert!(inherit.args.is_empty() && !inherit.env.iter().any(|(k, _)| k == ENV));
        std::fs::remove_dir_all(root).unwrap();
    }
    // M12.5/M12.6: observe the child's actual home/env. Risk: an off request is reported on or misses its provider.
    #[test]
    fn child_home_and_inherited_off_are_observed() {
        let root = crate::scratch("child-state");
        let install = root.join("install");
        let home = root.join("home");
        std::fs::create_dir_all(install.join("data")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.toml"), "model_provider = \"agentq\" # owned provider\n").unwrap();
        let mut spec = Spec { client: Client::Codex, root: root.clone(), session: "-".into(), resume: false, args: vec![], home: None,
            env: vec![("CODEX_HOME".into(), home.clone().into_os_string()), (ENV.into(), "off".into())] };
        for intent in [Intent::Inherit, Intent::On] {
            spec.args.clear();
            let observed = prepare(intent, "fixtureq", &mut spec, Some(&install));
            assert_eq!((observed.intent, observed.effective, observed.reason), (intent, "off", "launch_off"));
            assert_eq!(spec.args.len(), 2, "child env home and inline comment must not lose the off header");
        }
        spec.args.clear();
        spec.env.retain(|(k, _)| k != ENV);
        let observed = prepare(Intent::On, "fixtureq", &mut spec, Some(&install));
        assert_eq!((observed.effective, observed.reason), ("on", "configured"), "read the child's home, not the parent's");
        spec.home = Some(root.join("other-home"));
        assert_eq!(prepare(Intent::On, "fixtureq", &mut spec, Some(&install)).reason, "not_attached", "explicit account home wins");
        std::fs::remove_dir_all(root).unwrap();
    }

    // M4.8/M12.5: malformed lifecycle/compression state stays unknown, never a configured-on success.
    #[test]
    fn invalid_lifecycle_and_preferences_stay_unknown() {
        let root = crate::scratch("invalid-state");
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(root.join("config.toml"), "model_provider = \"agentq\"\n").unwrap();
        for (file, invalid) in [("integration.json", r#"{"schema":999,"generation":0,"host_disabled":false,"profiles":{}}"#),
            ("integration.json", r#"{"schema":1,"generation":0,"host_disabled":"false","profiles":{}}"#),
            ("config.json", r#"{"compression":{"enabled":"false"}}"#)] {
            std::fs::write(data.join(file), invalid).unwrap();
            let o = observe(Intent::On, Client::Codex, Some(&root), Some(&root));
            assert_eq!((o.effective, o.reason), ("unknown", "unreadable_state"), "{invalid}");
            std::fs::remove_file(data.join(file)).unwrap();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // M12.6: Codex accepts top-level provider values followed by TOML comments; off must reach these profiles too.
    #[test]
    fn commented_provider_gets_off_header() {
        let root = crate::scratch("comment-provider");
        for config in ["model_provider = \"agentq\" # owned\n", "model_provider = 'agentq' # owned\n"] {
            std::fs::write(root.join("config.toml"), config).unwrap();
            let mut spec = Spec { client: Client::Codex, root: root.clone(), session: "-".into(), resume: false,
                args: vec![], home: Some(root.clone()), env: vec![] };
            prepare(Intent::Off, "fixtureq", &mut spec, None);
            assert_eq!(spec.args.len(), 2, "off header for {config}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

}
