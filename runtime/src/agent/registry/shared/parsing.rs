//! Existing parsing utilities shared by the native modules; execution dispatches opaque callbacks.
use super::super::super::*;

/// A turn-end event: Claude `result`, Codex `turn.completed` or `turn.failed`, Antigravity terminal `PLANNER_RESPONSE` or `turn.cancelled`.
pub(crate) fn ends(v: &Value) -> bool {
    if v["event"] == "result" { return matches!(v["result"]["status"].as_str(), Some("SUCCESS" | "ERROR" | "CANCELED" | "INTERRUPTED" | "INVALID")); }
    match v["type"].as_str() {
        Some("result" | "turn.completed" | "turn.failed" | "turn.cancelled") => true,
        Some("PLANNER_RESPONSE") => {
            matches!(v["status"].as_str(), Some("ERROR" | "CANCELLED"))
                || (v["status"] == "DONE" && v["tool_calls"].as_array().is_none_or(Vec::is_empty))
        }
        _ => false,
    }
}

/// Model use of the turn's events: Claude assistant messages (each message id once: input, cache read, cache write,
/// output, the model of most calls), Codex `turn.completed` (`input_tokens` includes `cached_input_tokens`, no cache
/// writes), Antigravity `PLANNER_RESPONSE` steps (each `step_index` once: `input_tokens`, `cache_read_tokens`,
/// `output_tokens`). A counted call without a value makes that sum unknown, never smaller.
#[cfg(test)]
pub(crate) fn usage(events: &[Value]) -> Usage {
    let (calls, count, known) = read_calls(events);
    sum_usage(&calls, count, known)
}

/// The model requests of the turn's events, the Claude message count, and whether any usage was read.
pub(crate) fn read_calls(events: &[Value]) -> (Vec<Call>, Option<u64>, bool) {
    let (mut ids, mut turns, mut calls) = (std::collections::HashSet::new(), false, vec![]);
    let mut cli_usage = false;
    for (idx, v) in events.iter().enumerate() {
        let kind = if v["event"] == "step_update" {"agy-step"} else {v["type"].as_str().unwrap_or_default()};
        let cli = kind == "agy-step";
        let codex = kind == "turn.completed";
        let ag = kind == "PLANNER_RESPONSE";
        let mut id = None;
        let counted = match kind {
            "assistant" => { id = v["message"]["id"].as_str().map(String::from); v["message"]["id"].as_str().is_some_and(|id| ids.insert(id.to_string())) }
            "turn.completed" => { turns = true; id = v.get("total_usage").map(Value::to_string); true }
            "agy-step" => {
                let step = &v["step_update"];
                if step["state"] != "DONE" || (step["step_type"] != "agent_response" && !step["usage"].is_object()) { continue; }
                let Some(n) = step["step_index"].as_u64() else {continue;};
                if events[idx+1..].iter().any(|later| later["event"] == "step_update" && later["step_update"]["step_index"].as_u64() == Some(n) && later["step_update"]["state"] == "DONE" && (later["step_update"]["step_type"] == "agent_response" || later["step_update"]["usage"].is_object())) {continue;}
                cli_usage = true; turns = true;
                id = Some(format!("agy-step-{n}"));
                ids.insert(format!("agy-step-{n}"))
            }
            "PLANNER_RESPONSE" => {
                if !matches!(v["status"].as_str(), Some("DONE" | "ERROR" | "CANCELLED")) { continue; }
                if v["step_index"].as_u64().is_some_and(|step| events[idx+1..].iter().any(|later| later["type"] == "PLANNER_RESPONSE" && later["step_index"].as_u64() == Some(step))) { continue; }
                let Some(step) = v["step_index"].as_u64() else { continue; };
                id = Some(format!("ag-step-{step}"));
                ids.insert(format!("ag-step-{step}"))
            }
            _ => false,
        };
        if !counted { continue }
        let u = if cli { &v["step_update"]["usage"] } else if codex { &v["usage"] } else if ag { v } else { &v["message"]["usage"] };
        let input = u["input_tokens"].as_u64();
        let read = if codex {
            u["cached_input_tokens"].as_u64()
        } else if ag || cli {
            u["cache_read_tokens"].as_u64().or(u["cache_read_input_tokens"].as_u64())
        } else {
            u["cache_read_input_tokens"].as_u64()
        };
        let (outcome, error) = if v["isApiErrorMessage"] == true || v["error"].is_string() { ("failed", "client-error") }
            else if ag && v["status"] == "ERROR" { ("failed", "turn-failed") }
            else if ag && v["status"] == "CANCELLED" { ("cancelled", "turn-cancelled") }
            else { ("ok", "none") };
        calls.push(Call {
            id, at_ms: v["timestamp"].as_str().or(v["created_at"].as_str()).and_then(capture::time_ms),
            input: if codex { input.zip(read).and_then(|(i, r)| i.checked_sub(r)) } else { input },
            cache_read: read,
            cache_write: if codex { Some(0) } else if ag || cli { u["cache_write_tokens"].as_u64().or(u["cache_creation_input_tokens"].as_u64()) } else { u["cache_creation_input_tokens"].as_u64() },
            output: u["output_tokens"].as_u64(),
            reasoning: if codex { u["reasoning_output_tokens"].as_u64() } else { None },
            model: v["message"]["model"].as_str().or_else(|| v["model"].as_str()).filter(|m| model_code(m)).map(String::from),
            outcome, error, aggregate: codex && v["call"] != true,
        });
    }
    let count = (!cli_usage && !ids.is_empty()).then_some(ids.len() as u64);
    let known = count.is_some() || turns;
    (calls, count, known)
}

/// A usage or quota limit reported before any work started: `Some(reset)` with the reset (unix seconds) when known;
/// None after work started or without a limit.
pub(crate) fn limit_reached(events: &[Value]) -> Option<Option<i64>> {
    events.iter().find_map(StartLimit::read).and_then(|s| match s {StartLimit::Reached(reset)=>Some(reset),_=>None})
}

/// A Codex login the auth server rejected: Codex's own stderr `ERROR codex_` line or a top-level `error`/`turn.failed`
/// event naming a rejected refresh token or a 401; worker output (`item.*`) never matches.
pub(crate) fn auth_rejected(text: &str) -> Option<&'static str> {
    text.lines().find_map(|l| {
        let m = match serde_json::from_str::<Value>(l) {
            Ok(v) if matches!(v["type"].as_str(), Some("error" | "turn.failed")) => v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase(),
            Err(_) if l.contains("ERROR codex_") => l.to_lowercase(),
            _ => return None,
        };
        if m.contains("refresh token") && (m.contains("already used") || m.contains("expired") || m.contains("revoked") || m.contains("invalid")) || m.contains("refresh_token_") {
            Some("refresh token rejected")
        } else if m.contains("401 unauthorized") || m.contains("auth error: 401") { Some("401 Unauthorized") } else { None }
    })
}

/// The last turn end is a Codex `turn.failed` naming model capacity: its message.
pub(crate) fn transient_failure(events: &[Value]) -> Option<String> {
    let end = events.iter().filter(|v| matches!(v["type"].as_str(), Some("turn.completed" | "turn.failed"))).last()?;
    // ponytail: only the "Selected model is at capacity" text is verified; other overload texts read as failed.
    let m = end["error"]["message"].as_str().filter(|_| end["type"] == "turn.failed")?;
    m.to_lowercase().contains("model is at capacity").then(|| m.to_string())
}

/// The final text of a turn: Claude's last `result`, Codex's last `agent_message`, or Antigravity's last `PLANNER_RESPONSE` content.
pub(crate) fn final_text(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|v| {
        if v["event"] == "result" { v["result"]["response"].as_str().map(String::from) }
        else if v["type"] == "result" { v["result"].as_str().map(String::from) }
        else if v["type"] == "item.completed" && v["item"]["type"] == "agent_message" { v["item"]["text"].as_str().map(String::from) }
        else if v["type"] == "PLANNER_RESPONSE" && v["status"] == "DONE" && ends(v) { v["content"].as_str().filter(|s| !s.is_empty()).map(String::from) }
        else { None }
    })
}

pub(crate) fn native_turn_ended(mut ended:bool,v:&Value)->bool {
            if (v["type"] == "PLANNER_RESPONSE" && !ends(&v)) || v["type"] == "TOOL_RESPONSE" {
                ended = false;
            } else if ends(&v) {
                ended = true;
            }
 ended
}

pub(crate) fn classify_report(client:Client,text:&str,events:&[Value],rollout:Option<&Path>,end_index:Option<usize>,failure:Option<events::AssistantFailure>,live_background:Vec<String>)->Report {
        let reached = limit_reached(events);
        let end=end_index.map(|i|&events[i]);
        let (mut detail, auth, capacity) = (None, auth_rejected(text), transient_failure(events));
        let (outcome, error) = if reached.is_some() { ("refused", "usage-limit") }
            else if let Some(a) = auth { detail = Some(a.to_string()); ("failed", "auth-rejected") }
            else if capacity.is_some() { detail = capacity; ("failed", "model-capacity") }
            else if let Some(failure) = failure { detail = failure.detail; ("failed", failure.error) }
            else {
                match end {
                    Some(v) if v["event"] == "result" && matches!(v["result"]["status"].as_str(), Some("CANCELED" | "INTERRUPTED")) => ("cancelled", "turn-cancelled"),
                    Some(v) if v["event"] == "result" && v["result"]["status"] != "SUCCESS" => ("failed", "turn-failed"),
                    Some(v) if v["type"] == "turn.failed" || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "ERROR") => ("failed", "turn-failed"),
                    Some(v) if v["type"] == "turn.cancelled" || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "CANCELLED") => ("cancelled", "turn-cancelled"),
                    Some(v) if (v["type"] == "result" && v["is_error"] == true) || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "ERROR") => ("failed", "client-error"),
                    Some(_) => ("ok", "none"),
                    None => ("unknown", "no-end"),
                }
            };
        let (calls, count, known) = read_calls(events);
        Report { session: session_of(client, text), usage: sum_usage(&calls, count, known), calls, process: ProcessUse::default(), limit: snapshot(client, events, rollout), limits: limits::read(client, events, rollout), reached, background_job: !live_background.is_empty(), live_background, outcome, error, detail,
            text: final_text(events), exit: None, started_ms: unix_ms(), duration_us: 0, recorded: Ok(()) }
    }

impl StartLimit {
    // The first decisive record fixes whether a limit refused admission or interrupted work.
    pub(crate) fn read(v: &Value) -> Option<Self> {
        if v["type"]=="rate_limit_event" && v["rate_limit_info"]["status"]=="rejected" {return Some(Self::Reached(v["rate_limit_info"]["resetsAt"].as_i64()));}
        if v["event"] == "step_update" && (v["step_update"]["step_type"] == "tool" || v["step_update"]["usage"].is_object()) { return Some(Self::Working); }
        if v["event"] == "result" {
            let msg = v["result"]["error"].as_str().unwrap_or_default().to_lowercase();
            return Some(if v["result"]["status"] != "SUCCESS" && ["usage limit", "quota exceeded", "rate limit"].iter().any(|s| msg.contains(s)) {Self::Reached(None)} else {Self::Working});
        }
        let kind = v["type"].as_str().unwrap_or_default();
        if kind == "assistant" {
            if v["apiError"] == "usage_limit_reached" || v["quotaLimits"]["status"] == "rejected" {
                return Some(Self::Reached(limits::claude_info(v).and_then(|info|info["resetsAt"].as_i64())));
            }
            return Some(Self::Working);
        }
        if kind == "PLANNER_RESPONSE" {
            if v["status"] == "ERROR" {
                let msg = v["content"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
                if msg.contains("usage limit") || msg.contains("quota exceeded") || msg.contains("rate limit") {
                    return Some(Self::Reached(v["quotaLimits"]["resetsAt"].as_i64()));
                }
            }
            return Some(Self::Working);
        }
        if kind.starts_with("item.") || kind == "TOOL_RESPONSE" { return Some(Self::Working); }
        // ponytail: Codex limit text matched by words; it reports no reset. A structured field replaces this when one exists.
        let message = v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
        if matches!(kind, "error" | "turn.failed") && (message.contains("usage limit") || message.contains("quota exceeded")) { return Some(Self::Reached(None)); }
        None
    }
}
