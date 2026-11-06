//! M12.18: raw source truth, typed console fields, fork provenance and convergent revisions.
use agentq_runtime::agent::{self, State, history::{History, StepKind}};
use serde_json::{Value,json};
use std::fs;

fn path(name:&str)->std::path::PathBuf {std::env::temp_dir().join(format!("agentq-steps-{}-{name}",std::process::id()))}

#[test]
fn real_completed_items_and_claude_results_are_typed() {
    for (format,raw,expected) in [
        ("codex-rollout",include_str!("fixtures/history/codex-steps.jsonl"),vec![StepKind::Shell,StepKind::Thinking,StepKind::Image,StepKind::Mcp,StepKind::FileChange,StepKind::Compaction]),
        ("claude-native",include_str!("fixtures/history/claude-steps.jsonl"),vec![StepKind::Thinking,StepKind::Usage,StepKind::Shell]),
    ] {
        let p=path(format);fs::write(&p,raw).unwrap();
        let mut h=History::new(format,"fixture","session").unwrap();
        let page=h.steps(&p,u64::MAX).unwrap();
        assert_eq!(page.steps.iter().map(|s|s.kind).collect::<Vec<_>>(),expected);
        for step in &page.steps {
            let start=step.source["start"].as_u64().unwrap();let end=step.source["end"].as_u64().unwrap();
            assert_eq!(h.read_bytes(&p,start,end-start).unwrap(),step.source["raw"].as_str().unwrap().as_bytes());
            if step.kind==StepKind::Shell {
                for k in ["command","cwd","exit_code","duration","output","stdout","stderr","interrupted"] {
                    assert!(step.data.get(k).is_some());
                    if step.data[k].is_null(){assert!(step.data["unknown"][k].is_string());}
                }
                assert_eq!(step.data["stdout"],"scrubbed");assert_eq!(step.data["stderr"],"scrubbed");
            }
        }
        assert!(!page.more);
        assert!(History::recover(&page.checkpoint).unwrap().steps(&p,1).unwrap().steps.is_empty());
        fs::remove_file(p).unwrap();
    }
}

#[test]
fn revisions_converge_and_large_output_is_not_clipped() {
    let p=path("revisions");
    let rows=[json!({"method":"item/started","params":{"turnId":"turn","item":{"id":"shell","type":"commandExecution","command":"pwd"}}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"turnId":"turn","itemId":"shell","delta":"/tmp\n"}}),
        json!({"method":"item/completed","params":{"turnId":"turn","item":{"id":"shell","type":"commandExecution","command":"pwd","aggregatedOutput":"/tmp\n","exitCode":0}}})];
    fs::write(&p,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let mut h=History::new("codex-app-server","source","session").unwrap();
    let page=h.steps(&p,u64::MAX).unwrap();
    assert_eq!(page.steps.len(),1,"one converged step per page");
    assert_eq!(page.steps[0].origins.len(),3);
    assert_eq!(page.steps[0].data["output"],"/tmp\n");
    assert_eq!(page.steps[0].data["command"],"pwd");
    let large=json!({"type":"item.completed","item":{"type":"command_execution","id":"big","aggregated_output":"x".repeat(17*1024*1024)}});
    let raw=format!("{large}\n");fs::write(&p,&raw).unwrap();
    let mut h=History::new("codex-exec","source","session").unwrap();
    let page=h.steps(&p,1).unwrap();
    assert_eq!(page.steps[0].data["output"].as_str().unwrap().len(),17*1024*1024);
    assert_eq!(h.read_bytes(&p,raw.len() as u64-10,10).unwrap(),&raw.as_bytes()[raw.len()-10..]);
    assert!(!page.more);
    fs::remove_file(p).unwrap();
}

#[test]
fn forks_keep_parent_metadata_but_refuse_foreign_events() {
    let p=path("fork");
    let raw=concat!("{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"forked_from_id\":\"parent\"}}\n",
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"parent\"}}\n",
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"child\",\"item\":{\"type\":\"Reasoning\",\"id\":\"r\",\"summary_text\":[\"scrubbed\"]}}}\n");
    fs::write(&p,raw).unwrap();
    let mut h=History::new("codex-rollout","source","child").unwrap();
    let page=h.steps(&p,u64::MAX).unwrap();assert_eq!(page.steps.len(),3);
    assert_eq!(page.steps[1].source["session_binding"],"caller");
    assert_eq!(page.steps[1].source["provenance"],true);
    let before=h.checkpoint();
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"{\"type\":\"event_msg\",\"payload\":{\"thread_id\":\"foreign\"}}\n").unwrap();
    assert_eq!(h.steps(&p,1).unwrap_err(),"history native session differs");assert_eq!(h.checkpoint(),before);
    fs::remove_file(p).unwrap();
}

#[test]
fn trait_and_cli_discover_read_and_describe_resume() {
    let root=path("home");let dir=root.join("projects/project/subagents");fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("agent-child.jsonl"),"{\"type\":\"user\",\"sessionId\":\"child\",\"message\":{\"content\":\"exact\"}}\n").unwrap();
    let rt=agent::runtime("claude").unwrap();
    assert_eq!(rt.history_sessions(Some(&root)).unwrap().len(),1);
    let mut session=rt.history("child",Some(&root),None,None).unwrap();
    let page=session.steps(1).unwrap();assert_eq!(page.steps.len(),1);
    let request=json!({"runtime":"claude","session":"child","home":root,"bytes":1});
    let cli=agent::history::sessions::request(&request.to_string()).unwrap();assert_eq!(cli["steps"].as_array().unwrap().len(),1);
    for state in [State::Running,State::Unknown] {assert!(rt.descriptor().history_resume("child",state).is_err());}
    assert_eq!(rt.descriptor().history_resume("child",State::Finished).unwrap(),["claude","--resume","child"]);
    let saved:Value=page.checkpoint;assert!(rt.history("foreign",Some(&root),None,Some(&saved)).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn claude_revisions_keep_console_kind_and_ignore_replay_metadata() {
    let p=path("claude-convergence");
    let call=json!({"type":"assistant","uuid":"message","parentUuid":"previous-message","cwd":"/fixture","message":{"content":[{"type":"tool_use","id":"shell","name":"Bash","input":{"command":"printf proof"}}]}});
    let result=json!({"type":"user","uuid":"result","parentUuid":"message","toolUseResult":null,"message":{"content":[{"type":"tool_result","tool_use_id":"shell","content":"proof"}]}});
    let replay=json!({"type":"api-request-blob","message":call["message"]});
    let prompt=json!({"type":"user","uuid":"prompt","message":{"content":"exact prompt\n"}});
    fs::write(&p,format!("{call}\n{result}\n{replay}\n{prompt}\n")).unwrap();
    let mut h=History::new("claude-native","source","session").unwrap();
    let page=h.steps(&p,u64::MAX).unwrap();
    let shell=page.steps.iter().find(|s|s.native_id.as_deref()==Some("shell")).unwrap();
    assert_eq!(shell.kind,StepKind::Shell);assert_eq!(shell.state,"completed");
    assert_eq!(shell.data["command"],"printf proof");assert_eq!(shell.data["cwd"],"/fixture");assert_eq!(shell.data["output"],"proof");
    assert!(shell.parent.is_none(),"parentUuid is a message-chain edge, not work ownership");
    assert_eq!(shell.origins.len(),2,"metadata replay cannot revise an executed tool");
    assert!(page.steps.iter().any(|s|s.kind==StepKind::Metadata && s.source["raw"].as_str().unwrap().contains("api-request-blob")));
    assert!(page.steps.iter().any(|s|s.data["text"]=="exact prompt\n"));
    fs::remove_file(p).unwrap();
}

#[test]
fn output_deltas_are_cumulative_across_recovered_pages() {
    let p=path("delta-pages");
    let rows=[json!({"method":"item/started","params":{"item":{"id":"shell","type":"commandExecution","command":"printf ab","cwd":"/fixture"}}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"itemId":"shell","delta":"a"}}),
        json!({"method":"item/commandExecution/outputDelta","params":{"itemId":"shell","delta":"b"}})];
    fs::write(&p,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let mut h=History::new("codex-app-server","source","session").unwrap();h.steps(&p,1).unwrap();
    assert_eq!(h.steps(&p,1).unwrap().steps[0].data["output"],"a");
    let page=History::recover(&h.checkpoint()).unwrap().steps(&p,1).unwrap();
    assert_eq!(page.steps[0].data["output"],"ab");assert_eq!(page.steps[0].data["command"],"printf ab");
    assert_eq!(page.steps[0].data["cwd"],"/fixture");assert_eq!(page.steps[0].data["output_mode"],"cumulative");
    fs::remove_file(p).unwrap();
}

#[test]
fn native_subagent_ids_and_parent_calls_need_no_source_path() {
    let root=path("subagent-home");let project=root.join("projects/project");let children=project.join("parent/subagents");fs::create_dir_all(&children).unwrap();
    let call=json!({"type":"assistant","sessionId":"parent","message":{"content":[{"type":"tool_use","id":"spawn","name":"Agent","input":{"prompt":"scrubbed"}}]}});
    let result=json!({"type":"user","sessionId":"parent","toolUseResult":{"agentId":"child","isAsync":true},"message":{"content":[{"type":"tool_result","tool_use_id":"spawn","content":"scrubbed"}]}});
    fs::write(project.join("parent.jsonl"),format!("{call}\n{result}\n")).unwrap();
    fs::write(children.join("agent-child.jsonl"),"{\"type\":\"user\",\"agentId\":\"child\",\"sessionId\":\"parent\",\"message\":{\"content\":\"child prompt\"}}\n").unwrap();
    let rt=agent::runtime("claude").unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();
    assert_eq!(sources.len(),2);let child=sources.iter().find(|s|s.native_session=="child").unwrap();
    let parent_id=sources.iter().find(|s|s.session=="parent").unwrap().id.clone();
    assert_eq!(child.parent.as_deref(),Some("parent"));
    let parent=rt.history("parent",Some(&root),None,None).unwrap().steps(u64::MAX).unwrap();
    assert_eq!(parent.steps.len(),1);assert_eq!(parent.steps[0].kind,StepKind::Subagent);assert_eq!(parent.steps[0].data["session"],child.id);
    let child=rt.history(&child.id,Some(&root),None,None).unwrap().steps(u64::MAX).unwrap();
    assert_eq!(child.steps[0].parent.as_deref(),Some("spawn"));assert_eq!(child.steps[0].parent_session.as_deref(),Some(parent_id.as_str()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_spawn_usage_and_code_mode_keep_their_native_meaning() {
    let p=path("codex-meaning");
    let rows=[json!({"type":"response_item","payload":{"type":"function_call","name":"spawn_agent","call_id":"spawn","arguments":"{}"}}),
        json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"spawn","output":"{\"agent_id\":\"child\"}"}}),
        json!({"type":"event_msg","payload":{"item":{"type":"SubAgentActivity","id":"spawn","kind":"started","agent_thread_id":"child"}}}),
        json!({"type":"token_usage_record","payload":{"response_id":"request","usage":{"input_tokens":10,"cached_input_tokens":0},"thread_token_usage":{"input_tokens":100}}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","call_id":"code","input":"await tools.exec_command(dynamicArgs)"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"code","output":"full console"}})];
    fs::write(&p,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let page=History::new("codex-rollout","source","session").unwrap().steps(&p,u64::MAX).unwrap();
    let spawn=page.steps.iter().find(|s|s.native_id.as_deref()==Some("spawn")).unwrap();
    assert_eq!(spawn.kind,StepKind::Subagent);assert_eq!(spawn.data["session"],"child");
    let usage=page.steps.iter().find(|s|s.kind==StepKind::Usage).unwrap();assert_eq!(usage.data["input"],10);
    let code=page.steps.iter().find(|s|s.native_id.as_deref()==Some("code")).unwrap();
    assert_eq!(code.kind,StepKind::Tool);assert_eq!(code.data["code"],"await tools.exec_command(dynamicArgs)");assert_eq!(code.data["output"],"full console");
    assert!(code.data["command"].is_null());assert!(code.data["unknown"]["command"].is_string());
    fs::remove_file(p).unwrap();
}

#[test]
fn native_step_fixtures_do_not_contain_private_paths_or_timestamps() {
    for raw in [include_str!("fixtures/history/codex-steps.jsonl"),include_str!("fixtures/history/claude-steps.jsonl")] {
        for forbidden in ["/Users/","/home/","/private/","/var/folders/","/tmp/","C:\\Users\\","C:\\\\Users\\\\"] {assert!(!raw.contains(forbidden),"private path in fixture: {forbidden}");}
        for row in raw.lines() {
            let v:Value=serde_json::from_str(row).unwrap();
            for key in ["started_at_ms","completed_at_ms"] {if let Some(time)=v["payload"].get(key) {assert_eq!(time,0,"native timestamp must be scrubbed");}}
        }
    }
}

#[test]
fn raw_history_keeps_pascal_case_native_items() {
    let p=path("raw-pascal");
    let rows=[
        json!({"type":"response_item","payload":{"type":"AgentMessage","id":"m","content":[{"type":"Text","text":"exact"},{"type":"LocalImage","path":"fixture-image"}]}}),
        json!({"type":"response_item","payload":{"type":"CommandExecution","id":"s","command":"pwd"}}),
        json!({"type":"response_item","payload":{"type":"McpToolCall","id":"t"}}),
    ];
    fs::write(&p,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let page=History::new("codex-rollout","source","session").unwrap().poll(&p).unwrap();
    assert_eq!(page.events[0]["kind"],"message");assert_eq!(page.events[0]["blocks"][0]["type"],"text");
    assert_eq!(page.events[0]["blocks"][1]["type"],"image");
    for event in &page.events[1..] {assert_eq!(event["kind"],"tool");assert_eq!(event["blocks"][0]["type"],"tool_result");}
    fs::remove_file(p).unwrap();
}

#[test]
fn alias_steps_and_subagent_parent_bindings_survive_recovery() {
    let p=path("alias-steps");
    let first=json!({"type":"user","uuid":"original-row","sessionId":"original","message":{"content":"first"}});
    let second=json!({"type":"user","uuid":"copied-row","sessionId":"copy","message":{"content":"second"}});
    fs::write(&p,format!("{first}\n{second}\n")).unwrap();
    let mut h=History::with_native_session_alias("claude-native","copy-job","copy","original").unwrap();
    let page=h.steps(&p,1).unwrap();assert_eq!(page.steps[0].source["session_binding"],"alias");
    assert_eq!(page.checkpoint["native_session"],"copy");
    let page=History::recover(&page.checkpoint).unwrap().steps(&p,1).unwrap();assert_eq!(page.steps[0].source["session_binding"],"native");
    fs::remove_file(p).unwrap();

    let root=path("parent-binding-home");let dir=root.join("projects/project/parent/subagents");fs::create_dir_all(&dir).unwrap();
    let p=dir.join("agent-child.jsonl");
    fs::write(&p,format!("{}\n",json!({"type":"user","agentId":"child","sessionId":"parent","message":{"content":"child"}}))).unwrap();
    let rt=agent::runtime("claude").unwrap();let mut session=rt.history("parent/agent/child",Some(&root),None,None).unwrap();
    session.steps(1).unwrap();let saved=session.checkpoint();
    let mut h=rt.history("parent/agent/child",Some(&root),None,Some(&saved)).unwrap();
    use std::io::Write;
    fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(format!("{}\n",json!({"type":"user","agentId":"child","sessionId":"foreign-parent","message":{"content":"foreign"}})).as_bytes()).unwrap();
    assert_eq!(h.steps(u64::MAX).unwrap_err(),"history native parent differs");assert_eq!(h.checkpoint(),saved);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transport_separators_keep_ranges_and_malformed_records_still_refuse() {
    let p=path("separators");let separators="\n \t\r\n";
    let row=format!("{}\n",json!({"type":"user","sessionId":"session","message":{"content":"activity"}}));
    fs::write(&p,format!("{separators}{row}{separators}")).unwrap();
    let new=||History::new("claude-native","fixture","session").unwrap();
    let mut forward=new();let page=forward.poll(&p).unwrap();assert_eq!(page.events.len(),1);assert_eq!(page.events[0]["start"],separators.len());
    let mut backward=new();assert_eq!(backward.poll_before(&p).unwrap().events.len(),1);
    let mut steps=new();let page=steps.steps(&p,u64::MAX).unwrap();assert_eq!(page.steps.len(),1);assert_eq!(page.steps[0].source["start"],separators.len());
    use std::io::Write;fs::OpenOptions::new().append(true).open(&p).unwrap().write_all(b"malformed\n").unwrap();
    let before=forward.checkpoint();assert_eq!(forward.poll(&p).unwrap_err(),"history invalid JSON");assert_eq!(forward.checkpoint(),before);
    let mut backward=new();let before=backward.checkpoint();assert_eq!(backward.poll_before(&p).unwrap_err(),"history invalid JSON");assert_eq!(backward.checkpoint(),before);
    let before=steps.checkpoint();assert_eq!(steps.steps(&p,u64::MAX).unwrap_err(),"history invalid JSON");assert_eq!(steps.checkpoint(),before);
    fs::write(&p,separators).unwrap();assert!(new().poll_before(&p).unwrap().events.is_empty());
    fs::remove_file(p).unwrap();
}
