//! Client profiles (docs/spec/CONFIGURATION.md A1, A2): read-only integration state per configuration root for
//! status.json `configuration`. Typed values and fixed codes only; `snapshot.rs` re-validates each one before export.
//!
//! Per profile `verification` comes from `executor::verification` (CONFIGURATION "Host executor").

use crate::resource;
#[cfg(test)]
use crate::onboard;
use agentq_runtime::agent::registry;
use ring::hmac;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const SCHEMA: u64 = 1;
/// Kind, display label, client aliases that share its configuration root, supported operations (A1).
pub use registry::KINDS;

/// One configuration root and the files whose bytes make its revision.
#[derive(Clone)]
pub struct Client {
    pub kind: &'static str,
    /// Opaque registration identity used by lifecycle and hooks.
    pub agent: &'static str,
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
}

pub fn clients() -> Vec<Client> {
    let mut entries=registry::entries(); entries.sort_by_key(|e|e.profile_order);
    entries.into_iter().map(|e| { let root=(e.root)(); Client {kind:e.profile.kind,agent:e.name,files:(e.files)(&root),root} }).collect()
}

/// Read-only legacy diagnostics supplied by the selected registrations.
pub fn legacy_route(clients: &[Client]) -> Value {
    clients.iter().filter_map(|c|registry::entry(c.agent).map(|e|(e.diagnose)(&c.root)))
        .find(|d|d["state"]!="absent").unwrap_or_else(||json!({"state":"absent"}))
}

fn sign(key: &hmac::Key, parts: &[&[u8]]) -> String {
    let mut s = hmac::Context::with_key(key);
    for p in parts {
        s.update(&(p.len() as u64).to_le_bytes());
        s.update(p);
    }
    resource::hex(s.sign().as_ref())[..16].to_string()
}

/// status.json `configuration`: every client whose configuration root exists.
pub fn status(data: &Path, now: u64) -> Value {
    status_with(&clients(), data, now)
}

pub fn status_with(clients: &[Client], data: &Path, now: u64) -> Value {
    let key = resource::key(data);
    status_using(clients, data, now, &key)
}

pub fn readonly(clients: &[Client], data: &Path, now: u64) -> Value {
    match std::fs::read_to_string(data.join("resource-key")) {
        Ok(k) if k.trim().len() == 64 => status_using(clients, data, now, &hmac::Key::new(hmac::HMAC_SHA256, k.trim().as_bytes())),
        _ => json!({"schema": SCHEMA, "at": now, "state": "unknown", "reason": "profile_identity_unavailable", "profiles": []}),
    }
}

fn status_using(clients: &[Client], data: &Path, now: u64, key: &hmac::Key) -> Value {
    let journal = data.join("owned.json");
    let profiles: Vec<Value> = clients
        .iter()
        .filter(|c| c.root.is_dir() || crate::control::state(data).is_ok_and(|s| s["profiles"].get(c.agent).is_some()))
        .map(|c| {
            let mut p = profile(c, data, &journal, key);
            p["verification"] = crate::executor::verification(data, c.agent, &p);
            p
        })
        .collect();
    let defaults = json!({});
    let revs: Vec<&[u8]> = profiles.iter().filter_map(|p| p["revision"].as_str()).map(str::as_bytes).collect();
    json!({"schema": SCHEMA, "at": now, "revision": sign(key, &revs), "defaults": defaults, "profiles": profiles})
}

pub fn profile(c: &Client, data: &Path, journal: &Path, key: &hmac::Key) -> Value {
    let (mut conflicts, mut incomplete) = (vec![], vec![]);
    // ponytail: the revision signs whole files, so an unrelated edit also stales a plan; narrow to owned keys if
    // needless revision changes start rejecting confirmed plans
    let integration = crate::control::observation(data, c, &data.parent().unwrap_or(data).join(format!("agentq{}", std::env::consts::EXE_SUFFIX)));
    let settings = crate::control::config(data);
    if settings.is_err() || integration["desired"] == "unknown" { incomplete.push("unreadable_control"); }
    let mut parts: Vec<Vec<u8>> = vec![c.kind.into(),
        json!({"desired": integration["desired"], "phase": integration["phase"], "generation": integration["generation"]}).to_string().into()];
    let mut unreadable = vec![];
    let texts: Vec<Option<String>> = c
        .files
        .iter()
        .map(|f| match std::fs::read(f) {
            Ok(b) => {
                parts.push(b.clone());
                unreadable.push(false);
                Some(String::from_utf8_lossy(&b).into_owned())
            }
            Err(e) => {
                unreadable.push(e.kind() != std::io::ErrorKind::NotFound);
                parts.push(format!("missing {:?}", e.kind()).into());
                None
            }
        })
        .collect();
    if unreadable.contains(&true) {
        incomplete.push("unreadable_config");
    }
    let files: Vec<String> = c.files.iter().map(|f| f.display().to_string()).collect();
    let journal: Vec<Value> = std::fs::read_to_string(journal).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let owned: Vec<&Value> = journal.iter().filter(|e| e["file"].as_str().is_some_and(|f| files.iter().any(|x| x == f))).collect();
    parts.extend(owned.iter().map(|e| e.to_string().into_bytes()));
    let unread = |i: usize| unreadable[i];
    // a missing or empty file holds nothing; an unreadable or unparseable one is unknown
    let docs: Vec<Option<Value>> = texts
        .iter()
        .zip(&unreadable)
        .map(|(t, u)| match t.as_deref().map(|t| t.trim_start_matches('\u{feff}')) {
            _ if *u => None,
            None => Some(json!({})),
            Some(t) if t.trim().is_empty() => Some(json!({})),
            Some(t) => serde_json::from_str(t).ok(),
        })
        .collect();
    if c.files.iter().enumerate().any(|(i, f)| f.extension().is_some_and(|e| e == "json") && docs[i].is_none() && !unread(i)) {
        conflicts.push("unparseable_config");
    }
    let state = registry::profile(c.kind).map(|e|(e.inspect)(&texts,&docs,&unreadable));
    let (hooks,plugin,adapter,overrides)=state.map(|s|(s.hooks,s.plugin,s.adapter,s.overrides)).unwrap_or(("unknown","unknown","",None));
    if legacy_route(std::slice::from_ref(c))["state"] == "problem" { conflicts.push("legacy_agentq_route"); }
    let effective = if integration["desired"] == "unknown" { "unknown" }
        else if integration["desired"] == "disabled" { "disabled" }
        else if hooks == "registered" || plugin == "enabled" { "configured" }
        else if hooks == "unknown" || plugin == "unknown" { "unknown" }
        else if adapter.is_empty() { "unsupported" } else { "missing" };
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    let projection = json!({
        "kind": c.kind,
        "id": sign(key, &[b"profile", c.kind.as_bytes(), c.root.display().to_string().as_bytes()]),
        "revision": sign(key, &refs),
        "hooks": hooks,
        "hook_trust": if matches!(hooks,"registered"|"partial") {registry::hook_trust(c.agent,&c.root,Some(&data.join("owned.json")),false).map(|c|json!(c))} else {None},
        "plugin": plugin,
        "integration": integration,
        "effective": {"adapter": (!adapter.is_empty()).then_some(adapter), "state": effective},
        "owned": !owned.is_empty(),
        "overrides": overrides,
        "conflicts": conflicts,
        "incomplete": incomplete,
    });
    projection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-profiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn client(d: &Path, kind: &'static str) -> Client {
        let (agent, files): (_, &[&str]) = match kind {
            "codex" => ("codex", &["config.toml", "hooks.json"]),
            "claude-code" => ("claude", &["settings.json", ".claude.json"]),
            "claude-desktop-chat" => ("claude-desktop", &["claude_desktop_config.json"]),
            _ => ("hermes", &["config.yaml"]),
        };
        let root = d.join(kind);
        std::fs::create_dir_all(&root).unwrap();
        Client { kind, agent, files: files.iter().map(|f| root.join(f)).collect(), root }
    }

    /// #101 A2, A3: owned MCP/hooks show as configured, ids stay stable, an external edit changes the revision and a
    /// foreign provider, unparseable or unreadable config is flagged; no path leaves the projection.
    #[test]
    fn profile_state_and_revision() {
        let d = dir("state");
        let (data, journal) = (d.join("data"), d.join("data/owned.json"));
        let key = resource::key(&data);
        let codex = client(&d, "codex");
        let exe = d.join("agentq");
        onboard::apply(&onboard::wants("codex", &codex.root, Path::new("unused.json"), &exe), &journal).unwrap();
        let p = profile(&codex, &data, &journal, &key);
        assert_eq!(p["hooks"], "registered");
        assert_eq!(p["effective"], json!({"adapter":"hook","state":"configured"}));
        for field in ["routing", "mcp", "optimization"] { assert!(p.get(field).is_none()); }
        let toml = codex.root.join("config.toml");
        std::fs::write(&toml, "model_provider = \"other\"\n[profiles.work]\n").unwrap();
        let edited = profile(&codex, &data, &journal, &key);
        assert_ne!(edited["revision"], p["revision"]);
        assert_eq!(edited["id"], p["id"]);
        assert_eq!(edited["conflicts"], json!([]), "native providers are outside record-hook ownership");
        assert_eq!(edited["overrides"], 1);
        let claude = client(&d, "claude-code");
        std::fs::write(&claude.files[0], "{not json").unwrap();
        std::fs::create_dir_all(&claude.files[1]).unwrap();
        let c = profile(&claude, &data, &journal, &key);
        assert_eq!((c["hooks"].as_str(), c["owned"].as_bool()), (Some("unknown"), Some(false)));
        assert_eq!((c["conflicts"].clone(), c["incomplete"].clone()), (json!(["unparseable_config"]), json!(["unreadable_config"])));
        assert_eq!(c["effective"], json!({"adapter": "hook", "state": "unknown"}));
        assert!(c["overrides"].is_null(), "an unreadable .claude.json leaves overrides unknown");
        std::fs::remove_dir(&claude.files[1]).unwrap();
        let projects = json!({"projects": {"/Users/alice/secret": {"mcpServers": {"x": {"command": "sk-secret"}}}, "/Users/alice/plain": {"mcpServers": {}, "allowedTools": []}, "/Users/alice/env": {"env": {"TOKEN": "sk-secret"}}}});
        std::fs::write(&claude.files[1], projects.to_string()).unwrap();
        let o = profile(&claude, &data, &journal, &key);
        assert_eq!(o["overrides"], 2, "projects with MCP servers or env override the profile");
        assert!(!o.to_string().contains("alice") && !o.to_string().contains("sk-secret"));

        let hermes = client(&d, "hermes");
        std::fs::write(&hermes.files[0], "mcp_servers:\n  agentq:\n    command: x\nplugins:\n  enabled:\n    - agentq\n").unwrap();
        let h = profile(&hermes, &data, &journal, &key);
        assert_eq!((h["plugin"].as_str(), h["effective"]["state"].as_str()), (Some("enabled"), Some("configured")));

        let chat = client(&d, "claude-desktop-chat");
        let ch = profile(&chat, &data, &journal, &key);
        assert_eq!((ch["hooks"].as_str(), ch["effective"]["adapter"].is_null(), ch["overrides"].is_null()), (Some("unsupported"), true, true));

        let text = [p, edited, c, h, ch].iter().map(Value::to_string).collect::<String>();
        assert!(!text.contains(&d.display().to_string()) && !text.contains("agentq-profiles"), "no path in {text}");
        let _ = std::fs::remove_dir_all(d);
    }
    #[test]
    fn legacy_route_problem_is_readonly_and_not_release_health() {
        let root = std::env::temp_dir().join(format!("agentq-route-problem-{}", crate::resource::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        let clients = vec![Client {kind:"codex",agent:"codex",root:root.clone(),files:vec![root.join("config.toml")]}];
        for url in ["http://127.0.0.1:1234/v1", "http://localhost:1234/v1", "http://[::1]:1234/v1"] {
            std::fs::write(root.join("config.toml"), format!("model_provider = 'agentq' # selected\n[model_providers.agentq]\nbase_url = \"{url}\"\n")).unwrap();
            assert_eq!(legacy_route(&clients), json!({"state":"problem","reason":"legacy_agentq_route","next":"agentq direct"}));
        }
        std::fs::write(root.join("config.toml"), format!("model_provider = \"agentq\"\n{}", onboard::CODEX_DIRECT_PROVIDER)).unwrap();
        assert_eq!(legacy_route(&clients)["state"], "absent");
        std::fs::write(root.join("config.toml"), b"\xff").unwrap();
        assert_eq!(legacy_route(&clients)["state"], "unknown");
        std::fs::remove_dir_all(root).unwrap();
    }

}
