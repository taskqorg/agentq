//! Host configuration executor (docs/spec/CONFIGURATION.md A1, A3, "Host executor"): one request under the lifecycle
//! lock, a durable operation record, journaled writes through `onboard`, fresh readback, reconciliation of interrupted
//! records and hook evidence for verification. Acceptance itself stays in the pure `configure::handle`.
//!
//! Entries: `agentq configure` (local grant) and the enrolled operation pass in `connect.rs`.

use crate::{configure, onboard, profiles, routing};
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
        "compression": compression(&h.data)["agents"], "result": r,
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

fn config(data: &Path) -> Value {
    std::fs::read_to_string(data.join("config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

fn compression(data: &Path) -> Value {
    config(data)["compression"].clone()
}

/// `compression.agents.<agent>`: `false`, or absent for the default; host-wide keys and other agents stay.
fn set_preference(data: &Path, agent: &str, v: &Value) -> Result<(), String> {
    let mut c = config(data);
    if !c["compression"]["agents"].is_object() {
        c["compression"]["agents"] = json!({});
    }
    let agents = c["compression"]["agents"].as_object_mut().expect("object");
    match v {
        Value::Null => agents.shift_remove(agent),
        v => agents.insert(agent.into(), v.clone()),
    };
    write_atomic(&data.join("config.json"), &serde_json::to_string_pretty(&c).unwrap_or_default())
}

fn agent_of(c: &profiles::Client) -> &'static str {
    if c.kind == "claude-desktop-chat" { "claude-desktop" } else { c.agent }
}

/// One owned write; Err is the safe reason code.
fn write(h: &Host, c: &profiles::Client, field: &str, value: &str) -> Result<(), &'static str> {
    let journal = h.journal();
    let failed = |lines: Vec<String>| if lines.iter().any(|l| l.starts_with("conflict:")) { Err("conflict") } else { Ok(()) };
    match (field, value) {
        ("optimization", v) => set_preference(&h.data, c.agent, &if v == "disabled" { json!(false) } else { Value::Null }).map_err(|_| "write_failed"),
        ("routing", "direct") => {
            let files: Vec<String> = c.files.iter().map(|f| f.display().to_string()).collect();
            let mine = |_: usize, e: &Value| e["routing"] == c.agent && e["file"].as_str().is_some_and(|f| files.iter().any(|x| x == f));
            onboard::restore_where(&journal, false, true, &mine).map_err(|_| "write_failed").and_then(failed)
        }
        ("routing", _) => {
            let install = h.data.parent().unwrap_or(&h.data);
            crate::engines::headroom_exe(install).ok_or("engine_unavailable")?;
            let port = routing::fix_port(&h.data, c.agent).map_err(|_| "engine_unavailable")?;
            onboard::apply(&onboard::routing_wants(c.agent, &c.root, port), &journal).map_err(|_| "write_failed").and_then(failed)
        }
        (f, _) => {
            let claude_json = c.files.get(1).cloned().unwrap_or_default();
            let wants: Vec<onboard::Want> = onboard::wants(agent_of(c), &c.root, &claude_json, &h.exe).into_iter().filter(|w| w.field() == f).collect();
            onboard::apply(&wants, &journal).map_err(|_| "write_failed").and_then(failed)?;
            // M4.11: a hook Codex has not been told to trust is skipped.
            if f == "hooks" && c.agent == "codex" { crate::hook_trust::ensure(&c.root, &h.exe, &journal, None).map_err(|_| "write_failed").and_then(failed)?; }
            Ok(())
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
fn restore(h: &Host, c: &profiles::Client, rec: &Value, changes: &[Value]) {
    let before = &rec["journal"];
    let files: Vec<String> = c.files.iter().map(|f| f.display().to_string()).collect();
    let mine = |_: usize, e: &Value| before.as_array().is_some_and(|b| !b.contains(&json!(configure::digest(e)[..16]))) && e["file"].as_str().is_some_and(|f| files.iter().any(|x| x == f));
    let _ = onboard::restore_where(&h.journal(), false, true, &mine);
    if changes.iter().any(|ch| ch["field"] == "optimization") {
        let _ = set_preference(&h.data, c.agent, &rec["compression"][c.agent]);
    }
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
    let events: &[&str] = match (p["effective"]["state"].as_str(), p["effective"]["adapter"].as_str()) {
        (Some("configured"), Some("hook")) if p["effective"]["event"] == "PostToolUse" => &["PostToolUse"],
        (Some("configured"), Some("hook")) => &["PreToolUse"],
        (Some("bypassed" | "disabled"), _) => &EVENTS,
        // ponytail: the provider and plugin adapters have no evidence source yet
        _ => &[],
    };
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
        wants.extend(onboard::routing_wants("codex", &clients[0].root, 1));
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
    fn apply_reads_back_and_verifies_by_evidence() {
        let d = dir("apply");
        let h = host(&d);
        let (codex, desktop, claude) = (id(&h, "codex"), id(&h, "claude-desktop-chat"), id(&h, "claude-code"));
        let claude_before = std::fs::read_to_string(&h.clients[1].files[0]).unwrap();
        let changes = json!([{"profile": codex, "field": "routing", "value": "direct"}, {"profile": codex, "field": "optimization", "value": "disabled"}, {"profile": desktop, "field": "mcp", "value": "registered"}]);
        let (plan, req) = plan_apply(&h, "a0000000000000000000000000000001", changes, 1000);
        assert_eq!((plan["writable"].as_bool(), plan["blocked"].clone()), (Some(true), json!([])), "{plan}");
        assert!(records(&h.data).is_empty(), "a plan stores nothing");

        let r = execute(&h, &req.to_string(), 1001);
        assert_eq!((r["outcome"].as_str(), r["reason"].clone()), (Some("applied"), Value::Null), "{r}");
        assert_eq!(r["profiles"], json!([{"id": codex, "state": "applied"}, {"id": desktop, "state": "applied"}]));
        assert!(configure::valid_result(&r));
        let toml = std::fs::read_to_string(&h.clients[0].files[0]).unwrap();
        assert!(!toml.lines().any(|l| l.starts_with("model_provider =")) && toml.contains("[mcp_servers.agentq]") && toml.contains(onboard::CODEX_DIRECT_PROVIDER), "Direct keeps MCP and saved conversation access: {toml}");
        let cfg = config(&h.data)["compression"].clone();
        assert_eq!(cfg, json!({"enabled": true, "min_bytes": 4096, "agents": {"hermes": false, "codex": false}}), "host-wide keys and siblings stay");
        assert_eq!(std::fs::read_to_string(&h.clients[1].files[0]).unwrap(), claude_before, "sibling profile untouched");
        let desk: Value = serde_json::from_str(&std::fs::read_to_string(&h.clients[2].files[0]).unwrap()).unwrap();
        assert_eq!(desk["mcpServers"]["agentq"]["args"], json!(["mcp", "claude-desktop"]));

        let st = profiles::status_with(&h.clients, &h.data, 1002);
        let p = st["profiles"].as_array().unwrap().iter().find(|p| p["id"] == codex).unwrap().clone();
        assert_eq!((p["routing"].as_str(), p["optimization"].as_str(), p["effective"]["state"].as_str()), (Some("direct"), Some("disabled"), Some("disabled")));
        assert_eq!(p["verification"], json!({"state": "unknown", "reason": "no_evidence"}), "configured is not verified");

        let before = files(&h);
        assert_eq!(execute(&h, &req.to_string(), 1003), r, "duplicate delivery returns the first result");
        let mut changed = req.clone();
        changed["changes"][2]["profile"] = claude.clone();
        assert_eq!(execute(&h, &changed.to_string(), 1003)["reason"], "payload_mismatch");
        assert_eq!(files(&h), before, "neither repeat writes");

        stamp(&h.data, "codex", "SessionStart", 1000);
        assert_eq!(verification(&h.data, "codex", &p)["reason"], "no_evidence", "an event before the apply proves nothing");
        stamp(&h.data, "codex", "SessionStart", 1005);
        stamp(&h.data, "codex", "../x", 1005);
        let v = verification(&h.data, "codex", &p);
        assert_eq!((v["state"].as_str(), v["at"].as_u64(), v["revision"].clone()), (Some("verified"), Some(1005), p["revision"].clone()), "{v}");
        assert!(!d.join("install/data/x").exists() && !d.join("install/x").exists(), "event names no path");
        let mut native_settings: Value = serde_json::from_str(&claude_before).unwrap();
        native_settings["permissions"] = json!({"defaultMode": "auto"});
        std::fs::write(&h.clients[1].files[0], native_settings.to_string()).unwrap();
        let (_, auto_req) = plan_apply(&h, "a0000000000000000000000000000002", json!([{"profile": claude, "field": "hooks", "value": "registered"}]), 1010);
        assert_eq!(execute(&h, &auto_req.to_string(), 1010)["outcome"], "applied");
        let auto_status = profiles::status_with(&h.clients, &h.data, 1011);
        let auto_profile = auto_status["profiles"].as_array().unwrap().iter().find(|p| p["id"] == claude).unwrap();
        assert_eq!(auto_profile["effective"]["event"], "PostToolUse");
        stamp(&h.data, "claude", "PreToolUse", 1012);
        assert_eq!(verification(&h.data, "claude", auto_profile)["state"], "unknown", "the auto-mode pre-hook cannot verify result replacement");
        stamp(&h.data, "claude", "PostToolUse", 1013);
        assert_eq!(verification(&h.data, "claude", auto_profile)["state"], "verified");
        // A2: rapid applies and a wall-clock rollback still select the last committed profile state.
        for (index, (op, value, at)) in [
            ("a0000000000000000000000000000004", "disabled", 1010),
            ("a0000000000000000000000000000003", "enabled", 1010),
            ("a0000000000000000000000000000005", "disabled", 1009),
        ].into_iter().enumerate() {
            let (_, req) = plan_apply(&h, op, json!([{"profile": claude, "field": "optimization", "value": value}]), at);
            assert_eq!(execute(&h, &req.to_string(), at)["outcome"], "applied");
            let current = read_profile(&h, &h.clients[1]);
            assert_eq!(verification(&h.data, "claude", &current)["state"], "unknown", "an old hook stamp cannot verify a later apply after clock rollback: {op}");
            stamp(&h.data, "claude", "PostToolUse", 1014 + index as u64);
            assert_eq!(verification(&h.data, "claude", &current)["state"], "verified", "newest operation must win: {op}");
        }
        let current = read_profile(&h, &h.clients[1]);
        let latest = records(&h.data).into_iter().max_by_key(record_order).unwrap();
        let mut legacy = latest.clone();
        legacy.as_object_mut().unwrap().shift_remove("evidence_after");
        store(&h.data, &legacy).unwrap();
        assert_eq!(verification(&h.data, "claude", &current)["reason"], "no_evidence", "legacy records cannot prove event ordering");
        store(&h.data, &latest).unwrap();
        std::fs::write(&h.clients[0].files[0], format!("{toml}\n# edited\n")).unwrap();
        let st = profiles::status_with(&h.clients, &h.data, 1006);
        let p = st["profiles"].as_array().unwrap().iter().find(|p| p["id"] == codex).unwrap();
        assert_eq!(p["verification"]["reason"], "revision_changed", "a foreign edit drops verification");
        assert!(!st.to_string().contains(&d.display().to_string()), "no path in the projection");
        let _ = std::fs::remove_dir_all(d);
    }

    /// #103 A1, A2, A6: a foreign edit after review is stale, a held lock is busy, full storage refuses, an unwritable
    /// value is restored with its cause, revocation before acceptance rejects and after it keeps the actual outcome.
    #[test]
    fn guards_and_races_keep_files() {
        let d = dir("guards");
        let h = host(&d);
        let codex = id(&h, "codex");
        let off = json!([{"profile": codex, "field": "optimization", "value": "disabled"}]);

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

        let (_, req) = plan_apply(&h, "b0000000000000000000000000000004", json!([{"profile": codex, "field": "routing", "value": "direct"}]), 1000);
        assert_eq!(execute(&h, &req.to_string(), 1001)["outcome"], "applied");
        // Codex routing needs headroom, absent here: nothing is claimed, the cause is reported
        let routed = json!([{"profile": codex, "field": "routing", "value": "routed"}]);
        let (_, req) = plan_apply(&h, "b0000000000000000000000000000003", routed, 1000);
        let before = files(&h);
        let r = execute(&h, &req.to_string(), 1001);
        assert_eq!((r["outcome"].as_str(), r["reason"].as_str(), r["profiles"][0]["state"].as_str()), (Some("rejected"), Some("engine_unavailable"), Some("restored")), "{r}");
        assert_eq!(files(&h), before);
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
        let h = host(&d);
        let (codex, desktop) = (id(&h, "codex"), id(&h, "claude-desktop-chat"));
        let changes = json!([{"profile": codex, "field": "optimization", "value": "disabled"}, {"profile": desktop, "field": "mcp", "value": "registered"}]);
        let (plan, req) = plan_apply(&h, "c0000000000000000000000000000001", changes, 1000);
        // a crash after the first write: preference written, Desktop MCP not
        set_preference(&h.data, "codex", &json!(false)).unwrap();
        let rec = json!({"operation": "c0000000000000000000000000000001", "digest": configure::digest(&req), "plan": plan["digest"], "at": 1001, "journal": [],
            "changes": [{"profile": codex, "field": "optimization", "old": "enabled", "new": "disabled"}, {"profile": desktop, "field": "mcp", "old": "absent", "new": "registered"}],
            "result": configure::result(Some("c0000000000000000000000000000001"), "accepted", None, plan["digest"].as_str())});
        store(&h.data, &rec).unwrap();
        let before = files(&h);
        let r = execute(&h, &req.to_string(), 1002);
        assert_eq!((r["outcome"].as_str(), r["reason"].as_str()), (Some("partial"), Some("interrupted")), "{r}");
        assert_eq!(r["profiles"], json!([{"id": codex, "state": "applied"}, {"id": desktop, "state": "restored"}]));
        assert_eq!(files(&h), before, "never reapplied");
        let (_, next) = plan_apply(&h, "c0000000000000000000000000000002", json!([{"profile": desktop, "field": "mcp", "value": "registered"}]), 1003);
        assert_eq!(execute(&h, &next.to_string(), 1003)["outcome"], "applied", "the installation is free again");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // a read-only Desktop Chat root: its MCP write fails and is reported, the Codex preference still applies
            let desk = &h.clients[2].root;
            onboard::restore_where(&h.journal(), false, true, &|_, e: &Value| e["file"].as_str().is_some_and(|f| f.contains("desktop"))).unwrap();
            assert!(!h.clients[2].files[0].exists());
            std::fs::set_permissions(desk, std::fs::Permissions::from_mode(0o500)).unwrap();
            let both = json!([{"profile": codex, "field": "optimization", "value": "enabled"}, {"profile": desktop, "field": "mcp", "value": "registered"}]);
            let (_, req) = plan_apply(&h, "c0000000000000000000000000000004", both, 1005);
            let r = execute(&h, &req.to_string(), 1005);
            std::fs::set_permissions(desk, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!((r["outcome"].as_str(), r["reason"].as_str()), (Some("partial"), Some("write_failed")), "{r}");
            assert_eq!(r["profiles"], json!([{"id": codex, "state": "applied"}, {"id": desktop, "state": "restored"}]));
        }
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
