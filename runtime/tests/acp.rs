//! #293: a native protocol double, not model qualification. Exercises the public trait and process boundary.
use agentq_runtime::agent::{self,Control,Event,Launch,State};
use serde_json::{Value,json};
use std::{io::{BufRead,Write},path::PathBuf,time::{Duration,Instant}};

fn main() {
    if std::env::var_os("AGENTQ_FAKE_ACP").is_some() {return server();}
    let root=std::env::temp_dir().join(format!("agentq-acp-{}",std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let r=agent::runtime("antigravity").unwrap();
    let mut l=Launch::new(&root,root.join("acp.jsonl"),"first","fixtureq");
    l.command=vec![std::env::current_exe().unwrap().into_os_string()];
    l.home=Some(root.join("account"));
    l.env=vec![("AGENTQ_FAKE_ACP".into(),"1".into())];
    l.deadline=Some(Instant::now()+Duration::from_secs(20));
    l.model=Some("model-1".into());
    l.policy.approval=agent::Approval::Edits;
    let mut run=r.launch(&l).unwrap();
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;assert_eq!(std::fs::metadata(&l.log).unwrap().permissions().mode() & 0o777,0o600);}
    assert_eq!(r.checkpoint(&run).unwrap_err(),agent::Cause("no-checkpoint"));
    let mut seen=vec![];
    loop {
        let events=r.progress(&mut run).unwrap();
        for event in &events {if let Event::Request{id,..}=event {
            assert!(r.control(&mut run,Control::Reply{id:id.clone(),option:Some("invented".into())}).is_err());
            r.control(&mut run,Control::Reply{id:id.clone(),option:Some("once".into())}).unwrap();
            assert!(r.control(&mut run,Control::Reply{id:id.clone(),option:Some("once".into())}).is_err());
        }}
        seen.extend(events);
        if r.poll(&mut run).unwrap()!=State::Running {break;}
        std::thread::yield_now();
    }
    seen.extend(r.progress(&mut run).unwrap());
    let report=r.report(&run).unwrap();
    assert_eq!((report.outcome,report.text.as_deref(),report.usage.input,report.usage.calls),("ok",Some("answer"),Some(17),None));
    assert!(seen.iter().any(|e|matches!(e,Event::ToolStart{id:Some(id),..} if id=="tool-1")));
    assert!(seen.iter().any(|e|matches!(e,Event::ToolEnd{id:Some(id),..} if id=="tool-1")));
    assert!(seen.iter().any(|e|matches!(e,Event::Metadata{kind,..} if kind=="future_update")));
    assert!(r.progress(&mut run).unwrap().is_empty());
    assert!(r.control(&mut run,Control::Reply{id:json!("permission-1"),option:Some("once".into())}).is_err());
    let classified=r.classify(&std::fs::read_to_string(&l.log).unwrap());
    assert_eq!((classified.outcome,classified.text.as_deref(),classified.usage.input),("ok",Some("answer"),Some(17)));
    l.input="second".into();
    let mut resumed=r.resume("session-1",&l).unwrap();
    r.cancel(&mut resumed).unwrap();
    while r.poll(&mut resumed).unwrap()==State::Running {std::thread::yield_now();}
    let events=r.progress(&mut resumed).unwrap();
    assert!(!events.iter().any(|e|matches!(e,Event::Text{delta,..} if delta=="old replay")));
    assert_eq!(r.report(&resumed).unwrap().outcome,"cancelled");
    let raw=std::fs::read_to_string(&l.log).unwrap();
    assert!(raw.contains("old replay") && raw.contains("future_update"));
    let logged=r.log_events(&raw);
    assert!(logged.iter().any(|e|matches!(e,Event::ToolStart{..})));
    assert!(logged.iter().any(|e|matches!(e,Event::TurnEnd{outcome:"ok"})));
    assert!(!logged.iter().any(|e|matches!(e,Event::Text{delta,..} if delta=="old replay")));
    l.env.push(("AGENTQ_FAKE_ACP_BAD_VERSION".into(),"1".into()));
    assert!(r.launch(&l).err().unwrap().contains("workspace write defect"));
    l.env.pop();l.env.push(("AGENTQ_FAKE_ACP_EOF".into(),"1".into()));
    let mut ended=r.launch(&l).unwrap();
    while r.poll(&mut ended).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(r.poll(&mut ended).unwrap(),State::Unknown);
    assert!(r.report(&ended).is_err());
    std::fs::remove_dir_all(root).unwrap();
    println!("ACP trait: launch, choices, model/mode, tools, resume replay, cancel, unknown updates, version refusal and EOF passed");
}

fn server() {
    let mut prompt=Value::Null;
    let emit=|v:Value| {println!("{v}");std::io::stdout().flush().unwrap();};
    let reply=|v:&Value,result:Value|emit(json!({"jsonrpc":"2.0","id":v["id"],"result":result}));
    let update=|u:Value|emit(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session-1","update":u}}));
    for line in std::io::stdin().lock().lines() {
        let v:Value=serde_json::from_str(&line.unwrap()).unwrap();
        match v["method"].as_str() {
            Some("initialize")=>reply(&v,json!({"protocolVersion":1,"agentInfo":{"version":if std::env::var_os("AGENTQ_FAKE_ACP_BAD_VERSION").is_some(){"1.4.3"}else{"1.5.2"}},"agentCapabilities":{"loadSession":true}})),
            Some("session/new"|"session/load")=>{
                assert!(PathBuf::from(std::env::var_os("GEMINI_HOME").unwrap()).ends_with("account"));
                if v["method"]=="session/load" {update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old replay"}}));}
                reply(&v,json!({"sessionId":"session-1"}));
            },
            Some("session/set_model")=>{assert_eq!(v["params"]["modelId"],"model-1");reply(&v,json!({}));},
            Some("session/set_mode")=>{assert_eq!(v["params"]["modeId"],"accept-edits");reply(&v,json!({}));},
            Some("session/prompt")=>{
                if std::env::var_os("AGENTQ_FAKE_ACP_EOF").is_some() {return;}
                prompt=v["id"].clone();
                if v["params"]["prompt"][0]["text"]=="second" {continue;}
                emit(json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{"sessionId":"session-1","toolCall":{"toolCallId":"tool-1","title":"Run fixture"},"options":[{"optionId":"once","kind":"allow_once","name":"Allow"}]}}));
            },
            Some("session/cancel")=>emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}})),
            None if v["id"]=="permission-1"=>{
                assert_eq!(v["result"]["outcome"]["optionId"],"once");
                update(json!({"sessionUpdate":"tool_call","toolCallId":"tool-1","kind":"execute","title":"Fixture","status":"in_progress"}));
                update(json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed","rawOutput":{"stdout":"proof"}}));
                update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}));
                update(json!({"sessionUpdate":"future_update","raw":"kept"}));
                emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"end_turn","usage":{"inputTokens":17,"outputTokens":2}}}));
            },
            _=>panic!("unexpected ACP message: {v}"),
        }
    }
}
