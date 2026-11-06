//! M12.20: accounting and native context agree across turns and recovered pages.
use agentq_runtime::agent::{self,history::{History,StepKind,Step}};
use serde_json::{json,Value};
use std::{fs,io::Write,collections::BTreeMap};
fn path(n:&str)->std::path::PathBuf{std::env::temp_dir().join(format!("agentq-usage-scope-{}-{n}.jsonl",std::process::id()))}
fn append(path:&std::path::Path,rows:&[Value]){let mut f=fs::OpenOptions::new().create(true).append(true).open(path).unwrap();for row in rows{writeln!(f,"{row}").unwrap();}}
fn header(t:&str)->Value{json!({"type":"event_msg","payload":{"type":"task_started","turn_id":t}})}
fn legacy()->Value{json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2,"total_tokens":12},"total_token_usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2,"total_tokens":12},"model_context_window":1000}}})}
#[test]
fn retirement_in_second_turn_does_not_remove_first_turn_usage() {
 for late in [false,true] {
 let path=path(if late{"two-turns-late"}else{"two-turns-same-page"});append(&path,&[json!({"type":"session_meta","payload":{"id":"session"}}),header("t1"),legacy()]);
 let mut h=History::new("codex-rollout","source","session").unwrap();let first=h.steps(&path,u64::MAX).unwrap();
 let mut client=BTreeMap::<String,Step>::new();for s in first.steps{client.insert(s.id.clone(),s);}
 let request=json!({"type":"token_usage_record","payload":{"thread_id":"session","turn_id":"t2","response_id":"r2","usage":{"input_tokens":5,"cached_input_tokens":0,"output_tokens":1}}});
 append(&path,&[header("t2"),legacy()]);if !late{append(&path,&[request.clone()]);}
 h=History::recover(&first.checkpoint).unwrap();let second=h.steps(&path,u64::MAX).unwrap();for s in second.steps{client.insert(s.id.clone(),s);}
 if late{append(&path,&[request]);}
 h=History::recover(&second.checkpoint).unwrap();let third=h.steps(&path,u64::MAX).unwrap();for s in third.steps{client.insert(s.id.clone(),s);}
 let count=|s:&&Step|s.kind==StepKind::Usage && s.data["active"]!=false;
 let paged=client.values().filter(count).count();let fresh=History::new("codex-rollout","source","session").unwrap().steps(&path,u64::MAX).unwrap().steps.iter().filter(count).count();
 assert_eq!(paged,fresh,"an inactive revision for t2 cannot retire t1");assert_eq!(fresh,2);
 let live=agent::runtime("codex").unwrap().history("session",None,Some(&path),None).unwrap().live_data(None).unwrap();assert_eq!(live.calls.len(),fresh);fs::remove_file(path).unwrap();
 }
}
#[test]
fn live_context_uses_reported_native_context() {
 let path=path("context");append(&path,&[json!({"type":"session_meta","payload":{"id":"session"}}),header("t1"),legacy()]);
 let mut s=agent::runtime("codex").unwrap().history("session",None,Some(&path),None).unwrap();let page=s.steps(u64::MAX).unwrap();
 let context=page.steps.iter().find(|s|s.kind==StepKind::Context).unwrap();let live=s.live_data(None).unwrap();
 println!("CONTEXT step={} live={:?} source={}",context.data["used"],live.context.used,live.context.source);
 assert_eq!(live.context.used,context.data["used"].as_u64(),"native context is available; no input estimate is needed");assert_eq!(live.context.window,Some(1000));assert_eq!(live.context.source,"native-context");fs::remove_file(path).unwrap();
}

#[test]
fn unscoped_legacy_is_not_retired_by_an_unassociated_request() {
 let path=path("unassociated");append(&path,&[legacy()]);
 let mut history=History::new("codex-rollout","source","session").unwrap();let first=history.steps(&path,u64::MAX).unwrap();
 append(&path,&[json!({"type":"token_usage_record","payload":{"thread_id":"session","response_id":"r","usage":{"input_tokens":5,"output_tokens":1}}})]);
 let mut recovered=History::recover(&first.checkpoint).unwrap();let page=recovered.steps(&path,u64::MAX).unwrap();assert!(!page.steps.iter().any(|s|s.data["active"]==false));
 let fresh=History::new("codex-rollout","source","session").unwrap().steps(&path,u64::MAX).unwrap();assert_eq!(fresh.steps.iter().filter(|s|s.kind==StepKind::Usage).count(),2);fs::remove_file(path).unwrap();
}
#[test]
fn appserver_live_context_keeps_latest_missing_native_measurement_unknown() {
 let path=path("app-context");let row=|last:Value|json!({"method":"thread/tokenUsage/updated","params":{"threadId":"session","turnId":"t","tokenUsage":{"last":last,"total":{"inputTokens":10,"outputTokens":2},"modelContextWindow":1000}}});
 append(&path,&[json!({"type":"session_meta","payload":{"id":"session"}}),row(json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":2,"totalTokens":12}))]);
 let session=agent::runtime("codex").unwrap().history("session",None,Some(&path),None).unwrap();let first=session.live_data(None).unwrap();assert_eq!(first.context.used,Some(12));assert_eq!(first.context.window,Some(1000));
 append(&path,&[row(json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":2}))]);let last=session.live_data(None).unwrap();assert!(last.context.used.is_none());assert!(last.context.unknown.contains_key("used"));assert_eq!(last.context.source,"native-context");fs::remove_file(path).unwrap();
}
#[test]
fn identified_request_ids_are_scoped_to_native_turns_and_keep_raw_identity() {
 let path=path("request-turns");let request=|turn:&str,usage:Value|json!({"type":"token_usage_record","payload":{"thread_id":"session","turn_id":turn,"response_id":"r","usage":usage}});
 append(&path,&[json!({"type":"session_meta","payload":{"id":"session"}}),header("t1"),request("t1",json!({"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}))]);
 let mut history=History::new("codex-rollout","source","session").unwrap();let first=history.steps(&path,u64::MAX).unwrap();let mut client=BTreeMap::new();for s in first.steps{client.insert(s.id.clone(),s);}
 append(&path,&[header("t2"),request("t2",json!({"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}))]);let mut history=History::recover(&first.checkpoint).unwrap();for s in history.steps(&path,u64::MAX).unwrap().steps{client.insert(s.id.clone(),s);}
 let active:Vec<_>=client.values().filter(|s|s.kind==StepKind::Usage).collect();assert_eq!(active.len(),2);assert!(active.iter().all(|s|s.data["id"]=="r"));assert_ne!(active[0].data["native_turn"],active[1].data["native_turn"]);
 let live=agent::runtime("codex").unwrap().history("session",None,Some(&path),None).unwrap().live_data(None).unwrap();assert_eq!(live.calls.len(),2);assert!(live.calls.iter().all(|c|c["id"]=="r"));
 append(&path,&[request("t2",json!({"output_tokens":3}))]);let fresh=History::new("codex-rollout","source","session").unwrap().steps(&path,u64::MAX).unwrap();let usage:Vec<_>=fresh.steps.iter().filter(|s|s.kind==StepKind::Usage).collect();assert_eq!(usage.len(),2);assert_eq!(usage.iter().find(|s|s.data["native_turn"]=="t1").unwrap().data["input"],10);assert!(usage.iter().find(|s|s.data["native_turn"]=="t2").unwrap().data["input"].is_null());fs::remove_file(path).unwrap();
}
#[test]
fn unassociated_observations_revise_accounting_to_unknown_without_retiring_evidence() {
 for request_first in [false,true] {
 let path=path(if request_first{"overlap-reverse"}else{"overlap-forward"});let request=json!({"type":"token_usage_record","payload":{"thread_id":"session","response_id":"r","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}}});
 let (first,second)=if request_first{(request,legacy())}else{(legacy(),request)};append(&path,&[json!({"type":"session_meta","payload":{"id":"session"}}),first]);
 let mut history=History::new("codex-rollout","source","session").unwrap();let first=history.steps(&path,u64::MAX).unwrap();let mut client=BTreeMap::new();for s in first.steps{client.insert(s.id.clone(),s);}
 append(&path,&[second]);let mut history=History::recover(&first.checkpoint).unwrap();for s in history.steps(&path,u64::MAX).unwrap().steps{client.insert(s.id.clone(),s);}
 let usage:Vec<_>=client.values().filter(|s|s.kind==StepKind::Usage).collect();assert_eq!(usage.len(),2);assert!(usage.iter().all(|s|s.data["active"]==true && s.data["countable"]==false && s.data["input"].is_null() && s.data["unknown"]["accounting"]=="unassociated-usage-overlap"));assert!(usage.iter().any(|s|s.origins.len()==2));
 let rt=agent::runtime("codex").unwrap();let live=rt.history("session",None,Some(&path),None).unwrap().live_data(None).unwrap();assert_eq!(live.calls.len(),2);assert_eq!(live.unknown["usage"],"unassociated-usage-overlap");assert!(live.calls.iter().all(|c|c["countable"]==false && c["input"].is_null()));
 let report=rt.classify(&fs::read_to_string(&path).unwrap());assert_eq!(report.calls.len(),2);assert!(report.calls.iter().all(|c|c.input==Some(10) && c.accounting_unknown==Some("unassociated-usage-overlap")),"native buckets remain evidence, never independent count authority");assert!(report.usage.input.is_none() && report.usage.calls.is_none());assert_eq!(report.usage.unknown,Some("unassociated-usage-overlap"));fs::remove_file(path).unwrap();
 }
}
