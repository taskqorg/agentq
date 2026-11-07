//! M12.24: owned Claude-shaped protocol only; no native/model qualification.
use super::*;
use agent::{Answer,Control,Event,Part,State};
use std::io::{BufRead,Write};
use std::fs::File;

fn request(runtime:&dyn agent::Runtime,run:&mut agent::Run)->Event {
    let until=Instant::now()+Duration::from_secs(30);
    loop {
        if let Some(event)=runtime.progress(run).unwrap().into_iter().find(|e|matches!(e,Event::Request{..})) {return event;}
        assert_eq!(runtime.poll(run).unwrap(),State::Running);assert!(Instant::now()<until,"request not observed");std::thread::yield_now();
    }
}

#[cfg(unix)]
pub fn owner(root:&Path) {
    let runtime=agent::runtime("claude").unwrap();let mut l=fake_launch("claude",root,"first",&[]);l.controls=true;
    l.env.extend([("AGENTQ_CLAUDE_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CLAUDE_CONTROL_HOLD".into(),root.as_os_str().to_owned())]);
    let mut run=runtime.launch(&l).unwrap();let Event::Request{id,..}=request(runtime,&mut run) else {unreachable!()};
    runtime.respond(&mut run,Control::Reply{id:id.clone(),option:None}).unwrap();
    loop {if runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,Event::TurnEnd{..})) {break;}std::thread::yield_now();}
    let saved=runtime.checkpoint(&run).unwrap();let native=agent::NativeProcess::read(run.id().unwrap()).unwrap().unwrap();
    std::fs::write(root.join("owner-ready.json"),json!({"saved":saved,"native":native,"old_request":id}).to_string()).unwrap();std::process::exit(0);
}

#[cfg(unix)]
pub fn recovery() {
    let root=scratch("claude-control-recovery");
    struct Release(PathBuf);impl Drop for Release {fn drop(&mut self) {let _=std::fs::write(self.0.join("server-release"),"");}}
    let release=Release(root.clone());let mut child=Command::new(exe()).arg("--claude-control-owner").arg(&root).spawn().unwrap();
    wait_for(&root.join("owner-ready.json"));assert!(child.wait().unwrap().success());
    let value:Value=serde_json::from_slice(&std::fs::read(root.join("owner-ready.json")).unwrap()).unwrap();let native:agent::NativeProcess=serde_json::from_value(value["native"].clone()).unwrap();
    let runtime=agent::runtime("claude").unwrap();let mut l=fake_launch("claude",&root,"second",&[]);l.controls=true;
    l.env.extend([("AGENTQ_CLAUDE_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CLAUDE_CONTROL_HOLD".into(),root.as_os_str().to_owned())]);
    let session=value["saved"]["claude"]["session"].as_str().unwrap();assert_eq!(agent::NativeProcess::read(native.pid).unwrap(),Some(native.clone()));
    assert!(runtime.adopt(session,&l,&[]).is_err());let before=std::fs::read(root.join("claude-input.jsonl")).unwrap();
    let mut recovered=runtime.recover(&value["saved"],&l).unwrap();assert_eq!(runtime.poll(&mut recovered).unwrap(),State::Finished);
    assert!(runtime.progress(&mut recovered).unwrap().is_empty(),"saved progress does not replay drained native events");
    let ctx=Context {product:"fixtureq",version:"test",project:"other",task:None,route:None,account:"other"};let snapshot=recovered.snapshot(&ctx).unwrap().save();assert!(snapshot["records"][0]["observation"]["duration_us"].is_null(),"recovery does not fabricate a duration from replay time");
    assert!(runtime.respond(&mut recovered,Control::Reply{id:value["old_request"].clone(),option:Some("allow".into())}).is_err());
    assert!(runtime.input(&mut recovered,&[Part::Text("replay".into())]).is_err());assert!(runtime.set_option(&mut recovered,Control::Model("sonnet".into())).is_err());
    assert!(runtime.compact(&mut recovered).is_err());assert!(runtime.cancel(&mut recovered).is_err());drop(recovered);
    assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),before);assert_eq!(agent::NativeProcess::read(native.pid).unwrap(),Some(native));assert!(runtime.adopt(session,&l,&[]).is_err());
    drop(release);let until=Instant::now()+Duration::from_secs(30);
    loop {let entries=runtime.orphans(&l).unwrap();if entries.iter().all(|e|e.scope=="absent") {break;}assert!(Instant::now()<until,"original Claude scope remains occupied: {entries:?}");std::thread::yield_now();}
    runtime.adopt(session,&l,&[]).unwrap();teardown(root);
}

pub fn controls() {
    let ctx=Context {product:"fixtureq",version:"test",project:"other",task:None,route:None,account:"other"};
    let root=scratch("live-claude-controls");let runtime=agent::runtime("claude").unwrap();
    let mut l=fake_launch("claude",&root,"first",&[]);l.controls=true;l.env.push(("AGENTQ_CLAUDE_CONTROL_FIXTURE".into(),"1".into()));
    let mut run=runtime.launch(&l).unwrap();let session=runtime.session_id(&run).unwrap();
    let Event::Request{id:first,choices,..}=request(runtime,&mut run) else {unreachable!()};
    assert_ne!(first,json!("native-reused"));assert_eq!(choices.iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),["allow","deny"]);
    assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("invented".into())}).is_err());
    runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("allow".into())}).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("allow".into())}).is_err());
    let Event::Request{id:question,questions,..}=request(runtime,&mut run) else {unreachable!()};
    assert_ne!(first,question);assert!(questions[0].multiple);assert!(!questions[1].multiple);assert!(questions.iter().all(|q|q.free_text));
    let answers=std::collections::BTreeMap::from([("q1".into(),vec![Answer::Choice("A".into()),Answer::Choice("B".into())]),("q2".into(),vec![Answer::Text("typed answer".into())])]);
    runtime.respond(&mut run,Control::Answers{id:question,answers}).unwrap();finish(runtime,&mut run);
    assert_eq!(runtime.report(&run).unwrap().outcome,"ok");let original=run.snapshot(&ctx).unwrap().save();
    assert!(runtime.resume(&session,&l).is_err(),"native result retains live controlled writer");
    assert!(runtime.steer(&mut run,&[Part::Text("not a steer".into())]).is_err());
    runtime.set_option(&mut run,Control::Model("sonnet".into())).unwrap();
    assert_eq!(run.snapshot(&ctx).unwrap().save(),original,"model receipt does not rewrite prior result");
    runtime.compact(&mut run).unwrap();finish(runtime,&mut run);let compact=run.snapshot(&ctx).unwrap().save();
    assert_ne!(compact,original,"native compact command has its own physical turn boundary");
    runtime.input(&mut run,&[Part::Text("next".into())]).unwrap();let Event::Request{id:next,..}=request(runtime,&mut run) else {unreachable!()};assert_ne!(first,next);
    runtime.cancel(&mut run).unwrap();assert!(runtime.respond(&mut run,Control::Reply{id:next,option:Some("allow".into())}).is_err());finish(runtime,&mut run);
    assert_eq!(runtime.report(&run).unwrap().outcome,"failed","generic native error result does not prove native interruption cause");
    runtime.input(&mut run,&[Part::Text("after terminal".into())]).unwrap();let Event::Request{id:pending,..}=request(runtime,&mut run) else {unreachable!()};
    let owner=std::fs::read_dir(&l.session_store.path).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();
    let original_owner=std::fs::read(&owner).unwrap();let mut row:Value=serde_json::from_slice(&original_owner).unwrap();row["token"]=json!("foreign");std::fs::write(&owner,row.to_string()).unwrap();
    let before=std::fs::read(root.join("claude-input.jsonl")).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:pending.clone(),option:Some("allow".into())}).is_err());assert!(runtime.set_option(&mut run,Control::Model("haiku".into())).is_err());
    assert!(runtime.cancel(&mut run).is_err());assert!(runtime.input(&mut run,&[Part::Text("foreign".into())]).is_err());assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),before);
    std::fs::write(owner,original_owner).unwrap();runtime.respond(&mut run,Control::Reply{id:pending,option:None}).unwrap();finish(runtime,&mut run);
    let log=runtime.transcript(&run).unwrap();assert_eq!(log,l.log);
    let rows:Vec<Value>=std::fs::read_to_string(&log).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).filter(|r:&Value|r["type"]=="agentq_control").collect();
    assert!(rows.iter().all(|r|r["session_id"]==session));assert!(rows.iter().any(|r|r["state"]=="reply-sent" && r["native"]["response"]["response"]["updatedInput"]["answers"]["Pick"]==json!(["A","B"])));
    let mut history=runtime.history(&session,None,Some(&log),None).unwrap();let steps=history.steps(std::fs::metadata(&log).unwrap().len()).unwrap().steps;
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Question && s.state=="reply-sent"));drop(run);teardown(root);
}

pub fn boundaries() {
    for mode in ["resolved","foreign-session","unknown-result","deny-policy","foreign-receipt"] {
        let root=scratch("claude-control-boundaries");let runtime=agent::runtime("claude").unwrap();let mut l=fake_launch("claude",&root,"first",&[]);l.controls=true;
        l.env.extend([("AGENTQ_CLAUDE_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CLAUDE_CONTROL_EXTRA".into(),mode.into())]);if mode=="deny-policy" {l.policy.approval=agent::Approval::Deny;}
        let mut run=runtime.launch(&l).unwrap();let Event::Request{id:first,choices,..}=request(runtime,&mut run) else {unreachable!()};
        let before=std::fs::read(root.join("claude-input.jsonl")).unwrap();
        if mode=="foreign-receipt" {
            assert!(runtime.set_option(&mut run,Control::Model("sonnet".into())).is_err(),"a matching request ID does not override a foreign session claim");
            let after=std::fs::read(root.join("claude-input.jsonl")).unwrap();assert!(after.len()>before.len(),"only the original authorized model request was sent");
            assert!(runtime.respond(&mut run,Control::Reply{id:first,option:Some("allow".into())}).is_err());assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),after);
        }else if mode=="deny-policy" {
            assert_eq!(choices.iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),["deny"]);assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("allow".into())}).is_err());
            assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),before);runtime.respond(&mut run,Control::Reply{id:first,option:None}).unwrap();finish(runtime,&mut run);
        }else{
            std::fs::write(root.join("inject"),"").unwrap();wait_for(&root.join("emitted"));
            if mode=="resolved" {
                let Event::Request{id:next,..}=request(runtime,&mut run) else {unreachable!()};assert_ne!(first,next);
                assert!(runtime.respond(&mut run,Control::Reply{id:first,option:Some("allow".into())}).is_err());assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),before);
                runtime.respond(&mut run,Control::Reply{id:next,option:None}).unwrap();finish(runtime,&mut run);
            }else{
                let until=Instant::now()+Duration::from_secs(30);loop {if runtime.progress(&mut run).is_err() {break;}assert!(Instant::now()<until,"unknown native record not refused");std::thread::yield_now();}
                assert_eq!(runtime.poll(&mut run).unwrap(),State::Unknown);assert!(runtime.respond(&mut run,Control::Reply{id:first,option:Some("allow".into())}).is_err());
                assert!(runtime.input(&mut run,&[Part::Text("unknown".into())]).is_err());assert!(runtime.set_option(&mut run,Control::Model("sonnet".into())).is_err());
                assert!(runtime.compact(&mut run).is_err());assert!(runtime.cancel(&mut run).is_err());assert_eq!(std::fs::read(root.join("claude-input.jsonl")).unwrap(),before,"foreign/unknown protocol never generates an automatic native reply");
            }
        }
        drop(run);teardown(root);
    }
}

pub fn server() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.first().is_some_and(|s|s=="--version") {println!("2.1.319 (Claude Code)");return;}
    if args.first().is_some_and(|s|s=="auth") {println!("{}",json!({"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}));return;}
    assert!(args.windows(2).any(|p|p==["--input-format","stream-json"]));assert!(args.windows(2).any(|p|p==["--permission-prompt-tool","stdio"]));
    let session=args.windows(2).find(|p|p[0]=="--session-id" || p[0]=="--resume").unwrap()[1].clone();
    let emit=|value:Value| {println!("{value}");std::io::stdout().flush().unwrap();};let mut question=false;
    for line in std::io::stdin().lock().lines() {
        let value:Value=serde_json::from_str(&line.unwrap()).unwrap();let mut audit=File::options().create(true).append(true).open(std::env::current_dir().unwrap().join("claude-input.jsonl")).unwrap();writeln!(audit,"{value}").unwrap();audit.sync_data().unwrap();
        let result=|is_error:bool| json!({"type":"result","subtype":if is_error {"error_during_execution"}else{"success"},"is_error":is_error,"session_id":session,"result":"owned result"});
        match value["type"].as_str() {
            Some("control_request")=>{
                let subtype=value["request"]["subtype"].as_str().unwrap();assert!(matches!(subtype,"initialize"|"set_model"|"interrupt"));
                if subtype=="set_model" {assert_eq!(value["request"]["model"],"sonnet");}
                let mut receipt=json!({"type":"control_response","response":{"subtype":"success","request_id":value["request_id"],"response":{}}});
                if subtype=="set_model" && std::env::var("AGENTQ_CLAUDE_CONTROL_EXTRA").as_deref()==Ok("foreign-receipt") {receipt["session_id"]=json!("foreign");}
                emit(receipt);if subtype=="interrupt" {emit(result(true));}
            },
            Some("user")=>{
                assert_eq!(value["session_id"],session);question=false;emit(json!({"type":"system","subtype":"init","session_id":session}));
                if value["message"]["content"][0]["text"]=="/compact" {emit(json!({"type":"system","subtype":"compact_boundary","session_id":session}));emit(result(false));}
                else{
                    emit(json!({"type":"control_request","request_id":"native-reused","request":{"subtype":"can_use_tool","tool_name":"Bash","tool_use_id":"tool","input":{"command":"owned fixture"}}}));
                    if let Ok(mode)=std::env::var("AGENTQ_CLAUDE_CONTROL_EXTRA") {if mode!="deny-policy" && mode!="foreign-receipt" {
                        let root=std::env::current_dir().unwrap();let session=session.clone();
                        std::thread::spawn(move|| {wait_for(&root.join("inject"));
                            if mode=="resolved" {
                                println!("{}",json!({"type":"control_cancel_request","request_id":"unrelated"}));println!("{}",json!({"type":"control_cancel_request","request_id":[]}));
                                println!("{}",json!({"type":"control_cancel_request","request_id":"native-reused"}));
                                println!("{}",json!({"type":"control_request","request_id":"native-reused","request":{"subtype":"can_use_tool","tool_name":"Bash","tool_use_id":"tool-new","input":{"command":"owned fixture"}}}));
                            }else if mode=="foreign-session" {println!("{}",json!({"type":"control_request","session_id":"foreign","request_id":"extra","request":{"subtype":"can_use_tool","tool_name":"Bash","tool_use_id":"foreign","input":{}}}));}
                            else{println!("{}",json!({"type":"result","session_id":session,"subtype":"future-status","is_error":false}));}
                            std::io::stdout().flush().unwrap();std::fs::write(root.join("emitted"),"").unwrap();
                        });
                    }}
                }
            },
            Some("control_response")=>{
                assert_eq!(value["response"]["request_id"],"native-reused");let response=&value["response"]["response"];
                if response["behavior"]=="deny" {emit(result(false));continue;}
                assert_eq!(response["behavior"],"allow");
                if question {assert_eq!(response["updatedInput"]["answers"],json!({"Pick":["A","B"],"Enter":"typed answer"}));assert_eq!(response["updatedInput"]["retained"],true);emit(result(false));}
                else{
                    assert_eq!(response["updatedInput"],json!({"command":"owned fixture"}));question=true;
                    emit(json!({"type":"control_request","request_id":"native-reused","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"question","input":{"retained":true,"questions":[{"question":"Pick","options":[{"label":"A"},{"label":"B"}],"multiSelect":true},{"question":"Enter","options":[]}]}}}));
                }
            },
            _=>panic!("unexpected Claude fixture input: {value}"),
        }
    }
    if let Some(root)=std::env::var_os("AGENTQ_CLAUDE_CONTROL_HOLD") {wait_for(&PathBuf::from(root).join("server-release"));}
}
