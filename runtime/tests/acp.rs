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
    if let Some(marker)=std::env::var_os("AGENTQ_FAKE_RAW") {
        let args=std::env::args().skip(1).collect::<Vec<_>>();std::fs::write(marker,serde_json::to_vec(&args).unwrap()).unwrap();
        if args.iter().any(|a|a=="--version") {println!("agy 1.5.2");return;}
        let _=std::io::stdin().lock().lines().next();
        println!("{}",json!({"event":"init","session_id":"raw-session-1"}));return;
    }
    if std::env::var_os("AGENTQ_FAKE_ACP").is_some() {return server();}
    let root=std::env::temp_dir().join(format!("agentq-acp-{}",std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let expected=root.join("gemini-root");
    assert!(std::process::Command::new(std::env::current_exe().unwrap()).env("AGENTQ_CHECK_ACP_HOME",&expected).env("GEMINI_HOME",&expected).env("ANTIGRAVITY_HOME",root.join("old-data")).status().unwrap().success());
    let r=agent::runtime("antigravity").unwrap();
    raw_session_boundary(&root);
    let mut l=Launch::new(&root,root.join("acp.jsonl"),"first","fixtureq");
    l.command=vec![std::env::current_exe().unwrap().into_os_string()];
    l.home=Some(root.join("account"));std::fs::create_dir_all(l.home.as_ref().unwrap()).unwrap();
    l.env=vec![("AGENTQ_FAKE_ACP".into(),"1".into())];
    l.deadline=Some(Instant::now()+Duration::from_secs(20));
    l.process.configure=Some(std::sync::Arc::new(|command|{
        assert!(command.get_envs().any(|(name,value)|name=="GEMINI_API_KEY" && value.is_none()),"structured spawn must retain account environment cleaning");
    }));
    l.model=Some("model-1".into());
    l.policy.approval=agent::Approval::Edits;
    assert!(r.resume("untracked",&l).err().unwrap().starts_with("session-owner-unknown"));
    let mut bad_store=l.clone();bad_store.session_store=agent::SessionStore::new("relative-owners");
    assert_eq!(r.launch(&bad_store).err().unwrap(),"session-store-requires-absolute-path");
    l.inactivity=Some(Duration::from_secs(20));
    let mut run=r.launch(&l).unwrap();
    assert!(run.owner().is_some(),"ACP must retain the mandatory owner handle");
    let session=r.session_id(&run).unwrap();
    assert_eq!(run.owner().unwrap().checkpoint().unwrap()["session"],session,"native identity binds before input");
    assert!(r.resume(&session,&l).err().unwrap().starts_with("session-busy"));
    let mut conflicting=l.clone();conflicting.log=root.join("conflicting.jsonl");
    conflicting.env.push(("AGENTQ_FAKE_ACP_SESSION".into(),session.clone().into()));
    assert!(r.launch(&conflicting).err().unwrap().starts_with("session-busy"));
    let raw=std::fs::read_to_string(&conflicting.log).unwrap();
    assert!(!raw.lines().filter_map(|row|serde_json::from_str::<Value>(row).ok()).any(|row|row["outgoing"]==true && row["message"]["method"]=="session/prompt"),"failed binding cannot submit input");
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;assert_eq!(std::fs::metadata(&l.log).unwrap().permissions().mode() & 0o777,0o600);}
    assert_eq!(r.checkpoint(&run).unwrap_err(),agent::Cause("no-checkpoint"));
    assert_eq!(r.observe(&mut run).activity.state,"active","observation sees native activity without consuming progress");
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
    let mut history=r.history(&session,None,Some(&l.log),None).unwrap();
    let steps=history.steps(std::fs::metadata(&l.log).unwrap().len()).unwrap().steps;
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Message && s.data["text"]=="answer"));
    let tool=steps.iter().find(|s|s.native_id.as_deref()==Some("tool-1")).unwrap();
    assert_eq!(tool.kind,agent::history::StepKind::Shell);assert_eq!(tool.state,"completed");assert_eq!(tool.origins.len(),2);
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Approval));
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Unknown));
    l.input="second".into();
    let mut resumed=r.resume(&session,&l).unwrap();
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
    assert!(matches!(report.termination,agent::ProcessTermination::Unknown{..}),"native settlement does not prove server termination");
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
    #[cfg(unix)] {
        let mut signalled=l.clone();signalled.input="signalled".into();signalled.log=root.join("signalled.jsonl");
        let mut native=r.launch(&signalled).unwrap();
        let mut info:libc::siginfo_t=unsafe {std::mem::zeroed()};
        assert_eq!(unsafe {libc::waitid(libc::P_PID,native.id().unwrap(),&mut info,libc::WEXITED|libc::WNOWAIT)},0,"wait only for this owned no-model fixture");
        while r.poll(&mut native).unwrap()==State::Running {std::thread::yield_now();}
        let report=r.report(&native).unwrap();
        assert_eq!((report.outcome,report.exit),("ok",None));
        assert_eq!(report.termination,agent::ProcessTermination::Observed);
        assert_eq!(report.process.exit_unknown,"signalled");
        let stop=r.stop_event(&mut native,u64::MAX).unwrap().unwrap();
        assert_eq!(stop.cause,agent::history::stop::StopCause::Signal);assert_eq!(stop.exit_code,None);
        assert_eq!(r.report(&native).unwrap().outcome,"ok");
    }
    let mut malformed=l.clone();malformed.log=root.join("malformed.jsonl");malformed.deadline=Some(Instant::now()+Duration::from_secs(20));
    malformed.env.push(("AGENTQ_FAKE_ACP_BAD_FIELD".into(),"1".into()));
    let mut invalid=r.launch(&malformed).unwrap();
    let error=loop {match r.progress(&mut invalid) {Err(error)=>break error,Ok(_)=>std::thread::yield_now()}};
    assert!(error.contains("ACP payload invalid"));
    assert!(std::fs::read_to_string(&malformed.log).unwrap().contains("\"text\":17"),"malformed native fields stay in raw evidence");drop(invalid);
    // Rejected completions and transport bytes must stay evidence, never become success on readback.
    let mut rejected=l.clone();rejected.input="rejected-completion".into();rejected.log=root.join("rejected.jsonl");
    let mut invalid=r.launch(&rejected).unwrap();
    let error=loop {match r.progress(&mut invalid) {Err(error)=>break error,Ok(_)=>std::thread::yield_now()}};
    assert!(error.contains("ACP payload invalid"));
    let raw=std::fs::read_to_string(&rejected.log).unwrap();
    assert_ne!(r.classify(&raw).outcome,"ok","rejected usage must not become retained success");
    assert!(!r.log_events(&raw).iter().any(|e|matches!(e,Event::TurnEnd{..})),"rejected completion has no terminal authority");
    assert!(raw.contains("bad-counter"));drop(invalid);
    for (input,bytes) in [("raw-json",b"{\"jsonrpc\":\"2.0\",MALFORMED_RAW_SENTINEL}\n".as_slice()),("raw-utf8",b"{\xffUTF8_RAW_SENTINEL}\n".as_slice()),("raw-tail",b"{\"TAIL_RAW_SENTINEL\":".as_slice())] {
        let mut malformed=l.clone();malformed.input=input.into();malformed.log=root.join(format!("{input}.jsonl"));
        let mut invalid=r.launch(&malformed).unwrap();
        while r.poll(&mut invalid).unwrap()==State::Running {std::thread::yield_now();}
        assert_eq!(r.poll(&mut invalid).unwrap(),State::Unknown);
        assert_eq!(std::fs::read(malformed.log.with_extension("stderr")).unwrap(),bytes,"exact rejected bytes retained privately");
        drop(invalid);
    }
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

fn raw_session_boundary(root:&std::path::Path) {
    let r=agent::runtime("antigravity").unwrap();let marker=root.join("raw-spawn.json");
    let mut l=Launch::new(root,root.join("raw.jsonl"),"fixture","fixtureq");l.home=Some(root.into());
    l.env=vec![("AGENTQ_FAKE_RAW".into(),marker.clone().into_os_string())];
    for (session,resume) in [("--foreign-flag",true),("-x",false),("opaque:/file",true),("one two",true),("one\nline",true),("",false),("-",true)] {
        let owned=if session.is_empty() || session=="-" {"raw-owned-session"}else{session};
        r.adopt(owned,&l,&[]).unwrap();let owner=r.session_owner(Some(owned),&l).unwrap();
        let spec=agent::NativeSpec {root:root.into(),session:session.into(),resume,home:l.home.clone(),env:l.env.clone(),owner:Some(owner),..Default::default()};
        let result=r.turn(std::process::Command::new(std::env::current_exe().unwrap()),&spec,&l.log,&l.input,&agent::Process::default());
        let accepted=result.is_ok();if let Ok(mut turn)=result {turn.wait(Some(Instant::now()+Duration::from_secs(20))).unwrap();}
        assert!(!accepted && !marker.exists(),"opaque ownership must not authorize raw CLI spawn: {session:?}");
        assert!(r.native_arguments(&spec).is_err(),"raw argument inspection must reject {session:?}");
        assert_eq!(r.turn(std::process::Command::new(root.join("agentapi")),&spec,&l.log,&l.input,&agent::Process::default()).err().as_deref(),Some("native CLI session invalid"),"agentapi comparison seam rejects before preflight");
    }
    let owner=r.session_owner(None,&l).unwrap();
    let fresh=agent::NativeSpec {root:root.into(),session:"-".into(),owner:Some(owner),..Default::default()};
    assert!(r.native_arguments(&fresh).is_ok(),"new-start sentinel remains valid");drop(fresh);
    r.adopt("raw-session-1",&l,&[]).unwrap();
    let spec=agent::NativeSpec {root:root.into(),session:"raw-session-1".into(),resume:true,home:l.home.clone(),env:l.env.clone(),owner:Some(r.session_owner(Some("raw-session-1"),&l).unwrap()),..Default::default()};
    let mut turn=r.turn(std::process::Command::new(std::env::current_exe().unwrap()),&spec,&l.log,&l.input,&agent::Process::default()).unwrap();
    turn.wait(Some(Instant::now()+Duration::from_secs(20))).unwrap();
    let args:Vec<String>=serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    assert!(args.windows(2).any(|a|a==["--conversation","raw-session-1"]));std::fs::remove_file(marker).unwrap();
}

fn server() {
    if std::env::args().any(|s|s=="--version") {println!("agy {}",std::env::var("AGENTQ_FAKE_ACP_PROBE_VERSION").unwrap_or("1.5.2".into()));return;}
    #[cfg(windows)] if std::env::var_os("AGENTQ_FAKE_ACP_DESCENDANT").is_some() {
        assert_eq!(std::env::var("AGENTQ_JOB_CONFIGURED").unwrap(),"1");
        let child=std::process::Command::new(std::env::current_exe().unwrap()).env("AGENTQ_FAKE_DESCENDANT","1").spawn().unwrap();
        std::fs::write("descendant.pid",child.id().to_string()).unwrap();
    }
    let mut session=std::env::var("AGENTQ_FAKE_ACP_SESSION").unwrap_or_else(|_|format!("--session:opaque_/{}",std::process::id()));
    let mut asked=false;let mut cancelled=false;let mut linger=false;
    let mut prompt=Value::Null;
    let emit=|v:Value| {println!("{v}");std::io::stdout().flush().unwrap();};
    let reply=|v:&Value,result:Value|emit(json!({"jsonrpc":"2.0","id":v["id"],"result":result}));
    let update=|session:&str,u:Value|emit(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session,"update":u}}));
    for line in std::io::stdin().lock().lines() {
        let v:Value=serde_json::from_str(&line.unwrap()).unwrap();
        match v["method"].as_str() {
            Some("initialize")=>{emit(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error fixture"}}));reply(&v,json!({"protocolVersion":1,"agentInfo":{"version":if std::env::var_os("AGENTQ_FAKE_ACP_BAD_VERSION").is_some(){"1.4.3"}else{"1.5.2"}},"agentCapabilities":{"loadSession":true},"_meta":{"large":if std::env::var_os("AGENTQ_FAKE_ACP_LARGE").is_some(){"x".repeat(9*1024*1024)}else{String::new()}}}));},
            Some("session/new"|"session/load")=>{
                assert!(PathBuf::from(std::env::var_os("GEMINI_HOME").unwrap()).ends_with("account"));
                if v["method"]=="session/load" {session=v["params"]["sessionId"].as_str().unwrap().into();update(&session,json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old replay"}}));}
                reply(&v,json!({"sessionId":session}));
            },
            Some("session/set_model")=>{assert_eq!(v["params"]["modelId"],"model-1");reply(&v,json!({}));},
            Some("session/set_mode")=>{assert_eq!(v["params"]["modeId"],"accept-edits");reply(&v,json!({}));},
            Some("session/prompt")=>{
                if std::env::var_os("AGENTQ_FAKE_ACP_EOF").is_some() {return;}
                if std::env::var_os("AGENTQ_FAKE_ACP_BAD_FIELD").is_some() {update(&session,json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":17}}));return;}
                match v["params"]["prompt"][0]["text"].as_str() {
                    #[cfg(unix)] Some("signalled")=>{reply(&v,json!({"stopReason":"end_turn"}));unsafe {libc::raise(libc::SIGTERM)};unreachable!();},
                    Some("rejected-completion")=>{reply(&v,json!({"stopReason":"end_turn","usage":{"inputTokens":"bad-counter"}}));return;},
                    Some("raw-json")=>{std::io::stdout().write_all(b"{\"jsonrpc\":\"2.0\",MALFORMED_RAW_SENTINEL}\n").unwrap();return;},
                    Some("raw-utf8")=>{std::io::stdout().write_all(b"{\xffUTF8_RAW_SENTINEL}\n").unwrap();return;},
                    Some("raw-tail")=>{std::io::stdout().write_all(b"{\"TAIL_RAW_SENTINEL\":").unwrap();return;},
                    _=>{},
                }
                prompt=v["id"].clone();
                if v["params"]["prompt"][0]["text"]=="second" {continue;}
                if v["params"]["prompt"][0]["text"]=="linger" {linger=true;emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"end_turn"}}));continue;}
                asked=true;
                emit(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error while running"}}));
                emit(json!({"jsonrpc":"2.0","id":"permission-1","method":"session/request_permission","params":{"sessionId":session,"toolCall":{"toolCallId":"tool-1","title":"Run fixture"},"options":[{"optionId":"once","kind":"allow_once","name":"Allow"}]}}));
            },
            Some("session/cancel")=>{cancelled=true;if !asked {emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}}));}},
            None if v["id"]=="permission-1"=>{
                asked=false;
                if v["result"]["outcome"]["outcome"]=="cancelled" {assert!(cancelled);emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"cancelled"}}));continue;}
                assert_eq!(v["result"]["outcome"]["optionId"],"once");
                update(&session,json!({"sessionUpdate":"tool_call","toolCallId":"tool-1","kind":"execute","title":"Fixture","status":"in_progress"}));
                update(&session,json!({"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed","rawOutput":{"stdout":"proof"}}));
                update(&session,json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}));
                update(&session,json!({"sessionUpdate":"future_update","raw":"kept"}));
                emit(json!({"jsonrpc":"2.0","id":prompt,"result":{"stopReason":"end_turn","usage":{"inputTokens":17,"outputTokens":2}}}));
            },
            _=>panic!("unexpected ACP message: {v}"),
        }
    }
    if linger {loop {std::thread::park();}}
}
