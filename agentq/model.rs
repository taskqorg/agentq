//! Model calls: Claude's native transcript at the `Stop` hook, Codex responses seen by the loopback provider and
//! Hermes's `post_api_request` hook (agentq.md § 6 Model calls).
use ring::digest;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SOURCE: &str = "claude_transcript";
/// One batch: one local transaction, one queue prune and one saved offset.
const BATCH_LINES: usize = 512;
const BATCH_BYTES: usize = 4 << 20;
/// A hook pass starts no batch after this, inside the 5 s hook budget (§ 1 `hook`); the rest waits as backlog.
const HOOK_PASS: Duration = Duration::from_secs(3);

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
/// block and may correct its usage). Each value: (call key, source time, attributes without event ID, index of the
/// call's first line).
pub fn calls(text: &str) -> Vec<(String, Option<u128>, Value, usize)> {
    let mut out: Vec<(String, Option<u128>, Value, usize)> = vec![];
    for (n, line) in text.lines().enumerate() {
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
        let entry = (key, v["timestamp"].as_str().and_then(time_ns), Value::Object(a), n);
        match out.iter_mut().find(|c| !entry.0.is_empty() && c.0 == entry.0) {
            Some(c) => *c = (entry.0, entry.1, entry.2, c.3),
            None => out.push(entry),
        }
    }
    out
}

/// One Hermes `post_api_request` (one provider call): its key, source time and attributes without event ID. Hermes
/// normalizes usage to integer buckets and reports no cost in this hook.
pub fn hermes_call(v: &Value) -> (String, Option<u128>, Value) {
    let mut a = Map::new();
    a.insert("agentq.agent".into(), json!("hermes"));
    a.insert("agentq.model.source".into(), json!("hermes_hook"));
    for (key, field) in [("agentq.session", "session_id"), ("agentq.turn", "turn_id")] {
        if let Some(s) = v[field].as_str().filter(|s| !s.is_empty()) { a.insert(key.into(), json!(s)); }
    }
    let call = v["api_request_id"].as_str().filter(|s| !s.is_empty());
    let u = &v["usage"];
    for (key, value) in [
        ("name", if v["response_model"].is_string() { v["response_model"].clone() } else { v["model"].clone() }),
        ("call_id", call.map_or(Value::Null, |c| json!(c))),
        ("input_tokens", u["input_tokens"].clone()),
        ("output_tokens", u["output_tokens"].clone()),
        ("cache_read_tokens", u["cache_read_tokens"].clone()),
        ("cache_creation_tokens", u["cache_write_tokens"].clone()),
    ] {
        if value.as_str().is_some_and(|s| !s.is_empty()) || value.is_u64() { a.insert(format!("agentq.model.{key}"), value); } else { a.insert(format!("agentq.model.{key}_unknown"), json!("not reported")); }
    }
    a.insert("agentq.model.cost_unknown".into(), json!("not reported"));
    // ponytail: a call without its ID keys on session, turn and end time; a resend of it counts once only when those match.
    let key = call.map_or_else(|| format!("hermes:{}:{}:{}", v["session_id"], v["turn_id"], v["ended_at"]), String::from);
    let at = v["ended_at"].as_f64().filter(|s| s.is_finite() && *s > 0.0).map(|s| (s * 1e9) as u128);
    (key, at, Value::Object(a))
}

/// Queue the `agentq.model` span of one Hermes call; an unwritten event is a recorded hook failure.
pub fn observe_hermes(data: &Path, input: &Value) -> Result<(), String> {
    let (key, at, mut attrs) = hermes_call(input);
    attrs["agentq.event_id"] = json!(event_id(&crate::telemetry::installation(data), &key));
    crate::telemetry::emit_at(data, "agentq.model", "call", attrs, at.unwrap_or_else(crate::telemetry::now_ns)).map(|_| ()).ok_or_else(|| "model call not queued".into())
}

/// One finished Codex Responses call as the loopback provider saw it: `{session, id, model, usage, created_at}`
/// (§ 6 Model calls, Codex). Tokens keep the Claude meaning: input excludes cache reads.
pub fn codex_call(input: &Value) -> (String, Option<u128>, Value) {
    let (u, unknown) = (&input["usage"], "not in response");
    let cached = u["input_tokens_details"]["cached_tokens"].as_u64();
    let mut a = Map::new();
    a.insert("agentq.agent".into(), json!("codex"));
    a.insert("agentq.model.source".into(), json!("codex_provider"));
    if let Some(s) = input["session"].as_str() { a.insert("agentq.session".into(), json!(s)); }
    for (key, value, reason) in [
        ("name", input["model"].clone(), unknown),
        ("call_id", input["id"].clone(), unknown),
        ("input_tokens", u["input_tokens"].as_u64().zip(cached).and_then(|(i, c)| i.checked_sub(c)).map_or(Value::Null, |n| json!(n)), unknown),
        ("output_tokens", u["output_tokens"].clone(), unknown),
        ("cache_read_tokens", cached.map_or(Value::Null, |n| json!(n)), unknown),
        ("cache_creation_tokens", Value::Null, "not reported"),
        ("cost", Value::Null, "not reported"),
    ] {
        if value.is_string() || value.is_u64() { a.insert(format!("agentq.model.{key}"), value); } else { a.insert(format!("agentq.model.{key}_unknown"), json!(reason)); }
    }
    let key = input["id"].as_str().map_or_else(|| crate::resource::hex(digest::digest(&digest::SHA256, input.to_string().as_bytes()).as_ref()), String::from);
    (key, input["created_at"].as_u64().map(|s| s as u128 * 1_000_000_000), Value::Object(a))
}

/// `agentq model codex`: queue one `agentq.model` span unless the Codex integration is disabled.
pub fn codex(data: &Path, input: &str) -> Result<(), String> {
    if crate::control::disabled(data, "codex")? { return Ok(()); }
    let v: Value = serde_json::from_str(input).map_err(|e| format!("model call JSON: {e}"))?;
    let (key, at, mut attrs) = codex_call(&v);
    attrs["agentq.event_id"] = json!(event_id(&crate::telemetry::installation(data), &key));
    crate::telemetry::emit_at(data, "agentq.model", "call", attrs, at.unwrap_or_else(crate::telemetry::now_ns)).map(|_| ()).ok_or_else(|| "queue write failed".into())
}

/// Read offset per transcript (`data/model/<hash>`): its path, the next byte and the bytes left when a pass stopped.
fn mark(state: &Path, file: &Path) -> PathBuf {
    state.join(&event_id("transcript", &file.to_string_lossy())[..16])
}

/// Process `file` in batches from its saved offset to its last complete line. A batch starts only before `deadline`,
/// or as the first batch of a pass (`progressed` false); inside a batch no event chunk but the first starts past
/// `deadline`, so every pass makes progress, and the offset stops at the first line of the first call left. A shorter file was
/// rewritten and reads from the start. The mark records the bytes left; returns them.
fn drain_file(data: &Path, installation: &str, file: &Path, deadline: Instant, progressed: &mut bool) -> Result<u64, String> {
    let err = |e: std::io::Error| format!("transcript: {e}");
    let mark = mark(&data.join("model"), file);
    let saved: Value = std::fs::read_to_string(&mark).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let mut f = std::fs::File::open(file).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    let mut offset = saved["offset"].as_u64().unwrap_or(0);
    if offset > len { offset = 0; }
    f.seek(SeekFrom::Start(offset)).map_err(err)?;
    let mut r = std::io::BufReader::new(f);
    let save = |offset: u64, left: u64| coreq::write_atomic(&mark, &json!({"path": file, "offset": offset, "left": left}).to_string()).map_err(|e| format!("model state: {e}"));
    loop {
        if *progressed && Instant::now() >= deadline {
            let left = len.saturating_sub(offset);
            if saved["offset"] != offset || saved["left"] != left { save(offset, left)?; }
            return Ok(left);
        }
        // `starts`: byte offset of each line in `text`.
        let (mut text, mut line, mut starts) = (vec![], vec![], vec![]);
        while starts.len() < BATCH_LINES && text.len() < BATCH_BYTES {
            line.clear();
            r.read_until(b'\n', &mut line).map_err(err)?;
            // An unterminated last line is still being written: it waits for the next pass.
            if line.last() != Some(&b'\n') { break; }
            starts.push(text.len() as u64);
            text.extend_from_slice(&line);
        }
        if text.is_empty() {
            if saved["offset"] != offset || saved["left"] != 0 { save(offset, 0)?; }
            return Ok(0);
        }
        let found = calls(&String::from_utf8_lossy(&text));
        let firsts: Vec<usize> = found.iter().map(|c| c.3).collect();
        let events = found.into_iter().map(|(key, at, mut attrs, _)| {
            attrs["agentq.event_id"] = json!(event_id(installation, &key));
            (attrs, at.unwrap_or_else(crate::telemetry::now_ns))
        });
        let (handled, _) = crate::telemetry::emit_until(data, "agentq.model", "call", events.collect(), Some(deadline));
        // A call already sent whose correction lies past the cut is sent again under its own event ID.
        offset += firsts.get(handled).map_or(text.len() as u64, |&n| starts[n]);
        *progressed = true;
        save(offset, len.saturating_sub(offset))?;
    }
}

/// Queue one `agentq.model` span per new call of the session transcript and its subagent transcripts, bounded by
/// `HOOK_PASS`. Leftover bytes stay in the marks as backlog for the next pass or the service (`drain`).
pub fn observe(data: &Path, input: &Value) -> Result<(), String> {
    observe_until(data, input, Instant::now() + HOOK_PASS)
}

fn observe_until(data: &Path, input: &Value, deadline: Instant) -> Result<(), String> {
    let Some(main) = input["transcript_path"].as_str().map(PathBuf::from) else { return Err("Stop hook has no transcript_path".into()) };
    let mut files = vec![main.clone()];
    if let Ok(dir) = std::fs::read_dir(main.with_extension("").join("subagents")) {
        let mut subs: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        subs.sort();
        files.extend(subs);
    }
    std::fs::create_dir_all(data.join("model")).map_err(|e| format!("model state: {e}"))?;
    let installation = crate::telemetry::installation(data);
    let mut progressed = false;
    for file in files {
        drain_file(data, &installation, &file, deadline, &mut progressed)?;
    }
    expire(data);
    Ok(())
}

/// Marks untouched for 7 days go, a backlog of a deleted transcript included.
fn expire(data: &Path) {
    let week = std::time::SystemTime::now() - Duration::from_secs(7 * 86400);
    for e in std::fs::read_dir(data.join("model")).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < week) { let _ = std::fs::remove_file(e.path()); }
    }
}

/// Saved marks: (transcript, bytes left), or the read error of an unreadable mark.
fn marks(data: &Path) -> Vec<Result<(PathBuf, u64), String>> {
    std::fs::read_dir(data.join("model")).into_iter().flatten().flatten().filter(|e| !e.file_name().to_string_lossy().ends_with(".tmp")).map(|e| {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        // A bare offset (unreleased first format) records no backlog; its next pass rereads the file by event ID.
        if v.is_u64() { return Ok((PathBuf::new(), 0)); }
        Ok((PathBuf::from(v["path"].as_str().ok_or("mark has no path")?), v["left"].as_u64().ok_or("mark has no left")?))
    }).collect()
}

/// The service's pass over transcripts a hook pass left unfinished (a session that ended mid-backlog included).
pub fn drain(data: &Path, deadline: Instant) {
    let installation = crate::telemetry::installation(data);
    let mut progressed = false;
    for (file, _) in marks(data).into_iter().flatten().filter(|m| m.1 > 0) {
        if let Err(e) = drain_file(data, &installation, &file, deadline, &mut progressed) {
            eprintln!("agentq: model calls of a transcript unknown: {e}");
        }
    }
    expire(data);
}

/// `status.json` `model_backlog` and the heartbeat: transcripts with unread calls and their bytes; unreadable marks
/// make the backlog unknown, never zero.
pub fn backlog(data: &Path) -> Value {
    let (mut transcripts, mut bytes, mut unreadable) = (0, 0, 0);
    for m in marks(data) {
        match m {
            Ok((_, left)) if left > 0 => { transcripts += 1; bytes += left; }
            Ok(_) => {}
            Err(_) => unreadable += 1,
        }
    }
    if unreadable > 0 { return json!({"state": "unknown", "reason": format!("{unreadable} unreadable read offsets")}); }
    json!({"state": if transcripts > 0 { "pending" } else { "none" }, "transcripts": transcripts, "bytes": bytes})
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
    fn codex_responses_keep_claude_token_meaning_and_count_once() {
        let r = json!({"session": "t1", "id": "resp_a", "model": "gpt-5.5-codex", "created_at": 1_792_398_731u64,
            "usage": {"input_tokens": 1000, "input_tokens_details": {"cached_tokens": 800}, "output_tokens": 50, "output_tokens_details": {"reasoning_tokens": 20}}});
        let (key, at, a) = codex_call(&r);
        assert_eq!((key.as_str(), at), ("resp_a", Some(1_792_398_731_000_000_000)));
        assert_eq!((a["agentq.agent"].as_str(), a["agentq.session"].as_str(), a["agentq.model.name"].as_str(), a["agentq.model.call_id"].as_str()), (Some("codex"), Some("t1"), Some("gpt-5.5-codex"), Some("resp_a")));
        assert_eq!((a["agentq.model.input_tokens"].as_u64(), a["agentq.model.cache_read_tokens"].as_u64(), a["agentq.model.output_tokens"].as_u64()), (Some(200), Some(800), Some(50)), "input excludes cache reads, as for Claude");
        assert_eq!((a["agentq.model.cache_creation_tokens_unknown"].as_str(), a["agentq.model.cost_unknown"].as_str()), (Some("not reported"), Some("not reported")));
        let (bare_key, bare_at, b) = codex_call(&json!({"usage": {"input_tokens": 5}}));
        for f in ["name", "call_id", "input_tokens", "output_tokens", "cache_read_tokens"] {
            assert!(b.get(format!("agentq.model.{f}")).is_none());
            assert_eq!(b[format!("agentq.model.{f}_unknown")], "not in response", "{f}: unknown is never zero");
        }
        assert!(bare_at.is_none() && bare_key.len() == 64, "no id keys on a hash of the entry");
        let d = std::env::temp_dir().join(format!("agentq-model-codex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let ids = |d: &Path| crate::telemetry::entries(&crate::telemetry::queue(d)).len();
        codex(&d, &r.to_string()).unwrap();
        codex(&d, &r.to_string()).unwrap();
        assert_eq!(ids(&d), 1, "a resent response replaces its queued event");
        assert!(codex(&d, "not json").is_err());
        let _ = std::fs::remove_dir_all(&d);
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

    /// #163: Hermes calls report the same fields as Claude's; a missing field is unknown, a repeat counts once.
    #[test]
    fn hermes_calls_carry_identity_usage_and_unknowns() {
        let full = json!({"api_request_id": "r1", "session_id": "s1", "turn_id": "t1", "model": "gpt-5", "response_model": "gpt-5-2026",
            "ended_at": 1_792_398_731.5, "usage": {"input_tokens": 10, "output_tokens": 2, "cache_read_tokens": 0, "cache_write_tokens": 4}});
        let (key, at, a) = hermes_call(&full);
        assert_eq!((key.as_str(), at), ("r1", Some(1_792_398_731_500_000_000)));
        assert_eq!((a["agentq.model.name"].as_str(), a["agentq.model.call_id"].as_str(), a["agentq.model.cache_read_tokens"].as_u64(), a["agentq.model.cache_creation_tokens"].as_u64(), a["agentq.turn"].as_str()),
            (Some("gpt-5-2026"), Some("r1"), Some(0), Some(4), Some("t1")));
        assert_eq!(a["agentq.model.cost_unknown"], "not reported", "Hermes reports no cost: unknown, never zero");
        let (key, at, a) = hermes_call(&json!({"session_id": "s1", "model": "gpt-5", "usage": null}));
        assert!(at.is_none() && key.starts_with("hermes:"));
        for f in ["call_id", "input_tokens", "output_tokens", "cache_read_tokens", "cache_creation_tokens"] {
            assert!(a.get(format!("agentq.model.{f}")).is_none());
            assert_eq!(a[format!("agentq.model.{f}_unknown")], "not reported", "{f}");
        }
        assert_eq!(a["agentq.model.name"], "gpt-5", "the requested model when the response names none");
        let d = std::env::temp_dir().join(format!("agentq-model-hermes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        observe_hermes(&d, &full).unwrap();
        observe_hermes(&d, &full).unwrap();
        let q: Vec<String> = crate::telemetry::entries(&crate::telemetry::queue(&d)).into_iter().map(|e| std::fs::read_to_string(e.1).unwrap()).collect();
        assert_eq!(q.len(), 1, "a repeated call ID counts once");
        assert!(q[0].contains("\"agentq.model\"") && q[0].contains("hermes_hook"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #161 review: a 4008-call transcript took 115 s in one pass, was killed by the 5 s hook timeout and lost the rest.
    #[test]
    fn thousands_of_calls_drain_across_bounded_passes_once_each() {
        let d = std::env::temp_dir().join(format!("agentq-model-bulk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let t = d.join("s2.jsonl");
        let usage = json!({"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4});
        // Claude repeats each call once per content block; a user line sits between calls.
        let mut text = String::new();
        for i in 0..4008 {
            let at = format!("2026-10-19T08:{:02}:{:02}Z", i / 60 % 60, i % 60);
            for _ in 0..2 { text += &(line(Some(&format!("req_{i}")), usage.clone(), &at) + "\n"); }
            text += &(json!({"type": "user", "message": {"content": "x".repeat(200)}}).to_string() + "\n");
        }
        std::fs::write(&t, &text).unwrap();
        let data = d.join("data");
        let input = json!({"transcript_path": t});
        // Queued event IDs from file names (`<secs>-<id>.json`): reading thousands of files is slow on Windows CI.
        let ids = |data: &Path| -> Vec<String> {
            crate::telemetry::entries(&crate::telemetry::queue(data)).into_iter().filter_map(|e| Some(e.1.file_stem()?.to_str()?.split_once('-')?.1.to_string())).collect()
        };
        // An expired deadline still runs one event chunk, so each pass makes progress and leaves the rest as backlog.
        // #161 CI: a whole 512-line batch after the deadline took a Windows hook pass to 6.25 s.
        observe_until(&data, &input, Instant::now()).unwrap();
        assert_eq!(ids(&data).len(), crate::telemetry::EMIT_CHUNK, "one event chunk per expired pass, cut inside the batch");
        observe_until(&data, &input, Instant::now()).unwrap();
        assert_eq!(ids(&data).len(), 2 * crate::telemetry::EMIT_CHUNK, "the next pass resumes at the first call left");
        let b = backlog(&data);
        assert_eq!((b["state"].as_str(), b["transcripts"].as_u64()), (Some("pending"), Some(1)), "leftover calls are exposed: {b}");
        assert!(b["bytes"].as_u64().unwrap() > 0);
        let start = Instant::now();
        observe(&data, &input).unwrap();
        assert!(start.elapsed() < Duration::from_secs(5), "a hook pass ends inside the hook budget: {:?}", start.elapsed());
        // The service finishes what hook passes left, even after the session ended.
        while backlog(&data)["state"] == "pending" { drain(&data, Instant::now() + Duration::from_secs(10)); }
        let mut got = ids(&data);
        got.sort();
        let installation = crate::telemetry::installation(&data);
        let mut want: Vec<String> = (0..4008).map(|i| event_id(&installation, &format!("req_{i}"))).collect();
        want.sort();
        assert_eq!(got, want, "each call ID exactly once");
        assert_eq!(backlog(&data), json!({"state": "none", "transcripts": 0, "bytes": 0}));
        std::fs::write(mark(&data.join("model"), &t), "not json").unwrap();
        assert_eq!(backlog(&data)["state"], "unknown", "an unreadable offset is unknown backlog, never none");
        let _ = std::fs::remove_dir_all(&d);
    }
}
