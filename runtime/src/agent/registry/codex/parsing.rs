use super::*;
pub(super) fn ends(v: &Value) -> bool {
    matches!(
        v["type"].as_str(),
        Some("turn.completed" | "turn.failed" | "turn.cancelled")
    )
}
pub(super) fn native_turn_ended(ended: bool, v: &Value) -> bool {
    ended || ends(v)
}
pub(super) fn start_limit(v: &Value) -> Option<StartLimit> {
    let kind = v["type"].as_str().unwrap_or_default();
    if kind.starts_with("item.") {
        return Some(StartLimit::Working);
    }
    // shortcut: Codex limit text has no structured reset; replace when native evidence supplies one.
    let m = v["message"]
        .as_str()
        .or(v["error"]["message"].as_str())
        .unwrap_or_default()
        .to_lowercase();
    if matches!(kind, "error" | "turn.failed")
        && (m.contains("usage limit") || m.contains("quota exceeded"))
    {
        return Some(StartLimit::Reached(None));
    }
    None
}
pub(super) fn final_text(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|v| {
        if v["type"] == "item.completed" && v["item"]["type"] == "agent_message" {
            v["item"]["text"].as_str().map(String::from)
        } else {
            None
        }
    })
}
pub(super) fn read_calls(events: &[Value]) -> (Vec<Call>, Option<u64>, bool) {
    if events.iter().any(|v|matches!(v["type"].as_str(),Some("event_msg"|"token_usage_record"|"session_meta")) || v["method"]=="thread/tokenUsage/updated") {
        let calls=super::selected_calls(events,false);let known=!calls.is_empty();return (calls,None,known);
    }
    let mut calls = vec![];
    for (index,v) in events.iter().enumerate().filter(|(_,v)| v["type"] == "turn.completed") {
        if v["response_id"].is_string() && events[index+1..].iter().any(|later|later["type"]=="turn.completed" && later["response_id"]==v["response_id"] && later["turn_id"]==v["turn_id"]) {continue;}
        let id = v["response_id"].as_str().map(String::from).or_else(||v.get("total_usage").map(Value::to_string));
        let u = &v["usage"];
        let read = u["cached_input_tokens"].as_u64();
        let (outcome, error) = if v["isApiErrorMessage"] == true || v["error"].is_string() {
            ("failed", "client-error")
        } else {
            ("ok", "none")
        };
        calls.push(Call {native_turn:v["turn_id"].as_str().map(String::from),accounting_unknown:None,
            cost_usd:crate::agent::history::live::native_cost(u).or_else(||crate::agent::history::live::native_cost(v)),
            id,
            at_ms: v["timestamp"]
                .as_str()
                .or(v["created_at"].as_str())
                .and_then(capture::time_ms),
            input: u["input_tokens"]
                .as_u64()
                .zip(read)
                .and_then(|(i, r)| i.checked_sub(r)),
            cache_read: read,
            cache_write: Some(0),
            output: u["output_tokens"].as_u64(),
            reasoning: u["reasoning_output_tokens"].as_u64(),
            model: v["message"]["model"]
                .as_str()
                .or_else(|| v["model"].as_str())
                .filter(|m| model_code(m))
                .map(String::from),
            outcome,
            error,
            aggregate: v["call"] != true,
        });
    }
    let known = !calls.is_empty();
    (calls, None, known)
}
pub(crate) fn auth_rejected(text: &str) -> Option<&'static str> {
    text.lines().find_map(|l| {
        let m = match serde_json::from_str::<Value>(l) {
            Ok(v) if matches!(v["type"].as_str(), Some("error" | "turn.failed")) => v["message"]
                .as_str()
                .or(v["error"]["message"].as_str())
                .unwrap_or_default()
                .to_lowercase(),
            Err(_) if l.contains("ERROR codex_") => l.to_lowercase(),
            _ => return None,
        };
        if m.contains("refresh token")
            && (m.contains("already used")
                || m.contains("expired")
                || m.contains("revoked")
                || m.contains("invalid"))
            || m.contains("refresh_token_")
        {
            Some("refresh token rejected")
        } else if m.contains("401 unauthorized") || m.contains("auth error: 401") {
            Some("401 Unauthorized")
        } else {
            None
        }
    })
}

/// The last turn end is a Codex `turn.failed` naming model capacity: its message.
pub(crate) fn transient_failure(events: &[Value]) -> Option<String> {
    let end = events
        .iter()
        .filter(|v| matches!(v["type"].as_str(), Some("turn.completed" | "turn.failed")))
        .last()?;
    // ponytail: only the "Selected model is at capacity" text is verified; other overload texts read as failed.
    let m = end["error"]["message"]
        .as_str()
        .filter(|_| end["type"] == "turn.failed")?;
    m.to_lowercase()
        .contains("model is at capacity")
        .then(|| m.to_string())
}

pub(super) fn classify_report(
    text: &str,
    events: &[Value],
    rollout: Option<&Path>,
    end_index: Option<usize>,
) -> Report {
    let auth_reason = auth_rejected(text);
    let mut status = if limit_reached(Client::Codex, events).is_some() {
        ("refused", "usage-limit", None)
    } else if let Some(a) = auth_reason {
        ("failed", "auth-rejected", Some(a.into()))
    } else if let Some(c) = transient_failure(events) {
        ("failed", "model-capacity", Some(c))
    } else {
        match end_index.map(|i| &events[i]) {
            Some(v) if v["type"] == "turn.failed" => ("failed", "turn-failed", None),
            Some(v) if v["type"] == "turn.cancelled" => ("cancelled", "turn-cancelled", None),
            Some(_) => ("ok", "none", None),
            None => ("unknown", "no-end", None),
        }
    };
    if let Some(detail) = end_index
        .map(|i| &events[i])
        .filter(|v| v["type"] == "turn.failed")
        .and_then(|v| v["error"]["message"].as_str())
    {
        status.2 = Some(detail.into());
    }
    let mut result=report(Client::Codex, text, events, rollout, status, vec![]);
    if result.error=="auth-rejected" {result.reason=auth_reason.map(String::from);}
    if let Some(cost)=end_index.and_then(|i|crate::agent::history::live::native_cost(&events[i])) {result.usage.cost_usd=Some(cost);}
    result

}
