//! M12.20: native facts remain attributable, deduplicated and separate from telemetry content.
use agentq_runtime::agent::{self,history::{StepKind,live::{LiveEvent,statusline,write_statusline}}};
use serde_json::{Value,json};
use std::fs;
fn root(name:&str)->std::path::PathBuf {let p=std::env::temp_dir().join(format!("agentq-live-{}-{name}",std::process::id()));fs::create_dir_all(&p).unwrap();p}

#[test]
fn statusline_joins_native_facts_and_files_stay_per_turn() {
    let root=root("facts");let project=root.join("projects/project");fs::create_dir_all(&project).unwrap();
    let mut rows=vec![json!({"type":"user","sessionId":"session","promptId":"p1","version":"2.fixture","isMeta":false,"isSidechain":false,"message":{"content":"first"}}),
        json!({"type":"assistant","sessionId":"session","message":{"id":"request","model":"fixture-model","content":[{"type":"thinking","thinking":"SECRET_PLAN"}],"usage":{"input_tokens":2,"cache_read_input_tokens":3,"cache_creation_input_tokens":4,"output_tokens":5,"cost_usd":0.01}}})];
    for (turn,id,failed) in [("p1","edit1",false),("p2","edit2",false),("p3","denied",true)] {
        if turn!="p1" {rows.push(json!({"type":"user","sessionId":"session","promptId":turn,"message":{"content":"next"}}));}
        rows.push(json!({"type":"assistant","sessionId":"session","isMeta":true,"message":{"content":[{"type":"tool_use","id":id,"name":"Edit","input":{"file_path":"/fixture/secret","old_string":"old","new_string":"new"}}]}}));
        rows.push(json!({"type":"user","sessionId":"session","toolUseResult":if failed {Value::Null}else{json!({"filePath":"/fixture/secret","structuredPatch":[{"oldStart":1,"newStart":1,"lines":["-old","+new"]}]})},"message":{"content":[{"type":"tool_result","tool_use_id":id,"is_error":failed,"content":"native result"}]}}));
    }
    rows.push(json!({"type":"assistant","sessionId":"session","message":{"content":[{"type":"tool_use","name":"TodoWrite","id":"todo","input":{"todos":[{"content":"SECRET_PLAN","status":"in_progress"}]}}]}}));
    rows.push(json!({"type":"api-request-blob","message":{"content":[{"type":"tool_use","name":"TodoWrite","id":"replay","input":{"todos":[{"content":"OLD_REPLAY"}]}}]}}));
    fs::write(project.join("session.jsonl"),format!("\n{}",rows.iter().map(|v|format!("{v}\n")).collect::<String>())).unwrap();
    let feed=root.join("statusline.jsonl");
    let payload=json!({"session_id":"session","model":{"id":"fixture-model"},"context_window":{"context_window_size":1000,"current_usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":3}},"rate_limits":{"five_hour":{"used_percentage":25,"resets_at":1900000000u64},"seven_day":{"used_percentage":50,"resets_at":1900000100u64}}});
    write_statusline(&feed,&payload.to_string()).unwrap();
    let rt=agent::runtime("claude").unwrap();let mut session=rt.history("session",Some(&root),None,None).unwrap();
    let page=session.steps(u64::MAX).unwrap();
    assert!(page.steps.iter().filter(|s|s.native_id.as_deref()==Some("edit1")).all(|s|s.presentation["meta"].is_null()));
    assert_eq!(page.steps.last().unwrap().presentation["cli_version"],"2.fixture");
    assert_eq!(page.steps.last().unwrap().presentation["model"],"fixture-model");
    assert!(page.steps.last().unwrap().presentation["meta"].is_null());
    let data=session.live_data(Some(&feed)).unwrap();
    assert_eq!((data.context.used,data.context.window),(Some(33),Some(1000)));
    assert_eq!(data.windows[0].used,Some(0.25));assert_eq!(data.windows[1].used,Some(0.5));
    assert!(data.events.iter().any(|e|matches!(e,LiveEvent::Windows(_))));assert!(data.context.observed_at_ms.is_some());
    assert_eq!(data.calls.len(),1);assert_eq!(data.calls[0]["cost_source"],"native-reported");
    assert_eq!(data.files.len(),2,"two successful turns, excluding denied edit");
    assert!(data.file_steps.iter().all(|s|s.kind==StepKind::FileRollup && !s.origins.is_empty()));
    assert_eq!(data.file_steps.iter().map(|s|s.turn.native_id.clone().unwrap()).collect::<Vec<_>>(),["p1","p2"]);
    assert_eq!(data.todos["items"][0]["content"],"SECRET_PLAN");
    use std::io::Write;fs::OpenOptions::new().append(true).open(&feed).unwrap().write_all(b"{\"partial\":").unwrap();
    assert_eq!(session.live_data(Some(&feed)).unwrap().context.used,Some(33));
    let foreign=root.join("foreign.jsonl");let mut bad=payload.clone();bad["session_id"]=json!("foreign");write_statusline(&foreign,&bad.to_string()).unwrap();
    assert!(session.live_data(Some(&foreign)).is_err());
    let (unknown,windows)=statusline(&json!({"session_id":"session"})).unwrap();assert!(unknown.used.is_none() && unknown.unknown.contains_key("used"));assert!(windows.iter().all(|w|w.used.is_none() && w.unknown.is_some()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_requests_supersede_legacy_snapshots_and_metadata_never_contains_content() {
    let root=root("usage");let dir=root.join("sessions/2026/11/07");fs::create_dir_all(&dir).unwrap();
    let legacy=json!({"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":999,"cached_input_tokens":0,"output_tokens":999},"total_token_usage":{"input_tokens":999,"output_tokens":999}}}});
    let request=json!({"type":"token_usage_record","timestamp":"2026-11-07T00:00:00Z","payload":{"thread_id":"session","turn_id":"turn","response_id":"response","usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":2,"reasoning_output_tokens":1}}});
    let rows=[json!({"type":"session_meta","payload":{"id":"session"}}),json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn"}}),legacy,request.clone(),request];
    fs::write(dir.join("rollout-session.jsonl"),rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let session=agent::runtime("codex").unwrap().history("session",Some(&root),None,None).unwrap();
    let data=session.live_data(None).unwrap();assert_eq!(data.calls.len(),1);assert_eq!(data.calls[0]["input"],7);assert_eq!(data.calls[0]["cache_read"],3);
    assert!(data.calls[0]["cost_usd"].is_null());assert!(data.calls[0]["unknown"]["cost_usd"].is_string());
    let row=|input|json!({"type":"assistant","message":{"id":"message","model":"fixture-model","content":[{"type":"text","text":"SECRET_PLAN /fixture/secret"}],"usage":{"input_tokens":input,"cache_read_input_tokens":4,"cache_creation_input_tokens":0,"output_tokens":2,"cost_usd":0.02}}});
    let text=format!("{}\n{}\n{}\n",row(2),row(3),json!({"type":"result","session_id":"session","total_cost_usd":0.02,"is_error":false}));
    let ctx=agent::Context {product:"fixtureq",version:"1",project:"other",task:None,route:None,account:"default"};
    let turn=agent::SessionTurn {runtime:"claude",session:Some("session"),scope:"scope",key:"private-key",now_ms:1,outcome:"ok",error:"none",model:None};
    let events=agent::session_records(&turn,&ctx,&text).unwrap();
    let store=coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();agent::append_records(&store,&events).unwrap();agent::append_records(&store,&events).unwrap();
    let export=store.export(0,0,100).unwrap();let records=export["events"].as_array().unwrap();assert_eq!(records.len(),2,"one turn and one native request, including resend");
    let turn=records.iter().find(|r|r["observation"]["kind"]=="agent.turn").unwrap();let call=records.iter().find(|r|r["observation"]["kind"]=="agent.call").unwrap();
    assert_eq!(call["observation"]["input_tokens"],3);assert_eq!(call["observation"]["parent"],turn["observation"]["span"]);
    assert_eq!(call["observation"]["counts"]["cost_native_nanousd"],20000000);assert_eq!(call["observation"]["counts"]["cost_source_native"],1);
    assert!(!export.to_string().contains("SECRET_PLAN"));assert!(!export.to_string().contains("/fixture/secret"));assert!(!export.to_string().contains("private-key"));
    drop(store);fs::remove_dir_all(root).unwrap();
}

#[test]
fn metadata_queue_hooks_and_resolved_cursor_need_no_discovery() {
    let root=root("metadata");let project=root.join("projects/project");fs::create_dir_all(&project).unwrap();
    let mut rows=Vec::new();
    for kind in ["environment","hook_success","hook_blocking_error","hook_additional_context","read_truncation_notice","diagnostics","queued_command"] {
        rows.push(json!({"type":"attachment","sessionId":"session","uuid":kind,"attachment":{"type":kind,"prompt":"queued","text":"native detail"}}));
    }
    rows.extend([json!({"type":"system","sessionId":"session","subtype":"stop_hook_summary","hookErrors":["failure"],"preventedContinuation":true}),
        json!({"type":"queue-operation","sessionId":"session","operation":"remove","content":"removed"}),
        json!({"type":"pr-link","sessionId":"session","prUrl":"https://example.invalid/pull/1"}),
        json!({"type":"assistant","sessionId":"session","message":{"content":[{"type":"server_tool_use","name":"web_search","id":"web","input":{}},{"type":"tool_search_tool_result","tool_use_id":"search","content":[]}]}})]);
    let path=project.join("session.jsonl");fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let rt=agent::runtime("claude").unwrap();let mut session=rt.history("session",Some(&root),None,None).unwrap();let page=session.steps(u64::MAX).unwrap();
    let kinds:Vec<_>=page.steps.iter().map(|s|s.kind).collect();
    assert_eq!(kinds,[StepKind::Metadata,StepKind::Hook,StepKind::Hook,StepKind::Hook,StepKind::ReadTruncation,StepKind::Diagnostics,StepKind::Queue,StepKind::Hook,StepKind::Queue,StepKind::Link,StepKind::Web,StepKind::Tool]);
    assert_eq!(page.steps[8].data["text"],"removed");assert_eq!(page.steps[7].data["preventedContinuation"],true);
    // Discovery would fail on this unrelated malformed source; a resolved cursor never opens it.
    fs::write(project.join("unrelated.jsonl"),"not JSON\n").unwrap();
    let mut recovered=rt.history("session",Some(&root),None,Some(&page.checkpoint)).unwrap();
    assert!(recovered.steps(u64::MAX).unwrap().steps.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_claude_statusline_fixture_has_known_weekly_and_unknown_short_window() {
    let v:Value=serde_json::from_str(include_str!("fixtures/history/claude-statusline.json")).unwrap();
    let (context,windows)=statusline(&v).unwrap();
    assert_eq!(context.used,Some(19668));assert_eq!(context.window,Some(1000000));
    assert!(windows[0].used.is_none() && windows[0].unknown.is_some());assert_eq!(windows[1].used,Some(0.88));
}
