//! Enrollment snapshot (docs/spec/ENROLLMENT.md): a pure, versioned projection of `status.json` and the newest
//! `agentq.resource` span. Only allowlisted keys and enumerated values cross; paths, prompts, raw status text, raw
//! OTLP attributes and secrets never do. Unknown stays unknown with a reason, never zero.

use serde_json::{Map, Value, json};

pub const SCHEMA: u64 = 1;
/// `status.json` is unknown after 90 s (agentq.md § 5).
const STALE: u64 = 90;
const ROUTING: [&str; 4] = ["routed", "direct", "bypassed", "unknown"];
const ENGINES: [&str; 2] = ["rtk", "headroom"];
/// Resource reasons that pass as written; any other probe error becomes `probe failed`, so no probe path or OS text
/// crosses.
const REASONS: [&str; 5] = ["warmup", "counter reset", "wsl1 mirrors the Windows host", "no sampler on this OS", "no local filesystem"];
const NUMBERS: [&str; 6] = ["cpu_percent", "cpu_interval_ms", "memory_total_bytes", "memory_available_bytes", "memory_used_bytes", "clock_step_ms"];

fn reason(v: &Value) -> Value {
    json!(v.as_str().filter(|r| REASONS.contains(r)).unwrap_or("probe failed"))
}

fn pick<'a>(v: Option<&'a str>, allowed: &[&'a str]) -> &'a str {
    v.filter(|v| allowed.contains(v)).unwrap_or("unknown")
}

/// `installation`, `runtime`: `data/installation` and `telemetry::runtime()`; `status`: parsed `status.json` when
/// readable; `resource`: the newest queued `agentq.resource` span; `now`: unix seconds.
pub fn project(installation: &str, runtime: &str, status: Option<&Value>, resource: Option<&Value>, now: u64) -> Value {
    let at = status.and_then(|s| s["at"].as_u64());
    let service = match (status, at) {
        (None, _) => json!({"state": "unknown", "reason": "no status"}),
        (_, None) => json!({"state": "unknown", "reason": "status without time"}),
        (_, Some(a)) if now.saturating_sub(a) > STALE => json!({"state": "unknown", "reason": "stale status"}),
        _ => json!({"state": "running"}),
    };
    let s = status.cloned().unwrap_or(Value::Null);
    let agents: Map<String, Value> = s["agents"].as_object().into_iter().flatten().filter(|(a, _)| crate::onboard::AGENTS.contains(&a.as_str())).map(|(a, r)| (a.clone(), json!(pick(r["routing"].as_str(), &ROUTING)))).collect();
    let engines: Map<String, Value> = ENGINES
        .iter()
        .map(|e| {
            let x = &s["engines"][e];
            let v = match (x["state"].as_str(), x["version"].as_str()) {
                (Some(st @ ("ready" | "unavailable")), Some(ver)) if ver.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) => json!({"state": st, "version": ver}),
                _ => json!({"state": "unknown"}),
            };
            (e.to_string(), v)
        })
        .collect();
    let q = &s["queue"];
    let queue = json!({"pending": q["pending"].as_u64(), "oldest_age": q["oldest_age"].as_u64(), "last_delivery": q["last_delivery"].as_u64(), "dropped": q["dropped"].as_u64(), "send_error": q["error"].is_string()});
    json!({
        "schema": SCHEMA,
        "installation": installation,
        "runtime": runtime,
        "version": s["version"].as_str(),
        "status_at": at,
        "release": crate::release::project(&s["release"], now),
        "service": service,
        "update": {"available": s["update"].as_str(), "error": s["error"].is_string()},
        "routing": agents,
        "engines": engines,
        "queue": queue,
        "resource": resource.map_or_else(|| json!({"unknown": "no sample yet"}), project_resource),
        "configuration": configuration(status, now),
    })
}

/// At most this many profiles cross; more set `truncated` and disable every write.
const PROFILES: usize = 16;
const FIELDS: [(&str, &[&str]); 5] = [
    ("routing", &["routed", "direct", "bypassed", "unknown"]),
    ("optimization", &["enabled", "disabled"]),
    ("mcp", &["registered", "absent", "unknown"]),
    ("hooks", &["registered", "partial", "absent", "unknown"]),
    ("plugin", &["enabled", "absent", "unknown"]),
];
const EFFECTIVE: [&str; 6] = ["configured", "disabled", "bypassed", "missing", "unknown", "unsupported"];
const ADAPTERS: [&str; 3] = ["provider", "hook", "plugin"];
const CONFLICTS: [&str; 3] = ["foreign_provider", "unparseable_config", "integration_disabled"];
const INCOMPLETE: [&str; 2] = ["unreadable_config", "unreadable_control"];

fn id16(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn codes(v: &Value, allowed: &[&str], other: &'static str) -> Vec<Value> {
    let mut out: Vec<&str> = v.as_array().into_iter().flatten().map(|c| pick(c.as_str(), allowed)).map(|c| if c == "unknown" { other } else { c }).collect();
    out.dedup();
    out.into_iter().map(Value::from).collect()
}

/// A2 verification: `verified` only with a time, the profile's own adapter and revision and a 16-hex evidence ID;
/// anything else is unknown with a fixed reason.
fn verification(v: &Value, adapter: Option<&str>, revision: &str) -> Value {
    match (v["state"].as_str(), v["at"].as_u64(), id16(&v["evidence"])) {
        (Some("verified"), Some(at), Some(e)) if v["adapter"].as_str() == adapter && v["revision"] == revision => json!({"state": "verified", "at": at, "adapter": adapter, "revision": revision, "evidence": e}),
        _ => json!({"state": "unknown", "reason": pick(v["reason"].as_str(), &["no_evidence", "revision_changed"]).replace("unknown", "no_evidence")}),
    }
}

/// CONFIGURATION A2: profiles of status.json `configuration`. Labels, aliases and operations come from the kind,
/// every other value from a fixed set; ids and revisions are 16 hex. Configured never reads as verified, and a stale,
/// truncated, incomplete or conflicting profile is never writable.
/// ponytail: writes stay `not_released` until host and consumer qualification (CONFIGURATION A5) enable them.
pub fn configuration(status: Option<&Value>, now: u64) -> Value {
    let c = match status.map(|s| &s["configuration"]) {
        None => return json!({"state": "unknown", "reason": "no status"}),
        Some(Value::Null) => return json!({"state": "unknown", "reason": "older producer"}),
        Some(c) if c["schema"] != crate::profiles::SCHEMA => return json!({"state": "unknown", "reason": "unsupported schema"}),
        Some(c) => c,
    };
    let at = c["at"].as_u64();
    let fresh = at.is_some_and(|a| a <= now && now - a <= STALE);
    let all: Vec<&Value> = c["profiles"].as_array().into_iter().flatten().collect();
    let truncated = all.len() > PROFILES;
    let mut dropped = false;
    let profiles: Vec<Value> = all
        .into_iter()
        .take(PROFILES)
        .filter_map(|p| {
            let kind = crate::profiles::KINDS.iter().find(|k| p["kind"] == k.0);
            let (Some((kind, label, aliases, ops)), Some(id), Some(rev)) = (kind, id16(&p["id"]), id16(&p["revision"])) else {
                dropped = true;
                return None;
            };
            let mut out = json!({"id": id, "kind": kind, "label": label, "aliases": aliases, "revision": rev, "operations": ops});
            let mut unavailable = Map::new();
            for (f, allowed) in FIELDS {
                if ops.contains(&f) {
                    out[f] = json!(pick(p[f].as_str(), allowed));
                } else {
                    out[f] = json!("unsupported");
                    unavailable.insert(f.into(), json!("unsupported_client"));
                }
            }
            out["unavailable"] = unavailable.into();
            let state = pick(p["effective"]["state"].as_str(), &EFFECTIVE);
            let adapter = p["effective"]["adapter"].as_str().filter(|a| ADAPTERS.contains(a));
            out["effective"] = json!({"adapter": adapter, "state": state});
            if let Some(event) = p["effective"]["event"].as_str().filter(|e| ["PreToolUse", "PostToolUse"].contains(e)) {
                out["effective"]["event"] = event.into();
            }
            if p["optimization_preference"].is_string() {
                out["optimization_preference"] = pick(p["optimization_preference"].as_str(), &["enabled", "disabled", "unknown"]).into();
                out["optimization_reason"] = pick(p["optimization_reason"].as_str(), &["unreadable_control", "integration_disabled", "host_pause", "agent_pause", "direct", "adapter_missing", "configured", "unsupported", "unknown"]).into();
            }
            if p["integration"].is_object() {
                let i = &p["integration"];
                out["integration"] = json!({
                    "desired": pick(i["desired"].as_str(), &["enabled", "disabled", "unknown"]),
                    "configuration": pick(i["configuration"].as_str(), &["attached", "detached", "unknown"]),
                    "runtime": if i["runtime_at"].as_u64().is_some_and(|at| at <= now && now-at <= STALE) { pick(i["runtime"].as_str(), &["pending_reload", "pending_drain", "drained", "unknown"]) } else { "unknown" },
                    "phase": pick(i["phase"].as_str(), &["detaching", "detached", "enabling", "conflict"]),
                    "at": i["at"].as_u64(), "runtime_at": i["runtime_at"].as_u64(), "generation": i["generation"].as_u64(),
                });
            }
            out["configured"] = json!(state == "configured");
            out["verification"] = verification(&p["verification"], adapter, rev);
            out["owned"] = json!(p["owned"].as_bool());
            // A1: project overrides are read-only; any of them disables writes, an unknown count stays null
            let overrides = p["overrides"].as_u64().map(|n| n.min(999));
            out["overrides"] = json!(overrides);
            let (conflicts, incomplete) = (codes(&p["conflicts"], &CONFLICTS, "other"), codes(&p["incomplete"], &INCOMPLETE, "other"));
            let mut writes = vec!["not_released"];
            for (bad, code) in [(!fresh, "stale"), (truncated, "truncated"), (!incomplete.is_empty(), "incomplete"), (!conflicts.is_empty(), "conflict"), (overrides.is_some_and(|n| n > 0), "project_override")] {
                if bad {
                    writes.push(code);
                }
            }
            if p["integration"]["desired"] == "disabled" { writes.push("integration_disabled"); }
            out["conflicts"] = conflicts.into();
            out["incomplete"] = incomplete.into();
            out["writes"] = json!({"enabled": false, "reasons": writes});
            Some(out)
        })
        .collect();
    let d = &c["defaults"];
    json!({
        "schema": crate::profiles::SCHEMA,
        "at": at,
        "state": if fresh { "fresh" } else { "stale" },
        "revision": id16(&c["revision"]),
        "defaults": {"enabled": d["enabled"].as_bool(), "min_bytes": d["min_bytes"].as_u64(), "stats": d["stats"].as_str().filter(|s| ["queue", "off"].contains(s))},
        "truncated": truncated,
        "incomplete": dropped,
        "profiles": profiles,
    })
}

/// The `agentq.resource.*` allowlist of one OTLP span, with its source timestamp and event ID.
fn project_resource(span: &Value) -> Value {
    let attrs: Map<String, Value> = span["attributes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let v = a["value"].as_object()?.values().next()?.clone();
            Some((a["key"].as_str()?.strip_prefix("agentq.resource.")?.to_string(), v))
        })
        .collect();
    let mut out = Map::new();
    out.insert("event_id".into(), json!(span["traceId"].as_str()));
    // OTLP int64 travels as a decimal string
    out.insert("at_ns".into(), json!(span["startTimeUnixNano"].as_str().filter(|t| t.bytes().all(|b| b.is_ascii_digit()))));
    for k in NUMBERS {
        let v = &attrs.get(k).cloned().unwrap_or(Value::Null);
        let n = v.as_str().and_then(|s| s.parse::<f64>().ok()).map(Value::from).or_else(|| v.is_number().then(|| v.clone()));
        if let Some(n) = n {
            out.insert(k.into(), n);
        }
    }
    for (probe, has) in [("cpu", "cpu_percent"), ("memory", "memory_total_bytes"), ("disk", "disks")] {
        match attrs.get(&format!("{probe}_unknown")) {
            Some(r) => drop(out.insert(format!("{probe}_unknown"), reason(r))),
            None if !attrs.contains_key(has) => drop(out.insert(format!("{probe}_unknown"), json!("missing"))),
            None => {}
        }
    }
    if let Some(d) = attrs.get("disks").and_then(Value::as_str).and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok()) {
        let disks: Vec<Value> = d
            .iter()
            .map(|d| {
                let mut x = json!({"id": d["id"].as_str(), "role": pick(d["role"].as_str(), &["root", "other"]), "fstype": d["fstype"].as_str().filter(|f| f.len() <= 16 && f.bytes().all(|b| b.is_ascii_alphanumeric()))});
                match (d["total_bytes"].as_u64(), d["free_bytes"].as_u64(), d["used_bytes"].as_u64()) {
                    (Some(t), Some(f), Some(u)) => {
                        x["total_bytes"] = json!(t);
                        x["free_bytes"] = json!(f);
                        x["used_bytes"] = json!(u);
                    }
                    _ => x["unknown"] = if d["unknown"].is_null() { json!("missing") } else { reason(&d["unknown"]) },
                }
                x
            })
            .collect();
        out.insert("disks".into(), json!(disks));
    }
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::tests::{dir, reading, sampler};
    use std::time::{Duration, Instant};

    fn status() -> Value {
        json!({"host": "alice-mbp", "version": "1.2.3", "pid": 42, "at": 1000, "project": "host", "error": "update: GET /Users/alice/x: 404", "update": null,
            "queue": {"pending": 3, "oldest_age": 60, "last_delivery": 990, "dropped": 0, "error": "curl https://lf.example: secret sk-lf-1"},
            "agents": {"codex": {"routing": "routed", "reason": "127.0.0.1:4100"}, "claude": {"routing": "weird", "reason": "/Users/alice/.claude"}, "/tmp/x": {"routing": "routed"}},
            "engines": {"rtk": {"state": "ready", "version": "0.51.0"}, "headroom": {"state": "unavailable", "version": "0.40.0"}}})
    }

    /// A span from the real sampler and span builder on the existing resource fixtures.
    fn sample(cpu: Result<(u64, u64), String>, prev: Result<(u64, u64), String>, after: Duration) -> Value {
        let d = dir("snapshot");
        let t0 = Instant::now();
        let a = sampler(&d, prev, t0).sample(reading(cpu), t0 + after, 1_000_000_000_000 + after.as_nanos());
        let _ = std::fs::remove_dir_all(d);
        crate::telemetry::span("agentq.resource", "sample", Value::Object(a), 1_300_000_000_000, "inst", "alice-mbp")
    }

    /// A1, A2: identity, version, health, queue and source timestamp; no paths, hosts, prompts or secret text.
    #[test]
    fn allowlisted_projection() {
        let r = sample(Ok((350, 2000)), Ok((100, 1000)), Duration::from_secs(300));
        let p = project("inst", "macos", Some(&status()), Some(&r), 1030);
        assert_eq!((p["schema"].clone(), p["installation"].clone(), p["runtime"].clone(), p["version"].clone(), p["status_at"].clone()), (json!(1), json!("inst"), json!("macos"), json!("1.2.3"), json!(1000)));
        assert_eq!(p["service"], json!({"state": "running"}));
        assert_eq!(p["routing"], json!({"codex": "routed", "claude": "unknown"}), "enumerated values and known agents only");
        assert_eq!(p["engines"]["headroom"], json!({"state": "unavailable", "version": "0.40.0"}));
        assert_eq!(p["queue"], json!({"pending": 3, "oldest_age": 60, "last_delivery": 990, "dropped": 0, "send_error": true}));
        assert_eq!(p["update"], json!({"available": null, "error": true}));
        let res = &p["resource"];
        assert_eq!((res["at_ns"].clone(), res["cpu_percent"].clone(), res["memory_used_bytes"].clone()), (json!("1300000000000"), json!(25.0), json!(5u64 << 30)));
        assert_eq!(res["event_id"], r["traceId"]);
        assert_eq!(res["disks"][0]["total_bytes"], json!(100));
        assert_eq!(res["disks"][1]["unknown"], "probe failed", "a stat error names no probe path");
        let text = p.to_string();
        for leak in ["alice", "/Users", "/tmp", "/proc", "127.0.0.1", "lf.example", "sk-lf", "pid", "host"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }

    fn with_profiles(at: u64, profiles: Value) -> Value {
        let mut s = status();
        s["configuration"] = json!({"schema": 1, "at": at, "revision": "00112233445566ff", "defaults": {"enabled": true, "min_bytes": 2048, "stats": "queue"}, "profiles": profiles});
        s
    }

    fn codex(extra: Value) -> Value {
        let mut p = json!({"kind": "codex", "id": "0123456789abcdef", "revision": "fedcba9876543210", "routing": "routed", "mcp": "registered", "hooks": "registered", "plugin": "unsupported", "optimization": "enabled",
            "effective": {"adapter": "provider", "state": "configured"}, "owned": true, "conflicts": [], "incomplete": []});
        for (k, v) in extra.as_object().into_iter().flatten() {
            p[k] = v.clone();
        }
        p
    }

    /// #101 A1, A5: an older producer's status keeps every original field and reports configuration unknown; a
    /// future schema is unknown too.
    #[test]
    fn old_producer_configuration_unknown() {
        let old = project("inst", "macos", Some(&status()), None, 1030);
        assert_eq!(old["configuration"], json!({"state": "unknown", "reason": "older producer"}));
        let new = project("inst", "macos", Some(&with_profiles(1000, json!([codex(json!({}))]))), None, 1030);
        for k in ["schema", "installation", "runtime", "version", "status_at", "service", "update", "routing", "engines", "queue", "resource"] {
            assert_eq!(old[k], new[k], "{k} unchanged");
        }
        let mut future = with_profiles(1000, json!([]));
        future["configuration"]["schema"] = json!(2);
        assert_eq!(project("inst", "macos", Some(&future), None, 1030)["configuration"]["reason"], "unsupported schema");
        assert_eq!(project("inst", "macos", None, None, 1030)["configuration"]["reason"], "no status");
    }

    /// #101 A2–A5: ids, aliases, capabilities and effective state cross; configured is never verified; stale,
    /// truncated, incomplete or conflicting profiles disable writes; no path, credential, prompt or free text crosses.
    #[test]
    fn profile_projection() {
        let profiles = json!([
            codex(json!({})),
            {"kind": "claude-desktop-chat", "id": "1111111111111111", "revision": "2222222222222222", "routing": "routed", "mcp": "registered", "hooks": "registered", "effective": {"adapter": "/bin/sh", "state": "unsupported"}},
            codex(json!({"id": "3333333333333333", "conflicts": ["foreign_provider", "model sk-secret at /Users/alice/.codex"], "incomplete": ["unreadable_config"], "routing": "routed via /Users/alice", "label": "alice's prompt"})),
            codex(json!({"id": "/Users/alice/.codex"})),
            {"kind": "/tmp/evil", "id": "4444444444444444", "revision": "5555555555555555"},
        ]);
        let c = project("inst", "macos", Some(&with_profiles(1000, profiles)), None, 1030)["configuration"].clone();
        assert_eq!((c["state"].as_str(), c["revision"].as_str(), c["incomplete"].as_bool(), c["truncated"].as_bool()), (Some("fresh"), Some("00112233445566ff"), Some(true), Some(false)));
        let p = c["profiles"].as_array().unwrap();
        assert_eq!(p.len(), 3, "an invalid id or kind does not cross");
        assert_eq!(p[0]["aliases"], json!(["codex-cli", "codex-desktop"]), "one target, every alias shown");
        assert_eq!((p[0]["routing"].as_str(), p[0]["effective"]["state"].as_str(), p[0]["configured"].as_bool()), (Some("routed"), Some("configured"), Some(true)));
        assert_eq!(p[0]["verification"], json!({"state": "unknown", "reason": "no_evidence"}), "configured never implies verified");
        assert_eq!(p[0]["writes"], json!({"enabled": false, "reasons": ["not_released"]}));
        assert_eq!((p[1]["routing"].as_str(), p[1]["hooks"].as_str(), p[1]["mcp"].as_str(), p[1]["operations"].clone()), (Some("unsupported"), Some("unsupported"), Some("registered"), json!(["mcp"])));
        assert_eq!(p[1]["unavailable"]["routing"], "unsupported_client", "unsupported is explicit, not off");
        assert!(p[1]["effective"]["adapter"].is_null());
        assert_eq!((p[2]["conflicts"].clone(), p[2]["incomplete"].clone(), p[2]["routing"].as_str()), (json!(["foreign_provider", "other"]), json!(["unreadable_config"]), Some("unknown")));
        assert_eq!(p[2]["writes"]["reasons"], json!(["not_released", "incomplete", "conflict"]));
        assert_eq!(p[2]["label"], "Codex", "labels come from the kind");
        assert!(p[0]["overrides"].is_null() && p[1]["overrides"].is_null(), "an unknown override count stays null");

        let o = project("inst", "macos", Some(&with_profiles(1000, json!([codex(json!({"overrides": 3})), codex(json!({"overrides": 0})), codex(json!({"overrides": "/Users/alice"}))]))), None, 1030)["configuration"].clone();
        assert_eq!((o["profiles"][0]["overrides"].as_u64(), o["profiles"][0]["writes"]["reasons"].clone()), (Some(3), json!(["not_released", "project_override"])));
        assert_eq!((o["profiles"][1]["writes"]["reasons"].clone(), o["profiles"][2]["overrides"].is_null()), (json!(["not_released"]), true));

        let stale = project("inst", "macos", Some(&with_profiles(1000, json!([codex(json!({}))]))), None, 2000)["configuration"].clone();
        assert_eq!((stale["state"].as_str(), stale["profiles"][0]["writes"]["reasons"].clone()), (Some("stale"), json!(["not_released", "stale"])));
        let future = project("inst", "macos", Some(&with_profiles(1100, json!([codex(json!({}))]))), None, 1030)["configuration"].clone();
        assert_eq!(future["state"], "stale", "a future observation time is clock-uncertain");

        // #103 A5: verified crosses only for the profile's own adapter and revision with an evidence ID
        let ok = json!({"state": "verified", "at": 1010, "adapter": "provider", "revision": "fedcba9876543210", "evidence": "abcdefabcdefabcd"});
        let other = json!({"state": "verified", "at": 1010, "adapter": "hook", "revision": "fedcba9876543210", "evidence": "abcdefabcdefabcd"});
        let v = project("inst", "macos", Some(&with_profiles(1000, json!([codex(json!({"verification": ok})), codex(json!({"verification": other})), codex(json!({"verification": {"state": "unknown", "reason": "at /Users/alice"}}))]))), None, 1030)["configuration"].clone();
        assert_eq!(v["profiles"][0]["verification"], ok);
        assert_eq!((v["profiles"][1]["verification"]["state"].as_str(), v["profiles"][2]["verification"]["reason"].as_str()), (Some("unknown"), Some("no_evidence")));

        let many: Vec<Value> = (0..20).map(|_| codex(json!({}))).collect();
        let t = project("inst", "macos", Some(&with_profiles(1000, json!(many))), None, 1030)["configuration"].clone();
        assert_eq!((t["truncated"].as_bool(), t["profiles"].as_array().map(Vec::len)), (Some(true), Some(16)));
        assert!(t["profiles"][0]["writes"]["reasons"].as_array().unwrap().contains(&json!("truncated")));

        let text = c.to_string();
        for leak in ["alice", "/Users", "/tmp", "/bin", "sk-secret", "prompt", "model "] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
    }

    /// #141 A2: selected/running differ without implying health; stale and future observations cannot be healthy.
    #[test]
    fn release_observation() {
        let mut s = status();
        s["release"] = json!({"at": 1000, "selected": "2.0.0", "running": "1.0.0", "coreq_version": "0.11.0",
            "lifecycle": "waiting_for_activation", "reason": "selection_changed",
            "health": {"state": "verified", "scope": "status_written", "version": "1.0.0", "at": 990}});
        let p = project("inst", "macos", Some(&s), None, 1030);
        assert_eq!(p["release"]["selected"], "2.0.0");
        assert_eq!(p["release"]["lifecycle"], "waiting_for_activation");
        assert_eq!(p["release"]["health"]["version"], "1.0.0");
        for now in [999, 1091] {
            let p = project("inst", "macos", Some(&s), None, now);
            assert_eq!(p["release"]["health"]["state"], "unknown");
            assert_eq!(p["release"]["lifecycle"], "unknown");
            assert_eq!(p["release"]["selected"], "2.0.0");
        }
        s["release"]["selected"] = json!("/private/secret");
        s["release"]["reason"] = json!("secret");
        let p = project("inst", "macos", Some(&s), None, 1030);
        assert!(!p["release"].to_string().contains("secret"));
        assert_eq!(project("inst", "macos", None, None, 1030)["release"]["reason"], "not_reported");
    }

    /// A2, A3: missing status and fields, a stale status, counter reset, warmup and an unsupported probe stay explicit.
    #[test]
    fn unknown_stays_explicit() {
        let p = project("inst", "linux", None, None, 1000);
        assert_eq!(p["service"], json!({"state": "unknown", "reason": "no status"}));
        assert_eq!(p["resource"], json!({"unknown": "no sample yet"}));
        assert_eq!(p["engines"]["rtk"], json!({"state": "unknown"}));
        assert!(p["queue"]["pending"].is_null() && p["version"].is_null(), "missing is null, never zero");
        assert_eq!(project("inst", "linux", Some(&status()), None, 2000)["service"]["reason"], "stale status");
        assert_eq!(project("inst", "linux", Some(&json!({})), None, 2000)["service"]["reason"], "status without time");

        let reset = project("inst", "linux", None, Some(&sample(Ok((10, 3100)), Ok((400, 3000)), Duration::from_secs(300))), 0);
        assert_eq!(reset["resource"]["cpu_unknown"], "counter reset");
        assert!(reset["resource"]["cpu_percent"].is_null());
        let warm = project("inst", "linux", None, Some(&sample(Ok((50, 200)), Ok((0, 100)), Duration::from_secs(5))), 0);
        assert_eq!(warm["resource"]["cpu_unknown"], "warmup");
        let no = project("inst", "linux", None, Some(&sample(Err("no sampler on this OS".into()), Ok((0, 1)), Duration::from_secs(300))), 0);
        assert_eq!(no["resource"]["cpu_unknown"], "no sampler on this OS");

        let bare = json!({"traceId": "ab", "startTimeUnixNano": "5", "attributes": [{"key": "agentq.resource.cpu_unknown", "value": {"stringValue": "/proc/stat: permission denied"}}, {"key": "agentq.resource.secret", "value": {"stringValue": "x"}}]});
        let res = project("inst", "linux", None, Some(&bare), 0)["resource"].clone();
        assert_eq!(res, json!({"event_id": "ab", "at_ns": "5", "cpu_unknown": "probe failed", "memory_unknown": "missing", "disk_unknown": "missing"}));
    }
}
