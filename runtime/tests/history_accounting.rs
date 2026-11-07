//! M12.20: one native accounting selection across pages, transports and request revisions.
use agentq_runtime::agent::history::{History,StepKind};
use serde_json::json;
use std::{fs,io::Write};
fn path(n:&str)->std::path::PathBuf {std::env::temp_dir().join(format!("agentq-accounting-{}-{n}.jsonl",std::process::id()))}
#[test]
fn latest_usage_missing_counter_stays_unknown_after_recovery() {
 let path=path("unknown");
 let row=|usage|json!({"type":"assistant","sessionId":"s","message":{"id":"request","content":[],"usage":usage}});
 fs::write(&path,format!("{}\n",row(json!({"input_tokens":10,"output_tokens":1})))).unwrap();
 let mut h=History::new("claude-native","source","s").unwrap();h.steps(&path,u64::MAX).unwrap();
 let saved=h.checkpoint();
 fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{}\n",row(json!({"output_tokens":3}))).as_bytes()).unwrap();
 let steps=History::recover(&saved).unwrap().steps(&path,u64::MAX).unwrap().steps;
 let usage=steps.iter().find(|s|s.kind==StepKind::Usage).unwrap();

 assert!(usage.data["input"].is_null(),"latest missing input must be unknown");
 assert!(usage.data["unknown"]["input"].is_string());fs::remove_file(path).unwrap();
}
#[test]
fn raw_usage_must_not_duplicate_identified_request_and_legacy_snapshot() {
 let path=path("dup");
 let legacy=json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2},"total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}}});
 let request=json!({"type":"token_usage_record","payload":{"thread_id":"s","turn_id":"t","response_id":"r","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}});
 fs::write(&path,format!("{}\n{}\n{}\n",json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}}),legacy,request)).unwrap();
 let mut h=History::new("codex-rollout","source","s").unwrap();
 let page=h.steps(&path,u64::MAX).unwrap();
 let usage:Vec<_>=page.steps.iter().filter(|s|s.kind==StepKind::Usage).collect();

 assert_eq!(usage.len(),1,"one identified request supersedes its legacy snapshot");fs::remove_file(path).unwrap();
}
#[test]
fn appserver_usage_visible_in_history() {
 let path=path("app");
 let row=json!({"method":"thread/tokenUsage/updated","params":{"threadId":"s","turnId":"t","tokenUsage":{"last":{"inputTokens":10,"cachedInputTokens":0,"outputTokens":2,"reasoningOutputTokens":0},"total":{"inputTokens":10,"cachedInputTokens":0,"outputTokens":2,"reasoningOutputTokens":0},"modelContextWindow":1000}}});
 fs::write(&path,format!("{row}\n")).unwrap();
 let page=History::new("codex-app-server","source","s").unwrap().steps(&path,u64::MAX).unwrap();

 assert!(page.steps.iter().any(|s|s.kind==StepKind::Usage));
 assert!(page.steps.iter().any(|s|s.kind==StepKind::Context && s.data["window"]==1000));fs::remove_file(path).unwrap();
}

#[test]
fn identified_usage_retires_delivered_legacy_across_recovered_pages() {
    let path=path("retirement");
    let header=json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn"}});
    let legacy=json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2},"total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}}});
    fs::write(&path,format!("{header}\n{legacy}\n")).unwrap();let mut h=History::new("codex-rollout","source","session").unwrap();
    h.steps(&path,1).unwrap();let first=h.steps(&path,1).unwrap();let old=first.steps.iter().find(|s|s.kind==StepKind::Usage).unwrap();
    let request=json!({"type":"token_usage_record","payload":{"thread_id":"session","turn_id":"turn","response_id":"request","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}});
    fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{request}\n").as_bytes()).unwrap();
    let second=History::recover(&first.checkpoint).unwrap().steps(&path,u64::MAX).unwrap();
    let retired=second.steps.iter().find(|s|s.id==old.id).unwrap();assert_eq!(retired.state,"superseded");assert!(retired.revision>old.revision);assert_eq!(retired.data["active"],false);assert!(retired.data["input"].is_null());assert_eq!(retired.origins.len(),2);
    let mut client=std::collections::BTreeMap::new();for s in first.steps.into_iter().chain(second.steps){client.insert(s.id.clone(),s);}
    assert_eq!(client.values().filter(|s|s.kind==StepKind::Usage && s.data["active"]!=false).count(),1);
    let mut recovered=History::recover(&second.checkpoint).unwrap();assert!(recovered.steps(&path,u64::MAX).unwrap().steps.is_empty());fs::remove_file(path).unwrap();
}

#[test]
fn latest_usage_nulls_replace_cost_time_and_model() {
    let path=path("null-fields");let row=|message,timestamp|json!({"type":"assistant","sessionId":"session","timestamp":timestamp,"message":message});
    let first=row(json!({"id":"request","model":"fixture-model","usage":{"input_tokens":10,"output_tokens":2,"cost_usd":0.1}}),json!("2026-11-07T00:00:00Z"));
    let next=row(json!({"id":"request","usage":{"output_tokens":3}}),serde_json::Value::Null);
    fs::write(&path,format!("{first}\n")).unwrap();let saved=History::new("claude-native","source","session").unwrap().steps(&path,u64::MAX).unwrap().checkpoint;
    fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(format!("{next}\n").as_bytes()).unwrap();
    let page=History::recover(&saved).unwrap().steps(&path,u64::MAX).unwrap();let usage=page.steps.iter().find(|s|s.kind==StepKind::Usage).unwrap();
    for field in ["input","model","time","cost_usd"] {assert!(usage.data[field].is_null(),"{field}");assert!(usage.data["unknown"][field].is_string(),"{field}");}
    assert_eq!(usage.data["output"],3);fs::remove_file(path).unwrap();
}
