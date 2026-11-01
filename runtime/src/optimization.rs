//! Optional AgentQ optimization per launch (agentq.md § 12 M12.5-M12.7, docs/spec/RUNTIME.md § Optimization intent).
//! Reads the installation and the client home, never writes either; `off` reaches the optimizer only through the child.
use crate::agent::{Client, Spec, base_home};
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

fn json_file(path: &Path) -> Result<Value, ()> {
    match read(path)? { Some(s) => serde_json::from_str::<Value>(&s).ok().filter(Value::is_object).ok_or(()), None => Ok(json!({})) }
}

/// Codex selects the `agentq` provider at the top level of `config.toml`.
fn codex_routed(home: &Path) -> Result<bool, ()> {
    let text = read(&home.join("config.toml"))?.unwrap_or_default();
    // ponytail: line scan of the top-level key; a TOML parser replaces it when the key moves into profiles.
    Ok(text.lines().map(str::trim).take_while(|l| !l.starts_with('[')).any(|l| {
        l.split_once('=').is_some_and(|(k, v)| k.trim() == "model_provider" && v.trim().trim_matches(|c| c == '"' || c == '\'') == "agentq")
    }))
}

/// Claude Code runs the agentq hooks from this home's `settings.json`.
fn claude_attached(home: &Path) -> Result<bool, ()> {
    Ok(read(&home.join("settings.json"))?.is_some_and(|s| s.contains(" hook claude ")))
}

/// The effective state for `client` in `home` from the read-only installation state (M12.5). Host disable and the
/// host and agent switches win; `on` never enables anything.
pub fn observe(intent: Intent, client: Client, install: Option<&Path>, home: Option<&Path>) -> Optimization {
    let o = |effective, reason| Optimization { intent, effective, reason };
    if intent == Intent::Off { return o("off", "launch_off"); }
    let Some(install) = install.filter(|i| i.join("data").is_dir()) else { return o("off", "not_installed") };
    let data = install.join("data");
    let (Ok(control), Ok(config)) = (json_file(&data.join("integration.json")), json_file(&data.join("config.json"))) else {
        return o("unknown", "unreadable_state");
    };
    if control["host_disabled"] == true || control["profiles"].get(client.code()).is_some() { return o("off", "integration_disabled"); }
    let c = &config["compression"];
    if c["enabled"] == false { return o("off", "host_off"); }
    if c["agents"][client.code()] == false { return o("off", "agent_off"); }
    let Some(home) = home else { return o("unknown", "no_client_home") };
    let attached = match client { Client::Codex => codex_routed(home), Client::Claude => claude_attached(home) };
    match attached {
        Ok(true) => o("on", "configured"),
        Ok(false) => o("off", "not_attached"),
        Err(()) => o("unknown", "unreadable_client_config"),
    }
}

/// Observe the state for `spec` and carry `off` and the launcher marker into the child (M12.4, M12.6). Adds only child
/// environment and, for a Codex home routed through agentq, one per-launch header override; nothing is written.
pub fn prepare(intent: Intent, product: &str, spec: &mut Spec, install: Option<&Path>) -> Optimization {
    let home = spec.home.clone().or_else(|| base_home(spec.client));
    let o = observe(intent, spec.client, install, home.as_deref());
    let set = |spec: &Spec, k: &str| spec.env.iter().any(|(n, v)| n == k && !v.is_empty()) || std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if !set(spec, "TASKQ_TASK") && !set(spec, "AGENTQ_LAUNCHER") { spec.env.push(("AGENTQ_LAUNCHER".into(), product.into())); }
    if intent == Intent::Off {
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
        assert_eq!((seen(Intent::Inherit, Client::Codex), seen(Intent::On, Client::Claude)), (("on", "configured"), ("on", "configured")));
        assert_eq!(seen(Intent::Off, Client::Claude), ("off", "launch_off"));
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
}
