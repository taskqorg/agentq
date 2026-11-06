use super::*;
pub(super) fn ends(v: &Value) -> bool {
    v["type"] == "result"
}
pub(super) fn native_turn_ended(ended: bool, v: &Value) -> bool {
    ended || ends(v)
}
pub(super) fn start_limit(v: &Value) -> Option<StartLimit> {
    if v["type"] == "rate_limit_event" && v["rate_limit_info"]["status"] == "rejected" {
        return Some(StartLimit::Reached(
            v["rate_limit_info"]["resetsAt"].as_i64(),
        ));
    }
    if v["type"] == "assistant" {
        return Some(
            if v["apiError"] == "usage_limit_reached" || v["quotaLimits"]["status"] == "rejected" {
                StartLimit::Reached(claude_info(v).and_then(|i| i["resetsAt"].as_i64()))
            } else {
                StartLimit::Working
            },
        );
    }
    None
}
pub(super) fn final_text(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|v| {
        (v["type"] == "result")
            .then(|| v["result"].as_str().map(String::from))
            .flatten()
    })
}
pub(super) fn read_calls(events: &[Value]) -> (Vec<Call>, Option<u64>, bool) {
    let mut ids = std::collections::HashSet::new();
    let mut calls = vec![];
    for (index,v) in events.iter().enumerate() {
        if v["type"] != "assistant" {
            continue;
        }
        if events[index+1..].iter().any(|later|later["type"]=="assistant" && later["message"]["id"]==v["message"]["id"]) {continue;}
        let Some(id) = v["message"]["id"]
            .as_str()
            .filter(|id| ids.insert(id.to_string()))
        else {
            continue;
        };
        let u = &v["message"]["usage"];
        let (outcome, error) = if v["isApiErrorMessage"] == true || v["error"].is_string() {
            ("failed", "client-error")
        } else {
            ("ok", "none")
        };
        calls.push(Call {
            cost_usd:crate::agent::history::live::native_cost(u).or_else(||crate::agent::history::live::native_cost(v)),
            id: Some(id.into()),
            at_ms: v["timestamp"]
                .as_str()
                .or(v["created_at"].as_str())
                .and_then(capture::time_ms),
            input: u["input_tokens"].as_u64(),
            cache_read: u["cache_read_input_tokens"].as_u64(),
            cache_write: u["cache_creation_input_tokens"].as_u64(),
            output: u["output_tokens"].as_u64(),
            reasoning: u["output_tokens_details"]["thinking_tokens"].as_u64().or(u["reasoning_tokens"].as_u64()),
            model: v["message"]["model"]
                .as_str()
                .or_else(|| v["model"].as_str())
                .filter(|m| model_code(m))
                .map(String::from),
            outcome,
            error,
            aggregate: false,
        });
    }
    let count = (!ids.is_empty()).then_some(ids.len() as u64);
    let known = count.is_some();
    (calls, count, known)
}
pub(super) fn classify_report(
    text: &str,
    events: &[Value],
    rollout: Option<&Path>,
    end_index: Option<usize>,
    failure: Option<events::AssistantFailure>,
    live: Vec<String>,
) -> Report {
    let mut status = if limit_reached(Client::Claude, events).is_some() {
        ("refused", "usage-limit", None)
    } else if let Some(f) = failure {
        ("failed", f.error, f.detail)
    } else {
        match end_index.map(|i| &events[i]) {
            Some(v) if v["is_error"] == true => ("failed", "client-error", None),
            Some(_) => ("ok", "none", None),
            None => ("unknown", "no-end", None),
        }
    };
    if let Some(detail) = end_index
        .map(|i| &events[i])
        .filter(|v| v["type"] == "result")
        .and_then(|v| v["subtype"].as_str())
    {
        status.2 = Some(detail.into());
    }
    let mut result=report(Client::Claude, text, events, rollout, status, live);
    if let Some(cost)=end_index.and_then(|i|crate::agent::history::live::native_cost(&events[i])) {result.usage.cost_usd=Some(cost);}
    result
}
