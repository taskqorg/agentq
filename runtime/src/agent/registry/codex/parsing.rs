use super::*;
pub(super) fn ends(v:&Value)->bool {matches!(v["type"].as_str(),Some("turn.completed"|"turn.failed"|"turn.cancelled"))}
pub(super) fn native_turn_ended(ended:bool,v:&Value)->bool {ended || ends(v)}
pub(super) fn start_limit(v:&Value)->Option<StartLimit> {
    let kind=v["type"].as_str().unwrap_or_default();
    if kind.starts_with("item.") {return Some(StartLimit::Working);}
    // shortcut: Codex limit text has no structured reset; replace when native evidence supplies one.
    let m=v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
    if matches!(kind,"error"|"turn.failed") && (m.contains("usage limit") || m.contains("quota exceeded")) {return Some(StartLimit::Reached(None));} None
}
pub(super) fn final_text(events:&[Value])->Option<String> {events.iter().rev().find_map(|v|if v["type"]=="item.completed" && v["item"]["type"]=="agent_message" {v["item"]["text"].as_str().map(String::from)} else {None})}
pub(super) fn read_calls(events:&[Value])->(Vec<Call>,Option<u64>,bool) {
    let mut calls=vec![];
    for v in events.iter().filter(|v|v["type"]=="turn.completed") {
        let id=v.get("total_usage").map(Value::to_string);let u=&v["usage"];
        let read=u["cached_input_tokens"].as_u64();
        let (outcome,error)=if v["isApiErrorMessage"]==true || v["error"].is_string() {("failed","client-error")} else {("ok","none")};
        calls.push(Call {id,at_ms:v["timestamp"].as_str().or(v["created_at"].as_str()).and_then(capture::time_ms),
            input:u["input_tokens"].as_u64().zip(read).and_then(|(i,r)|i.checked_sub(r)),cache_read:read,cache_write:Some(0),output:u["output_tokens"].as_u64(),reasoning:u["reasoning_output_tokens"].as_u64(),
            model:v["message"]["model"].as_str().or_else(||v["model"].as_str()).filter(|m|model_code(m)).map(String::from),outcome,error,aggregate:v["call"]!=true});
    }
    let known=!calls.is_empty();(calls,None,known)
}
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

pub(super) fn classify_report(text:&str,events:&[Value],rollout:Option<&Path>,end_index:Option<usize>)->Report {
    let status=if limit_reached(Client::Codex,events).is_some() {("refused","usage-limit",None)}
        else if let Some(a)=auth_rejected(text) {("failed","auth-rejected",Some(a.into()))}
        else if let Some(c)=transient_failure(events) {("failed","model-capacity",Some(c))}
        else {match end_index.map(|i|&events[i]) {
            Some(v) if v["type"]=="turn.failed" => ("failed","turn-failed",None),
            Some(v) if v["type"]=="turn.cancelled" => ("cancelled","turn-cancelled",None),
            Some(_) => ("ok","none",None),None => ("unknown","no-end",None)}};
    report(Client::Codex,text,events,rollout,status,vec![])
}
