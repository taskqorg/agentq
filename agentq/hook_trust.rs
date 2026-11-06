//! M4.11-M4.13: Codex trust of AgentQ's exact hook entries (docs/spec/RUNTIME.md § Hook trust).
use crate::onboard;
use agentq_runtime::agent::{self, AccountScope};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Owned Codex hook events: the `hooks.json` name, the native name and the key label.
const EVENTS: [(&str, &str, &str); 2] = [("SessionStart", "sessionStart", "session_start"), ("Stop", "stop", "stop")];
const NEXT: &str = "run `agentq onboard --yes codex`";
const READ_BUDGET: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq, Eq)]
pub struct Reading {
    pub event: &'static str,
    /// `trusted`, `untrusted`, `modified`, `disabled` or `absent`.
    pub state: String,
    pub key: Option<String>,
    pub hash: Option<String>,
}

fn same_file(a: &Path, b: &Path) -> bool {
    let plain = |p: &Path| { let t = p.display().to_string().replace('\\', "/"); if cfg!(windows) { t.to_ascii_lowercase() } else { t } };
    plain(a) == plain(b)
}

fn command_of(root: &Path, exe: &Path, event: &str) -> Option<String> {
    onboard::wants("codex", root, Path::new(""), exe).into_iter().find(|w| w.hook_event() == Some(event)).and_then(|w| w.hook_command())
}

/// The native reading: only entries with the exact command, event and source file of AgentQ's own (M4.11).
fn native(root: &Path, exe: &Path, codex: Option<&Path>) -> Result<Vec<Reading>, String> {
    let runtime = agent::runtime("codex").ok_or("runtime missing")?;
    let scope = AccountScope { home: Some(root.into()), env: vec![], executable: codex.map(PathBuf::from) };
    let hooks = runtime.hooks(root, &scope, Instant::now() + READ_BUDGET).map_err(|e| e.to_string())?;
    let file = root.join("hooks.json");
    Ok(EVENTS.iter().map(|(event, name, _)| {
        let command = command_of(root, exe, event);
        let hit = hooks.iter().find(|h| h.event == *name && h.command == command && h.source_path.as_deref().is_some_and(|p| same_file(p, &file)));
        match hit {
            Some(h) if !h.enabled => Reading { event, state: "disabled".into(), key: Some(h.key.clone()), hash: h.hash.clone() },
            Some(h) => Reading { event, state: h.trust.clone(), key: Some(h.key.clone()), hash: h.hash.clone() },
            None => Reading { event, state: "absent".into(), key: None, hash: None },
        }
    }).collect())
}

/// The hash Codex computes: SHA-256 of the sorted compact identity of the normalized handler.
fn computed_hash(label: &str, command: &str, timeout: u64) -> String {
    let identity = json!({"event_name": label, "hooks": [{"async": false, "command": command, "timeout": timeout, "type": "command"}]}).to_string();
    let digest = ring::digest::digest(&ring::digest::SHA256, identity.as_bytes());
    format!("sha256:{}", digest.as_ref().iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// `Value::to_string` keeps insertion order here, so the identity above lists its keys sorted.
fn computed(root: &Path, exe: &Path) -> Result<Vec<Reading>, String> {
    let file = root.join("hooks.json");
    let doc: Value = match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|_| "hooks.json unreadable")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(_) => return Err("hooks.json unreadable".into()),
    };
    let config = match std::fs::read_to_string(root.join("config.toml")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err("config.toml unreadable".into()),
    };
    Ok(EVENTS.iter().map(|(event, _, label)| {
        let command = command_of(root, exe, event).unwrap_or_default();
        let found = doc["hooks"][*event].as_array().into_iter().flatten().enumerate().find_map(|(g, group)| {
            group["hooks"].as_array().into_iter().flatten().enumerate().find_map(|(h, handler)| {
                (handler["command"] == command.as_str()).then(|| (g, h, handler["timeout"].as_u64().unwrap_or(600)))
            })
        });
        let Some((group, handler, timeout)) = found else { return Reading { event, state: "absent".into(), key: None, hash: None } };
        let key = format!("{}:{label}:{group}:{handler}", file.display());
        let hash = computed_hash(label, &command, timeout);
        let state = onboard::hook_state(&config, &key);
        let trust = match state.as_ref() {
            Some(s) if s.enabled == Some(false) => "disabled",
            Some(s) if s.trusted_hash.as_deref() == Some(hash.as_str()) => "trusted",
            Some(s) if s.trusted_hash.is_some() => "modified",
            _ => "untrusted",
        };
        Reading { event, state: trust.into(), key: Some(key), hash: Some(hash) }
    }).collect())
}

/// Native first; computed only when that read fails (M4.11). The second value names the source.
fn read(root: &Path, exe: &Path, codex: Option<&Path>) -> Result<(Vec<Reading>, &'static str), String> {
    match native(root, exe, codex) {
        Ok(r) => Ok((r, "native")),
        Err(native_error) => computed(root, exe).map(|r| (r, "computed")).map_err(|e| format!("{native_error}; {e}")),
    }
}

/// The read-only state of the owned hooks for `doctor` and `status` (M4.12).
pub fn status(root: &Path, exe: &Path, codex: Option<&Path>) -> Value {
    match read(root, exe, codex) {
        Ok((rows, source)) => {
            let ok = rows.iter().all(|r| r.state == "trusted");
            json!({"state": if ok { "ok" } else { "failed" }, "source": source,
                "hooks": rows.iter().map(|r| json!({"event": r.event, "state": r.state})).collect::<Vec<_>>(),
                "next": (!ok).then_some(NEXT)})
        }
        Err(reason) => json!({"state": "unknown", "reason": reason, "hooks": [], "next": NEXT}),
    }
}

/// `doctor` lines: one per owned hook, a failure with the next step unless trusted (M4.12).
pub fn lines(status: &Value) -> Vec<(bool, String)> {
    if status["state"] == "unknown" {
        return vec![(false, format!("hooks: codex trust unknown ({}); {NEXT}", status["reason"].as_str().unwrap_or("unknown")))];
    }
    let source = status["source"].as_str().unwrap_or("unknown");
    status["hooks"].as_array().into_iter().flatten().map(|h| {
        let (event, state) = (h["event"].as_str().unwrap_or("unknown"), h["state"].as_str().unwrap_or("unknown"));
        let good = state == "trusted";
        (good, format!("hooks: codex {event} {state} ({source}){}", if good { String::new() } else { format!("; {NEXT}") }))
    }).collect()
}

/// M4.11: journal and write trust for the exact owned entries, then read it back. A foreign state stays and reads as a conflict.
pub fn ensure(root: &Path, exe: &Path, journal: &Path, codex: Option<&Path>) -> Result<Vec<String>, String> {
    let mut out = onboard::migrate_quoted_codex_hooks(root, exe, journal)?;
    let (rows, source) = read(root, exe, codex)?;
    let current: Vec<&str> = rows.iter().filter_map(|r| r.key.as_deref()).collect();
    let config = root.join("config.toml").display().to_string();
    let owned = |e: &Value| e["file"] == config.as_str() && onboard::hook_state_key(e).is_some();
    // A journaled block of a key that moved, or with a hash Codex no longer reports, gives way to the current one.
    let stale = |e: &Value| owned(e) && onboard::hook_state_key(e).is_some_and(|key| {
        !current.contains(&key.as_str()) || rows.iter().any(|r| r.key.as_deref() == Some(key.as_str()) && r.state == "modified")
    });
    let restored = onboard::restore_where(journal, false, true, &|_, e: &Value| stale(e))?;
    let (rows, source) = if restored.is_empty() { (rows, source) } else { read(root, exe, codex)? };
    out.extend(restored);
    let wants: Vec<_> = rows.iter().filter(|r| r.state == "untrusted").filter_map(|r| Some(onboard::hook_trust_want(root, r.key.as_deref()?, r.hash.as_deref()?))).collect();
    if !wants.is_empty() { out.extend(onboard::apply(&wants, journal)?); }
    let (after, source) = if wants.is_empty() { (rows, source) } else { read(root, exe, codex)? };
    out.extend(after.iter().map(|r| match r.state.as_str() {
        "trusted" => format!("hooks: codex {} trusted ({source})", r.event),
        "absent" => format!("hooks: codex {} absent", r.event),
        state => format!("conflict: codex hook {} reads {state}; a state AgentQ does not own stays; {NEXT} after clearing it", r.event),
    }));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let d = std::env::temp_dir().join(format!("agentq-hook-trust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let root = d.join("codex");
        std::fs::create_dir_all(&root).unwrap();
        (root, d.join("install/agentq"), d.join("install/data/owned.json"))
    }

    const ABSENT: &str = "agentq-absent-codex-binary";

    // Native values of codex-cli 0.162.1 for these exact entries (docs/evidence/runtime-284.md).
    #[test]
    fn computed_hash_matches_native_hooks_list() {
        assert_eq!(computed_hash("session_start", "\"C:/x/agentq\" hook codex SessionStart", 5), "sha256:bfd5783ca49905e8bef8f99cdb65bbbfcde4478fc38f0581a93a4829d3d49773");
        assert_eq!(computed_hash("stop", "\"C:/x/agentq\" hook codex Stop", 5), "sha256:7303615bec869a9baf123ae1f4a05b6f8ea7530a2f1ed3acd89eeaaa91fb85aa");
    }

    // M4.11, M4.12, M9. Risk: a skipped Stop hook silently loses records; a foreign hook gains trust; uninstall leaves trust behind.
    #[test]
    fn ensure_trusts_only_owned_entries_reports_failures_and_restores() {
        let (root, exe, journal) = fixture("ensure");
        let codex = Path::new(ABSENT);
        let foreign = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"foreign"}]}]}}"#;
        std::fs::write(root.join("hooks.json"), foreign).unwrap();
        std::fs::write(root.join("config.toml"), "model = \"m\"\n").unwrap();
        let before = status(&root, &exe, Some(codex));
        assert_eq!(before["state"], "failed", "absent owned hooks fail: {before}");
        assert_eq!(before["hooks"][0]["state"], "absent");
        assert!(lines(&before).iter().all(|(good, l)| !good && l.contains("agentq onboard --yes codex")), "{before}");

        onboard::apply(&onboard::wants("codex", &root, Path::new(""), &exe), &journal).unwrap();
        let untrusted = status(&root, &exe, Some(codex));
        assert_eq!(untrusted["hooks"][1]["state"], "untrusted", "an edit alone is not trust: {untrusted}");
        assert_eq!(untrusted["source"], "computed");
        assert_eq!(untrusted["state"], "failed");

        let lines_out = ensure(&root, &exe, &journal, Some(codex)).unwrap();
        assert!(lines_out.iter().any(|l| l == "hooks: codex Stop trusted (computed)"), "{lines_out:?}");
        let toml = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert_eq!(toml.matches("[hooks.state.").count(), 2, "only AgentQ's two entries: {toml}");
        assert!(toml.contains(":stop:1:0"), "the foreign group keeps index 0: {toml}");
        assert!(!toml.contains(":stop:0:0"), "the foreign hook gains no trust: {toml}");
        assert_eq!(status(&root, &exe, Some(codex))["state"], "ok");

        let n = onboard::load(&journal).len();
        assert!(ensure(&root, &exe, &journal, Some(codex)).unwrap().iter().all(|l| l.starts_with("hooks:")), "repeat is safe");
        assert_eq!(onboard::load(&journal).len(), n, "repeat journals nothing");
        assert!(onboard::load(&journal).iter().any(|e| onboard::hook_state_key(e).is_some()), "the trust edit is journaled");

        // The foreign group goes: the owned key moves, its old block gives way.
        let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(root.join("hooks.json")).unwrap()).unwrap();
        doc["hooks"]["Stop"].as_array_mut().unwrap().remove(0);
        std::fs::write(root.join("hooks.json"), doc.to_string()).unwrap();
        assert_eq!(status(&root, &exe, Some(codex))["state"], "failed", "a moved key is no longer trusted");
        ensure(&root, &exe, &journal, Some(codex)).unwrap();
        let toml = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(toml.contains(":stop:0:0") && !toml.contains(":stop:1:0"), "{toml}");
        assert_eq!(status(&root, &exe, Some(codex))["state"], "ok");

        onboard::restore(&journal, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("config.toml")).unwrap(), "model = \"m\"\n", "uninstall leaves no trust");
    }

    // M4.6, M4.7, M4.11. Risk: disable leaves trust behind or breaks on the new block, or enable loses it.
    #[test]
    fn disable_detaches_trust_and_enable_restores_it() {
        let (root, exe, journal) = fixture("control");
        let codex = Path::new(ABSENT);
        std::fs::write(root.join("config.toml"), "model = \"m\"\n").unwrap();
        onboard::apply(&onboard::wants("codex", &root, Path::new(""), &exe), &journal).unwrap();
        ensure(&root, &exe, &journal, Some(codex)).unwrap();
        let all = |entries: &[Value]| { let mut w = onboard::wants("codex", &root, Path::new(""), &exe); w.extend(onboard::hook_trust_wants("codex", &root, entries)); w };
        let saved = onboard::capture(&all(&onboard::load(&journal)), &journal).unwrap();
        assert!(saved.iter().any(|e| onboard::hook_state_key(e).is_some() && e["field"] == "hooks"), "{saved:?}");
        assert!(onboard::saved_wants(all(&saved), &saved).is_ok());
        onboard::detach(&journal, &saved, false).unwrap();
        assert!(saved.iter().all(|e| onboard::detached(e) == Ok(true)));
        assert!(!std::fs::read_to_string(root.join("config.toml")).unwrap().contains("hooks.state"), "disable removes trust");
        let wants = onboard::saved_wants(all(&saved), &saved).unwrap();
        onboard::apply(&wants, &journal).unwrap();
        assert!(onboard::present(&wants).unwrap(), "enable restores what disable saved");
        assert_eq!(status(&root, &exe, Some(codex))["state"], "ok");
    }

    // M4.2, M4.11. Risk: AgentQ rewrites a state the user set, such as a hook the user disabled.
    #[test]
    fn a_foreign_state_for_the_key_stays_and_reads_as_a_conflict() {
        let (root, exe, journal) = fixture("foreign");
        let codex = Path::new(ABSENT);
        onboard::apply(&onboard::wants("codex", &root, Path::new(""), &exe), &journal).unwrap();
        let key = |event: &str| format!("{}:{event}:0:0", root.join("hooks.json").display());
        let foreign = format!("[hooks.state.'{}']\nenabled = false\n\n[hooks.state.\"{}\"]\ntrusted_hash = \"sha256:other\"\n", key("stop"), key("session_start").replace('\\', "\\\\"));
        let config = std::fs::read_to_string(root.join("config.toml")).unwrap();
        std::fs::write(root.join("config.toml"), format!("{config}\n{foreign}")).unwrap();
        let out = ensure(&root, &exe, &journal, Some(codex)).unwrap();
        assert_eq!(out.iter().filter(|l| l.starts_with("conflict:")).count(), 2, "{out:?}");
        let text = std::fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(text.contains("enabled = false") && text.contains("sha256:other"), "{text}");
        let s = status(&root, &exe, Some(codex));
        assert_eq!((s["hooks"][0]["state"].as_str(), s["hooks"][1]["state"].as_str(), s["state"].as_str()), (Some("modified"), Some("disabled"), Some("failed")));
    }

    // #284 A6: the real Codex on PATH, on an isolated home named by the qualification script (docs/evidence/runtime-284.md).
    #[test]
    #[ignore = "native qualification; needs the real Codex CLI and an isolated home"]
    fn native_onboard_trust() {
        let (root, exe) = (PathBuf::from(std::env::var_os("AGENTQ_QUAL_CODEX_HOME").unwrap()), PathBuf::from(std::env::var_os("AGENTQ_QUAL_EXE").unwrap()));
        let journal = exe.parent().unwrap().join("data/owned.json");
        for line in onboard::apply(&onboard::wants("codex", &root, Path::new(""), &exe), &journal).unwrap() { println!("{line}"); }
        println!("status before trust: {}", status(&root, &exe, None));
        if std::env::var_os("AGENTQ_QUAL_TRUST").is_some() {
            for line in ensure(&root, &exe, &journal, None).unwrap() { println!("{line}"); }
            println!("status after trust: {}", status(&root, &exe, None));
        }
    }

    // M4.12. Risk: an unreadable file reads as healthy.
    #[test]
    fn unreadable_reads_unknown_and_fails() {
        let (root, exe, _) = fixture("unknown");
        std::fs::write(root.join("hooks.json"), "not json").unwrap();
        let s = status(&root, &exe, Some(Path::new(ABSENT)));
        assert_eq!(s["state"], "unknown");
        assert!(lines(&s).iter().all(|(good, l)| !good && l.contains("trust unknown")));
    }
}
