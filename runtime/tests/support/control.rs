//! M12.24: owned protocol proof, never native/model qualification.
use super::*;
use agent::{Answer,Control,Event,Part,Runtime};
use std::io::{BufRead,Write};

pub fn owner_transitions() {
    for mode in ["bind-owner","terminal-owner"] {
        let root=scratch("control-owner-transition");let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",&root,"first",&[]);l.controls=true;
        l.env.extend([("AGENTQ_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CONTROL_EXTRA".into(),mode.into())]);
        if mode=="bind-owner" {
            assert_eq!(runtime.launch(&l).err().unwrap().detail,"control-owner-changed");
        }else{
            let mut run=runtime.launch(&l).unwrap();
            loop {if runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,Event::Request{..})) {break;}std::thread::yield_now();}
            let path=std::fs::read_dir(&l.session_store.path).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();
            let mut row:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();row["token"]=json!("foreign");std::fs::write(&path,row.to_string()).unwrap();
            std::fs::write(root.join("changed-owner"),row.to_string()).unwrap();let before=std::fs::read(root.join("control-input.jsonl")).unwrap();
            std::fs::write(root.join("inject"),"").unwrap();wait_for(&root.join("emitted"));let until=Instant::now()+Duration::from_secs(30);
            let error=loop {if let Err(error)=runtime.poll(&mut run) {break error;}assert!(Instant::now()<until,"terminal not observed");std::thread::yield_now();};
            assert_eq!(error,"control-owner-changed");assert!(runtime.input(&mut run,&[Part::Text("unauthorized next".into())]).is_err());drop(run);
            assert_eq!(std::fs::read(root.join("control-input.jsonl")).unwrap(),before,"lost terminal owner sends no further input");
        }
        let rows=std::fs::read_dir(&l.session_store.path).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e=="json")).collect::<Vec<_>>();
        assert_eq!(rows.len(),1);assert_eq!(std::fs::read(&rows[0]).unwrap(),std::fs::read(root.join("changed-owner")).unwrap(),"neither transition nor drop restores stale authority");
        let audit:Vec<Value>=std::fs::read_to_string(root.join("control-input.jsonl")).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect();
        assert_eq!(audit.iter().filter(|v|v["method"]=="turn/start").count(),if mode=="bind-owner" {0}else{1});teardown(root);
    }
}

pub fn resolution() {
    for mode in ["foreign-resolved","resolved"] {
        let root=scratch("control-resolution");let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",&root,"first",&[]);l.controls=true;
        l.env.extend([("AGENTQ_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CONTROL_EXTRA".into(),mode.into())]);
        let mut run=runtime.launch(&l).unwrap();let first=loop {if let Some(Event::Request{id,..})=runtime.progress(&mut run).unwrap().into_iter().find(|e|matches!(e,Event::Request{..})) {break id;}std::thread::yield_now();};
        std::fs::write(root.join("inject"),"").unwrap();wait_for(&root.join("emitted"));let until=Instant::now()+Duration::from_secs(30);let mut later=vec![];
        loop {
            later.extend(runtime.progress(&mut run).unwrap().into_iter().filter_map(|e|if let Event::Request{id,..}=e {Some(id)}else{None}));
            let rows:Vec<Value>=std::fs::read_to_string(&l.log).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect();
            if mode=="resolved" && rows.iter().any(|r|r["type"]=="agentq_control" && r["generation"]==3 && r["state"]=="pending") || mode=="foreign-resolved" && rows.iter().filter(|r|r["method"]=="serverRequest/resolved").count()==2 {break;}
            assert!(Instant::now()<until,"resolution not observed");std::thread::yield_now();
        }
        if mode=="resolved" {
            assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("accept".into())}).is_err());assert_eq!(later.len(),2);
            assert!(later.iter().all(|id|id!=&first));assert_ne!(later[0],later[1]);
            for id in later {runtime.respond(&mut run,Control::Reply{id,option:Some("decline".into())}).unwrap();}
        }else{runtime.respond(&mut run,Control::Reply{id:first,option:Some("decline".into())}).unwrap();}
        finish(runtime,&mut run);drop(run);
        let audit:Vec<Value>=std::fs::read_to_string(root.join("control-input.jsonl")).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).filter(|r:&Value|r.get("method").is_none()).collect();
        assert_eq!(audit.len(),if mode=="resolved" {2}else{1});assert!(audit.iter().all(|r|r["result"]["decision"]=="decline"),"resolved old handle sent no accept; unrelated/fresh occurrence still answered");teardown(root);
    }
}

pub fn pump_authority() {
    for mode in ["owned","foreign","cancel-pump"] {
        let root=scratch("control-pump-owner");let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",&root,"first",&[]);l.controls=true;
        l.env.extend([("AGENTQ_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CONTROL_EXTRA".into(),mode.into())]);
        let mut run=runtime.launch(&l).unwrap();
        loop {if runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,Event::Request{..})) {break;}std::thread::yield_now();}
        if mode=="cancel-pump" {runtime.cancel(&mut run).unwrap();wait_for(&root.join("interrupt-seen"));}
        let path=std::fs::read_dir(&l.session_store.path).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();
        let original=std::fs::read(&path).unwrap();let mut changed:Value=serde_json::from_slice(&original).unwrap();changed["token"]=json!("foreign-token");
        let before=std::fs::read(root.join("control-input.jsonl")).unwrap();std::fs::write(&path,changed.to_string()).unwrap();std::fs::write(root.join("inject"),"").unwrap();
        wait_for(&root.join("emitted"));let until=Instant::now()+Duration::from_secs(30);
        let error=loop {if let Err(error)=runtime.progress(&mut run) {break error;}assert!(Instant::now()<until,"callback not observed");std::thread::yield_now();};
        assert!(error=="control-owner-changed" || error=="control-native-request-scope-differs","{error}");
        std::fs::write(path,original).unwrap();drop(run);
        assert_eq!(std::fs::read(root.join("control-input.jsonl")).unwrap(),before,"pump refusal sends no input with a changed owner, including foreign callbacks");teardown(root);
    }
}

pub fn policy() {
    let root=scratch("live-codex-deny");let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",&root,"denied",&[]);
    l.controls=true;l.policy.approval=agent::Approval::Deny;l.env.push(("AGENTQ_CONTROL_FIXTURE".into(),"1".into()));
    let mut run=runtime.launch(&l).unwrap();let request=loop {
        if let Some(event)=runtime.progress(&mut run).unwrap().into_iter().find(|e|matches!(e,Event::Request{..})) {break event;}
        assert_eq!(runtime.poll(&mut run).unwrap(),agent::State::Running);std::thread::yield_now();
    };
    let Event::Request{id,choices,..}=request else {unreachable!()};assert!(choices.iter().all(|c|c.kind=="deny"));
    assert!(runtime.respond(&mut run,Control::Reply{id:id.clone(),option:Some("accept".into())}).is_err());
    runtime.respond(&mut run,Control::Reply{id,option:Some("decline".into())}).unwrap();finish(runtime,&mut run);
    let rows:Vec<Value>=std::fs::read_to_string(&l.log).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).filter(|r:&Value|r["type"]=="agentq_control").collect();
    assert_eq!(rows.iter().filter(|r|r["state"]=="reply-pending").map(|r|r["native"]["result"]["decision"].clone()).collect::<Vec<_>>(),[json!("decline")]);
    let native=agent::NativeProcess::read(run.id().unwrap()).unwrap().unwrap();drop(run);
    #[cfg(windows)] crate::agent::test_receipts::finish(&[root.canonicalize().unwrap()],false).unwrap();
    assert!(matches!(agent::NativeProcess::read(native.pid),Ok(None)),"the owned native writer is reaped before admission");
    let until=Instant::now()+Duration::from_secs(30);
    loop {let entries=runtime.orphans(&l).unwrap();if entries.iter().all(|e|e.scope=="absent") {break;}assert!(Instant::now()<until,"owned scope not absent: {entries:?}");std::thread::yield_now();}
    runtime.adopt("control-thread",&l,&[]).unwrap();teardown(root);
}

pub fn codex() {
    let root=scratch("live-codex-controls");let runtime=agent::runtime("codex").unwrap();
    let mut l=fake_launch("codex",&root,"first",&[]);l.controls=true;l.home=Some(root.join("native home"));std::fs::create_dir_all(l.home.as_ref().unwrap()).unwrap();
    l.env.push(("AGENTQ_CONTROL_FIXTURE".into(),"1".into()));
    let mut run=runtime.launch(&l).unwrap();let session=runtime.session_id(&run).unwrap();
    let request=|run:&mut agent::Run|loop {
        if let Some(event)=runtime.progress(run).unwrap().into_iter().find(|e|matches!(e,Event::Request{..})) {break event;}
        assert_eq!(runtime.poll(run).unwrap(),agent::State::Running);std::thread::yield_now();
    };
    let Event::Request{id:first,choices,..}=request(&mut run) else {unreachable!()};
    assert_ne!(first,json!("native-reused"));assert_eq!(choices.iter().map(|c|c.id.as_str()).collect::<Vec<_>>(),["accept","decline","cancel"]);
    let saved=runtime.checkpoint(&run).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("invented".into())}).is_err());
    runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("accept".into())}).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:first.clone(),option:Some("accept".into())}).is_err());
    let Event::Request{id:question,questions,..}=request(&mut run) else {unreachable!()};
    assert_ne!(first,question);assert_eq!(questions.len(),2);
    let answers=std::collections::BTreeMap::from([("choice".into(),vec![Answer::Choice("Exact label".into())]),("text".into(),vec![Answer::Text("typed answer".into())])]);
    runtime.respond(&mut run,Control::Answers{id:question.clone(),answers}).unwrap();
    runtime.steer(&mut run,&[Part::Text("steer".into())]).unwrap();
    while runtime.poll(&mut run).unwrap()==agent::State::Running {std::thread::yield_now();}
    assert_eq!(runtime.report(&run).unwrap().outcome,"ok");
    assert!(runtime.resume(&session,&l).err().unwrap().detail.starts_with("session-busy"),"a live controlled writer retains its owner between turns");
    runtime.compact(&mut run).unwrap();
    runtime.input(&mut run,&[Part::Text("second".into())]).unwrap();
    let Event::Request{id:next,..}=request(&mut run) else {unreachable!()};
    assert_ne!(next,first);assert_ne!(next,question);
    assert!(runtime.respond(&mut run,Control::Reply{id:first,option:Some("accept".into())}).is_err());
    let owner=std::fs::read_dir(&l.session_store.path).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();
    let durable=std::fs::read(&owner).unwrap();let mut changed:Value=serde_json::from_slice(&durable).unwrap();changed["token"]=json!("foreign-token");std::fs::write(&owner,changed.to_string()).unwrap();
    let before=std::fs::read(root.join("control-input.jsonl")).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:next.clone(),option:Some("accept".into())}).is_err());
    assert!(runtime.steer(&mut run,&[Part::Text("foreign steer".into())]).is_err());
    assert!(runtime.compact(&mut run).is_err());assert!(runtime.input(&mut run,&[Part::Text("foreign input".into())]).is_err());
    assert_eq!(std::fs::read(root.join("control-input.jsonl")).unwrap(),before,"changed durable owner sends no native input");
    std::fs::write(&owner,durable).unwrap();
    runtime.cancel(&mut run).unwrap();
    assert!(runtime.respond(&mut run,Control::Reply{id:next.clone(),option:Some("accept".into())}).is_err());
    while runtime.poll(&mut run).unwrap()==agent::State::Running {std::thread::yield_now();}
    assert_eq!(runtime.report(&run).unwrap().outcome,"cancelled");
    runtime.input(&mut run,&[Part::Text("after confirmed interruption".into())]).unwrap();
    let Event::Request{id:after_cancel,..}=request(&mut run) else {unreachable!()};assert_ne!(after_cancel,next);
    runtime.cancel(&mut run).unwrap();finish(runtime,&mut run);
    let rows:Vec<Value>=std::fs::read_to_string(&l.log).unwrap().lines().map(|s|serde_json::from_str::<Value>(s).unwrap()).filter(|r|r["type"]=="agentq_control").collect();
    assert!(rows.iter().all(|r|r["run_id"]==saved["stop_id"] && r["session_id"]==session));
    assert_eq!(rows.iter().filter(|r|r["state"]=="pending").map(|r|r["generation"].as_u64().unwrap()).collect::<Vec<_>>(),[1,2,3,4]);
    assert!(rows.iter().any(|r|r["state"]=="cancelled" && r["generation"]==3));
    let transcript=runtime.transcript(&run).unwrap();assert_eq!(transcript,l.log);
    let mut history=runtime.history(&session,None,Some(&transcript),None).unwrap();
    let steps=history.steps(std::fs::metadata(&l.log).unwrap().len()).unwrap().steps;
    assert!(steps.iter().any(|s|s.kind==agent::history::StepKind::Question && s.state=="reply-sent"));
    drop(run);teardown(root);
}

// Observe completion without terminal poll, then lose the controller without dropping the native channel.
#[cfg(unix)]
pub fn owner(root:&Path) {
    let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",root,"first",&[]);l.controls=true;
    l.env.extend([("AGENTQ_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CONTROL_TERMINAL".into(),root.as_os_str().to_owned())]);
    let mut run=runtime.launch(&l).unwrap();
    loop {if runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,Event::TurnEnd{..})) {break;}std::thread::yield_now();}
    let saved=runtime.checkpoint(&run).unwrap();let native=agent::NativeProcess::read(run.id().unwrap()).unwrap().unwrap();
    std::fs::write(root.join("owner-ready.json"),json!({"saved":saved,"native":native}).to_string()).unwrap();
    std::process::exit(0);
}

#[cfg(unix)]
pub fn controller_loss() {
    let root=scratch("controlled-owner-loss");
    struct Release(PathBuf);impl Drop for Release {fn drop(&mut self) {let _=std::fs::write(self.0.join("server-release"),"");}}
    let release=Release(root.clone());
    let mut child=Command::new(exe()).arg("--control-owner").arg(&root).spawn().unwrap();
    wait_for(&root.join("owner-ready.json"));assert!(child.wait().unwrap().success());
    let value:Value=serde_json::from_slice(&std::fs::read(root.join("owner-ready.json")).unwrap()).unwrap();
    let native:agent::NativeProcess=serde_json::from_value(value["native"].clone()).unwrap();
    let runtime=agent::runtime("codex").unwrap();let mut l=fake_launch("codex",&root,"second",&[]);l.controls=true;
    l.env.extend([("AGENTQ_CONTROL_FIXTURE".into(),"1".into()),("AGENTQ_CONTROL_TERMINAL".into(),root.as_os_str().to_owned())]);
    let live=agent::NativeProcess::read(native.pid).unwrap()==Some(native.clone());
    let direct=runtime.adopt("control-thread",&l,&[]);
    let recovered=if direct.is_err() {
        let mut run=runtime.recover(&value["saved"],&l).unwrap();assert_eq!(runtime.poll(&mut run).unwrap(),agent::State::Finished);
        assert!(runtime.respond(&mut run,Control::Reply{id:json!("native-reused"),option:None}).is_err());drop(run);
        runtime.adopt("control-thread",&l,&[])
    }else{Ok(agent::OwnerObservation::default())};
    let same=agent::NativeProcess::read(native.pid).unwrap()==Some(native.clone());
    drop(release);let until=Instant::now()+Duration::from_secs(30);
    while !matches!(agent::NativeProcess::read(native.pid),Ok(None)) {assert!(Instant::now()<until,"owned server did not exit");std::thread::yield_now();}
    // A reaped leader alone is not proof that its guardian/group has finished.
    loop {
        let entries=runtime.orphans(&l).unwrap();if entries.iter().all(|e|e.scope=="absent") {break;}
        assert!(Instant::now()<until,"owned scope completion not observed: {entries:?}");std::thread::yield_now();
    }
    let completed=runtime.adopt("control-thread",&l,&[]);assert!(completed.is_ok(),"proven native writer/scope exit permits admission: {completed:?}");
    println!("CONTROLLED_LOSS live={live} direct={direct:?} recovered={recovered:?} same={same} completed={completed:?}");
    teardown(root);assert!(live && same);assert!(direct.is_err(),"native completion before poll cannot release live writer");assert!(recovered.is_err(),"recovered terminal report cannot release live writer");
}

pub fn server() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.first().is_some_and(|s|s=="--version") {println!("codex-cli 0.162.0");return;}
    if args.first().is_some_and(|s|s=="login") {println!("Logged in using ChatGPT");return;}
    assert_eq!(args.first().map(String::as_str),Some("app-server"));
    let emit=|value:Value| {println!("{value}");std::io::stdout().flush().unwrap();};
    let mut turn=0;let mut phase="";
    for line in std::io::stdin().lock().lines() {
        let value:Value=serde_json::from_str(&line.unwrap()).unwrap();let id=&value["id"];
        let mut audit=std::fs::OpenOptions::new().create(true).append(true).open(std::env::current_dir().unwrap().join("control-input.jsonl")).unwrap();writeln!(audit,"{value}").unwrap();audit.sync_data().unwrap();
        let scope=||json!({"threadId":"control-thread","turnId":format!("control-turn-{turn}"),"itemId":"tool"});
        match value["method"].as_str() {
            Some("initialize")=>emit(json!({"id":id,"result":{}})),Some("initialized")=>{},
            Some("thread/start")=>{
                if std::env::var("AGENTQ_CONTROL_EXTRA").as_deref()==Ok("bind-owner") {
                    let root=std::env::current_dir().unwrap();let path=std::fs::read_dir(root.join(".agentq-sessions")).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();
                    let mut row:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();row["token"]=json!("foreign-before-bind");std::fs::write(path,row.to_string()).unwrap();std::fs::write(root.join("changed-owner"),row.to_string()).unwrap();
                }
                emit(json!({"id":id,"result":{"thread":{"id":"control-thread"},"model":"fixture-model"}}));
            },
            Some("turn/start")=>{
                turn+=1;phase="approval";emit(json!({"id":id,"result":{"turn":{"id":format!("control-turn-{turn}")}}}));
                emit(json!({"method":"turn/started","params":{"threadId":"control-thread","turn":{"id":format!("control-turn-{turn}")}}}));
                if std::env::var_os("AGENTQ_CONTROL_TERMINAL").is_some() {emit(json!({"method":"turn/completed","params":{"threadId":"control-thread","turn":{"id":format!("control-turn-{turn}"),"status":"completed"}}}));continue;}
                let mut params=json!({"threadId":"control-thread","turnId":format!("control-turn-{turn}"),"itemId":"tool"});params["command"]=json!("owned fixture");params["availableDecisions"]=json!(["accept","decline","cancel"]);
                emit(json!({"id":"native-reused","method":"item/commandExecution/requestApproval","params":params}));
                if let Some(mode)=std::env::var_os("AGENTQ_CONTROL_EXTRA") {
                    let root=std::env::current_dir().unwrap();let mode=mode.to_string_lossy().into_owned();
                    std::thread::spawn(move|| {wait_for(&root.join("inject"));
                        if mode=="terminal-owner" {
                            println!("{}",json!({"method":"turn/completed","params":{"threadId":"control-thread","turn":{"id":"control-turn-1","status":"completed"}}}));
                        }else if mode=="resolved" || mode=="foreign-resolved" {
                            println!("{}",json!({"method":"serverRequest/resolved","params":{"threadId":"foreign","requestId":"native-reused"}}));
                            println!("{}",json!({"method":"serverRequest/resolved","params":{"threadId":"control-thread","requestId":[]}}));
                            if mode=="resolved" {
                                let params=json!({"threadId":"control-thread","turnId":"control-turn-1","itemId":"tool","availableDecisions":["accept","decline","cancel"]});
                                println!("{}",json!({"id":"unrelated","method":"item/commandExecution/requestApproval","params":params}));
                                println!("{}",json!({"method":"serverRequest/resolved","params":{"threadId":"control-thread","requestId":"native-reused"}}));
                                println!("{}",json!({"id":"native-reused","method":"item/commandExecution/requestApproval","params":params}));
                            }
                        }else{println!("{}",json!({"id":"extra","method":"future/nativeRequest","params":{"threadId":if mode=="foreign" {"foreign"}else{"control-thread"},"turnId":if mode=="foreign" {"foreign"}else{"control-turn-1"}}}));}
                        std::io::stdout().flush().unwrap();std::fs::write(root.join("emitted"),"").unwrap();
                    });
                }
            },
            None if id=="unrelated"=>assert_eq!(value["result"]["decision"],"decline"),
            None if id=="native-reused"=>match phase {
                "approval"=>{
                    if value["result"]["decision"]=="decline" {emit(json!({"method":"turn/completed","params":{"threadId":"control-thread","turn":{"id":format!("control-turn-{turn}"),"status":"completed"}}}));continue;}
                    assert_eq!(value["result"]["decision"],"accept");phase="question";
                    let mut params=scope();params["questions"]=json!([{"id":"choice","question":"Choose","options":[{"label":"Exact label","description":"fixture"}]},{"id":"text","question":"Enter","options":null,"isOther":true}]);
                    emit(json!({"id":"native-reused","method":"item/tool/requestUserInput","params":params}));
                },
                "question"=>{
                    assert_eq!(value["result"]["answers"]["choice"]["answers"],json!(["Exact label"]));assert_eq!(value["result"]["answers"]["text"]["answers"],json!(["typed answer"]));phase="steer";
                },
                _=>panic!("duplicate or late native reply"),
            },
            Some("turn/steer")=>{
                assert_eq!(phase,"steer");assert_eq!(value["params"]["expectedTurnId"],format!("control-turn-{turn}"));assert_eq!(value["params"]["threadId"],"control-thread");assert_eq!(value["params"]["input"][0]["text"],"steer");
                emit(json!({"id":id,"result":{"turnId":format!("control-turn-{turn}")}}));emit(json!({"method":"turn/completed","params":{"threadId":"control-thread","turn":{"id":format!("control-turn-{turn}"),"status":"completed"}}}));
            },
            Some("thread/compact/start")=>{assert_eq!(value["params"]["threadId"],"control-thread");emit(json!({"id":id,"result":{}}));},
            Some("turn/interrupt")=>{emit(json!({"id":id,"result":{}}));
                if std::env::var("AGENTQ_CONTROL_EXTRA").as_deref()==Ok("cancel-pump") {std::fs::write(std::env::current_dir().unwrap().join("interrupt-seen"),"").unwrap();}
                else{emit(json!({"method":"turn/completed","params":{"threadId":"control-thread","turn":{"id":format!("control-turn-{turn}"),"status":"interrupted"}}}));}
            },
            _=>panic!("unexpected owned control fixture message: {value}"),
        }
    }
    if let Some(root)=std::env::var_os("AGENTQ_CONTROL_TERMINAL") {wait_for(&PathBuf::from(root).join("server-release"));}
}
