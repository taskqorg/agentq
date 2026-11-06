//! Configuration operations (docs/spec/CONFIGURATION.md A3, A4, "Operation envelopes"): strict request, plan and
//! result envelopes and host acceptance. Pure: the caller passes the raw request, the host's grant/storage state and
//! `profiles::status`. Shared conformance fixtures: docs/spec/configuration-fixtures.json.
//!
//! The executor (durable records, journaled apply, readback) is `executor.rs`.

use ring::digest;
use serde_json::{Value, json};

pub const SCHEMA: u64 = 1;
pub const CAPABILITY: &str = "agentq.configure";
pub const MAX_BYTES: usize = 16 * 1024;
/// Seconds an operation may wait before its execution begins (A4).
pub const EXPIRY: u64 = 300;
/// Changeable field, allowed new values, whether the client must reload to see it (A3).
const FIELDS: [(&str, &[&str], bool); 5] = [
    ("routing", &["routed", "direct"], true),
    ("optimization", &["enabled", "disabled"], false),
    ("mcp", &["registered"], true),
    ("hooks", &["registered"], true),
    ("plugin", &["enabled"], true),
];
/// Kind, field and value the client supports but the host cannot write yet (CONFIGURATION "Host executor").
const NO_WRITER: [(&str, &str, &str); 1] = [("claude-code", "routing", "routed")];
const REQUEST: [&str; 10] = ["schema", "operation", "kind", "actor", "workspace", "installation", "generation", "issued_at", "expires_at", "changes"];
pub const OUTCOMES: [&str; 5] = ["rejected", "accepted", "applied", "partial", "unknown"];
const PROFILE_STATES: [&str; 4] = ["applied", "unchanged", "restored", "unknown"];

fn hex(v: &Value, n: usize) -> Option<&str> {
    v.as_str().filter(|s| s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// Actor, workspace and installation IDs: opaque tokens, never paths.
fn token(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) && !s.starts_with('.'))
}

/// Object keys sorted at every level, so a retransmission digests the same whatever its key order.
fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), canonical(&m[k]))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => v.clone(),
    }
}

pub fn digest(v: &Value) -> String {
    crate::resource::hex(digest::digest(&digest::SHA256, canonical(v).to_string().as_bytes()).as_ref())
}

pub fn result(operation: Option<&str>, outcome: &str, reason: Option<&str>, plan: Option<&str>) -> Value {
    json!({"schema": SCHEMA, "kind": "result", "operation": operation, "outcome": outcome, "reason": reason, "plan": plan, "profiles": []})
}

/// A result envelope with only known keys, outcomes and per-profile states; `rejected` and `unknown` carry a reason.
pub fn valid_result(r: &Value) -> bool {
    let keys = ["schema", "kind", "operation", "outcome", "reason", "plan", "profiles"];
    let o = r["outcome"].as_str().unwrap_or("");
    r.as_object().is_some_and(|m| m.keys().all(|k| keys.contains(&k.as_str())))
        && r["schema"] == SCHEMA
        && r["kind"] == "result"
        && (r["operation"].is_null() || hex(&r["operation"], 32).is_some())
        && OUTCOMES.contains(&o)
        && (r["reason"].is_null() || token(&r["reason"]).is_some())
        && (!["rejected", "unknown"].contains(&o) || r["reason"].is_string())
        && (r["plan"].is_null() || hex(&r["plan"], 64).is_some())
        && r["profiles"].as_array().is_some_and(|a| a.iter().all(|p| p.as_object().is_some_and(|m| m.len() == 2) && hex(&p["id"], 16).is_some() && PROFILE_STATES.contains(&p["state"].as_str().unwrap_or(""))))
}

/// A request with only known keys, fixed enums and opaque IDs; else the rejection code.
fn parse(v: &Value) -> Result<(), &'static str> {
    let m = v.as_object().ok_or("invalid")?;
    let apply = v["kind"] == "apply";
    if m.keys().any(|k| !REQUEST.contains(&k.as_str()) && !(apply && k == "plan")) {
        return Err("unknown_field");
    }
    let ids = [&v["actor"], &v["workspace"], &v["installation"]].iter().all(|x| token(x).is_some());
    let times = [&v["generation"], &v["issued_at"], &v["expires_at"]].iter().all(|x| x.is_u64());
    if v["schema"] != SCHEMA || !(apply || v["kind"] == "plan") || hex(&v["operation"], 32).is_none() || !ids || !times {
        return Err("invalid");
    }
    if apply {
        let p = &v["plan"];
        if p.as_object().is_none_or(|m| m.len() != 2) || hex(&p["digest"], 64).is_none() || hex(&p["revision"], 16).is_none() {
            return Err("invalid");
        }
    }
    let changes = v["changes"].as_array().filter(|a| !a.is_empty() && a.len() <= 64).ok_or("invalid")?;
    let mut seen = vec![];
    for c in changes {
        let m = c.as_object().ok_or("invalid")?;
        if m.keys().any(|k| !["profile", "field", "value"].contains(&k.as_str())) {
            return Err("unknown_field");
        }
        let id = hex(&c["profile"], 16).ok_or("invalid")?;
        let (field, values, _) = FIELDS.iter().find(|f| c["field"] == f.0).ok_or("unsupported_field")?;
        if !values.iter().any(|x| c["value"] == *x) {
            return Err("unsupported_value");
        }
        if seen.contains(&(id, *field)) {
            return Err("invalid");
        }
        seen.push((id, field));
    }
    Ok(())
}

/// The plan of `req`'s changes against the host's current `profiles::status`. The digest covers everything the
/// browser review confirms (target, revisions, old/new values, aliases, conflicts, reload), not the operation or time.
pub fn plan(req: &Value, configuration: &Value) -> Value {
    let mut profiles: Vec<Value> = vec![];
    for c in req["changes"].as_array().into_iter().flatten() {
        let id = &c["profile"];
        if !profiles.iter().any(|p| p["id"] == *id) {
            let p = configuration["profiles"].as_array().into_iter().flatten().find(|p| p["id"] == *id);
            let kind = p.and_then(|p| crate::profiles::KINDS.iter().find(|k| p["kind"] == k.0));
            let mut conflicts: Vec<Value> = vec![];
            if let (Some(p), Some(_)) = (p, kind) {
                conflicts.extend(p["conflicts"].as_array().into_iter().flatten().chain(p["incomplete"].as_array().into_iter().flatten()).cloned());
                if p["integration"]["desired"] == "disabled" { conflicts.push("integration_disabled".into()); }
            } else {
                conflicts.push("unknown_profile".into());
            }
            profiles.push(json!({"id": id, "kind": kind.map(|k| k.0), "aliases": kind.map(|k| k.2), "revision": p.map(|p| &p["revision"]), "changes": [], "conflicts": conflicts, "reload": false}));
        }
        let entry = profiles.iter_mut().find(|p| p["id"] == *id).expect("pushed above");
        let field = c["field"].as_str().unwrap_or("");
        let target = configuration["profiles"].as_array().into_iter().flatten().find(|p| p["id"] == *id);
        if field != "optimization" && target.is_some_and(|p| p["overrides"].as_u64().is_some_and(|n| n > 0))
            && !entry["conflicts"].as_array().is_some_and(|a| a.contains(&json!("project_override")))
        {
            entry["conflicts"].as_array_mut().expect("array").push("project_override".into());
        }
        let ops = crate::profiles::KINDS.iter().find(|k| entry["kind"] == k.0).map_or(&[][..], |k| k.3);
        let no_writer = NO_WRITER.iter().any(|w| entry["kind"] == w.0 && field == w.1 && c["value"] == w.2);
        if entry["kind"].is_string() && (!ops.contains(&field) || no_writer) && !entry["conflicts"].as_array().is_some_and(|a| a.contains(&json!("unsupported_client"))) {
            entry["conflicts"].as_array_mut().expect("array").push("unsupported_client".into());
        }
        let old = configuration["profiles"].as_array().into_iter().flatten().find(|p| p["id"] == *id).map_or(Value::Null, |p| p[field].clone());
        let reload = FIELDS.iter().any(|f| f.0 == field && f.2) && old != c["value"];
        entry["changes"].as_array_mut().expect("array").push(json!({"field": field, "old": old, "new": c["value"]}));
        entry["reload"] = json!(entry["reload"] == true || reload);
    }
    let mut blocked: Vec<Value> = vec![];
    for c in profiles.iter().flat_map(|p| p["conflicts"].as_array().into_iter().flatten()) {
        if !blocked.contains(c) {
            blocked.push(c.clone());
        }
    }
    let mut out = json!({"schema": SCHEMA, "kind": "plan", "installation": req["installation"], "generation": req["generation"], "revision": configuration["revision"], "profiles": profiles, "blocked": blocked});
    let d = digest(&out);
    out["writable"] = json!(blocked.is_empty());
    out["digest"] = json!(d);
    out["operation"] = req["operation"].clone();
    out["expires_at"] = req["expires_at"].clone();
    out
}

/// One delivered request: a plan, an `accepted` apply (the caller then stores it durably before any write, #103) or a
/// result. `host`: `installation`, `generation`, `now`, `durable` (control storage writable), `grant` (null when
/// unenrolled or revoked), `outstanding` (unresolved mutation's operation or null), `seen` (this operation's stored
/// `digest` and `result`, or null).
pub fn handle(raw: &str, host: &Value, configuration: &Value) -> Value {
    if raw.len() > MAX_BYTES {
        return result(None, "rejected", Some("payload_too_large"), None);
    }
    let Ok(req) = serde_json::from_str::<Value>(raw) else {
        return result(None, "rejected", Some("invalid"), None);
    };
    let op = hex(&req["operation"], 32);
    let reject = |reason| result(op, "rejected", Some(reason), None);
    if let Err(reason) = parse(&req) {
        return reject(reason);
    }
    // an operation ID is immutable: a repeat returns the first result, a changed payload never runs
    if let Some(seen) = host["seen"].as_object() {
        return if seen.get("digest") == Some(&json!(digest(&req))) { seen["result"].clone() } else { reject("payload_mismatch") };
    }
    let g = &host["grant"];
    let now = host["now"].as_u64().unwrap_or(u64::MAX);
    let (issued, expires) = (req["issued_at"].as_u64().unwrap_or(0), req["expires_at"].as_u64().unwrap_or(0));
    let in_scope = |c: &Value| g["profiles"].as_array().is_some_and(|a| a.contains(&c["profile"]));
    let checks = [
        (g.is_null(), "no_grant"),
        // an observation credential never grants or broadens configuration authority
        (g["capability"] != CAPABILITY, "observation_only"),
        (g["expires_at"].as_u64().is_none_or(|e| now >= e), "grant_expired"),
        (req["installation"] != host["installation"] || g["installation"] != host["installation"], "wrong_installation"),
        (req["generation"] != host["generation"] || g["generation"] != host["generation"], "stale_generation"),
        (req["workspace"] != g["workspace"], "wrong_workspace"),
        (req["actor"] != g["actor"], "wrong_actor"),
        (!req["changes"].as_array().into_iter().flatten().all(in_scope), "profile_out_of_scope"),
        (expires <= issued || expires - issued > EXPIRY, "invalid_expiry"),
        (now >= expires, "expired"),
    ];
    if let Some((_, reason)) = checks.iter().find(|c| c.0) {
        return reject(reason);
    }
    let p = plan(&req, configuration);
    if req["kind"] == "plan" {
        return p;
    }
    let busy = host["outstanding"].as_str().is_some_and(|o| Some(o) != op);
    let checks = [
        (host["durable"] != true, "storage_unavailable"),
        (busy, "busy"),
        (req["plan"]["revision"] != configuration["revision"], "stale_revision"),
        (p["writable"] != true, "blocked"),
        (req["plan"]["digest"] != p["digest"], "plan_changed"),
    ];
    match checks.iter().find(|c| c.0) {
        Some((_, reason)) => reject(reason),
        None => result(op, "accepted", None, p["digest"].as_str()),
    }
}

/// RFC 7386 merge patch: a fixture case states only what differs from the base.
fn patch(base: &mut Value, p: &Value) {
    if p.is_null() {
        return;
    }
    match (base.as_object_mut(), p.as_object()) {
        (Some(b), Some(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    b.remove(k);
                } else {
                    patch(b.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
        }
        _ => *base = p.clone(),
    }
}

/// Every key of `want` equals in `got`; objects compare recursively, other values exactly.
fn subset(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Object(w), _) => w.iter().all(|(k, v)| subset(v, &got[k])),
        (Value::Array(w), Value::Array(g)) => w.len() == g.len() && w.iter().zip(g).all(|(w, g)| subset(w, g)),
        _ => want == got,
    }
}

pub const FIXTURES: &str = include_str!("../docs/spec/configuration-fixtures.json");

/// Names of fixture cases and result examples that fail; empty when the host conforms.
pub fn conform(fixtures: &str) -> Vec<String> {
    let Ok(f) = serde_json::from_str::<Value>(fixtures) else { return vec!["unparseable fixtures".into()] };
    let mut bad = vec![];
    for c in f["cases"].as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("?").to_string();
        let mut req = f["requests"][c["request"].as_str().unwrap_or("")].clone();
        patch(&mut req, &c["patch"]);
        let mut host = f["host"].clone();
        patch(&mut host, &c["host"]);
        if let Some(s) = host["seen"].as_object_mut().filter(|s| s.get("digest") == Some(&json!("request"))) {
            s.insert("digest".into(), json!(digest(&req)));
        }
        let mut raw = if c["reorder"] == true { canonical(&req) } else { req.clone() }.to_string();
        raw.push_str(&" ".repeat(c["pad"].as_u64().unwrap_or(0) as usize));
        let mut configuration = f["configuration"].clone();
        patch(&mut configuration, &c["configuration"]);
        let got = handle(&raw, &host, &configuration);
        let shape = got["kind"] == "plan" || valid_result(&got);
        if !shape || !subset(&c["expect"], &got) {
            bad.push(format!("{name}: {got}"));
        }
    }
    for r in f["results"].as_array().into_iter().flatten() {
        if valid_result(&r["result"]) != (r["valid"] == true) {
            bad.push(format!("result {}", r["name"]));
        }
    }
    bad
}

pub fn selftest() -> bool {
    conform(FIXTURES).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #102 A1–A5: every shared fixture case and result example conforms.
    #[test]
    fn fixtures_conform() {
        let bad = conform(FIXTURES);
        assert!(bad.is_empty(), "{bad:#?}");
        let f: Value = serde_json::from_str(FIXTURES).unwrap();
        assert!(f["cases"].as_array().unwrap().len() >= 30);
    }

    /// #102 A2: the digest ignores key order, operation and expiry but changes with any reviewed value.
    #[test]
    fn plan_digest_covers_review() {
        let f: Value = serde_json::from_str(FIXTURES).unwrap();
        let (req, cfg) = (&f["requests"]["plan"], &f["configuration"]);
        let p = plan(req, cfg);
        let mut other = req.clone();
        other["operation"] = json!("ffffffffffffffffffffffffffffffff");
        other["expires_at"] = json!(1200);
        assert_eq!(plan(&other, cfg)["digest"], p["digest"]);
        let mut edited = cfg.clone();
        edited["profiles"][0]["hooks"] = json!("partial");
        assert_ne!(plan(req, &edited)["digest"], p["digest"], "an old value is reviewed");
        let shuffled: Value = serde_json::from_str(&canonical(req).to_string()).unwrap();
        assert_eq!(digest(&shuffled), digest(req));
        let text = p.to_string();
        assert!(!text.contains('/') && !text.contains("command"), "no path or command in {text}");
    }
}
