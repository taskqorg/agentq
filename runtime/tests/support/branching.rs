//! Owned no-model F300 proofs; this executable is the disposable native protocol double.
use super::*;
use agent::Part;
use std::io::{BufRead,Write};

fn setup(tag:&str)->(PathBuf,agent::Launch) {
    let root=scratch(tag);let mut launch=fake_launch("codex",&root,"first",&[]);launch.controls=true;
    launch.env.push(("AGENTQ_BRANCH_FIXTURE".into(),"1".into()));
    std::fs::write(root.join("file-sentinel"),"original working files").unwrap();(root,launch)
}
fn steps(launch:&agent::Launch)->Vec<agent::history::Step> {
    let mut reader=agent::history::History::new("codex-app-server",launch.log.to_str().unwrap(),"parent").unwrap();
    reader.steps(&launch.log,std::fs::metadata(&launch.log).unwrap().len()).unwrap().steps
}
fn selected(launch:&agent::Launch,turn:&str)->agent::history::Step {
    steps(launch).into_iter().find(|s|s.turn.native_id.as_deref()==Some(turn) && s.source["raw"].as_str().unwrap().contains("item/completed")).unwrap()
}
fn audit(root:&Path)->Vec<Value> {std::fs::read_to_string(root.join("branch-audit")).unwrap().split_inclusive('\n').filter(|s|s.ends_with('\n')).map(|s|serde_json::from_str(s).unwrap()).collect()}
fn threads(root:&Path)->Value {serde_json::from_slice(&std::fs::read(root.join("branch-threads")).unwrap()).unwrap()}
fn owners(launch:&agent::Launch)->Vec<Value> {std::fs::read_dir(&launch.session_store.path).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e=="json")).map(|p|serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()).collect()}

pub fn prefix() {
    let (root,launch)=setup("branch-prefix");let runtime=agent::runtime("codex").unwrap();let mut source=runtime.launch(&launch).unwrap();finish(runtime,&mut source);
    let original_log=std::fs::read(&launch.log).unwrap();
    let context=Context {product:"fixtureq",version:"1",project:"other",task:None,route:None,account:"default"};let snapshot=source.snapshot(&context).unwrap();
    let frozen=runtime.report(&source).unwrap();let original=threads(&root)["parent"].clone();let step=selected(&launch,"A");
    let mut child_launch=launch.clone();child_launch.log=root.join("child.log");child_launch.input="exact child prompt".into();child_launch.model=Some("gpt-6.1-sol".into());
    let configured=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let spawned=configured.clone();
    child_launch.process.configure=Some(std::sync::Arc::new(move |c| {c.env("AGENTQ_BRANCH_HOOK","preserved");spawned.fetch_add(1,std::sync::atomic::Ordering::SeqCst);}));
    let attached=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let attached_hook=attached.clone();child_launch.process.spawned=Some(std::sync::Arc::new(move |_| {attached_hook.fetch_add(1,std::sync::atomic::Ordering::SeqCst);Ok(())}));
    let mut child=runtime.fork(&mut source,Some(&step),&child_launch).unwrap();finish(runtime,&mut child);
    assert_eq!(configured.load(std::sync::atomic::Ordering::SeqCst),1);assert_eq!(attached.load(std::sync::atomic::Ordering::SeqCst),1);
    assert_eq!(runtime.session_id(&child).as_deref(),Some("child-1"));assert_eq!(threads(&root)["parent"],original);assert!(std::fs::read(&launch.log).unwrap().starts_with(&original_log));
    assert_eq!(owners(&launch).iter().filter(|r|r["session"]=="child-1" && !r["lineage"].is_null()).count(),1);
    assert_ne!(runtime.checkpoint(&source).unwrap()["owner"]["token"],runtime.checkpoint(&child).unwrap()["owner"]["token"]);
    assert_ne!(runtime.checkpoint(&source).unwrap()["stop_id"],runtime.checkpoint(&child).unwrap()["stop_id"]);
    let rows=audit(&root);let forks:Vec<_>=rows.iter().filter(|r|r["method"]=="thread/fork").collect();assert_eq!(forks.len(),1);let parent_pid=rows.iter().find(|r|r["method"]=="turn/start" && r["params"]["threadId"]=="parent").unwrap()["fixturePid"].clone();assert_ne!(forks[0]["fixturePid"],parent_pid);
    let prompt:Vec<_>=rows.iter().filter(|r|r["method"]=="turn/start" && r["params"]["threadId"]=="child-1").collect();assert_eq!(prompt.len(),1);assert_eq!(prompt[0]["fixturePid"],forks[0]["fixturePid"]);assert_eq!(prompt[0]["params"]["input"][0]["text"],"exact child prompt");
    runtime.rewind(&mut source,Some(&step)).unwrap();assert_eq!(threads(&root)["parent"],json!([original[0].clone()]));
    let before=audit(&root).len();runtime.rewind(&mut source,Some(&step)).unwrap();assert!(audit(&root)[before..].iter().all(|r|r["method"]!="thread/revert"));
    runtime.rewind(&mut source,None).unwrap();assert_eq!(threads(&root)["parent"],json!([]));
    assert_eq!(format!("{:?}",runtime.report(&source).unwrap()),format!("{frozen:?}"));assert_eq!(std::fs::read_to_string(root.join("file-sentinel")).unwrap(),"original working files");
    assert_eq!(audit(&root).iter().filter(|r|r["method"]=="turn/start").count(),2,"rewind submits no prompt");assert_eq!(source.snapshot(&context).unwrap(),snapshot);
    runtime.input(&mut source,&[Part::Text("explicit after rewind".into())]).unwrap();finish(runtime,&mut source);assert_eq!(threads(&root)["parent"][0]["id"],"next-turn");
    let lineage=owners(&launch).iter().find(|r|r["session"]=="parent").unwrap()["lineage"].clone();
    drop(child);drop(source);crate::agent::test_receipts::finish(&[root.canonicalize().unwrap()],false).unwrap();runtime.adopt("parent",&launch,&[]).unwrap();assert_eq!(owners(&launch).iter().find(|r|r["session"]=="parent").unwrap()["lineage"],lineage,"confirmed lineage survives explicit adoption without archive");teardown(root);
}

pub fn refusals() {
    {
        let (root,launch)=setup("branch-pending");std::fs::write(root.join("branch-mode"),"pending").unwrap();let runtime=agent::runtime("codex").unwrap();let mut source=runtime.launch(&launch).unwrap();
        let mut child=launch.clone();child.log=root.join("child.log");assert!(runtime.fork(&mut source,None,&child).is_err());assert!(runtime.rewind(&mut source,None).is_err());
        assert!(!audit(&root).iter().any(|r|matches!(r["method"].as_str(),Some("thread/fork"|"thread/revert"))));drop(source);teardown(root);
    }
    for mode in ["forged","mid-item","continued-chain","unknown-chain","later-chain","goal","unknown-goal","partial-items","active","foreign-owner","recovered","empty-fork","different-store","unknown-native","incomplete-tail","foreign-step","changed-env","unsupported-condition","replaced","truncated","foreign-append","alias-log"] {
        let (root,launch)=setup(&format!("branch-refuse-{mode}"));let runtime=agent::runtime("codex").unwrap();if mode=="unknown-native" {std::fs::write(root.join("branch-mode"),mode).unwrap();}let mut source=runtime.launch(&launch).unwrap();finish(runtime,&mut source);
        let mut step=selected(&launch,"A");let mut child_launch=launch.clone();child_launch.log=root.join("child.log");
        match mode {
            "replaced"=>{std::fs::rename(&launch.log,root.join("previous.log")).unwrap();std::fs::copy(root.join("previous.log"),&launch.log).unwrap();},
            "truncated"=>{let file=std::fs::OpenOptions::new().write(true).open(&launch.log).unwrap();file.set_len(std::fs::metadata(&launch.log).unwrap().len()-1).unwrap();},
            "foreign-append"=>{let mut log=std::fs::OpenOptions::new().append(true).open(&launch.log).unwrap();writeln!(log,"{{}} ").unwrap();},
            "alias-log"=>{child_launch.log=root.join("alias.log");std::fs::hard_link(&launch.log,&child_launch.log).unwrap();},
            "forged"=>step.revision+=1,
            "foreign-step"=>step.source["source"]=json!("foreign"),
            "incomplete-tail"=>{let mut log=std::fs::OpenOptions::new().append(true).open(&launch.log).unwrap();write!(log,"{{").unwrap();},
            "unsupported-condition"=>child_launch.mcp=Some(root.join("not-opened")),
            "changed-env"=>child_launch.env.push(("OPENAI_API_KEY".into(),"no-secret-fixture".into())),
            "mid-item"=>{step.kind=agent::history::StepKind::Tool;step.state="completed".into();},
            "different-store"=>child_launch.session_store=agent::SessionStore::new(root.join("foreign-store")),
            "foreign-owner"=>{let path=std::fs::read_dir(&launch.session_store.path).unwrap().map(|e|e.unwrap().path()).find(|p|p.extension().is_some_and(|e|e=="json")).unwrap();let mut row:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();row["token"]=json!("foreign");std::fs::write(&path,row.to_string()).unwrap();},
            "recovered"=>{},
            _=>std::fs::write(root.join("branch-mode"),mode).unwrap(),
        }
        if mode=="recovered" {
            let saved=runtime.checkpoint(&source).unwrap();assert!(runtime.recover(&saved,&launch).is_err(),"live owner cannot be reacquired");
        }else{assert!(runtime.fork(&mut source,if mode=="empty-fork" {None}else{Some(&step)},&child_launch).is_err(),"{mode}");}
        assert!(!audit(&root).iter().any(|r|matches!(r["method"].as_str(),Some("thread/fork"|"thread/revert"))),"{mode} wrote mutation");
        assert_eq!(std::fs::read_to_string(root.join("file-sentinel")).unwrap(),"original working files");drop(source);teardown(root);
    }
}

pub fn uncertain() {
    for operation in ["fork","rewind"] {for mode in ["wrong-prefix","wrong-thread","lost-reply","wrong-model","wrong-policy"] {
        if operation=="rewind" && matches!(mode,"wrong-model"|"wrong-policy") {continue;}
        let (root,launch)=setup(&format!("branch-unknown-{operation}-{mode}"));let runtime=agent::runtime("codex").unwrap();let mut source=runtime.launch(&launch).unwrap();finish(runtime,&mut source);let step=selected(&launch,"A");
        std::fs::write(root.join("branch-mode"),mode).unwrap();let mut child_launch=launch.clone();child_launch.log=root.join("child.log");child_launch.model=Some("gpt-6.1-sol".into());
        if mode=="wrong-policy" {child_launch.policy.approval=agent::Approval::Deny;}
        if operation=="fork" {assert!(runtime.fork(&mut source,Some(&step),&child_launch).is_err());}else{assert!(runtime.rewind(&mut source,Some(&step)).is_err());}
        assert!(!owners(&launch).iter().find(|r|r["session"]=="parent").unwrap()["mutation"].is_null());let before=audit(&root).len();
        assert!(runtime.input(&mut source,&[Part::Text("never replay".into())]).is_err());
        let native=agent::NativeSpec {root:launch.root.clone(),session:"parent".into(),resume:true,args:vec![],home:launch.home.clone(),env:launch.env.clone(),owner:source.owner().cloned()};
        assert!(runtime.turn(Command::new(exe()),&native,&root.join("seam.log"),"never through raw seam",&agent::Process::default()).is_err());assert!(runtime.rewind(&mut source,None).is_err());assert_eq!(audit(&root).len(),before);
        let rows=owners(&launch);let parent_intent=rows.iter().find(|r|r["session"]=="parent").unwrap()["mutation"].clone();
        if operation=="fork" {assert!(rows.iter().any(|r|r["session"]=="-" && r["mutation"]==parent_intent),"unverified fork retains the admission alias");assert!(rows.iter().filter(|r|r["branch_admission"]==true).all(|r|r["mutation"]==parent_intent));}
        let saved=runtime.checkpoint(&source).unwrap();drop(source);
        crate::agent::test_receipts::finish(&[root.canonicalize().unwrap()],false).unwrap();
        if operation=="fork" {
            assert!(runtime.adopt("child-1",&child_launch,&[]).unwrap_err().contains("session-mutation-unknown"));
            assert!(runtime.resume("child-1",&child_launch).is_err());assert!(runtime.session_owner(Some("child-1"),&child_launch).is_err());
            assert!(!audit(&root).iter().any(|r|r["method"]=="turn/start" && r["params"]["threadId"]=="child-1"));
        }
        if let Ok(mut recovered)=runtime.recover(&saved,&launch) {assert!(runtime.rewind(&mut recovered,None).is_err());drop(recovered);}
        assert!(!owners(&launch).iter().find(|r|r["session"]=="parent").unwrap()["mutation"].is_null(),"recovery never clears intent");
        assert!(runtime.adopt("parent",&launch,&[]).is_err(),"adoption cannot clear durable unknown mutation");
        assert_eq!(audit(&root).iter().filter(|r|matches!(r["method"].as_str(),Some("thread/fork"|"thread/revert"))).count(),1);
        assert_eq!(std::fs::read_to_string(root.join("file-sentinel")).unwrap(),"original working files");teardown(root);
    }}
}


pub fn admission_boundaries() {
    for mode in ["configure-owner","spawned-owner","configure-replace","spawned-replace","configure-rewrite","configure-append"] {
        let (root,launch)=setup(mode);let runtime=agent::runtime("codex").unwrap();let mut source=runtime.launch(&launch).unwrap();finish(runtime,&mut source);let step=selected(&launch,"A");
        let mut child=launch.clone();child.log=root.join("child.log");let fault_root=root.clone();let fault=move|| {
            if mode.ends_with("owner") {
                let path=std::fs::read_dir(fault_root.join(".agentq-sessions")).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e=="json")).find(|p|serde_json::from_slice::<Value>(&std::fs::read(p).unwrap()).unwrap()["session"]=="parent").unwrap();
                let mut row:Value=serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();row["token"]=json!("foreign-after-selection");std::fs::write(path,row.to_string()).unwrap();
            }else if mode.ends_with("replace") {
                let path=fault_root.join("worker.log");std::fs::rename(&path,fault_root.join("old-parent.log")).unwrap();std::fs::copy(fault_root.join("old-parent.log"),path).unwrap();
            }else if mode.ends_with("rewrite") {
                let path=fault_root.join("worker.log");let mut bytes=std::fs::read(&path).unwrap();bytes[0]=b' ';std::fs::write(path,bytes).unwrap();
            }else {append(&fault_root.join("worker.log"),&[json!({"foreign":"append"})]);}
        };
        if mode.starts_with("configure") {child.process.configure=Some(std::sync::Arc::new(move |_|fault()));}
        else {child.process.spawned=Some(std::sync::Arc::new(move |_|{fault();Ok(())}));}
        assert!(runtime.fork(&mut source,Some(&step),&child).is_err(),"{mode}");
        assert!(!audit(&root).iter().any(|r|r["method"]=="thread/fork"),"{mode} crossed the original parent boundary");
        assert!(threads(&root).get("child-1").is_none());assert_eq!(std::fs::read_to_string(root.join("file-sentinel")).unwrap(),"original working files");
        assert!(owners(&launch).iter().all(|r|r["mutation"].is_null()),"pre-write hook refusal created an uncertain branch");
        drop(source);teardown(root);
    }
    for mode in ["lifetime-configure","lifetime-spawned","missing-command","log-directory","configure-error","spawned-error","invalid-model","invalid-parts"] {
        let (root,launch)=setup(mode);let runtime=agent::runtime("codex").unwrap();let mut source=runtime.launch(&launch).unwrap();finish(runtime,&mut source);let step=selected(&launch,"A");
        let original=threads(&root)["parent"].clone();let mut child=launch.clone();child.log=root.join("child.log");
        let configured=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let observed=configured.clone();
        match mode {
            "lifetime-configure"=>{child.lifetime=agent::Lifetime::StopWithOwner;child.process.configure=Some(std::sync::Arc::new(move |_|{observed.fetch_add(1,std::sync::atomic::Ordering::SeqCst);}));},
            "lifetime-spawned"=>{child.lifetime=agent::Lifetime::StopWithOwner;child.process.spawned=Some(std::sync::Arc::new(move |_|{observed.fetch_add(1,std::sync::atomic::Ordering::SeqCst);Ok(())}));},
            "missing-command"=>child.command=vec![root.join("absent-executable").into()],
            "log-directory"=>child.log=root.clone(),
            "configure-error"=>{let missing=root.join("absent-cwd");child.process.configure=Some(std::sync::Arc::new(move |c|{c.current_dir(&missing);}));},
            "spawned-error"=>child.process.spawned=Some(std::sync::Arc::new(|_|Err("owned setup refusal".into()))),
            "invalid-model"=>child.model=Some("not a model code".into()),
            "invalid-parts"=>child.parts=vec![Part::Image(root.join("absent-image"))],
            _=>unreachable!(),
        }
        assert!(runtime.fork(&mut source,Some(&step),&child).is_err(),"{mode}");
        if mode.starts_with("lifetime") {assert_eq!(configured.load(std::sync::atomic::Ordering::SeqCst),0);}
        assert_eq!(threads(&root)["parent"],original);assert!(!audit(&root).iter().any(|r|r["method"]=="thread/fork"));
        assert!(owners(&launch).iter().all(|r|r["mutation"].is_null()));
        if mode.starts_with("lifetime") {assert_eq!(owners(&launch).len(),1,"known unsupported setup leaves no child alias");}
        runtime.input(&mut source,&[Part::Text("separate explicit next input".into())]).unwrap();finish(runtime,&mut source);
        assert_eq!(threads(&root)["parent"].as_array().unwrap().last().unwrap()["id"],"next-turn");
        drop(source);teardown(root);
    }
}

fn turn(id:&str)->Value {json!({"id":id,"rootTurnId":id,"itemsView":"full","status":"completed","items":[{"type":"agentMessage","id":format!("item-{id}"),"text":format!("answer-{id}")}],"error":null})}
pub fn server() {
    let args:Vec<_>=std::env::args().skip(1).collect();
    if args.first().is_some_and(|s|s=="--version") {println!("codex-cli {}",if std::fs::read_to_string("branch-mode").is_ok_and(|s|s=="unknown-native") {"0.163.0"}else{"0.162.0"});return;}
    if args.first().is_some_and(|s|s=="login") {println!("Logged in using ChatGPT");return;}
    assert_eq!(args.first().map(String::as_str),Some("app-server"));let root=std::env::current_dir().unwrap();
    let emit=|v:Value| {println!("{v}");std::io::stdout().flush().unwrap();};
    let mut started=false;
    for line in std::io::stdin().lock().lines() {
        let request:Value=serde_json::from_str(&line.unwrap()).unwrap();let p=&request["params"];let id=&request["id"];let thread=p["threadId"].as_str().unwrap_or("parent");
        let mut log=std::fs::OpenOptions::new().create(true).append(true).open(root.join("branch-audit")).unwrap();let mut audit=request.clone();audit["fixturePid"]=json!(std::process::id());log.write_all(format!("{audit}\n").as_bytes()).unwrap();log.sync_data().unwrap();
        let mode=std::fs::read_to_string(root.join("branch-mode")).unwrap_or_default();let mut state=if root.join("branch-threads").exists(){threads(&root)}else{json!({"parent":[turn("A"),turn("B"),turn("C")]})};
        let metadata=|name:&str|json!({"thread":{"id":name,"turns":[],"status":{"type":if mode=="active" {"active"}else{"idle"}}},"model":p["model"].as_str().unwrap_or("gpt-6.1-sol"),"cwd":root,"approvalPolicy":"never","sandbox":{"type":"workspaceWrite"},"turnsBackwardsCursor":null,"itemsBackwardsCursor":null});
        let result=match request["method"].as_str() {
            Some("initialize")=>json!({}),Some("initialized")=>continue,
            Some("config/read")=>{emit(json!({"id":id,"error":{"code":-32601,"message":"Fixture does not expose native configuration"}}));continue;},
            Some("thread/start"|"thread/resume")=>metadata(thread),
            Some("turn/start")=>{
                if thread=="parent" && started {state[thread].as_array_mut().unwrap().push(turn("next-turn"));}started=true;
                if thread!="parent" {
                    assert_eq!(std::env::var("AGENTQ_BRANCH_HOOK").unwrap(),"preserved");
                    let row=std::fs::read_dir(root.join(".agentq-sessions")).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e=="json")).map(|p|serde_json::from_slice::<Value>(&std::fs::read(p).unwrap()).unwrap()).find(|r|r["session"]==thread).expect("admitted before prompt");assert!(!row["lineage"].is_null());
                    state[thread].as_array_mut().unwrap().push(turn("child-turn"));
                }
                std::fs::write(root.join("branch-threads"),state.to_string()).unwrap();
                let turns=state[thread].as_array().unwrap();let last=turns.last().unwrap()["id"].clone();emit(json!({"id":id,"result":{"turn":{"id":last}}}));
                if mode=="pending" {emit(json!({"id":"pending-approval","method":"item/commandExecution/requestApproval","params":{"threadId":thread,"turnId":last,"itemId":"pending-tool","availableDecisions":["decline"]}}));continue;}
                for turn in turns {emit(json!({"method":"item/completed","params":{"threadId":thread,"turnId":turn["id"],"item":turn["items"][0]}}));}
                emit(json!({"method":"turn/completed","params":{"threadId":thread,"turn":{"id":last,"status":"completed"}}}));continue;
            },
            Some("thread/goal/get")=>if mode=="unknown-goal" {json!({})}else{json!({"goal":if mode=="goal" {json!({"status":"active"})}else{Value::Null}})},
            Some("thread/read")=>metadata(thread),
            Some("thread/turns/list")=>{
                let mut data=state[thread].clone();if mode=="continued-chain" {data[1]["rootTurnId"]=json!("A");}if mode=="unknown-chain" {data[1]["rootTurnId"]=Value::Null;}if mode=="later-chain" {data[2]["rootTurnId"]=json!("A");}if mode=="partial-items" {data[0]["itemsView"]=json!("summary");}
                json!({"data":data,"nextCursor":null})
            },
            Some("thread/fork"|"thread/revert")=>{
                if request["method"]=="thread/fork" {
                    let admitted=std::fs::read_dir(root.join(".agentq-sessions")).unwrap().map(|e|e.unwrap().path()).filter(|p|p.extension().is_some_and(|e|e=="json")).map(|p|serde_json::from_slice::<Value>(&std::fs::read(p).unwrap()).unwrap()).any(|r|r["session"]=="-" && r["native"]["pid"]==std::process::id() && r["branch_admission"]==true && !r["lineage"].is_null() && r["mutation"]["operation"]=="fork");assert!(admitted,"fresh native admission and original mutation fence precede fork");
                }
                if mode=="lost-reply" {return;}
                let count=state[thread].as_array().unwrap().iter().position(|t|t["id"]==p[if request["method"]=="thread/fork" {"lastTurnId"}else{"beforeTurnId"}]).unwrap();
                let keep=count+usize::from(request["method"]=="thread/fork");let prefix=if mode=="wrong-prefix" {json!([])}else{json!(&state[thread].as_array().unwrap()[..keep])};
                let name=if request["method"]=="thread/fork" {"child-1"}else{thread};state[name]=prefix;
                let mut result=metadata(if mode=="wrong-thread" {"foreign"}else{name});
                if mode=="wrong-model" {result["model"]=json!("not-requested");}if mode=="wrong-policy" {result["approvalPolicy"]=json!("on-request");}
                std::fs::write(root.join("branch-threads"),state.to_string()).unwrap();result
            },
            _=>panic!("unexpected fixture method"),
        };
        std::fs::write(root.join("branch-threads"),state.to_string()).unwrap();emit(json!({"id":id,"result":result}));
    }
}
