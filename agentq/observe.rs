//! `agent.turn` and `agent.call` records of manual and PM sessions (agentq.md M6.11, docs/spec/TELEMETRY.md § Session
//! records). The runtime builds them; this module finds the new transcript bytes and writes them.
use agentq_runtime::agent::{Context, append_records, session_records};
use serde_json::{Value, json};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Bytes one pass reads per file; the next `Stop` continues.
const PASS_BYTES: u64 = 8 << 20;

/// New complete lines of `file` from its saved offset, and the offset after them. A shorter file reads from the start.
fn read_new(mark: &Path, file: &Path) -> Result<(String, u64, u64), String> {
    let err = |e: std::io::Error| format!("transcript: {e}");
    let mut f = std::fs::File::open(file).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    let saved = std::fs::read_to_string(mark).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    let start = if saved > len { 0 } else { saved };
    f.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut bytes = vec![];
    f.take(PASS_BYTES).read_to_end(&mut bytes).map_err(err)?;
    // An unterminated last line is still being written: it waits for the next pass.
    bytes.truncate(bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1));
    let end = start + bytes.len() as u64;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), start, end))
}

/// The transcript of the hook input and, for Claude, its subagent transcripts.
fn files(runtime: &str, input: &Value) -> Vec<PathBuf> {
    let Some(main) = input["transcript_path"].as_str().filter(|p| !p.is_empty()).map(PathBuf::from) else { return vec![] };
    let mut out = vec![main.clone()];
    if runtime == "claude" {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(main.with_extension("").join("subagents")).into_iter().flatten().flatten()
            .map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        subs.sort();
        out.extend(subs);
    }
    out
}

/// One `Stop`: write one `agent.turn` and its `agent.call` records. A launched session writes none (M6.9).
pub fn stop(data: &Path, runtime: &str, input: &Value, launched: bool) -> Result<(), String> {
    if launched { return Ok(()); }
    let session = input["session_id"].as_str().filter(|s| !s.is_empty()).ok_or("Stop hook has no session_id")?;
    let files = files(runtime, input);
    // A Claude `Stop` without a transcript is already a recorded failure of the `agentq.model` path.
    if files.is_empty() && runtime == "claude" { return Ok(()); }
    let state = data.join("observe");
    std::fs::create_dir_all(&state).map_err(|e| format!("observe state: {e}"))?;
    let (mut text, mut marks, mut position) = (String::new(), vec![], String::new());
    for file in &files {
        let mark = state.join(&crate::model::event_id("transcript", &file.to_string_lossy())[..16]);
        let (new, start, end) = read_new(&mark, file)?;
        if !new.is_empty() {
            text.push_str(&new);
            position.push_str(&format!("{start}-{end}:"));
            marks.push((mark, end));
        }
    }
    // A `Stop` that found nothing new repeats an earlier one, unless no transcript exists to read.
    if text.is_empty() && !files.is_empty() { return Ok(()); }
    if files.is_empty() { position = input["turn_id"].as_str().unwrap_or("no-turn").into(); }
    let cwd = input["cwd"].as_str().filter(|c| !c.is_empty()).map(PathBuf::from).or_else(|| std::env::current_dir().ok());
    let project = cwd.and_then(|c| crate::model::project(&c).ok()).unwrap_or_else(|| "other".into());
    let ctx = Context { product: "agentq", version: env!("CARGO_PKG_VERSION"), project: &project, task: None, route: Some("hook"), account: "default" };
    let key = crate::model::event_id("turn", &format!("{session}:{position}"));
    let events = session_records(runtime, &ctx, session, &text, &key, (crate::telemetry::now_ns() / 1_000_000) as u64)?;
    let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3"))?;
    append_records(&store, &events)?;
    for (mark, end) in marks { coreq::write_atomic(&mark, &json!(end).to_string()).map_err(|e| format!("observe state: {e}"))?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-observe-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn records(data: &Path) -> Vec<Value> {
        let store = coreq::telemetry::Store::open(&data.join("observations.sqlite3")).unwrap();
        store.export(0, 0, 100).unwrap()["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()).collect()
    }
    fn kinds<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> { rows.iter().filter(|r| r["kind"] == kind).collect() }

    // Chain: agentq.md M6.11, specq Telemetry contract § Capture (A4). Risk: a manual Claude session writes other
    // records than a launched one, counts a call twice on a later Stop, or leaks prompt, output, tool input or a path.
    #[test]
    fn claude_stop_writes_the_runtime_records_with_metadata_only() {
        let d = dir("claude");
        let (data, transcript) = (d.join("data"), d.join("private-folder-name.jsonl"));
        std::fs::create_dir_all(&data).unwrap();
        let line = |id: &str, tokens: u64, at: &str| json!({"type": "assistant", "timestamp": at, "cwd": "/private/secret-cwd", "message": {"id": id, "model": "claude-opus-5-5",
            "content": [{"type": "text", "text": "SECRET-OUTPUT"}], "usage": {"input_tokens": tokens, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4}}}).to_string() + "\n";
        let prompt = json!({"type": "user", "message": {"content": "SECRET-PROMPT"}}).to_string() + "\n";
        let tool = json!({"type": "assistant", "message": {"id": "m0", "model": "<synthetic>", "content": [{"type": "tool_use", "input": {"command": "SECRET-TOOL-INPUT"}}]}}).to_string() + "\n";
        std::fs::write(&transcript, prompt + &tool + &line("m1", 10, "2026-10-19T08:32:09.030Z") + &line("m1", 10, "2026-10-19T08:32:09.030Z") + &line("m2", 20, "2026-10-19T08:32:10.000Z") + "{\"partial").unwrap();
        let input = json!({"session_id": "s-1", "transcript_path": transcript, "cwd": "/private/secret-cwd"});
        stop(&data, "claude", &input, false).unwrap();
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

        stop(&data, "claude", &input, false).unwrap();
        assert_eq!(records(&data).len(), 3, "a Stop with nothing new writes nothing");
        std::fs::write(&transcript, std::fs::read_to_string(&transcript).unwrap().replace("{\"partial", "") + &line("m3", 5, "2026-10-19T08:33:00.000Z")).unwrap();
        stop(&data, "claude", &input, false).unwrap();
        let rows = records(&data);
        assert_eq!((kinds(&rows, "agent.turn").len(), kinds(&rows, "agent.call").len()), (2, 3), "the next Stop adds its own turn and call");
        std::fs::write(&transcript, line("m1", 10, "2026-10-19T08:32:09.030Z")).unwrap();
        stop(&data, "claude", &input, false).unwrap();
        assert_eq!(kinds(&records(&data), "agent.call").len(), 3, "a rewritten transcript never counts a call twice");
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M6.11, M6.9, M4.6. Risk: Codex manual sessions stay invisible, a launched session counts twice, or a client
    // without a transcript reads as zero tokens.
    #[test]
    fn codex_stop_reads_the_rollout_and_launched_sessions_write_nothing() {
        let d = dir("codex");
        let (data, rollout) = (d.join("data"), d.join("rollout.jsonl"));
        std::fs::create_dir_all(&data).unwrap();
        let usage = |i: u64, t: u64| json!({"type": "event_msg", "timestamp": "2026-10-19T08:32:09.030Z", "payload": {"type": "token_count", "info": {
            "last_token_usage": {"input_tokens": i, "cached_input_tokens": 1, "output_tokens": 2, "reasoning_output_tokens": 1},
            "total_token_usage": {"input_tokens": t, "cached_input_tokens": 1, "output_tokens": 2, "reasoning_output_tokens": 1}}}}).to_string() + "\n";
        let context = json!({"type": "turn_context", "payload": {"model": "gpt-5.5-codex", "cwd": "/private/secret"}}).to_string() + "\n";
        std::fs::write(&rollout, context + &usage(10, 10)).unwrap();
        let input = json!({"session_id": "t-1", "transcript_path": rollout, "cwd": "/private/secret"});
        stop(&data, "codex", &input, true).unwrap();
        assert!(!data.join("observations.sqlite3").exists(), "a launcher records its own session");
        stop(&data, "codex", &input, false).unwrap();
        let rows = records(&data);
        let (turns, calls) = (kinds(&rows, "agent.turn"), kinds(&rows, "agent.call"));
        assert_eq!((turns.len(), calls.len()), (1, 1));
        assert_eq!((calls[0]["input_tokens"].as_u64(), calls[0]["cache_read_tokens"].as_u64(), calls[0]["reasoning_tokens"].as_u64(), calls[0]["model"].as_str()), (Some(9), Some(1), Some(1), Some("gpt-5.5-codex")));
        assert_eq!((turns[0]["input_tokens"].as_u64(), turns[0]["reasoning_tokens"].as_u64()), (Some(9), Some(1)));
        assert!(!serde_json::to_string(&rows).unwrap().contains("secret"));

        stop(&data, "codex", &json!({"session_id": "t-2", "turn_id": "turn-1"}), false).unwrap();
        let rows = records(&data);
        let turn = kinds(&rows, "agent.turn").into_iter().find(|t| t["session_id"] == "t-2").unwrap().clone();
        assert!(turn["input_tokens"].is_null() && turn["unknown"]["input_tokens"] == "no-transcript", "no transcript reads null with a reason, never zero");
        assert!(stop(&data, "codex", &json!({}), false).is_err(), "a Stop without a session is a recorded hook failure");
        std::fs::remove_dir_all(d).unwrap();
    }
}
