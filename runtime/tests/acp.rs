//! #293: a native protocol double, not model qualification. Exercises the public trait and process boundary.
use agentq_runtime::agent::{self,Control,Event,Launch,State};
use serde_json::{Value,json};
use std::{io::{BufRead,Write},path::PathBuf,time::{Duration,Instant}};

fn main() {
    if std::env::var_os("AGENTQ_FAKE_DESCENDANT").is_some() {loop {std::thread::park();}}
    if let Some(home)=std::env::var_os("AGENTQ_CHECK_ACP_HOME") {
        let r=agent::runtime("antigravity").unwrap();let home=PathBuf::from(home);
        assert_eq!(r.home(),Some(home.clone()));
        assert_eq!(r.home_env("fixtureq","other"),vec![("GEMINI_HOME".into(),home.join("fixtureq-accounts").join("other").into_os_string())]);
        assert_eq!(r.protected_paths(None),vec![home.clone(),home.join("config")]);return;
    }
    if std::env::var_os("AGENTQ_FAKE_ACP").is_some() {return server();}
    let root=std::env::temp_dir().join(format!("agentq-acp-{}",std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let expected=root.join("gemini-root");
    assert!(std::process::Command::new(std::env::current_exe().unwrap()).env("AGENTQ_CHECK_ACP_HOME",&expected).env("GEMINI_HOME",&expected).env("ANTIGRAVITY_HOME",root.join("old-data")).status().unwrap().success());
    let r=agent::runtime("antigravity").unwrap();
    let mut l=Launch::new(&root,root.join("acp.jsonl"),"first","fixtureq");
    l.command=vec![std::env::current_exe().unwrap().into_os_string()];
    l.home=Some(root.join("account"));std::fs::create_dir_all(l.home.as_ref().unwrap()).unwrap();
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
    assert_eq!(seen.iter().filter(|e|matches!(e,Event::Metadata{kind,..} if kind=="protocol-error")).count(),2);
    assert!(r.progress(&mut run).unwrap().is_empty());
    assert!(r.control(&mut run,Control::Reply{id:json!("permission-1"),option:Some("once".into())}).is_err());
    let classified=r.classify(&std::fs::read_to_string(&l.log).unwrap());
    assert_eq!((classified.outcome,classified.text.as_deref(),classified.usage.input),("ok",Some("answer"),Some(17)));
    let mut history=r.history("session:opaque_/1",None,Some(&l.log),None).unwrap();
    let steps=history.steps(std::fs::metadata(&l.log).unwrap().len()).unwrap().steps;
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Message && s.data["text"]=="answer"));
    let tool=steps.iter().find(|s|s.native_id.as_deref()==Some("tool-1")).unwrap();
    assert_eq!(tool.kind,agent::history::StepKind::Shell);assert_eq!(tool.state,"completed");assert_eq!(tool.origins.len(),2);
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Approval));
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Unknown));
    l.input="second".into();
    let mut resumed=r.resume("session:opaque_/1",&l).unwrap();
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
    let named=root.join(if cfg!(windows){"agentapi.exe"}else{"agentapi"});std::fs::copy(std::env::current_exe().unwrap(),&named).unwrap();
    let mut named_launch=l.clone();named_launch.command=vec![named.into_os_string()];named_launch.deadline=Some(Instant::now()+Duration::from_secs(20));
    let mut named_run=r.launch(&named_launch).unwrap();r.cancel(&mut named_run).unwrap();
    while r.poll(&mut named_run).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(r.report(&named_run).unwrap().outcome,"cancelled");
    let mut pending=l.clone();pending.input="approval".into();pending.deadline=None;
    let mut waiting=r.launch(&pending).unwrap();
    while !r.progress(&mut waiting).unwrap().iter().any(|e|matches!(e,Event::Request{..})) {std::thread::yield_now();}
    r.cancel(&mut waiting).unwrap();
    while r.poll(&mut waiting).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(r.report(&waiting).unwrap().outcome,"cancelled");
    let cancel_raw=std::fs::read_to_string(&l.log).unwrap();assert!(cancel_raw.contains("\"outcome\":\"cancelled\""));
    let mut linger=l.clone();linger.input="linger".into();linger.deadline=None;
    let mut lingering=r.launch(&linger).unwrap();
    while r.poll(&mut lingering).unwrap()==State::Running {std::thread::yield_now();}
    let report=r.report(&lingering).unwrap();assert_eq!(report.outcome,"ok");assert_eq!(report.exit,None);assert_eq!(report.process.exit_unknown,"session-process-running");
    assert!(r.progress(&mut lingering).is_ok());
    let stop=r.stop_event(&mut lingering,u64::MAX).unwrap().unwrap();
    assert_eq!(stop.cause,agent::history::stop::StopCause::TurnEndProcessLive);
    assert!(stop.last_activity.is_some());
    assert_eq!(r.stop_event(&mut lingering,u64::MAX).unwrap().unwrap().id,stop.id);
    assert_eq!(r.report(&lingering).unwrap().outcome,"ok");drop(lingering);
    #[cfg(windows)] {
        let shim=root.join("native.cmd");std::fs::write(&shim,format!("@echo off\r\n\"{}\" %*\r\n",std::env::current_exe().unwrap().display())).unwrap();
        let mut wrapped=l.clone();wrapped.command=vec![shim.into_os_string()];wrapped.deadline=Some(Instant::now()+Duration::from_secs(20));
        wrapped.env.push(("AGENTQ_FAKE_ACP_DESCENDANT".into(),"1".into()));
        wrapped.process.configure=Some(std::sync::Arc::new(|command|{use std::os::windows::process::CommandExt;command.creation_flags(0x200).env("AGENTQ_JOB_CONFIGURED","1");}));
        let mut run=r.launch(&wrapped).unwrap();r.cancel(&mut run).unwrap();
        while r.poll(&mut run).unwrap()==State::Running {std::thread::yield_now();}
        let pid=std::fs::read_to_string(root.join("descendant.pid")).unwrap().parse::<u32>().unwrap();
        let identity=coreq::process::identity(pid).unwrap().unwrap();drop(run);
        let until=Instant::now()+Duration::from_secs(5);
        while coreq::process::identity(pid).unwrap().as_ref()==Some(&identity) {assert!(Instant::now()<until,"owned descendant survived native job teardown");std::thread::yield_now();}
    }
    let mut large=l.clone();large.input="x".repeat(9*1024*1024);large.env.push(("AGENTQ_FAKE_ACP_LARGE".into(),"1".into()));
    large.deadline=None;
    let mut large_run=r.launch(&large).unwrap();r.cancel(&mut large_run).unwrap();
    while r.poll(&mut large_run).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(r.report(&large_run).unwrap().outcome,"cancelled");
    let mut malformed=l.clone();malformed.log=root.join("malformed.jsonl");malformed.deadline=Some(Instant::now()+Duration::from_secs(20));
    malformed.env.push(("AGENTQ_FAKE_ACP_BAD_FIELD".into(),"1".into()));
    let mut invalid=r.launch(&malformed).unwrap();
    let error=loop {match r.progress(&mut invalid) {Err(error)=>break error,Ok(_)=>std::thread::yield_now()}};
    assert!(error.contains("ACP payload invalid"));
    assert!(std::fs::read_to_string(&malformed.log).unwrap().contains("\"text\":17"),"malformed native fields stay in raw evidence");drop(invalid);
    l.deadline=Some(Instant::now()+Duration::from_secs(20));
    l.env.push(("AGENTQ_FAKE_ACP_BAD_VERSION".into(),"1".into()));
    assert!(r.launch(&l).err().unwrap().contains("workspace write defect"));
    l.env.pop();
    for version in ["1.4.2","1.4.3","1.4.4","1.4.5","1.5.1","unknown"] {
        let mut old=l.clone();old.env.push(("AGENTQ_FAKE_ACP_PROBE_VERSION".into(),version.into()));
        let before=std::fs::metadata(&l.log).unwrap().len();assert!(r.preflight(&old,&agent::Repair::default()).checks.iter().any(|c|c.check=="client" && c.state=="blocked"));assert!(r.launch(&old).is_err());assert_eq!(std::fs::metadata(&l.log).unwrap().len(),before,"version refused before ACP spawn");
    }
    l.env.push(("AGENTQ_FAKE_ACP_EOF".into(),"1".into()));
    let mut ended=r.launch(&l).unwrap();
    while r.poll(&mut ended).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(r.poll(&mut ended).unwrap(),State::Unknown);
    assert!(r.report(&ended).is_err());
    let stop=loop {if let Some(stop)=r.stop_event(&mut ended,u64::MAX).unwrap() {break stop;}std::thread::yield_now();};
    assert_eq!(stop.cause,agent::history::stop::StopCause::ExitWithoutTurnEnd);
    assert_eq!(stop.exit_code,Some(0));assert_ne!(stop.outcome,"ok");
    assert!(stop.last_activity.is_some());
    std::fs::remove_dir_all(root).unwrap();
    println!("ACP trait: launch, choices, model/mode, tools, resume replay, cancel, unknown updates, version refusal and EOF passed");
}

fn server() {
    if std::env::args().any(|s|s=="--version") {println!("agy {}",std::env::var("AGENTQ_FAKE_ACP_PROBE_VERSION").unwrap_or("1.5.2".into()));return;}
    #[cfg(windows)] if std::env::var_os("AGENTQ_FAKE_ACP_DESCENDANT").is_some() {
        assert_eq!(std::env::var("AGENTQ_JOB_CONFIGURED").unwrap(),"1");
        let child=std::process::Command::new(std::env::current_exe().unwrap()).env("AGENTQ_FAKE_DESCENDANT","1").spawn().unwrap();
        std::fs::write("descendant.pid",child.id().to_string()).unwrap();
    }
    let mut asked=false;let mut cancelled=false;let mut linger=false;
    let mut prompt=Value::Null;
    let emit=|v:Value| {println!("{v}");std::io::stdout().flush().unwrap();};
    let reply=|v:&Value,result:Value|emit(json!({"jsonrpc":"2.0","id":v["id"],"result":result}));
    let update=|u:Value|emit(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"session:opaque_/1","update":u}}));
    for line in std::io::stdin().lock().lines() {
        let v:Value=serde_json::from_str(&line.unwrap()).unwrap();
        match v["method"].as_str() {
            Some("initialize")=>{emit(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error fixture"}}));reply(&v,json!({"protocolVersion":1,"agentInfo":{"version":if std::env::var_os("AGENTQ_FAKE_ACP_BAD_VERSION").is_some(){"1.4.3"}else{"1.5.2"}},"agentCapabilities":{"loadSession":true},"_meta":{"large":if std::env::var_os("AGENTQ_FAKE_ACP_LARGE").is_some(){"x".repeat(9*1024*1024)}else{String::new()}}}));},
            Some("session/new"|"session/load")=>{
                assert!(PathBuf::from(std::env::var_os("GEMINI_HOME").unwrap()).ends_with("account"));
                if v["method"]=="session/load" {update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old replay"}}));}
                reply(&v,json!({"sessionId":"session:opaque_/1"}));
            },
            Some("session/set_model")=>{assert_eq!(v["params"]["modelId"],"model-1");reply(&v,json!({}));},
            Some("session/set_mode")=>{assert_eq!(v["params"]["modeId"],"accept-edits");reply(&v,json!({}));},
            Some("session/prompt")=>{
                if std::env::var_os("AGENTQ_FAKE_ACP_EOF").is_some() {return;}
                if std::env::var_os("AGENTQ_FAKE_ACP_BAD_FIELD").is_some() {update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":17}}));return;}
                prompt=v["id"].clone();
                if v["params"]["prompt"][0]["text"]=="second" {continue;}
                if v["params"]["prompt"][0]["text"]=="linger" {linger=true;emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"end_turn"}}));continue;}
                asked=true;
                emit(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error while running"}}));
                emit(json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{"sessionId":"session:opaque_/1","toolCall":{"toolCallId":"tool-1","title":"Run fixture"},"options":[{"optionId":"once","kind":"allow_once","name":"Allow"}]}}));
            },
            Some("session/cancel")=>{cancelled=true;if !asked {emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}}));}},
            None if v["id"]=="permission-1"=>{
                asked=false;
                if v["result"]["outcome"]["outcome"]=="cancelled" {assert!(cancelled);emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}}));continue;}
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
    if linger {loop {std::thread::park();}}
}
