//! Claude model calls read from the native transcript at the `Stop` hook (agentq.md § 6 Model calls).
use ring::digest;
use serde_json::{Map, Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const SOURCE: &str = "claude_transcript";

/// Same installation and call ID, same event: a duplicate counts once and a later reading replaces it.
fn event_id(installation: &str, call: &str) -> String {
    crate::resource::hex(digest::digest(&digest::SHA256, format!("{installation}:model:{call}").as_bytes()).as_ref())[..32].to_string()
}

/// `2026-10-19T08:32:09.030Z` as unix nanoseconds; other shapes are unknown.
fn time_ns(s: &str) -> Option<u128> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if s.len() < 20 || !s.ends_with('Z') { return None; }
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146097 + (y - era * 400) * 365 + (y - era * 400) / 4 - (y - era * 400) / 100 + doy - 719468;
    let secs = days * 86400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?;
    let frac = s[19..s.len() - 1].strip_prefix('.').unwrap_or("");
    let nanos = format!("{:0<9}", frac).get(..9)?.parse::<u128>().ok()?;
    u128::try_from(secs).ok().map(|s| s * 1_000_000_000 + nanos)
}

/// One call per native call ID, in transcript order; the last line of a call wins (Claude repeats a call per content
/// block and may correct its usage). Each value: (call key, source time, attributes without event ID).
pub fn calls(text: &str) -> Vec<(String, Option<u128>, Value)> {
    let mut out: Vec<(String, Option<u128>, Value)> = vec![];
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        let m = &v["message"];
        // `<synthetic>` marks a client-made message, not a model call.
        if v["type"] != "assistant" || !m.is_object() || m["model"] == "<synthetic>" { continue; }
        let mut a = Map::new();
        a.insert("agentq.agent".into(), json!("claude"));
        a.insert("agentq.model.source".into(), json!(SOURCE));
        if let Some(s) = v["sessionId"].as_str() { a.insert("agentq.session".into(), json!(s)); }
        let call = v["requestId"].as_str().or(m["id"].as_str());
        let u = &m["usage"];
        for (key, value) in [
            ("name", m["model"].clone()),
            ("call_id", call.map_or(Value::Null, |c| json!(c))),
            ("input_tokens", u["input_tokens"].clone()),
            ("output_tokens", u["output_tokens"].clone()),
            ("cache_read_tokens", u["cache_read_input_tokens"].clone()),
            ("cache_creation_tokens", u["cache_creation_input_tokens"].clone()),
        ] {
            if value.is_string() || value.is_u64() { a.insert(format!("agentq.model.{key}"), value); } else { a.insert(format!("agentq.model.{key}_unknown"), json!("not in transcript")); }
        }
        match v["costUSD"].as_f64() {
            Some(c) => { a.extend([("agentq.model.cost".into(), json!(c)), ("agentq.model.cost_currency".into(), json!("USD")), ("agentq.model.cost_source".into(), json!(SOURCE))]); }
            None => { a.insert("agentq.model.cost_unknown".into(), json!("not reported")); }
        }
        let key = call.or(v["uuid"].as_str()).unwrap_or_default().to_string();
        let entry = (key, v["timestamp"].as_str().and_then(time_ns), Value::Object(a));
        match out.iter_mut().find(|c| !entry.0.is_empty() && c.0 == entry.0) {
            Some(c) => *c = entry,
            None => out.push(entry),
        }
    }
    out
}

/// Bytes from `offset` up to the last complete line; a shorter file was rewritten and reads from the start.
fn tail(path: &Path, offset: u64) -> Result<(String, u64), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("transcript: {e}"))?;
    let len = f.metadata().map_err(|e| format!("transcript: {e}"))?.len();
    let start = if offset > len { 0 } else { offset };
    f.seek(SeekFrom::Start(start)).map_err(|e| format!("transcript: {e}"))?;
    let mut b = vec![];
    f.take(len - start).read_to_end(&mut b).map_err(|e| format!("transcript: {e}"))?;
    let end = b.iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
    b.truncate(end);
    Ok((String::from_utf8_lossy(&b).into_owned(), start + end as u64))
}

/// Queue one `agentq.model` span per new call of the session transcript and its subagent transcripts. A read offset
/// per transcript (`data/model/<hash>`, 7 days) keeps each pass incremental; a lost offset only resends the same IDs.
pub fn observe(data: &Path, input: &Value) -> Result<(), String> {
    let Some(main) = input["transcript_path"].as_str().map(PathBuf::from) else { return Err("Stop hook has no transcript_path".into()) };
    let mut files = vec![main.clone()];
    if let Ok(dir) = std::fs::read_dir(main.with_extension("").join("subagents")) {
        let mut subs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        subs.sort();
        files.extend(subs);
    }
    let state = data.join("model");
    std::fs::create_dir_all(&state).map_err(|e| format!("model state: {e}"))?;
    let installation = crate::telemetry::installation(data);
    for file in files {
        let mark = state.join(&event_id("transcript", &file.to_string_lossy())[..16]);
        let offset = std::fs::read_to_string(&mark).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let (text, end) = tail(&file, offset)?;
        if end == offset { continue; }
        for (key, at, mut attrs) in calls(&text) {
            attrs["agentq.event_id"] = json!(event_id(&installation, &key));
            crate::telemetry::emit_at(data, "agentq.model", "call", attrs, at.unwrap_or_else(crate::telemetry::now_ns));
        }
        coreq::write_atomic(&mark, &end.to_string()).map_err(|e| format!("model state: {e}"))?;
    }
    let week = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 86400);
    for e in std::fs::read_dir(&state).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < week) { let _ = std::fs::remove_file(e.path()); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(req: Option<&str>, usage: Value, at: &str) -> String {
        let mut v = json!({"type": "assistant", "sessionId": "s1", "uuid": format!("u-{at}"), "timestamp": at, "message": {"id": "msg_1", "model": "claude-opus-5-5", "usage": usage}});
        if let Some(r) = req { v["requestId"] = json!(r); } else { v["message"].as_object_mut().unwrap().remove("id"); }
        v.to_string()
    }

    #[test]
    fn duplicate_calls_count_once_and_corrections_supersede() {
        let full = json!({"input_tokens": 2, "output_tokens": 89, "cache_read_input_tokens": 30256, "cache_creation_input_tokens": 20822});
        let mut corrected = full.clone();
        corrected["output_tokens"] = json!(120);
        let text = [line(Some("req_a"), full.clone(), "2026-10-19T08:32:09.030Z"), line(Some("req_a"), full.clone(), "2026-10-19T08:32:09.031Z"), line(Some("req_b"), full, "2026-10-19T08:32:10Z"), line(Some("req_a"), corrected, "2026-10-19T08:32:11Z")].join("\n");
        let c = calls(&text);
        assert_eq!(c.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["req_a", "req_b"], "one call per call ID");
        assert_eq!(c[0].2["agentq.model.output_tokens"], 120, "a later reading of the same call supersedes");
        assert_eq!(c[0].1, Some(1_792_398_731_000_000_000));
        let a = &c[1].2;
        assert_eq!((a["agentq.model.name"].as_str(), a["agentq.model.call_id"].as_str(), a["agentq.model.cache_read_tokens"].as_u64(), a["agentq.session"].as_str()), (Some("claude-opus-5-5"), Some("req_b"), Some(30256), Some("s1")));
        assert_eq!(a["agentq.model.cost_unknown"], "not reported", "an unreported cost is unknown, never zero or an estimate");
        assert_eq!(event_id("i", "req_a"), event_id("i", "req_a"));
        assert_ne!(event_id("i", "req_a"), event_id("i", "req_b"));
    }

    #[test]
    fn partial_calls_name_each_unknown_field() {
        let text = [
            line(None, json!({"input_tokens": 5}), "2026-10-19T08:32:09Z"),
            json!({"type": "assistant", "requestId": "req_c", "costUSD": 0.25, "message": {"model": "claude-haiku-5-5", "usage": {"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0}}}).to_string(),
            json!({"type": "assistant", "message": {"model": "<synthetic>", "usage": {}}}).to_string(),
            "not json".into(),
        ].join("\n");
        let c = calls(&text);
        assert_eq!(c.len(), 2, "synthetic and unreadable lines are not calls");
        let a = &c[0].2;
        assert_eq!(c[0].0, "u-2026-10-19T08:32:09Z", "a call without native ID keys on its line");
        for f in ["call_id", "output_tokens", "cache_read_tokens", "cache_creation_tokens"] {
            assert!(a.get(format!("agentq.model.{f}")).is_none());
            assert_eq!(a[format!("agentq.model.{f}_unknown")], "not in transcript", "{f}");
        }
        assert_eq!(a["agentq.model.input_tokens"], 5);
        let b = &c[1].2;
        assert_eq!((b["agentq.model.cost"].as_f64(), b["agentq.model.cost_currency"].as_str(), b["agentq.model.cost_source"].as_str(), b["agentq.model.cache_read_tokens"].as_u64()), (Some(0.25), Some("USD"), Some(SOURCE), Some(0)));
        assert!(c[1].1.is_none(), "a missing source time is unknown");
    }

    #[test]
    fn stop_hook_queues_new_calls_once() {
        let d = std::env::temp_dir().join(format!("agentq-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let t = d.join("projects/p/s1.jsonl");
        std::fs::create_dir_all(d.join("projects/p/s1/subagents")).unwrap();
        let usage = json!({"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4});
        std::fs::write(&t, format!("{}\n{}", line(Some("req_a"), usage.clone(), "2026-10-19T08:32:09Z"), line(Some("req_b"), usage.clone(), "2026-10-19T08:32:10Z"))).unwrap();
        std::fs::write(d.join("projects/p/s1/subagents/agent-x.jsonl"), line(Some("req_s"), usage.clone(), "2026-10-19T08:32:11Z") + "\n").unwrap();
        let data = d.join("data");
        let input = json!({"session_id": "s1", "transcript_path": t, "hook_event_name": "Stop"});
        let queued = |data: &Path| -> Vec<String> {
            let mut v: Vec<String> = crate::telemetry::entries(&crate::telemetry::queue(data)).into_iter().map(|e| std::fs::read_to_string(e.1).unwrap()).filter(|s| s.contains("\"agentq.model\"")).collect();
            v.sort();
            v
        };
        observe(&data, &input).unwrap();
        let first = queued(&data);
        assert_eq!(first.len(), 2, "the unterminated last line waits for the next pass");
        assert!(first.iter().any(|s| s.contains("req_s")), "subagent calls count");
        std::fs::write(&t, format!("{}\n{}\n{}\n", line(Some("req_a"), usage.clone(), "2026-10-19T08:32:09Z"), line(Some("req_b"), usage.clone(), "2026-10-19T08:32:10Z"), line(Some("req_a"), usage, "2026-10-19T08:32:12Z"))).unwrap();
        observe(&data, &input).unwrap();
        let second = queued(&data);
        assert_eq!(second.len(), 3, "req_b is new; the repeated req_a replaces its queued event");
        std::fs::remove_dir_all(data.join("model")).unwrap();
        observe(&data, &input).unwrap();
        assert_eq!(queued(&data).len(), 3, "a lost offset resends the same event IDs");
        assert!(observe(&data, &json!({})).is_err(), "a Stop input without transcript is a recorded hook failure");
        let _ = std::fs::remove_dir_all(&d);
    }
}
