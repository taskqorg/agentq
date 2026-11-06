//! Host configuration executor (docs/spec/CONFIGURATION.md A1, A3, "Host executor"): one request under the lifecycle
//! lock, a durable operation record, journaled writes through `onboard`, fresh readback, reconciliation of interrupted
//! records and hook evidence for verification. Acceptance itself stays in the pure `configure::handle`.
//!
//! Entries: `agentq configure` (local grant) and the enrolled operation pass in `connect.rs`.

use crate::{configure, onboard, profiles};
use coreq::write_atomic;
use serde_json::{Value, json};
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Resolved records kept; an unresolved (`accepted`) one is never pruned.
const KEEP: usize = 64;
/// Events whose stamp proves a client turn through the profile (CONFIGURATION "Host executor").
const EVENTS: [&str; 4] = ["SessionStart", "Stop", "PreToolUse", "PostToolUse"];

/// Everything one request runs against. `grant`: the `agentq.configure` grant, or null.
pub struct Host {
    pub data: PathBuf,
    pub exe: PathBuf,
    pub clients: Vec<profiles::Client>,
    pub installation: String,
    pub generation: u64,
    pub grant: Value,
}

impl Host {
    /// The local owner's authority (`agentq configure`): actor and workspace `local`, generation 0, every current
    /// profile, a grant valid for one expiry window.
    pub fn local(install: &Path, exe: PathBuf, now: u64) -> Host {
        let data = install.join("data");
        let clients = profiles::clients();
        let installation = crate::telemetry::installation(&data);
        let ids: Vec<Value> = profiles::status_with(&clients, &data, now)["profiles"].as_array().into_iter().flatten().map(|p| p["id"].clone()).collect();
        let grant = json!({"capability": configure::CAPABILITY, "actor": "local", "workspace": "local", "installation": installation, "generation": 0, "profiles": ids, "expires_at": now + configure::EXPIRY});
        Host { data, exe, clients, installation, generation: 0, grant }
    }

    fn journal(&self) -> PathBuf {
        self.data.join("owned.json")
    }
}

/// The lifecycle lock (agentq.md § 9): one writer of owned configuration at a time; released on drop.
pub fn lock(data: &Path) -> Result<File, String> {
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let f = File::options().create(true).truncate(false).write(true).open(data.join("lifecycle.lock")).map_err(|e| format!("lifecycle lock: {e}"))?;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(50)),
            Err(TryLockError::Error(e)) => return Err(format!("lifecycle lock: {e}")),
        }
    }
    Err("another AgentQ change holds the lifecycle lock".into())
}

fn ops(data: &Path) -> PathBuf {
    data.join("operations")
}

fn records(data: &Path) -> Vec<Value> {
    let files = std::fs::read_dir(ops(data)).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".json"));
    files.filter_map(|e| std::fs::read_to_string(e.path()).ok()).filter_map(|s| serde_json::from_str(&s).ok()).collect()
}

// The lifecycle lock serializes writes; wall-clock seconds do not order rapid applies.
fn record_order(rec: &Value) -> (u64, u64) {
    (rec["sequence"].as_u64().unwrap_or(0), rec["at"].as_u64().unwrap_or(0))
}

fn store(data: &Path, rec: &Value) -> Result<(), String> {
    std::fs::create_dir_all(ops(data)).map_err(|e| e.to_string())?;
    write_atomic(&ops(data).join(format!("{}.json", rec["operation"].as_str().unwrap_or_default())), &rec.to_string())
}

/// Durable control storage answers a write now (A4: full or unavailable storage refuses acceptance).
fn durable(data: &Path) -> bool {
    let probe = ops(data).join(".probe");
    let ok = std::fs::create_dir_all(ops(data)).is_ok() && write_atomic(&probe, "probe").is_ok();
    let _ = std::fs::remove_file(probe);
    ok
}

fn prune(data: &Path) {
    let mut done: Vec<Value> = records(data).into_iter().filter(|r| r["result"]["outcome"] != "accepted").collect();
    done.sort_by_key(|r| std::cmp::Reverse(record_order(r)));
    for r in done.iter().skip(KEEP) {
        let _ = std::fs::remove_file(ops(data).join(format!("{}.json", r["operation"].as_str().unwrap_or_default())));
    }
}

fn op_id(req: &Value) -> Option<&str> {
    req["operation"].as_str().filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// One delivered request: a plan, or an apply that runs to its actual outcome. Duplicates return the stored result.
pub fn execute(h: &Host, raw: &str, now: u64) -> Value {
    let req: Value = serde_json::from_str(raw).unwrap_or_default();
    // the ID names a file only after this check: 32 lowercase hex, no path
    let op = op_id(&req);
    let Ok(_lock) = lock(&h.data) else { return configure::result(op, "rejected", Some("busy"), None) };
    reconcile(h);
    let seen = op.and_then(|o| std::fs::read_to_string(ops(&h.data).join(format!("{o}.json"))).ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let history = records(&h.data);
    let outstanding = history.iter().find(|r| r["result"]["outcome"] == "accepted").map(|r| r["operation"].clone());
    let configuration = profiles::status_with(&h.clients, &h.data, now);
    let host = json!({
        "installation": h.installation, "generation": h.generation, "now": now, "durable": durable(&h.data), "grant": h.grant,
        "outstanding": outstanding, "seen": seen.as_ref().map(|s| json!({"digest": s["digest"], "result": s["result"]})),
    });
    let r = configure::handle(raw, &host, &configuration);
    if r["outcome"] != "accepted" || seen.is_some() {
        return r;
    }
    let plan = configure::plan(&req, &configuration);
    let changes: Vec<Value> = plan["profiles"].as_array().into_iter().flatten().flat_map(|p| p["changes"].as_array().into_iter().flatten().map(move |c| json!({"profile": p["id"], "field": c["field"], "old": c["old"], "new": c["new"]}))).collect();
    let mut rec = json!({
        "operation": op, "digest": configure::digest(&req), "actor": h.grant["actor"], "workspace": h.grant["workspace"], "installation": req["installation"],
        "generation": req["generation"], "plan": plan["digest"], "at": now,
        "sequence": history.iter().map(|r| record_order(r).0).max().unwrap_or(0).saturating_add(1), "changes": changes, "journal": entries(h),
        "result": r,
    });
    // A3: the record is durable before any write
    if store(&h.data, &rec).is_err() {
        return configure::result(op, "rejected", Some("storage_unavailable"), None);
    }
    apply(h, &mut rec);
    // ponytail: a failed final store leaves `accepted`; the next request or service start resolves it from state
    let _ = store(&h.data, &rec);
    prune(&h.data);
    rec["result"].clone()
}

/// Short digests of the journal's entries, so a restore picks only this operation's edits; entries may hold file
/// copies and never enter a record.
fn entries(h: &Host) -> Vec<String> {
    onboard::load(&h.journal()).iter().map(|e| configure::digest(e)[..16].to_string()).collect()
}

fn agent_of(c: &profiles::Client) -> &'static str {
    if c.kind == "claude-desktop-chat" { "claude-desktop" } else { c.agent }
}

/// One owned write; Err is the safe reason code.
fn write(h: &Host, c: &profiles::Client, field: &str, value: &str) -> Result<(), &'static str> {
    let journal = h.journal();
    let failed = |lines: Vec<String>| if lines.iter().any(|l| l.starts_with("conflict:")) { Err("conflict") } else { Ok(()) };
    match (field, value) {
        ("routing", "direct") => {
            let files: Vec<String> = c.files.iter().map(|f| f.display().to_string()).collect();
            let mine = |_: usize, e: &Value| e["routing"] == c.agent && e["file"].as_str().is_some_and(|f| files.iter().any(|x| x == f));
            onboard::restore_where(&journal, false, true, &mine).map_err(|_| "write_failed").and_then(failed)
        }
        ("routing" | "optimization" | "mcp", _) => Err("feature_retired"),
        (f, _) => {
            let claude_json = c.files.get(1).cloned().unwrap_or_default();
            let wants: Vec<onboard::Want> = onboard::wants(agent_of(c), &c.root, &claude_json, &h.exe).into_iter().filter(|w| w.field() == f).collect();
            onboard::setup(&wants, &journal).map_err(|e| if e.contains("CLI") {"client_missing"} else {"write_failed"}).and_then(failed)
        }
    }
}

fn read_profile(h: &Host, c: &profiles::Client) -> Value {
    profiles::profile(c, &h.data, &h.journal(), &crate::resource::key(&h.data))
}

fn client<'a>(h: &'a Host, id: &Value) -> Option<&'a profiles::Client> {
    h.clients.iter().find(|c| c.root.is_dir() && read_profile(h, c)["id"] == *id)
}

/// Undo this operation's owned edits and preference for one client; later foreign edits stay.
fn restore(h: &Host, c: &profiles::Client, rec: &Value, _changes: &[Value]) {
    let before = &rec["journal"];
    let files: Vec<String> = c.files.iter().map(|f| f.display().to_string()).collect();
    let mine = |_: usize, e: &Value| before.as_array().is_some_and(|b| !b.contains(&json!(configure::digest(e)[..16]))) && e["file"].as_str().is_some_and(|f| files.iter().any(|x| x == f));
    let _ = onboard::restore_where(&h.journal(), false, true, &mine);

}

/// The per-profile changes of `rec`, grouped by profile ID in plan order.
fn by_profile(rec: &Value) -> Vec<(Value, Vec<Value>)> {
    let mut out: Vec<(Value, Vec<Value>)> = vec![];
    for ch in rec["changes"].as_array().into_iter().flatten() {
        match out.iter_mut().find(|p| p.0 == ch["profile"]) {
            Some(p) => p.1.push(ch.clone()),
            None => out.push((ch["profile"].clone(), vec![ch.clone()])),
        }
    }
    out
}

fn record_applied(data: &Path, rec: &mut Value, id: &Value, profile: &Value, agent: &str) {
    let id = id.as_str().unwrap_or_default();
    rec["applied"][id] = profile["revision"].clone();
    // Existing hook stamps cannot verify a later apply, even after the wall clock moves back.
    rec["evidence_after"][id] = EVENTS.iter().filter_map(|event| stamped(data, agent, event)).max().unwrap_or(0)
        .max(rec["at"].as_u64().unwrap_or(u64::MAX)).into();
}

/// Write, read back, restore a mismatch; `rec.result` becomes the actual outcome and `rec.applied` the read-back
/// revision of every applied profile.
fn apply(h: &Host, rec: &mut Value) {
    let mut states = vec![];
    let mut reason = None;
    for (id, changes) in by_profile(rec) {
        let Some(c) = client(h, &id) else {
            states.push((id, "unknown"));
            reason.get_or_insert("unknown_profile");
            continue;
        };
        for ch in changes.iter().filter(|ch| ch["old"] != ch["new"]) {
            if let Err(e) = write(h, c, ch["field"].as_str().unwrap_or(""), ch["new"].as_str().unwrap_or("")) {
                reason.get_or_insert(e);
            }
        }
        let p = read_profile(h, c);
        let state = if changes.iter().all(|ch| p[ch["field"].as_str().unwrap_or("")] == ch["new"]) {
            if changes.iter().all(|ch| ch["old"] == ch["new"]) { "unchanged" } else { "applied" }
        } else if changes.iter().any(|ch| ch["field"] == "routing" && ch["new"] == "routed" && p["routing"] == "unknown") {
            // the config is written but the proxy is silent: ambiguous, never restored
            reason.get_or_insert("not_answering");
            "unknown"
        } else {
            reason.get_or_insert("readback_mismatch");
            restore(h, c, rec, &changes);
            let back = read_profile(h, c);
            if changes.iter().all(|ch| back[ch["field"].as_str().unwrap_or("")] == ch["old"]) { "restored" } else { "unknown" }
        };
        if state == "applied" || state == "unchanged" {
            record_applied(&h.data, rec, &id, &p, c.agent);
        }
        states.push((id, state));
    }
    finish(rec, &states, reason);
}

fn finish(rec: &mut Value, states: &[(Value, &str)], reason: Option<&str>) {
    let ok = |s: &(Value, &str)| ["applied", "unchanged"].contains(&s.1);
    let outcome = if states.iter().all(ok) {
        "applied"
    } else if states.iter().any(ok) {
        "partial"
    } else if states.iter().any(|s| s.1 == "unknown") {
        "unknown"
    } else {
        "rejected"
    };
    let reason = (outcome != "applied").then(|| reason.unwrap_or("interrupted"));
    let mut r = configure::result(rec["operation"].as_str(), outcome, reason, rec["plan"].as_str());
    r["profiles"] = states.iter().map(|(id, s)| json!({"id": id, "state": s})).collect();
    rec["result"] = r;
}

/// Resolve every `accepted` record left by an interrupted run from current state; never reapply.
pub fn reconcile(h: &Host) {
    for mut rec in records(&h.data).into_iter().filter(|r| r["result"]["outcome"] == "accepted") {
        let mut states = vec![];
        for (id, changes) in by_profile(&rec) {
            let c = client(h, &id);
            let p = c.map(|c| read_profile(h, c)).unwrap_or_default();
            let all = |k: &str| changes.iter().all(|ch| p[ch["field"].as_str().unwrap_or("")] == ch[k]);
            let state = if all("new") {
                if let Some(c) = c { record_applied(&h.data, &mut rec, &id, &p, c.agent); }
                "applied"
            } else if all("old") {
                "restored"
            } else {
                "unknown"
            };
            states.push((id, state));
        }
        finish(&mut rec, &states, Some("interrupted"));
        let _ = store(&h.data, &rec);
    }
}

/// `agentq hook <agent> <event>`: the newest time a client ran this event; a success moves the event's failure record
/// to `.last_failure`, so `.failed` exists only while the latest run failed, whatever the clock says (#132 A2), and the
/// heartbeat still names the last failure after the client recovered (#134).
pub fn stamp(data: &Path, agent: &str, event: &str, now: u64) {
    if onboard::AGENTS.contains(&agent) && EVENTS.contains(&event) {
        let ev = data.join("evidence");
        let _ = std::fs::create_dir_all(&ev);
        let _ = write_atomic(&ev.join(format!("{agent}.{event}")), &now.to_string());
        let _ = std::fs::rename(ev.join(format!("{agent}.{event}.failed")), ev.join(format!("{agent}.{event}.last_failure")));
    }
}

/// A hook run that could not finish (#132 A2): `evidence/<agent>.<event>.failed` keeps its time and error.
pub fn hook_failed(data: &Path, agent: &str, event: &str, now: u64, error: &str) {
    if onboard::AGENTS.contains(&agent) && EVENTS.contains(&event) {
        let _ = std::fs::create_dir_all(data.join("evidence"));
        let _ = write_atomic(&data.join("evidence").join(format!("{agent}.{event}.failed")), &json!({"at": now, "error": error}).to_string());
    }
}

/// Hook events whose latest run failed, as `<agent> <event> (<error>)`.
pub fn hook_failures(data: &Path) -> Vec<String> {
    let mut out = vec![];
    for a in onboard::AGENTS {
        for e in EVENTS {
            // only a missing record means no failure; an unreadable one (invalid UTF-8, no access) is reported
            let f: Value = match std::fs::read_to_string(data.join("evidence").join(format!("{a}.{e}.failed"))) {
                Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => Value::Null,
            };
            out.push(format!("{a} {e} ({})", f["error"].as_str().unwrap_or("unreadable failure record")));
        }
    }
    out
}

/// Heartbeat `agentq.hooks` (#134): per agent and event with any record, the state of its latest run (`ok`, `failed`,
/// or `unknown` with a `reason` when its success stamp, failure record or retained last failure is unreadable), its newest successful run and its last failure `{at, error}`, which
/// stays after a later success; null when it never failed.
pub fn hooks(data: &Path) -> Value {
    let unreadable = || json!({"at": null, "error": "unreadable failure record"});
    let read = |name: String| match std::fs::read_to_string(data.join("evidence").join(name)) {
        Ok(s) => Some(serde_json::from_str::<Value>(&s).ok().filter(|f| f["at"].is_u64() && f["error"].is_string()).unwrap_or_else(unreadable)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(unreadable()),
    };
    let mut out = vec![];
    for a in onboard::AGENTS {
        for e in EVENTS {
            // a missing success stamp means no success; a present but unreadable one is Some(None)
            let ok = match std::fs::read_to_string(data.join("evidence").join(format!("{a}.{e}"))) {
                Ok(s) => Some(s.trim().parse::<u64>().ok()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => Some(None),
            };
            let failed = read(format!("{a}.{e}.failed"));
            let last = failed.clone().or_else(|| read(format!("{a}.{e}.last_failure")));
            if ok.is_none() && last.is_none() {
                continue;
            }
            // an unreadable success stamp or retained failure leaves the latest run unproved, never `ok` (R12)
            let (state, reason) = match (&failed, &last) {
                _ if ok == Some(None) => ("unknown", Some("unreadable success stamp")),
                (_, Some(f)) if f["at"].is_null() => ("unknown", Some("unreadable failure record")),
                (Some(_), _) => ("failed", None),
                _ => ("ok", None),
            };
            let mut hook = json!({"agent": a, "event": e, "state": state, "last_ok_at": ok.flatten(), "last_failure": last});
            if let Some(r) = reason {
                hook["reason"] = r.into();
            }
            out.push(hook);
        }
    }
    out.into()
}

fn stamped(data: &Path, agent: &str, event: &str) -> Option<u64> {
    std::fs::read_to_string(data.join("evidence").join(format!("{agent}.{event}"))).ok()?.trim().parse().ok()
}

/// A2 per-profile verification: `verified` only when the revision read back after the newest applied operation is
/// still current and a newer client event proves the effective state; otherwise unknown with a reason.
pub fn verification(data: &Path, agent: &str, p: &Value) -> Value {
    let id = p["id"].as_str().unwrap_or_default();
    let unknown = |r: &str| json!({"state": "unknown", "reason": r});
    let Some(rec) = records(data).into_iter().filter(|r| r["applied"][id].is_string()).max_by_key(record_order) else {
        return unknown("no_evidence");
    };
    if rec["applied"][id] != p["revision"] {
        return unknown("revision_changed");
    }
    let Some(applied) = rec["evidence_after"][id].as_u64() else { return unknown("no_evidence"); };
    let events: &[&str] = if p["effective"]["state"] == "configured" && p["effective"]["adapter"] == "hook" {
        &["SessionStart", "Stop"]
    } else { &[] };
    let newest = events.iter().filter_map(|e| stamped(data, agent, e).filter(|t| *t > applied).map(|t| (t, *e))).max();
    match newest {
        Some((at, event)) => json!({
            "state": "verified", "at": at, "adapter": p["effective"]["adapter"], "revision": p["revision"],
            "evidence": &configure::digest(&json!([agent, event, at]))[..16],
        }),
        None => unknown("no_evidence"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-exec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Codex, Claude Code and Desktop Chat roots under `d`, all onboarded except Desktop Chat; data in `d/install/data`.
    fn host(d: &Path) -> Host {
        let c = |kind, agent, sub: &str, files: &[&str]| {
            let root = d.join(sub);
            std::fs::create_dir_all(&root).unwrap();
            profiles::Client { kind, agent, files: files.iter().map(|f| root.join(f)).collect(), root }
        };
        let clients = vec![
            c("codex", "codex", "codex", &["config.toml", "hooks.json"]),
            c("claude-code", "claude", "claude", &["settings.json", ".claude.json"]),
            c("claude-desktop-chat", "claude-desktop", "desktop", &["claude_desktop_config.json"]),
        ];
        let data = d.join("install/data");
        let exe = d.join("install/agentq");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(&clients[0].files[0], "model = \"gpt-5\"\n").unwrap();
        std::fs::write(&clients[1].files[0], "{\n  \"model\": \"opus\"\n}\n").unwrap();
        std::fs::write(data.join("config.json"), r#"{"ports": {"codex": 1}, "compression": {"enabled": true, "min_bytes": 4096, "agents": {"hermes": false}}}"#).unwrap();
        let mut wants = onboard::wants("codex", &clients[0].root, &clients[1].files[1], &exe);
        wants.extend(onboard::wants("claude", &clients[1].root, &clients[1].files[1], &exe));
        onboard::apply(&wants, &data.join("owned.json")).unwrap();
        let mut h = Host { data, exe, clients, installation: "inst-1".into(), generation: 0, grant: Value::Null };
        let ids: Vec<Value> = profiles::status_with(&h.clients, &h.data, 1000)["profiles"].as_array().unwrap().iter().map(|p| p["id"].clone()).collect();
        h.grant = json!({"capability": configure::CAPABILITY, "actor": "local", "workspace": "local", "installation": "inst-1", "generation": 0, "profiles": ids, "expires_at": 5000});
        h
    }

    fn id(h: &Host, kind: &str) -> Value {
        profiles::status_with(&h.clients, &h.data, 1000)["profiles"].as_array().unwrap().iter().find(|p| p["kind"] == kind).unwrap()["id"].clone()
    }

    fn request(op: &str, kind: &str, changes: Value) -> Value {
        json!({"schema": 1, "operation": op, "kind": kind, "actor": "local", "workspace": "local", "installation": "inst-1", "generation": 0, "issued_at": 990, "expires_at": 1200, "changes": changes})
    }

    /// Plan, then apply the confirmed plan at `now`.
    fn plan_apply(h: &Host, op: &str, changes: Value, now: u64) -> (Value, Value) {
        let plan = execute(h, &request(op, "plan", changes.clone()).to_string(), now);
        let mut req = request(op, "apply", changes);
        req["plan"] = json!({"revision": plan["revision"], "digest": plan["digest"]});
        (plan, req)
    }

    fn files(h: &Host) -> Vec<Option<String>> {
        h.clients.iter().flat_map(|c| c.files.iter()).chain([&h.data.join("config.json")]).map(|f| std::fs::read_to_string(f).ok()).collect()
    }

    /// #103 A1, A2, A4, A5: Codex Direct plus optimization off and Desktop Chat MCP apply under the lock, read back as
    /// configured (not verified), leave Claude Code and host-wide defaults alone; a duplicate returns the stored
    /// result, a changed payload is refused; a newer hook event verifies the Direct bypass until a foreign edit.
    #[test]
    fn hermes_plugin_plan_apply_uses_owned_files_and_preserves_other_plugins() {
        let d = dir("hermes-plugin");
        let _cli = crate::test_support::native_clis(&d);
        let mut h = host(&d);
        let root = d.join("hermes"); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.yaml"), "plugins:\n  enabled:\n    - other\n").unwrap();
        h.clients.push(profiles::Client { kind: "hermes", agent: "hermes", files: vec![root.join("config.yaml")], root: root.clone() });
        let profile = id(&h, "hermes");
        h.grant["profiles"].as_array_mut().unwrap().push(profile.clone());
        let (plan, req) = plan_apply(&h, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", json!([{ "profile": profile, "field": "plugin", "value": "enabled" }]), 1000);
        assert_eq!(plan["writable"], true, "{plan}");
        let result = execute(&h, &req.to_string(), 1001);
        assert_eq!(result["outcome"], "applied", "{result}");
        assert!(root.join("plugins/agentq/__init__.py").is_file());
        assert!(std::fs::read_to_string(root.join("config.yaml")).unwrap().contains("    - other\n"));
        assert!(onboard::restore(&h.journal(), false, None).unwrap().iter().all(|s| s.starts_with("restored:")));
        assert_eq!(std::fs::read_to_string(root.join("config.yaml")).unwrap(), "plugins:\n  enabled:\n    - other\n");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn guards_and_races_keep_files() {
        let d = dir("guards");
        let _cli = crate::test_support::native_clis(&d);
        let h = host(&d);
        let codex = id(&h, "codex");
        let off = json!([{"profile": codex, "field": "hooks", "value": "registered"}]);

        let (_, req) = plan_apply(&h, "b0000000000000000000000000000001", off.clone(), 1000);
        let toml = h.clients[0].files[0].clone();
        std::fs::write(&toml, format!("{}# foreign\n", std::fs::read_to_string(&toml).unwrap())).unwrap();
        let before = files(&h);
        assert_eq!(execute(&h, &req.to_string(), 1001)["reason"], "stale_revision");

        let (_, req) = plan_apply(&h, "b0000000000000000000000000000002", off.clone(), 1000);
        let held = lock(&h.data).unwrap();
        assert_eq!(execute(&h, &req.to_string(), 1001)["reason"], "busy");
        drop(held);

        std::fs::remove_dir_all(ops(&h.data)).unwrap();
        std::fs::write(ops(&h.data), "not a directory").unwrap();
        assert_eq!(execute(&h, &req.to_string(), 1001)["reason"], "storage_unavailable");
        assert_eq!(files(&h), before, "nothing written on refusal");
        std::fs::remove_file(ops(&h.data)).unwrap();

        let revoked = Host { grant: Value::Null, data: h.data.clone(), exe: h.exe.clone(), clients: host_clients(&h), installation: h.installation.clone(), generation: 0 };
        assert_eq!(execute(&revoked, &req.to_string(), 1001)["reason"], "no_grant", "revoked before acceptance");
        assert_eq!(files(&h), before);
        let r = execute(&h, &req.to_string(), 1001);
        assert_eq!(r["outcome"], "applied", "{r}");
        assert_eq!(execute(&revoked, &req.to_string(), 1002), r, "revoked after it: the actual outcome, never an undo");

        let _ = std::fs::remove_dir_all(d);
    }

    fn host_clients(h: &Host) -> Vec<profiles::Client> {
        h.clients.iter().map(|c| profiles::Client { kind: c.kind, agent: c.agent, root: c.root.clone(), files: c.files.clone() }).collect()
    }

    /// #103 A2, A6: an operation interrupted after acceptance resolves from current state, is never reapplied and
    /// frees the installation; a partial write reads back as such.
    #[test]
    fn interrupted_operation_reconciles_without_reapply() {
        let d = dir("crash");
        let _cli = crate::test_support::native_clis(&d);
        let h = host(&d);
        let (codex, desktop) = (id(&h, "codex"), id(&h, "claude-code"));
        let changes = json!([{"profile": codex, "field": "hooks", "value": "registered"}, {"profile": desktop, "field": "hooks", "value": "registered"}]);
        let (plan, req) = plan_apply(&h, "c0000000000000000000000000000001", changes, 1000);
        // A crash after Codex hooks committed, before the Claude hooks were written.
        onboard::restore_where(&h.journal(), false, true, &|_, e: &Value| e["agent"] == "claude").unwrap();
        let rec = json!({"operation": "c0000000000000000000000000000001", "digest": configure::digest(&req), "plan": plan["digest"], "at": 1001, "journal": [],
            "changes": [{"profile": codex, "field": "hooks", "old": "absent", "new": "registered"}, {"profile": desktop, "field": "hooks", "old": "absent", "new": "registered"}],
            "result": configure::result(Some("c0000000000000000000000000000001"), "accepted", None, plan["digest"].as_str())});
        store(&h.data, &rec).unwrap();
        let before = files(&h);
        let r = execute(&h, &req.to_string(), 1002);
        assert_eq!((r["outcome"].as_str(), r["reason"].as_str()), (Some("partial"), Some("interrupted")), "{r}");
        assert_eq!(r["profiles"], json!([{"id": codex, "state": "applied"}, {"id": desktop, "state": "restored"}]));
        assert_eq!(files(&h), before, "never reapplied");
        let (_, next) = plan_apply(&h, "c0000000000000000000000000000002", json!([{"profile": desktop, "field": "hooks", "value": "registered"}]), 1003);
        assert_eq!(execute(&h, &next.to_string(), 1003)["outcome"], "applied", "the installation is free again");

        let _ = std::fs::remove_dir_all(d);
    }

    /// #132 A2: a recorded hook failure is a problem until a later run of the same event succeeds.
    #[test]
    fn hook_failure_stays_until_a_later_success() {
        let d = std::env::temp_dir().join(format!("agentq-hookfail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        stamp(&d, "codex", "Stop", 100);
        hook_failed(&d, "codex", "Stop", 200, "stdin: broken pipe");
        hook_failed(&d, "codex", "../x", 200, "ignored");
        assert_eq!(hook_failures(&d), ["codex Stop (stdin: broken pipe)"]);
        stamp(&d, "codex", "Stop", 300);
        assert!(hook_failures(&d).is_empty());
        // a failure after a success in the same second stays: order, not the clock, decides
        stamp(&d, "codex", "Stop", 400);
        hook_failed(&d, "codex", "Stop", 400, "stdout: broken pipe");
        assert_eq!(hook_failures(&d), ["codex Stop (stdout: broken pipe)"]);
        stamp(&d, "codex", "Stop", 400);
        assert!(hook_failures(&d).is_empty());
        hook_failed(&d, "claude", "PostToolUse", 401, "stdout: broken pipe");
        assert_eq!(hook_failures(&d), ["claude PostToolUse (stdout: broken pipe)"], "result-hook failures must remain visible");
        stamp(&d, "claude", "PostToolUse", 402);
        assert!(hook_failures(&d).is_empty());
        std::fs::write(d.join("evidence").join("claude.SessionStart.failed"), "{").unwrap();
        assert_eq!(hook_failures(&d), ["claude SessionStart (unreadable failure record)"]);
        // #134: the heartbeat keeps the last failure after the success that cleared it; unreadable reads unknown
        assert_eq!(hooks(&d), json!([
            {"agent": "codex", "event": "Stop", "state": "ok", "last_ok_at": 400, "last_failure": {"at": 400, "error": "stdout: broken pipe"}},
            {"agent": "claude", "event": "SessionStart", "state": "unknown", "reason": "unreadable failure record", "last_ok_at": null, "last_failure": {"at": null, "error": "unreadable failure record"}},
            {"agent": "claude", "event": "PostToolUse", "state": "ok", "last_ok_at": 402, "last_failure": {"at": 401, "error": "stdout: broken pipe"}},
        ]));
        let _ = std::fs::remove_dir_all(&d);
    }
}
