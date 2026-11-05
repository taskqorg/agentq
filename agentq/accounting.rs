//! Optimization results (docs/spec/ACCOUNTING.md O1-O3): §3 stats records or exported `agentq.compress` spans ->
//! one deduplicated, coverage-qualified aggregate. Pure; `agentq stats --accounting` and the consumer fixtures
//! `docs/spec/accounting-fixtures.json` share it.
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SCHEMA: &str = "agentq.accounting/1";
/// A record later than `now` by more than this has an uncertain time and never enters totals.
const SKEW: u64 = 300;
/// Records read per query unless the input sets `cap`.
const CAP: u64 = 100_000;
const EVIDENCE: usize = 3;
const RETENTION: u64 = 7 * 86400;
/// Keys that split cohorts: provenance and recipe. A null value is its own unknown cohort, never inferred.
const COHORT: [&str; 10] = [
    "installation",
    "runtime",
    "version",
    "profile",
    "agent",
    "adapter",
    "tool",
    "runner",
    "engine",
    "engine_version",
];
/// Copies of one event ID must agree on these, else the event is a conflict and counts nowhere.
const FINGERPRINT: [&str; 10] = [
    "time",
    "agent",
    "adapter",
    "tool",
    "runner",
    "engine",
    "outcome",
    "reason",
    "bytes_in",
    "bytes_out",
];

/// The one outcome a §3 reason belongs to; a normal pass-through is `passed`, never an error.
pub fn outcome_of(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "compressed" => "compressed",
        "disabled" | "short" | "code" | "json" | "not_qualified" | "not_smaller" => "passed",
        "no_engine" | "timeout" | "error" => "failed",
        _ => return None,
    })
}

/// A record as is, or an exported span as its record: `agentq.` attributes unprefixed. OTLP has no null, so the
/// producer's `"null"` string reads as null; int64 may arrive as a string. None: an event
/// that is no core optimization call.
fn event(v: &Value) -> Option<Value> {
    let Some(attrs) = v["attributes"].as_array() else {
        return Some(v.clone());
    };
    if v["name"] != "agentq.compress" {
        return None;
    }
    let mut r = Map::new();
    for a in attrs {
        let (Some(k), x) = (a["key"].as_str().and_then(|k| k.strip_prefix("agentq.")), &a["value"]) else {
            continue;
        };
        let x = if let Some(s) = x.get("stringValue") {
            if s == "null" { Value::Null } else { s.clone() }
        } else if let Some(i) = x.get("intValue") {
            i.as_str().and_then(|s| s.parse::<i64>().ok()).map_or_else(|| i.clone(), |n| json!(n))
        } else if let Some(y) = x.get("doubleValue").or_else(|| x.get("boolValue")) {
            y.clone()
        } else {
            continue;
        };
        r.insert(k.into(), x);
    }
    if r.get("time").is_none_or(Value::is_null) {
        if let Some(ns) = v["startTimeUnixNano"].as_str().and_then(|s| s.parse::<u128>().ok()) {
            r.insert("time".into(), json!((ns / 1_000_000_000) as u64));
        }
    }
    Some(Value::Object(r))
}

fn hex32(v: &Value) -> bool {
    v.as_str().is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Why a compress record cannot enter totals, if it cannot.
fn invalid(r: &Value, now: u64) -> Option<&'static str> {
    if r["schema"] != 1 {
        return Some(if r["schema"].is_u64() { "unsupported_schema" } else { "malformed" });
    }
    let t = r["time"].as_u64()?;
    if t > now.saturating_add(SKEW) {
        return Some("future_time");
    }
    let shape = hex32(&r["event_id"])
        && ["agent", "tool"].iter().all(|k| r[k].is_string())
        && r["adapter"].as_str().is_some_and(|a| ["run", "hook", "provider"].contains(&a))
        && r["engine"].as_str().is_some_and(|e| ["none", "rtk", "headroom"].contains(&e))
        && r["reason"].as_str().and_then(outcome_of).is_some_and(|o| r["outcome"] == o)
        && ["runner", "profile", "version", "engine_version"].iter().all(|k| r[k].is_null() || r[k].is_string());
    (!shape).then_some("malformed")
}

fn bump(m: &mut Value, k: &str) {
    m[k] = json!(m[k].as_u64().unwrap_or(0) + 1);
}

/// Covered and missing parts of `[start, end)` from a source's observed `[from, to)` intervals.
fn coverage(observed: &Value, start: u64, end: u64) -> Result<(Vec<[u64; 2]>, Vec<[u64; 2]>), String> {
    let mut iv = vec![];
    for o in observed.as_array().into_iter().flatten() {
        match (o[0].as_u64(), o[1].as_u64()) {
            (Some(a), Some(b)) if a < b && o.as_array().is_some_and(|x| x.len() == 2) => {
                if a.max(start) < b.min(end) {
                    iv.push([a.max(start), b.min(end)]);
                }
            }
            _ => return Err("observed requires [from, to) pairs".into()),
        }
    }
    iv.sort();
    let (mut covered, mut gaps, mut at) = (Vec::<[u64; 2]>::new(), vec![], start);
    for [a, b] in iv {
        if a > at {
            gaps.push([at, a]);
        }
        match covered.last_mut() {
            Some(l) if a <= l[1] => l[1] = l[1].max(b),
            _ => covered.push([a, b]),
        }
        at = at.max(b);
    }
    if at < end {
        gaps.push([at, end]);
    }
    Ok((covered, gaps))
}

/// ACCOUNTING O1-O3 over `{window: {start, end}, now, sources: [..], cap?, bucket?}`; the published schema and cases
/// are `docs/spec/accounting-fixtures.json`.
pub fn aggregate(input: &Value) -> Result<Value, String> {
    let (Some(start), Some(end), Some(now)) =
        (input["window"]["start"].as_u64(), input["window"]["end"].as_u64(), input["now"].as_u64())
    else {
        return Err("accounting requires window.start, window.end and now in unix seconds".into());
    };
    if start >= end {
        return Err("accounting window requires start < end".into());
    }
    let sources = input["sources"].as_array().ok_or("accounting requires sources")?;
    let cap = input["cap"].as_u64().unwrap_or(CAP);
    let (mut read, mut capped, mut duplicates, mut out_of_window) = (0, false, 0, 0);
    let mut excluded = json!({});
    let mut events = BTreeMap::<String, Value>::new();
    let mut conflicts = BTreeSet::new();
    let mut found = vec![0u64; sources.len()];
    for (i, s) in sources.iter().enumerate() {
        for v in s["records"].as_array().into_iter().flatten() {
            if read == cap {
                capped = true;
                break;
            }
            read += 1;
            let Some(mut r) = event(v) else { continue };
            if !r.is_object() {
                bump(&mut excluded, "malformed");
                continue;
            }
            for k in ["installation", "runtime"] {
                if r[k].is_null() {
                    r[k] = s[k].clone();
                }
            }
            if let Some(why) = invalid(&r, now).or(r["time"].as_u64().is_none().then_some("malformed")) {
                bump(&mut excluded, why);
                continue;
            }
            let t = r["time"].as_u64().unwrap_or(0);
            if t < start || t >= end {
                out_of_window += 1;
                continue;
            }
            found[i] += 1;
            let id = r["event_id"].as_str().unwrap_or_default().to_string();
            match events.get_mut(&id) {
                None => {
                    events.insert(id, r);
                }
                // Known provenance must agree too; a copy may only fill what another left unknown.
                Some(p)
                    if FINGERPRINT.iter().all(|k| p[k] == r[k])
                        && COHORT.iter().all(|k| p[k].is_null() || r[k].is_null() || p[k] == r[k]) =>
                {
                    duplicates += 1;
                    for k in COHORT {
                        if p[k].is_null() {
                            p[k] = r[k].clone();
                        }
                    }
                }
                Some(_) => {
                    conflicts.insert(id);
                }
            }
        }
    }
    for id in &conflicts {
        events.remove(id);
    }
    if !conflicts.is_empty() {
        excluded["conflicting_duplicate"] = json!(conflicts.len());
    }

    let mut covered_all = true;
    let mut out_sources = vec![];
    for (i, s) in sources.iter().enumerate() {
        let (covered, gaps) = coverage(&s["observed"], start, end)?;
        // Corrupt coverage metadata proves nothing; never read it as zero or false.
        let dropped = match &s["dropped"] {
            Value::Null => 0,
            d => d.as_u64().ok_or("dropped requires a count")?,
        };
        let source_capped = match &s["capped"] {
            Value::Null => false,
            c => c.as_bool().ok_or("capped requires a boolean")?,
        };
        // A count, or the producer's reset times of which those in the window count.
        let resets = match &s["resets"] {
            Value::Null => 0,
            Value::Array(a) => {
                let t: Option<Vec<u64>> = a.iter().map(Value::as_u64).collect();
                t.ok_or("resets requires unix times")?.into_iter().filter(|t| (start..end).contains(t)).count() as u64
            }
            n => n.as_u64().ok_or("resets requires a count or unix times")?,
        };
        if !s["unknown"].is_null() && !s["unknown"].is_string() {
            return Err("unknown requires a reason string".into());
        }
        let mut why = vec![];
        if let Some(u) = s["unknown"].as_str() {
            // Observed intervals under an unknown reason are what was seen, not proof of absence.
            why.push(json!({"reason": if covered.is_empty() { "no_observation" } else { "unproven" }, "detail": u}));
        } else if covered.is_empty() {
            why.push(json!({"reason": "no_observation", "detail": "source states no observed interval"}));
        }
        for (hit, reason) in [(!gaps.is_empty() && !covered.is_empty(), "gap"), (resets > 0, "reset"), (dropped > 0, "dropped"), (source_capped, "capped")] {
            if hit {
                why.push(json!({"reason": reason}));
            }
        }
        let state = if covered.is_empty() && found[i] == 0 {
            "unknown"
        } else if gaps.is_empty() && dropped == 0 && !source_capped && s["unknown"].is_null() {
            "complete"
        } else {
            "partial"
        };
        covered_all &= state == "complete";
        out_sources.push(json!({"installation": s["installation"], "runtime": s["runtime"], "state": state,
            "observed": covered, "gaps": gaps, "dropped": dropped, "resets": resets, "records": found[i], "reasons": why}));
    }
    let state = if out_sources.iter().all(|s| s["state"] == "unknown") && events.is_empty() {
        "unknown"
    } else if covered_all && excluded.as_object().is_some_and(Map::is_empty) && !capped {
        "complete"
    } else {
        "partial"
    };

    let mut outcomes = json!({"compressed": 0, "passed": 0, "failed": 0});
    let mut reasons = json!({});
    let mut cohorts = BTreeMap::<String, Value>::new();
    let mut volume = BTreeMap::<(String, String), Value>::new();
    let mut profile_unknown = 0;
    for (id, r) in &events {
        let (outcome, reason) = (r["outcome"].as_str().unwrap_or_default(), r["reason"].as_str().unwrap_or_default());
        bump(&mut outcomes, outcome);
        bump(&mut reasons, reason);
        profile_unknown += u64::from(r["profile"].is_null());
        let key: Vec<Value> = COHORT.iter().map(|k| r[*k].clone()).collect();
        let c = cohorts.entry(Value::from(key).to_string()).or_insert_with(|| {
            let mut c = json!({"calls": 0, "outcomes": {"compressed": 0, "passed": 0, "failed": 0}, "reasons": {}, "evidence": []});
            for k in COHORT {
                c[k] = r[k].clone();
            }
            c
        });
        bump(c, "calls");
        bump(&mut c["outcomes"], outcome);
        bump(&mut c["reasons"], reason);
        if let Some(e) = c["evidence"].as_array_mut().filter(|e| e.len() < EVIDENCE) {
            e.push(json!(id));
        }
        if outcome != "compressed" {
            continue;
        }
        let (adapter, engine) = (r["adapter"].as_str().unwrap_or_default(), r["engine"].as_str().unwrap_or_default());
        let v = volume.entry((adapter.into(), engine.into())).or_insert_with(|| {
            json!({"adapter": adapter, "engine": engine, "unit": "byte", "compressed": 0, "paired": 0, "bytes_in": 0, "bytes_out": 0,
                "excluded": {}, "estimated_tokens": {"kind": "estimate", "method": "ceil(bytes/4)", "paired": 0, "tokens_in": 0, "tokens_out": 0}})
        });
        bump(v, "compressed");
        match (r["bytes_in"].as_u64(), r["bytes_out"].as_u64()) {
            (Some(a), Some(b)) => {
                bump(v, "paired");
                v["bytes_in"] = json!(v["bytes_in"].as_u64().unwrap_or(0).saturating_add(a));
                v["bytes_out"] = json!(v["bytes_out"].as_u64().unwrap_or(0).saturating_add(b));
                if let (Some(a), Some(b)) = (r["est_tokens_in"].as_u64(), r["est_tokens_out"].as_u64()) {
                    let e = &mut v["estimated_tokens"];
                    bump(e, "paired");
                    e["tokens_in"] = json!(e["tokens_in"].as_u64().unwrap_or(0).saturating_add(a));
                    e["tokens_out"] = json!(e["tokens_out"].as_u64().unwrap_or(0).saturating_add(b));
                }
            }
            _ if r["bytes_in"].is_null() || r["bytes_out"].is_null() => bump(&mut v["excluded"], "unmeasured"),
            _ => bump(&mut v["excluded"], "invalid"),
        }
    }
    // Sum pairs first, then one ratio: no averaged percentages and no extrapolated missing pairs.
    for v in volume.values_mut() {
        let (a, b) = (v["bytes_in"].as_u64().unwrap_or(0), v["bytes_out"].as_u64().unwrap_or(0));
        v["reduction"] = if a > 0 { json!(((1.0 - b as f64 / a as f64) * 10000.0).round() / 10000.0) } else { Value::Null };
    }

    let known = state != "unknown";
    let mut out = json!({"schema": SCHEMA, "window": {"start": start, "end": end}, "now": now, "state": state,
        "lower_bound": state == "partial",
        "calls": known.then_some(events.len()), "outcomes": if known { outcomes } else { Value::Null },
        "reasons": if known { reasons } else { Value::Null },
        "volume": volume.into_values().collect::<Vec<_>>(), "cohorts": cohorts.into_values().collect::<Vec<_>>(),
        "coverage": {"sources": out_sources, "read": read, "counted": events.len(), "duplicates": duplicates,
            "out_of_window": out_of_window, "excluded": excluded, "capped": capped,
            "profile_unknown": profile_unknown},
        "observed_tokens": {"state": "unknown", "reason": "core records carry no observed model usage"},
        "task_effect": {"state": "unknown", "reason": "no paired experiment cohort"}});
    if let Some(b) = input["bucket"].as_u64().filter(|b| *b > 0) {
        let mut buckets = vec![];
        let mut sub = input.clone();
        sub["bucket"] = Value::Null;
        for at in (start..end).step_by(b as usize) {
            sub["window"] = json!({"start": at, "end": (at + b).min(end)});
            let a = aggregate(&sub)?;
            buckets.push(json!({"start": at, "end": a["window"]["end"], "state": a["state"], "calls": a["calls"], "outcomes": a["outcomes"]}));
        }
        out["buckets"] = json!(buckets);
    }
    Ok(out)
}

/// Why a readable local journal still proves no completeness.
pub const UNPROVEN: &str = "calls that wrote no evidence are undetectable";

/// Local records and coverage from one SQLite snapshot, with the existing failure-marker semantics.
pub fn local(data: &Path, now: u64) -> Value {
    let snapshot = match crate::local::snapshot(data) {
        Ok(v) => v,
        Err(e) => return json!({"installation":null,"runtime":crate::telemetry::runtime(),"observed":[],"resets":[],"unknown":if e=="stats unknown: no stats file" { "no stats recorded".to_string() } else { e },"records":[]}),
    };
    let records = snapshot["records"].clone();
    let mut unknown = None;
    let held: std::collections::HashMap<String,u64> = snapshot["held"].as_object().into_iter().flatten()
        .filter_map(|(k,v)| v.as_u64().map(|n|(k.clone(),n))).collect();
    let journal = (!snapshot["journal"].is_null()).then(||snapshot["journal"].clone());
    let mut observed = vec![];
    let mut resets = vec![];
    if unknown.is_none() && !crate::compression::failed(data).is_empty() {
        unknown = Some("stats write failed");
    }
    match journal.filter(crate::compression::journal_valid) {
        _ if unknown.is_some() => {}
        // Fewer lines than the producer wrote: a deleted or truncated day file. Its calls are unprovable, never zero.
        Some(j) if j["counts"].as_object().into_iter().flatten().any(|(d, n)| held.get(d).copied().unwrap_or(0) < n.as_u64().unwrap_or(0)) => {
            unknown = Some("stats records lost")
        }
        Some(j) => {
            let mut at = j["since"].as_u64().unwrap_or(u64::MAX).max(now.saturating_sub(RETENTION));
            let mut gaps: Vec<(u64, u64)> = j["gaps"].as_array().into_iter().flatten().filter_map(|g| g[0].as_u64().zip(g[1].as_u64())).collect();
            gaps.sort();
            for (a, b) in gaps.into_iter().chain([(now + 1, now + 1)]) {
                let a = a.min(now + 1);
                if a > at {
                    observed.push(json!([at, a]));
                }
                at = at.max(b);
            }
            resets = j["resets"].as_array().cloned().unwrap_or_default();
            // A call that can write neither its record nor a failure marker leaves no evidence at all, so the journal
            // shows only what was written: lower bounds, never complete or zero.
            unknown = Some(UNPROVEN);
        }
        None => unknown = Some(if snapshot["journal"].is_null() { "no coverage journal" } else { "coverage journal unreadable" }),
    }
    let installation = std::fs::read_to_string(data.join("installation")).ok().map(|s| s.trim().to_string());
    json!({"installation": installation, "runtime": crate::telemetry::runtime(), "observed": observed, "resets": resets,
        "unknown": unknown, "records": records})
}

/// Exported spans from a queue folder, an OTLP JSON body, a span array or one span. Export carries no coverage, so
/// the source has no observed interval; a queue folder adds its `dropped` count.
pub fn exported(path: &Path) -> Result<Value, String> {
    fn spans(v: Value, out: &mut Vec<Value>) {
        if let Some(rs) = v["resourceSpans"].as_array() {
            for ss in rs.iter().flat_map(|r| r["scopeSpans"].as_array().into_iter().flatten()) {
                out.extend(ss["spans"].as_array().into_iter().flatten().cloned());
            }
        } else if let Value::Array(a) = v {
            out.extend(a);
        } else {
            out.push(v);
        }
    }
    let parse = |p: &Path| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!("unreadable"));
    let mut records = vec![];
    let mut dropped = 0;
    if path.is_dir() {
        for (_, p, _) in crate::telemetry::entries(path) {
            if !p.to_string_lossy().ends_with(crate::telemetry::RECORD) {
                spans(parse(&p), &mut records);
            }
        }
        dropped = crate::telemetry::state(path)["dropped"].as_u64().unwrap_or(0);
    } else if path.is_file() {
        spans(parse(path), &mut records);
    } else {
        return Err(format!("no events at {}", path.display()));
    }
    Ok(json!({"installation": null, "runtime": null, "observed": [], "dropped": dropped,
        "unknown": "exported events carry no coverage", "records": records}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `expect` is a subset: listed object keys match recursively; arrays match in length and element by element.
    fn subset(expect: &Value, got: &Value, at: &str) {
        match (expect, got) {
            (Value::Object(e), _) => {
                for (k, v) in e {
                    subset(v, &got[k], &format!("{at}.{k}"));
                }
            }
            (Value::Array(e), Value::Array(g)) if e.len() == g.len() => {
                for (i, (e, g)) in e.iter().zip(g).enumerate() {
                    subset(e, g, &format!("{at}[{i}]"));
                }
            }
            _ => assert_eq!(expect, got, "{at}"),
        }
    }

    /// ACCOUNTING O4 / A2, A4: every published consumer case through the one aggregate; exact outcome/reason sums.
    #[test]
    fn consumer_fixtures() {
        let f: Value = serde_json::from_str(include_str!("../docs/spec/accounting-fixtures.json")).unwrap();
        assert_eq!(f["schema"], SCHEMA);
        let base = f["base_record"].as_object().unwrap();
        let cases = f["cases"].as_array().unwrap();
        assert!(cases.len() >= 12);
        for c in cases {
            let name = c["name"].as_str().unwrap();
            let mut input = c["input"].clone();
            for s in input["sources"].as_array_mut().unwrap() {
                for r in s["records"].as_array_mut().into_iter().flatten() {
                    if let Some(o) = r.as_object().filter(|o| !o.contains_key("attributes")) {
                        let mut m = base.clone();
                        m.extend(o.clone());
                        *r = Value::Object(m.into_iter().filter(|(_, v)| *v != json!("$absent")).collect());
                    }
                }
            }
            if let Some(field) = c["error"].as_str() {
                let e = aggregate(&input).expect_err(name);
                assert!(e.contains(field), "{name}: {e}");
                continue;
            }
            let got = aggregate(&input).unwrap_or_else(|e| panic!("{name}: {e}"));
            subset(&c["expect"], &got, name);
            if got["state"] != "unknown" {
                let calls = got["calls"].as_u64().unwrap();
                let sum = |k: &str| got[k].as_object().unwrap().values().map(|v| v.as_u64().unwrap()).sum::<u64>();
                assert_eq!((sum("outcomes"), sum("reasons")), (calls, calls), "{name}: one outcome and one reason per event");
                let cohorts: u64 = got["cohorts"].as_array().unwrap().iter().map(|c| c["calls"].as_u64().unwrap()).sum();
                assert_eq!(cohorts, calls, "{name}: cohorts partition the events");
            }
            let text = got.to_string();
            assert!(!text.contains("input_tokens") && !text.contains("cost"), "{name}: no observed usage or cost from records");
        }
    }

    #[test]
    fn rejects_unusable_queries() {
        assert!(aggregate(&json!({"window": {"start": 5, "end": 5}, "now": 9, "sources": []})).is_err());
        assert!(aggregate(&json!({"window": {"start": 1}, "now": 9, "sources": []})).is_err());
        let bad = json!({"window": {"start": 1, "end": 5}, "now": 9, "sources": [{"observed": [[4, 2]]}]});
        assert!(aggregate(&bad).is_err());
    }
}
