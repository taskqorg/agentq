//! Client profiles (docs/spec/CONFIGURATION.md A1, A2): read-only integration state per configuration root for
//! status.json `configuration`. Typed values and fixed codes only; `snapshot.rs` re-validates each one before export.
//!
//! Per profile `verification` comes from `executor::verification` (CONFIGURATION "Host executor").

use crate::{onboard, resource};
use ring::hmac;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const SCHEMA: u64 = 1;
/// Kind, display label, client aliases that share its configuration root, supported operations (A1).
pub const KINDS: [(&str, &str, &[&str], &[&str]); 5] = [
    ("codex", "Codex", &["codex-cli", "codex-desktop"], &["hooks"]),
    ("claude-code", "Claude Code", &["claude-code-cli", "claude-desktop-code"], &["hooks"]),
    ("claude-desktop-chat", "Claude Desktop Chat", &["claude-desktop-chat"], &[]),
    ("hermes", "Hermes", &["hermes"], &["plugin"]),
    ("antigravity", "Antigravity", &["agy"], &["hooks"]),
];

/// One configuration root and the files whose bytes make its revision.
#[derive(Clone)]
pub struct Client {
    pub kind: &'static str,
    /// The agent key of `compression.agents` and routing (agentq.md § 3, § 5).
    pub agent: &'static str,
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
}

fn desktop_root() -> PathBuf {
    let home = onboard::root("claude").parent().map(Path::to_path_buf).unwrap_or_default();
    match std::env::consts::OS {
        "macos" => home.join("Library/Application Support/Claude"),
        "windows" => std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Claude"),
        _ => std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config")).join("Claude"),
    }
}

pub fn clients() -> Vec<Client> {
    let c = |kind, agent, root: PathBuf, files: &[&str]| Client { kind, agent, files: files.iter().map(|f| root.join(f)).collect(), root };
    let mut code = c("claude-code", "claude", onboard::root("claude"), &["settings.json"]);
    code.files.push(onboard::claude_json());
    let mut clients = vec![
        c("codex", "codex", onboard::root("codex"), &["config.toml", "hooks.json"]),
        code,
        c("claude-desktop-chat", "claude-desktop", desktop_root(), &["claude_desktop_config.json"]),
        c("hermes", "hermes", onboard::root("hermes"), &["config.yaml"]),
        c("antigravity", "antigravity", onboard::root("antigravity"), &["config/hooks.json"]),
    ];
    for c in &mut clients {
        if !matches!(c.agent, "claude-desktop" | "antigravity") { c.files.push(c.root.join(if c.agent == "claude" { "CLAUDE.md" } else { "AGENTS.md" })); }
    }
    clients
}

pub struct Provider {pub selected:bool,pub present:bool,pub loopback:bool}

pub fn codex_provider(root: &Path) -> Result<Provider,String> {
    let text = match std::fs::read_to_string(root.join("config.toml")) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Provider {selected:false,present:false,loopback:false}),
        Err(_) => return Err("unreadable_config".into()),
    };
    let mut section = "";
    let (mut selected, mut loopback, mut present) = (false, false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') { section = line.split('#').next().unwrap_or("").trim();
            present |= section == "[model_providers.agentq]"; continue; }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches(['"', '\'']).split(['"', '\'']).next().unwrap_or("");
        if section.is_empty() && key.trim() == "model_provider" { selected = value == "agentq"; }
        if section == "[model_providers.agentq]" && key.trim() == "base_url" {
            let host = value.split_once("://").map(|(_, s)| s.split('/').next().unwrap_or(""));
            loopback = host.is_some_and(|s| s == "localhost" || s.starts_with("localhost:") || s.parse::<std::net::SocketAddr>().is_ok_and(|a| a.ip().is_loopback()) || s.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback()));
        }
    }
    Ok(Provider {selected,present,loopback})
}

/// Read-only evidence of a selected legacy provider; no endpoint or credential is exported.
pub fn legacy_route(clients: &[Client]) -> Value {
    let Some(c)=clients.iter().find(|c|c.agent=="codex") else {return json!({"state":"absent"})};
    match codex_provider(&c.root) {
        Err(_)=>json!({"state":"unknown","reason":"unreadable_config"}),
        Ok(p) if p.selected && (!p.present || p.loopback)=>json!({"state":"problem","reason":if p.present {"legacy_agentq_route"} else {"legacy_agentq_provider_missing"},"next":"agentq direct"}),
        Ok(_)=>json!({"state":"absent"}),
    }
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

/// The registered events of native hook entries whose command runs `agentq hook <agent> <event>`.
fn hooked(doc: &Value, agent: &str, events: &[&str]) -> Vec<bool> {
    let hooks = &doc[if agent == "antigravity" { "agentq" } else { "hooks" }];
    events.iter().map(|e| (agent != "antigravity" || hooks["enabled"] != false) && hooks[e].as_array().into_iter().flatten()
        .filter(|m| agent != "antigravity" || m["matcher"].as_str().is_none_or(|m| matches!(m, "" | "*" | "run_command")))
        .flat_map(|m| if agent == "antigravity" && *e == "Stop" { vec![m] } else { m["hooks"].as_array().into_iter().flatten().collect() })
        .any(|h| h["command"].as_str().is_some_and(|c| c.ends_with(&format!(" hook {agent} {e}"))))).collect()
}

fn hooks_state(h: &[bool]) -> &'static str {
    match h.iter().filter(|x| **x).count() {
        0 => "absent",
        n if n == h.len() => "registered",
        _ => "partial",
    }
}

/// The indented lines of the top-level YAML block `head:`.
fn yaml_block<'a>(text: &'a str, head: &str) -> Vec<&'a str> {
    text.lines().skip_while(|l| l.trim_end() != format!("{head}:")).skip(1).take_while(|l| l.trim().is_empty() || l.starts_with([' ', '\t', '-'])).collect()
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
    let json_doc = |i: usize| docs[i].clone();
    if c.files.iter().enumerate().any(|(i, f)| f.extension().is_some_and(|e| e == "json") && docs[i].is_none() && !unread(i)) {
        conflicts.push("unparseable_config");
    }
    let mut overrides = None;
    let (hooks, plugin, adapter) = match c.kind {
        "codex" => {
            if legacy_route(std::slice::from_ref(c))["state"] == "problem" { conflicts.push("legacy_agentq_route"); }
            overrides = (!unread(0)).then(|| texts[0].as_deref().unwrap_or("").lines().filter(|l| l.trim_start().starts_with("[profiles.")).count());
            (json_doc(1).map(|d| hooked(&d, "codex", &["SessionStart", "Stop"])).as_deref().map_or("unknown", hooks_state), "unsupported", "hook")
        }
        "claude-code" => {
            let set = |v: &Value| v.as_object().is_some_and(|o| !o.is_empty()) || v.as_array().is_some_and(|a| !a.is_empty());
            overrides = json_doc(1).map(|d| d["projects"].as_object().into_iter().flatten().filter(|(_, p)| ["mcpServers", "env", "enabledMcpjsonServers"].iter().any(|k| set(&p[*k]))).count());
            (json_doc(0).map(|d| hooked(&d, "claude", &["SessionStart", "Stop"])).as_deref().map_or("unknown", hooks_state), "unsupported", "hook")
        }
        "antigravity" => (json_doc(0).map(|d| hooked(&d, "antigravity", &["Stop"])).as_deref().map_or("unknown", hooks_state), "unsupported", "hook"),
        "claude-desktop-chat" => ("unsupported", "unsupported", ""),
        _ => {
            let plugin = if unread(0) { "unknown" } else if yaml_block(texts[0].as_deref().unwrap_or(""), "plugins").iter().any(|l| l.contains("agentq")) { "enabled" } else { "absent" };
            ("unsupported", plugin, "plugin")
        }
    };
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
