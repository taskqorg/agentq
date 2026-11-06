//! `agent.turn` and `agent.call` records of manual and PM sessions (agentq.md M6.11, docs/spec/TELEMETRY.md § Session
//! records). The runtime builds them; this module finds the new native bytes and writes them.
use agentq_runtime::agent::{APPEND_CHUNK, Call, Context, SessionTurn, append_records, session_call, session_call_records, session_records};
use serde_json::{Value, json};
use std::io::{BufRead, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Bytes one `Stop` reads over all its files; the next `Stop` continues.
const PASS_BYTES: u64 = 8 << 20;
/// Reading ends this long after the hook started: the remainder of the 5 s budget (M1.2) covers one store wait.
pub const STOP_PASS: Duration = Duration::from_millis(3500);

/// New complete lines of `file` from its saved offset, and the offset range they cover. A line is always read whole;
/// `budget` and `deadline` end the read between lines. A shorter file reads from the start.
fn read_new(mark: &Path, file: &Path, budget: u64, deadline: Instant) -> Result<(String, u64, u64), String> {
    let err = |e: std::io::Error| format!("transcript: {e}");
    let mut f = std::fs::File::open(file).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    let saved = std::fs::read_to_string(mark).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    let start = if saved > len { 0 } else { saved };
    f.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut r = std::io::BufReader::new(f);
    let (mut bytes, mut line) = (vec![], vec![]);
    while (bytes.len() as u64) < budget && Instant::now() < deadline {
        line.clear();
        r.read_until(b'\n', &mut line).map_err(err)?;
        // An unterminated last line is still being written: it waits for the next pass.
        if line.last() != Some(&b'\n') { break; }
        bytes.extend_from_slice(&line);
    }
    let end = start + bytes.len() as u64;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), start, end))
}

fn text<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> { keys.iter().find_map(|k| input[*k].as_str().filter(|s| !s.is_empty())) }

/// The transcript of the hook input and, for Claude, its subagent transcripts.
fn files(runtime: &str, input: &Value) -> Vec<PathBuf> {
    let Some(main) = text(input, &["transcript_path", "transcriptPath"]).map(PathBuf::from) else { return vec![] };
    let mut out = vec![main.clone()];
    if runtime == "claude" {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(main.with_extension("").join("subagents")).into_iter().flatten().flatten()
            .map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        subs.sort();
        out.extend(subs);
    }
    out
}

/// A key that no other `Stop` of this installation repeats.
fn unique() -> String { format!("{}-{}", crate::telemetry::now_ns(), std::process::id()) }

fn now_ms() -> u64 { (crate::telemetry::now_ns() / 1_000_000) as u64 }

/// The M6.10 code of the session's folder, else `other`.
fn project(input: &Value) -> String {
    let folder = text(input, &["cwd"]).or(input["workspacePaths"][0].as_str().filter(|p| !p.is_empty())).map(PathBuf::from).or_else(|| std::env::current_dir().ok());
    folder.and_then(|c| crate::model::project(&c).ok()).unwrap_or_else(|| "other".into())
}

fn context(project: &str) -> Context<'_> {
    Context { product: "agentq", version: env!("CARGO_PKG_VERSION"), project, task: None, route: Some("hook"), account: "default" }
}

/// The end state of an Antigravity `Stop`; the other clients report none here.
fn stop_outcome(input: &Value) -> (&'static str, &'static str) {
    let reason = input["terminationReason"].as_str().unwrap_or_default().to_ascii_lowercase();
    if text(input, &["error"]).is_some() || reason.contains("error") { ("failed", "turn-failed") }
    else if reason.contains("max_steps") { ("failed", "step-limit") }
    else { ("ok", "none") }
}

/// One `Stop`: write one `agent.turn` and its `agent.call` records. A launched session writes none (M6.9). Bytes not read
/// or not written by `deadline` wait for the next `Stop`.
pub fn stop(data: &Path, runtime: &str, input: &Value, launched: bool, deadline: Instant) -> Result<(), String> {
    if launched { return Ok(()); }
    // More executions of this user turn follow; the last `Stop` reads their bytes too.
    if input["fullyIdle"] == false { return Ok(()); }
    let session = text(input, &["session_id", "conversationId"]);
    let files = files(runtime, input);
    // A Claude `Stop` without a transcript is already a recorded failure of the `agentq.model` path.
    if files.is_empty() && runtime == "claude" { return Ok(()); }
    let state = data.join("observe");
    std::fs::create_dir_all(&state).map_err(|e| format!("observe state: {e}"))?;
    let (mut new_text, mut marks, mut position) = (String::new(), vec![], None);
    for file in &files {
        let left = PASS_BYTES.saturating_sub(new_text.len() as u64);
        if left == 0 || Instant::now() >= deadline { break; }
        let name = crate::model::event_id("transcript", &file.to_string_lossy())[..16].to_string();
        let mark = state.join(&name);
        let (new, start, end) = read_new(&mark, file, left, deadline)?;
        if !new.is_empty() {
            new_text.push_str(&new);
            // A retry after a failed append starts at this same place, so it keeps one turn.
            position.get_or_insert_with(|| format!("{name}:{start}"));
            marks.push((mark, end));
        }
    }
    // A `Stop` that found nothing new repeats an earlier one, unless no transcript exists to read.
    if new_text.is_empty() && !files.is_empty() { return Ok(()); }
    if Instant::now() >= deadline { return Ok(()); }
    let position = position.or_else(|| text(input, &["turn_id"]).map(String::from)).unwrap_or_else(unique);
    let scope = session.map(String::from).or_else(|| files.first().map(|f| crate::model::event_id("transcript", &f.to_string_lossy()))).unwrap_or_else(|| "no-session".into());
    let key = crate::model::event_id("turn", &format!("{scope}:{position}"));
    let (outcome, error) = stop_outcome(input);
    let turn = SessionTurn { runtime, session, scope: &scope, key: &key, now_ms: now_ms(), outcome, error, model: text(input, &["modelName"]) };
    let project = project(input);
    let events = session_records(&turn, &context(&project), &new_text)?;
    let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3"))?;
    for chunk in events.chunks(APPEND_CHUNK) {
        // The offsets stay, and the next `Stop` keeps the first payload of each stored ID.
        if Instant::now() >= deadline { return Ok(()); }
        append_records(&store, chunk)?;
    }
    for (mark, end) in marks { coreq::write_atomic(&mark, &json!(end).to_string()).map_err(|e| format!("observe state: {e}"))?; }
    Ok(())
}

/// The allowlisted record of one Hermes provider call as a `Call`.
fn hermes_request(v: &Value) -> Call {
    let u = &v["usage"];
    let id = text(v, &["api_request_id"]).map_or_else(|| format!("{}:{}:{}", v["session_id"], v["turn_id"], v["ended_at"]), String::from);
    Call {native_turn:None,accounting_unknown:None,
        id: Some(id),
        at_ms: v["ended_at"].as_f64().filter(|s| s.is_finite() && *s > 0.0).map(|s| (s * 1000.0) as u64),
        input: u["input_tokens"].as_u64(),
        cache_read: u["cache_read_tokens"].as_u64(),
        cache_write: u["cache_write_tokens"].as_u64(),
        output: u["output_tokens"].as_u64(),
        reasoning: u["reasoning_tokens"].as_u64(),
        cost_usd: None,
        model: text(v, &["response_model", "model"]).map(String::from),
        outcome: "ok",
        error: "none",
        aggregate: false,
    }
}

/// The calls a Hermes session waits to sum into its turn.
fn hermes_pending(data: &Path, session: Option<&str>) -> PathBuf {
    data.join("observe").join("hermes").join(format!("{}.jsonl", crate::model::event_id("hermes", session.unwrap_or("no-session"))))
}

fn hermes_turn<'a>(session: Option<&'a str>, scope: &'a str, key: &'a str, outcome: (&'static str, &'static str)) -> SessionTurn<'a> {
    SessionTurn { runtime: "hermes", session, scope, key, now_ms: now_ms(), outcome: outcome.0, error: outcome.1, model: None }
}

/// One Hermes `post_api_request`: write its `agent.call` at once and keep it for the turn. A launched session writes none.
pub fn hermes_call(data: &Path, input: &Value, launched: bool) -> Result<(), String> {
    if launched { return Ok(()); }
    let session = text(input, &["session_id"]);
    let project = project(input);
    let call = hermes_request(input);
    let turn = hermes_turn(session, session.unwrap_or("no-session"), "", ("ok", "none"));
    let event = session_call(&turn, &context(&project), &call)?;
    let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3"))?;
    append_records(&store, &[event])?;
    let pending = hermes_pending(data, session);
    let dir = pending.parent().ok_or("observe state: no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("observe state: {e}"))?;
    crate::model::expire(dir);
    let line = json!({"api_request_id": call.id, "session_id": input["session_id"], "turn_id": input["turn_id"], "model": input["model"], "response_model": input["response_model"], "ended_at": input["ended_at"], "usage": input["usage"]}).to_string() + "\n";
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&pending).map_err(|e| format!("observe state: {e}"))?;
    f.write_all(line.as_bytes()).map_err(|e| format!("observe state: {e}"))
}

/// One Hermes `on_session_end`: one `run_conversation` ended, so write its `agent.turn` over the calls kept for it.
pub fn hermes_turn_end(data: &Path, input: &Value, launched: bool) -> Result<(), String> {
    if launched { return Ok(()); }
    let session = text(input, &["session_id"]);
    let pending = hermes_pending(data, session);
    let kept = match std::fs::read_to_string(&pending) {
        Ok(kept) => kept,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("observe state: {e}")),
    };
    let mut seen = std::collections::HashSet::new();
    let calls: Vec<Call> = kept.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).map(|v| hermes_request(&v)).filter(|c| seen.insert(c.id.clone())).collect();
    let outcome = match (input["completed"].as_bool(), input["interrupted"].as_bool()) {
        (Some(true), _) => ("ok", "none"),
        (_, Some(true)) => ("cancelled", "interrupted"),
        (None, None) => ("unknown", "not-reported"),
        _ => ("failed", "turn-incomplete"),
    };
    let scope = session.unwrap_or("no-session");
    let position = calls.first().and_then(|c| c.id.clone()).unwrap_or_else(unique);
    let key = crate::model::event_id("turn", &format!("hermes:{scope}:{position}"));
    let project = project(input);
    let events = session_call_records(&hermes_turn(session, scope, &key, outcome), &context(&project), calls)?;
    let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3"))?;
    append_records(&store, &events)?;
    match std::fs::remove_file(&pending) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("observe state: {e}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-observe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("data")).unwrap();
        d
    }
    fn records(data: &Path) -> Vec<Value> {
        let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3")).unwrap();
        let (mut rows, mut after) = (vec![], 0);
        loop {
            let page = store.export(0, after, 1024).unwrap();
            rows.extend(page["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()));
            after = page["next"].as_i64().unwrap();
            if page["more"] != true { return rows; }
        }
    }
    fn kinds<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> { rows.iter().filter(|r| r["kind"] == kind).collect() }
    fn later() -> Instant { Instant::now() + Duration::from_secs(60) }
    fn claude_line(id: &str, tokens: u64) -> String {
        json!({"type": "assistant", "timestamp": "2026-10-19T08:32:09.030Z", "message": {"id": id, "model": "claude-opus-5-5", "usage": {"input_tokens": tokens, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4}}}).to_string() + "\n"
    }

    // Chain: agentq.md M6.11, specq Telemetry contract § Capture (A4). Risk: a manual Claude session writes other
    // records than a launched one, counts a call twice on a later Stop, or leaks prompt, output, tool input or a path.
    #[test]
    fn claude_stop_writes_the_runtime_records_with_metadata_only() {
        let d = dir("claude");
        let (data, transcript) = (d.join("data"), d.join("private-folder-name.jsonl"));
        let line = |id: &str, tokens: u64, at: &str| json!({"type": "assistant", "timestamp": at, "cwd": "/private/secret-cwd", "message": {"id": id, "model": "claude-opus-5-5",
            "content": [{"type": "text", "text": "SECRET-OUTPUT"}], "usage": {"input_tokens": tokens, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4}}}).to_string() + "\n";
        let prompt = json!({"type": "user", "message": {"content": "SECRET-PROMPT"}}).to_string() + "\n";
        let tool = json!({"type": "assistant", "message": {"id": "m0", "model": "<synthetic>", "content": [{"type": "tool_use", "input": {"command": "SECRET-TOOL-INPUT"}}]}}).to_string() + "\n";
        std::fs::write(&transcript, prompt + &tool + &line("m1", 10, "2026-10-19T08:32:09.030Z") + &line("m1", 10, "2026-10-19T08:32:09.030Z") + &line("m2", 20, "2026-10-19T08:32:10.000Z") + "{\"partial").unwrap();
        let input = json!({"session_id": "s-1", "transcript_path": transcript, "cwd": "/private/secret-cwd"});
        stop(&data, "claude", &input, false, later()).unwrap();
        let rows = records(&data);
        let (turns, calls) = (kinds(&rows, "agent.turn"), kinds(&rows, "agent.call"));
        assert_eq!((turns.len(), calls.len()), (1, 2), "one turn, one call per message ID, no synthetic message");
        let t = turns[0];
        assert_eq!((t["input_tokens"].as_u64(), t["output_tokens"].as_u64(), t["session_id"].as_str(), t["route"].as_str(), t["started_ms"].as_u64()), (Some(30), Some(4), Some("s-1"), Some("hook"), Some(1_792_398_729_030)));
        for field in ["exit_code", "user_cpu_us", "system_cpu_us", "peak_rss_bytes"] {
            assert!(t[field].is_null() && t["unknown"][field] == "hook-session", "{field}: a hook owns no client process");
        }
        assert_eq!((calls[0]["input_tokens"].as_u64(), calls[0]["model"].as_str(), calls[0]["started_ms"].as_u64()), (Some(10), Some("claude-opus-5-5"), Some(1_792_398_729_030)));
        assert!(calls[0]["reasoning_tokens"].is_null() && calls[0]["unknown"]["reasoning_tokens"] == "not-reported", "an unreported bucket is null, never zero");
        let all = serde_json::to_string(&rows).unwrap();
        for private in ["SECRET", "private", "secret"] { assert!(!all.contains(private), "metadata only: {private}"); }

        stop(&data, "claude", &input, false, later()).unwrap();
        assert_eq!(records(&data).len(), 3, "a Stop with nothing new writes nothing");
        std::fs::write(&transcript, std::fs::read_to_string(&transcript).unwrap().replace("{\"partial", "") + &line("m3", 5, "2026-10-19T08:33:00.000Z")).unwrap();
        stop(&data, "claude", &input, false, later()).unwrap();
        let rows = records(&data);
        assert_eq!((kinds(&rows, "agent.turn").len(), kinds(&rows, "agent.call").len()), (2, 3), "the next Stop adds its own turn and call");
        std::fs::write(&transcript, line("m1", 10, "2026-10-19T08:32:09.030Z")).unwrap();
        stop(&data, "claude", &input, false, later()).unwrap();
        assert_eq!(kinds(&records(&data), "agent.call").len(), 3, "a rewritten transcript never counts a call twice");
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11, M6.9, M4.6. Risk: Codex manual sessions stay invisible, a launched session counts twice, a client
    // without a transcript reads as zero tokens, or a Stop without a session fails the hook (tests/hook.rs) or merges turns.
    #[test]
    fn codex_stop_reads_the_rollout_and_launched_sessions_write_nothing() {
        let d = dir("codex");
        let (data, rollout) = (d.join("data"), d.join("rollout.jsonl"));
        let usage = |i: u64, t: u64| json!({"type": "event_msg", "timestamp": "2026-10-19T08:32:09.030Z", "payload": {"type": "token_count", "info": {
            "last_token_usage": {"input_tokens": i, "cached_input_tokens": 1, "output_tokens": 2, "reasoning_output_tokens": 1},
            "total_token_usage": {"input_tokens": t, "cached_input_tokens": 1, "output_tokens": 2, "reasoning_output_tokens": 1}}}}).to_string() + "\n";
        let context = json!({"type": "turn_context", "payload": {"model": "gpt-5.5-codex", "cwd": "/private/secret"}}).to_string() + "\n";
        std::fs::write(&rollout, context + &usage(10, 10)).unwrap();
        let input = json!({"session_id": "t-1", "transcript_path": rollout, "cwd": "/private/secret"});
        stop(&data, "codex", &input, true, later()).unwrap();
        assert!(!data.join("observations.sqlite3").exists(), "a launcher records its own session");
        stop(&data, "codex", &input, false, later()).unwrap();
        let rows = records(&data);
        let (turns, calls) = (kinds(&rows, "agent.turn"), kinds(&rows, "agent.call"));
        assert_eq!((turns.len(), calls.len()), (1, 1));
        assert_eq!((calls[0]["input_tokens"].as_u64(), calls[0]["cache_read_tokens"].as_u64(), calls[0]["reasoning_tokens"].as_u64(), calls[0]["model"].as_str()), (Some(9), Some(1), Some(1), Some("gpt-5.5-codex")));
        assert_eq!((turns[0]["input_tokens"].as_u64(), turns[0]["reasoning_tokens"].as_u64()), (Some(9), Some(1)));
        assert!(!serde_json::to_string(&rows).unwrap().contains("secret"));

        stop(&data, "codex", &json!({"session_id": "t-2", "turn_id": "turn-1"}), false, later()).unwrap();
        let rows = records(&data);
        let turn = kinds(&rows, "agent.turn").into_iter().find(|t| t["session_id"] == "t-2").unwrap().clone();
        assert!(turn["input_tokens"].is_null() && turn["unknown"]["input_tokens"] == "no-transcript", "no transcript reads null with a reason, never zero");

        for _ in 0..2 { stop(&data, "codex", &json!({"hook_event_name": "Stop"}), false, later()).unwrap(); }
        let rows = records(&data);
        let unnamed: Vec<_> = kinds(&rows, "agent.turn").into_iter().filter(|t| t["session_id"].is_null()).collect();
        assert_eq!(unnamed.len(), 2, "a Stop without a session succeeds and keeps its own turn");
        assert_eq!(unnamed[0]["unknown"]["session_id"], "session-unknown");
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11 (A2, A4), P2. Risk: a line longer than the read budget empties the pass, the offset never moves and
    // every later call of the session is lost without a failure.
    #[test]
    fn a_line_over_the_read_budget_never_stalls_the_session() {
        let d = dir("long-line");
        let (data, transcript) = (d.join("data"), d.join("t.jsonl"));
        let long = json!({"type": "user", "message": {"content": "x".repeat(9 << 20)}}).to_string() + "\n";
        std::fs::write(&transcript, long + &claude_line("after", 1)).unwrap();
        let input = json!({"session_id": "s", "transcript_path": transcript});
        stop(&data, "claude", &input, false, later()).unwrap();
        stop(&data, "claude", &input, false, later()).unwrap();
        assert_eq!(kinds(&records(&data), "agent.call").len(), 1, "the call after the long line is recorded");
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11 (A2). Risk: more than 1024 events in one Stop lose the turn and stall the offset.
    #[test]
    fn a_stop_over_the_store_batch_keeps_the_turn_and_every_call_once() {
        let d = dir("batch");
        let (data, transcript) = (d.join("data"), d.join("t.jsonl"));
        std::fs::write(&transcript, (0..1100).map(|i| claude_line(&format!("m{i}"), 1)).collect::<String>()).unwrap();
        let input = json!({"session_id": "s", "transcript_path": transcript});
        for _ in 0..2 { stop(&data, "claude", &input, false, later()).unwrap(); }
        let rows = records(&data);
        assert_eq!(rows[0]["kind"], "agent.turn", "the turn goes first");
        assert_eq!((kinds(&rows, "agent.turn").len(), kinds(&rows, "agent.call").len()), (1, 1100));
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M1.2, M6.11. Risk: a Stop with many or large files outlives the 5 s hook budget, or unread bytes are lost.
    #[test]
    fn a_stop_is_bounded_and_unread_bytes_wait_for_the_next_one() {
        let d = dir("bound");
        let (data, transcript) = (d.join("data"), d.join("t.jsonl"));
        let subs = d.join("t").join("subagents");
        std::fs::create_dir_all(&subs).unwrap();
        let filler = json!({"type": "user", "message": {"content": "x".repeat((8 << 20) + 1)}}).to_string() + "\n";
        std::fs::write(&transcript, filler + &claude_line("main", 1)).unwrap();
        std::fs::write(subs.join("a.jsonl"), claude_line("sub", 1)).unwrap();
        let input = json!({"session_id": "s", "transcript_path": transcript});

        let past = Instant::now();
        stop(&data, "claude", &input, false, past).unwrap();
        assert!(!data.join("observations.sqlite3").exists(), "no read and no write after the deadline");

        stop(&data, "claude", &input, false, later()).unwrap();
        assert!(kinds(&records(&data), "agent.call").is_empty(), "the byte budget ends this pass before the calls");
        stop(&data, "claude", &input, false, later()).unwrap();
        stop(&data, "claude", &input, false, later()).unwrap();
        let rows = records(&data);
        assert_eq!(kinds(&rows, "agent.call").len(), 2, "later Stops read the main and the subagent call once each");
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11 (A4), specq Telemetry contract § Capture. Risk: Antigravity sessions stay invisible, a client reporting
    // no tokens reads as zero, an intermediate Stop splits the turn, or a transcript leaks content.
    #[test]
    fn antigravity_stop_writes_one_call_per_step_with_null_tokens() {
        let d = dir("antigravity");
        let (data, transcript) = (d.join("data"), d.join("transcript_full.jsonl"));
        let step = |n: u64, status: &str| json!({"step_index": n, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": status, "created_at": "2026-10-19T08:32:09Z", "content": "SECRET", "thinking": "SECRET"}).to_string() + "\n";
        std::fs::write(&transcript, json!({"step_index": 0, "type": "USER_INPUT", "status": "DONE", "content": "SECRET"}).to_string() + "\n" + &step(1, "RUNNING") + &step(1, "DONE") + &step(2, "DONE")).unwrap();
        let input = json!({"conversationId": "c-1", "transcriptPath": transcript, "workspacePaths": ["/private/secret"], "modelName": "gemini-3.1-pro", "lastUserInput": "SECRET", "terminationReason": "NO_TOOL_CALL", "fullyIdle": false, "executionNum": 1});
        stop(&data, "antigravity", &input, false, later()).unwrap();
        assert!(!data.join("observations.sqlite3").exists(), "more executions of this turn follow");
        let idle = json!({"conversationId": "c-1", "transcriptPath": transcript, "workspacePaths": ["/private/secret"], "modelName": "gemini-3.1-pro", "lastUserInput": "SECRET", "terminationReason": "NO_TOOL_CALL", "fullyIdle": true, "executionNum": 2});
        stop(&data, "antigravity", &idle, false, later()).unwrap();
        let rows = records(&data);
        let (turns, calls) = (kinds(&rows, "agent.turn"), kinds(&rows, "agent.call"));
        assert_eq!((turns.len(), calls.len()), (1, 2), "one call per completed step");
        assert_eq!((turns[0]["session_id"].as_str(), turns[0]["runtime"].as_str(), turns[0]["outcome"].as_str(), turns[0]["model"].as_str()), (Some("c-1"), Some("antigravity"), Some("ok"), Some("gemini-3.1-pro")));
        for row in [turns[0], calls[0], calls[1]] {
            assert!(row["input_tokens"].is_null() && row["output_tokens"].is_null() && row["unknown"]["input_tokens"] == "not-reported", "{row}");
        }
        assert_eq!((calls[0]["model"].as_str(), calls[0]["started_ms"].as_u64()), (Some("gemini-3.1-pro"), Some(1_792_398_729_000)));
        let all = serde_json::to_string(&rows).unwrap();
        for private in ["SECRET", "secret", "transcript_full"] { assert!(!all.contains(private), "metadata only: {private}"); }

        let failed = json!({"conversationId": "c-2", "terminationReason": "max_steps_exceeded"});
        stop(&data, "antigravity", &failed, false, later()).unwrap();
        let rows = records(&data);
        let turn = kinds(&rows, "agent.turn").into_iter().find(|t| t["session_id"] == "c-2").unwrap().clone();
        assert_eq!((turn["outcome"].as_str(), turn["error"].as_str(), turn["unknown"]["input_tokens"].as_str()), (Some("failed"), Some("step-limit"), Some("no-transcript")));
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11 (A4), M6.9. Risk: Hermes sessions stay invisible, a turn counts another turn's calls, a missing bucket
    // reads as zero, or a launched session counts twice.
    #[test]
    fn hermes_hooks_write_one_call_per_request_and_one_turn_per_run() {
        let d = dir("hermes");
        let data = d.join("data");
        let call = |id: &str, input: u64| json!({"api_request_id": id, "session_id": "h-1", "turn_id": "t", "model": "m-1", "response_model": "m-2", "ended_at": 1_792_398_729.5, "usage": {"input_tokens": input, "output_tokens": 2, "cache_read_tokens": 3, "cache_write_tokens": 4, "reasoning_tokens": 5}});
        hermes_call(&data, &call("r-0", 9), true).unwrap();
        hermes_turn_end(&data, &json!({"session_id": "h-1", "completed": true}), true).unwrap();
        assert!(!data.join("observations.sqlite3").exists(), "a launcher records its own session");

        hermes_call(&data, &call("r-1", 10), false).unwrap();
        hermes_call(&data, &call("r-2", 20), false).unwrap();
        hermes_call(&data, &call("r-2", 20), false).unwrap();
        let rows = records(&data);
        assert_eq!(kinds(&rows, "agent.call").len(), 2, "each call is written at once and counts once");
        assert!(kinds(&rows, "agent.turn").is_empty());
        hermes_turn_end(&data, &json!({"session_id": "h-1", "completed": true, "interrupted": false}), false).unwrap();
        let rows = records(&data);
        assert_eq!((kinds(&rows, "agent.turn").len(), kinds(&rows, "agent.call").len()), (1, 2), "the turn adds no second call record");
        let (t, c) = (kinds(&rows, "agent.turn")[0], kinds(&rows, "agent.call")[0]);
        assert_eq!((t["input_tokens"].as_u64(), t["reasoning_tokens"].as_u64(), t["outcome"].as_str(), t["runtime"].as_str(), t["session_id"].as_str(), t["model"].as_str()), (Some(30), Some(10), Some("ok"), Some("hermes"), Some("h-1"), Some("m-2")));
        assert_eq!((c["input_tokens"].as_u64(), c["cache_write_tokens"].as_u64(), c["model"].as_str(), c["started_ms"].as_u64(), c["runtime"].as_str()), (Some(10), Some(4), Some("m-2"), Some(1_792_398_729_500), Some("hermes")));

        hermes_call(&data, &json!({"api_request_id": "r-3", "session_id": "h-1", "usage": null}), false).unwrap();
        hermes_turn_end(&data, &json!({"session_id": "h-1", "completed": false, "interrupted": true}), false).unwrap();
        let rows = records(&data);
        let turns = kinds(&rows, "agent.turn");
        assert_eq!(turns.len(), 2, "the next run has its own turn over its own calls");
        assert_eq!((turns[1]["outcome"].as_str(), turns[1]["error"].as_str()), (Some("cancelled"), Some("interrupted")));
        assert!(turns[1]["input_tokens"].is_null() && turns[1]["unknown"]["input_tokens"] == "not-reported", "a call without tokens reads null, never zero");

        hermes_call(&data, &json!({"api_request_id": "r-4", "session_id": "h-2", "response_model": "vendor/model name", "usage": {"input_tokens": 1}}), false).unwrap();
        hermes_turn_end(&data, &json!({"session_id": "h-2", "completed": true}), false).unwrap();
        let rows = records(&data);
        let named: Vec<_> = rows.iter().filter(|r| r["session_id"] == "h-2").collect();
        assert_eq!(named.len(), 2, "a model name the store refuses must not lose the call or the turn");
        assert!(named.iter().all(|r| r["model"].is_null() && r["unknown"]["model"] == "not-reported"), "{named:?}");

        hermes_turn_end(&data, &json!({}), false).unwrap();
        let rows = records(&data);
        let last = kinds(&rows, "agent.turn").into_iter().find(|t| t["session_id"].is_null()).unwrap().clone();
        assert_eq!((last["outcome"].as_str(), last["unknown"]["input_tokens"].as_str(), last["unknown"]["session_id"].as_str()), (Some("unknown"), Some("no-usage"), Some("session-unknown")));
        std::fs::remove_dir_all(d).unwrap();
    }
}

/// Project attribution reused by the separate archive hook.
pub fn archive_project(input: &Value) -> String { project(input) }
