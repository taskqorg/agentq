use super::*;
pub(crate) fn ends(v: &Value) -> bool {
    if v["event"] == "result" { return matches!(v["result"]["status"].as_str(), Some("SUCCESS" | "ERROR" | "CANCELED" | "INTERRUPTED" | "INVALID")); }
    match v["type"].as_str() {
        Some("turn.failed" | "turn.cancelled") => true,
        Some("PLANNER_RESPONSE") => {
            matches!(v["status"].as_str(), Some("ERROR" | "CANCELLED"))
                || (v["status"] == "DONE" && v["tool_calls"].as_array().is_none_or(Vec::is_empty))
        }
        _ => false,
    }
}

pub(super) fn start_limit(v:&Value)->Option<StartLimit> {
    if v["event"]=="step_update" && (v["step_update"]["step_type"]=="tool" || v["step_update"]["usage"].is_object()) {return Some(StartLimit::Working);}
    if v["event"]=="result" {
        let m=v["result"]["error"].as_str().unwrap_or_default().to_lowercase();
        return Some(if v["result"]["status"]!="SUCCESS" && ["usage limit","quota exceeded","rate limit"].iter().any(|s|m.contains(s)) {StartLimit::Reached(None)} else {StartLimit::Working});
    }
    if v["type"]=="PLANNER_RESPONSE" {
        if v["status"]=="ERROR" {
            let m=v["content"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
            if ["usage limit","quota exceeded","rate limit"].iter().any(|s|m.contains(s)) {return Some(StartLimit::Reached(v["quotaLimits"]["resetsAt"].as_i64()));}
        } return Some(StartLimit::Working);
    }
    if v["type"]=="TOOL_RESPONSE" {return Some(StartLimit::Working);} None
}
pub(crate) fn read_calls(events: &[Value]) -> (Vec<Call>, Option<u64>, bool) {
    let (mut ids, mut turns, mut calls) = (std::collections::HashSet::new(), false, vec![]);
    let mut cli_usage = false;
    for (idx, v) in events.iter().enumerate() {
        let kind = if v["event"] == "step_update" {"agy-step"} else {v["type"].as_str().unwrap_or_default()};
        let cli = kind == "agy-step";

        let ag = kind == "PLANNER_RESPONSE";
        let mut id = None;
        let counted = match kind {
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
        let u = if cli { &v["step_update"]["usage"] } else { v };
        let input = u["input_tokens"].as_u64();
        let read = if ag || cli {
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
            input,
            cache_read: read,
            cache_write: if ag || cli { u["cache_write_tokens"].as_u64().or(u["cache_creation_input_tokens"].as_u64()) } else { u["cache_creation_input_tokens"].as_u64() },
            output: u["output_tokens"].as_u64(),
            reasoning: None,
            model: v["message"]["model"].as_str().or_else(|| v["model"].as_str()).filter(|m| model_code(m)).map(String::from),
            outcome, error, aggregate: false,
        });
    }
    let count = (!cli_usage && !ids.is_empty()).then_some(ids.len() as u64);
    let known = count.is_some() || turns;
    (calls, count, known)
}

pub(crate) fn native_turn_ended(mut ended:bool,v:&Value)->bool {
            if (v["type"] == "PLANNER_RESPONSE" && !ends(&v)) || v["type"] == "TOOL_RESPONSE" {
                ended = false;
            } else if ends(&v) {
                ended = true;
            }
 ended
}

pub(super) fn final_text(events:&[Value])->Option<String> {events.iter().rev().find_map(|v|if v["event"]=="result" {v["result"]["response"].as_str().map(String::from)} else if v["type"]=="PLANNER_RESPONSE" && v["status"]=="DONE" && ends(v) {v["content"].as_str().filter(|s|!s.is_empty()).map(String::from)} else {None})}
pub(super) fn classify_report(text:&str,events:&[Value],rollout:Option<&Path>,end_index:Option<usize>)->Report {
    let status=if limit_reached(Client::Antigravity,events).is_some() {("refused","usage-limit",None)} else {
        match end_index.map(|i|&events[i]) {
            Some(v) if v["event"]=="result" && matches!(v["result"]["status"].as_str(),Some("CANCELED"|"INTERRUPTED")) => ("cancelled","turn-cancelled",None),
            Some(v) if v["event"]=="result" && v["result"]["status"]!="SUCCESS" => ("failed","turn-failed",None),
            Some(v) if v["type"]=="turn.failed" || (v["type"]=="PLANNER_RESPONSE" && v["status"]=="ERROR") => ("failed","turn-failed",None),
            Some(v) if v["type"]=="turn.cancelled" || (v["type"]=="PLANNER_RESPONSE" && v["status"]=="CANCELLED") => ("cancelled","turn-cancelled",None),
            Some(_) => ("ok","none",None),None => ("unknown","no-end",None)}};
    report(Client::Antigravity,text,events,rollout,status,vec![])
}
