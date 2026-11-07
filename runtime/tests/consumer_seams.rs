//! M12.3/M12.9: typed observations retain native acceptance, refusal and physical presentation.
use agentq_runtime::agent::{self, history::{History, StepKind}, ProcessTermination};
use serde_json::{json, Value};
use std::{fs, io::Write};

#[test]
fn antigravity_input_acceptance_keeps_state_scope_and_physical_boundary() {
    let path=std::env::temp_dir().join(format!("agentq-input-seam-{}.jsonl",std::process::id()));
    let row=|kind,state|json!({"event":"step_update","step_update":{"conversation_id":"fixture","turn_id":"turn","step_index":0,"step_type":kind,"state":state}});
    let mut history=History::new("agy-stream","source","fixture").unwrap();
    fs::write(&path,format!("{}\n",row("user_input",json!("ACTIVE")))).unwrap();
    let active=history.steps(&path,u64::MAX).unwrap().steps.remove(0);
    assert_eq!((active.kind,active.author.as_str(),active.state.as_str()),(StepKind::Message,"user","started"));
    let mut physical=History::new("agy-stream","source","fixture").unwrap();
    let event=physical.poll(&path).unwrap().events.remove(0);
    assert_eq!(event["state"],active.state);
    let mut physical=History::recover(&physical.checkpoint()).unwrap();
    let boundary=active.source["end"].as_u64().unwrap();
    let mut file=fs::OpenOptions::new().append(true).open(&path).unwrap();
    for kind in ["agent_response","checkpoint","system_message"] {writeln!(file,"{}",row(kind,json!("DONE"))).unwrap();}
    let unrelated=history.steps(&path,u64::MAX).unwrap();
    assert!(unrelated.steps.iter().all(|s|s.author!="user" || s.state!="completed"));
    for state in [Value::Null,json!("PENDING"),json!(42)] {writeln!(file,"{}",row("user_input",state)).unwrap();}
    assert!(history.steps(&path,u64::MAX).unwrap().steps.iter().all(|s|s.state=="unknown"));
    let done=format!("{}\n",row("user_input",json!("DONE")));
    let start=fs::metadata(&path).unwrap().len();
    file.write_all(&done.as_bytes()[..done.len()-1]).unwrap();
    assert!(history.steps(&path,u64::MAX).unwrap().steps.is_empty(),"partial native acceptance waits");
    file.write_all(b"\n").unwrap();
    let accepted=history.steps(&path,u64::MAX).unwrap().steps.remove(0);
    let events=physical.poll(&path).unwrap().events;
    let done_event=events.iter().find(|e|e["state"]=="completed").unwrap();
    assert_eq!(done_event["start"],start);assert_eq!(done_event["end"],start+done.len() as u64);
    assert_eq!(done_event["author"],"user");assert_eq!(done_event["kind"],"message");
    assert_eq!(events.iter().filter(|e|e["state"]=="completed").count(),1);
    assert!(events.iter().all(|e|e["start"].as_u64().unwrap()>=boundary));
    let mut backwards=History::new("agy-stream","source","fixture").unwrap();
    let backward=backwards.poll_before(&path).unwrap().events;
    assert_eq!(backward.last().unwrap(),done_event);
    assert_eq!((accepted.kind,accepted.author.as_str(),accepted.state.as_str()),(StepKind::Message,"user","completed"));
    assert_eq!((accepted.turn.session.as_str(),accepted.turn.native_id.as_deref()),("fixture",Some("turn")));
    assert_eq!(accepted.source["start"],start);assert!(start>=boundary);
    assert_eq!(accepted.source["end"],start+done.len() as u64);
    assert_eq!(accepted.origins[0]["start"],start);
    assert_eq!(history.read_bytes(&path,start,done.len() as u64).unwrap(),done.as_bytes());
    writeln!(file,"{}",json!({"event":"step_update","step_update":{"conversation_id":"fixture","step_type":"user_input","state":"DONE"}})).unwrap();
    let unscoped=history.steps(&path,u64::MAX).unwrap().steps.remove(0);
    assert!(unscoped.turn.native_id.is_none());assert!(unscoped.turn.unknown.is_some());
    writeln!(file,"{}",json!({"event":"step_update","step_update":{"conversation_id":"foreign","step_type":"user_input","state":"DONE"}})).unwrap();
    assert_eq!(history.steps(&path,u64::MAX).unwrap_err(),"history native session differs");
    fs::remove_file(path).unwrap();
}

#[test]
fn codex_native_writer_refusal_is_typed_without_claiming_ownership() {
    let runtime=agent::runtime("codex").unwrap();
    let detail="Error: thread fixture already has an active writer; close it there first";
    for text in [detail.to_string(),json!({"type":"error","message":detail}).to_string(),json!({"type":"turn.failed","error":{"message":detail}}).to_string()] {
        let report=runtime.classify(&text);
        assert_eq!((report.outcome,report.error,report.detail.as_deref()),("refused","session-busy",Some(detail)));
        assert!(matches!(report.termination,ProcessTermination::Unknown {..}));
        assert_eq!(report.owner.state,"unknown");
        assert_eq!(runtime.launch_refusal(&text),Some(agent::Cause("session-busy")));
    }
    for text in ["unrecognized native failure".to_string(),json!({"type":"item.completed","item":{"type":"agent_message","text":detail}}).to_string()] {
        let report=runtime.classify(&text);assert_ne!(report.error,"session-busy");
    }
    assert_eq!(runtime.launch_refusal("unrecognized native failure"),None);
    assert_eq!(runtime.classify("unrecognized native failure").outcome,"unknown");
}

#[test]
fn physical_presentation_matches_steps_in_both_page_directions() {
    let path=std::env::temp_dir().join(format!("agentq-presentation-seam-{}.jsonl",std::process::id()));
    let rows=(0..260).map(|i|json!({"type":"assistant","uuid":i.to_string(),"sessionId":"fixture","isSidechain":i%2==0,"isMeta":false,"claude_code_version":"fixture-version","message":{"model":"fixture-model","content":[{"type":"text","text":"fixture"}]}})).chain([json!({"type":"assistant","sessionId":"fixture","message":{"content":[]}})]).collect::<Vec<_>>();
    fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let mut forward=History::new("claude-native","source","fixture").unwrap();let mut events=vec![];
    loop {let page=forward.poll(&path).unwrap();events.extend(page.events);if !page.more{break;}}
    let mut backward=History::new("claude-native","source","fixture").unwrap();let mut reverse=vec![];
    loop {let page=backward.poll_before(&path).unwrap();reverse.splice(0..0,page.events);if !page.more{break;}}
    assert_eq!(reverse,events,"physical IDs, source ranges and presentation survive backward paging");
    let steps=History::new("claude-native","source","fixture").unwrap().steps(&path,u64::MAX).unwrap().steps;
    for (event,step) in events[..260].iter().zip(&steps) {assert_eq!(event["presentation"],step.presentation);}
    let absent=&events.last().unwrap()["presentation"];
    for field in ["model","effort","cli_version","sidechain","meta"] {assert!(absent[field].is_null());assert!(absent["unknown"][field].is_string());}
    assert_eq!(events[0]["presentation"]["sidechain"],true);assert_eq!(events[0]["presentation"]["meta"],false);
    fs::remove_file(path).unwrap();
}
