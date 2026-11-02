//! M12.8 / #186 A3-A7: portable adapter doubles test the input gate, never Mac containment.
use super::*;
use agent::{ProtectedHost, ProtectedIdentity};
use agentq_runtime::optimization::Intent;

fn digest(path:&Path)->String {
    ring::digest::digest(&ring::digest::SHA256,&std::fs::read(path).unwrap()).as_ref().iter().map(|b|format!("{b:02x}")).collect()
}
fn read(path:&Path)->Value {serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()}
fn save(path:&Path,v:&Value) {std::fs::write(path,v.to_string()).unwrap();}
fn until()->Instant {Instant::now()+Duration::from_secs(10)}

pub fn fake_host() {
    let args:Vec<_>=std::env::args().collect();
    assert_eq!(&args[1..4],["service","app-server","--action"]);
    assert_eq!(args[5],"--request"); assert_eq!(args.len(),7,"no native flags or caller environment");
    let r:Value=serde_json::from_str(&args[6]).unwrap();let root=PathBuf::from(r["slot"]["binding"]["root"].as_str().unwrap());
    let original=root.join("request.json"); let receipt=root.join("receipt.json");
    match args[4].as_str() {
        "launch"=>{
            assert!(!original.exists(),"uncertain launch must never replay"); save(&original,&r);
            let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
            let e=read(&root.join("expected.json"));
            let mut command=Command::new(&exe);
            command.args(["app-server","--disable","apps"]).env("COREQ_FAKE_AGENT","codex").env("COREQ_FAKE_APP_SERVER","1")
                .env_remove("COREQ_FAKE_LINGER").env_remove("COREQ_FAKE_HANG").current_dir(&root);
            let mut child=command.spawn().unwrap();
            let supervisor=coreq::process::identity(std::process::id()).unwrap().unwrap();let native=coreq::process::identity(child.id()).unwrap().unwrap();
            save(&root.join("child.json"),&json!({"pid":native.pid,"started":native.started}));
            let effective=json!({"state":"verified-effective","scope":"invented process double, no native enforcement",
                "workspace_write":true,"protected_write_create_unlink_denied":["invented-control"],"reads":"existing profile","network":"existing profile"});
            let authority=json!({"project":root,"runtime":"codex","profile_sha256":e["profile_sha256"],"requested":"workspace-write",
                "saved":{"revision":r["policy_revision"],"grant":{"runtime":"codex","mode":"workspace","project":root,"by":e["approver"],"at":1}},
                "resolved":{"runtime":"codex","requested":"workspace-write","grant":"workspace","effective":"workspace",
                    "enforcement":"invented process double, no native enforcement","refused":null},"executable":exe,"executable_sha256":digest(&exe),
                "adapter":{"executable":exe,"sha256":digest(&exe)},"account":r["account"],"account_home":e["account_home"]});
            save(&receipt,&json!({"schema":"taskq.app-server/1","operation":r["slot"]["binding"]["op_id"],"caller":r["slot"]["binding"]["caller"],
                "state":"verified-effective","verified_effective":effective,"launch":{"schema":"taskq.app-server/1","policy_revision":r["policy_revision"],
                "account":r["account"],"authority":authority,"supervisor":{"pid":supervisor.pid,"started":supervisor.started},
                "native":{"pid":native.pid,"started":native.started},"host_instance":"invented-host-1","state":"verified-effective",
                "verified_effective":effective,"cancel_requested":false,"reason":null}}));
            save(&root.join("intent.json"),&json!(std::env::var("AGENTQ_OPTIMIZATION").unwrap()));
            let _=child.wait();
        },
        "observe"=>{
            assert_eq!(read(&original),r,"host independently rejects changed operation/key");
            if root.join("lost-reply").exists() {print!("incomplete");return;}
            println!("{}",read(&receipt));
        },
        "cancel"=>{
            assert_eq!(read(&original),r);
            let id=read(&root.join("child.json"));
            coreq::process::terminate(&coreq::process::Identity{pid:id["pid"].as_u64().unwrap() as u32,started:id["started"].as_str().unwrap().into()}).unwrap();
            let mut seen=read(&receipt);seen["state"]=json!("unknown");seen["launch"]["cancel_requested"]=json!(true);save(&receipt,&seen);
            println!("{seen}");
        },_=>panic!("unexpected adapter action"),
    }
}

// Requirement: no input before fresh exact authority, no widening/replay from untrusted task/env/receipt text.
// Risk: a forged/stale/unknown receipt starts work, kills a foreign child or reuses an uncertain operation.
// #193: accept producer-shaped resolved.effective; absent/null/Full/mode-only evidence never opens stdin.
pub fn protected_launch_refuses_tampering_and_preserves_native_transport() {
    let root=scratch("protected").canonicalize().unwrap();let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
    let home=root.join("account");std::fs::create_dir(&home).unwrap();
    let host=ProtectedHost::taskq(&exe,&digest(&exe)).unwrap();
    assert!(ProtectedHost::taskq(&exe,&"0".repeat(64)).is_err(),"untrusted adapter refused before execution");
    let owner=coreq::process::identity(std::process::id()).unwrap().unwrap();
    let r=json!({"schema":"taskq.app-server/1","slot":{"binding":{"schema":"taskq.slots/1","caller":"invented-consumer","op_id":"job-1",
        "root":root,"runtime":"codex","pid":owner.pid,"started":owner.started,"boot":1,"expires_at":9999999999u64},"key":"a".repeat(64)},"policy_revision":29,"account":"default"});
    let e=ProtectedIdentity{profile_sha256:"b".repeat(64),executable:exe.clone(),executable_sha256:digest(&exe),account_home:home.clone(),approver:"invented-operator".into()};
    save(&root.join("expected.json"),&json!({"profile_sha256":e.profile_sha256,"account_home":home,"approver":e.approver}));
    for pointer in ["/grant","/env","/executable","/slot/grant","/slot/binding/profile"] {
        let mut changed=r.clone();let (parent,key)=pointer.rsplit_once('/').unwrap();
        changed.pointer_mut(parent).unwrap()[key]=json!("forged-full");
        assert!(host.launch(changed,e.clone(),Intent::Off,&root.join("private.log")).is_err(),"{pointer}");
    }
    let mut stale=r.clone();stale["slot"]["binding"]["started"]=json!("stale-incarnation");
    assert!(host.launch(stale,e.clone(),Intent::Off,&root.join("private.log")).is_err());
    assert!(!root.join("request.json").exists(),"refusal has no native effects");
    let mut server=host.launch(r.clone(),e,Intent::Off,&root.join("private.log")).unwrap();
    let deadline=until();
    while !root.join("intent.json").exists() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert_eq!(read(&root.join("intent.json")),"off");assert_eq!(server.optimization().effective,"off");
    let path=root.join("receipt.json");let good=read(&path);
    let message=json!({"id":1,"method":"initialize","params":{}});
    // Each path is a distinct trust/identity/refusal failure; none may reach native stdin.
    for (pointer,value) in [
        ("/state",json!("unknown")),("/operation",json!("foreign-job")),("/caller",json!("foreign-consumer")),
        ("/launch/state",json!("unknown")),("/launch/cancel_requested",json!(true)),("/launch/host_instance",Value::Null),
        ("/launch/policy_revision",json!(30)),("/launch/authority/saved/revision",json!(30)),
        ("/launch/authority/saved/grant/by",json!("forged-operator")),("/launch/authority/saved/grant/mode",json!("full")),
        ("/launch/authority/resolved/effective",json!("full")),("/launch/authority/resolved/effective",Value::Null),
        ("/launch/authority/resolved",json!({})),("/launch/authority/resolved",Value::Null),
        ("/launch/authority/resolved",json!({"mode":"workspace"})),
        ("/launch/authority/project",json!(home)),("/launch/authority/profile_sha256",json!("c".repeat(64))),
        ("/launch/authority/account",json!("foreign")),("/launch/authority/account_home",json!(root)),
        ("/launch/authority/executable_sha256",json!("c".repeat(64))),("/launch/authority/adapter/sha256",json!("c".repeat(64))),
        ("/launch/native/started",json!("stale-child")),("/launch/supervisor/started",json!("stale-supervisor")),
        ("/verified_effective/state",json!("unknown")),("/launch/verified_effective",Value::Null),
    ] {
        let mut changed=good.clone();*changed.pointer_mut(pointer).unwrap()=value;save(&path,&changed);
        assert!(server.send(&message,until()).is_err(),"{pointer}");
        assert!(!root.join("app-input.jsonl").exists(),"{pointer}: no native input");
    }
    save(&path,&good);
    std::fs::write(root.join("lost-reply"),b"invented interrupted reply").unwrap();
    assert!(server.send(&message,until()).is_err());assert!(!root.join("app-input.jsonl").exists());
    std::fs::remove_file(root.join("lost-reply")).unwrap();
    server.verify(until()).unwrap();
    let mut unknown=good.clone();unknown["state"]=json!("unknown");save(&path,&unknown);
    assert!(server.send(&message,until()).is_err());assert!(!root.join("app-input.jsonl").exists());
    save(&path,&good);server.verify(until()).expect("same binding must recover from an unknown readback without resubmitting");
    std::fs::write(root.join("lost-reply"),b"invented interrupted reply").unwrap();
    assert!(server.send(&message,until()).is_err());assert!(!root.join("app-input.jsonl").exists());
    std::fs::remove_file(root.join("lost-reply")).unwrap();server.verify(until()).unwrap();
    let mut changed=r.clone();changed["slot"]["key"]=json!("c".repeat(64));assert!(host.observe(&changed,until()).is_err());
    let spec=Spec{client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],home:Some(home),env:vec![]};
    let mut app=agent::AppSession::new(&spec).unwrap();let req=app.initialize(json!({})).unwrap();server.send(&req,until()).unwrap();
    app.observe(&server.receive(until()).unwrap().unwrap()).unwrap();server.send(&app.initialized().unwrap(),until()).unwrap();
    let open=app.open(json!({"approvalPolicy":"never","sandbox":"read-only","dynamicTools":[{"name":"caller-tool"}],"developerInstructions":"private rules"})).unwrap();
    server.send(&open,until()).unwrap();app.observe(&server.receive(until()).unwrap().unwrap()).unwrap();
    let turn=app.start_turn(json!({"input":[],"model":"fixture-model"})).unwrap();server.send(&turn,until()).unwrap();
    while app.report().is_none() {
        let event=server.receive(until()).unwrap().unwrap();app.observe(&event).unwrap();
        if event["method"]=="item/tool/call" {server.send(&json!({"id":event["id"],"result":{"success":true}}),until()).unwrap();}
    }
    assert_eq!(app.report().unwrap().outcome,"ok");
    let store=coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let context=Context{product:"fixtureq",version:"1",project:"other",task:None,route:Some("job-1"),account:"default"};
    app.record(&context,&store).unwrap().unwrap().recorded.unwrap();
    let checkpoint=app.checkpoint();agent::AppSession::recover(&spec,&checkpoint).unwrap().record(&context,&store).unwrap().unwrap().recorded.unwrap();
    assert_eq!(observations(&store).len(),1,"one usage observation across recovery");drop(store);
    let mut duplicate=host.launch(r.clone(),ProtectedIdentity{profile_sha256:"b".repeat(64),executable:exe.clone(),executable_sha256:digest(&exe),
        account_home:root.join("account"),approver:"invented-operator".into()},Intent::Off,&root.join("duplicate.log")).unwrap();
    let deadline=until();while duplicate.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert!(duplicate.send(&message,until()).is_err(),"consumed operation cannot launch again even with a copied receipt");drop(duplicate);
    // A valid-looking new generation must fence the already-bound transport permanently.
    let mut changed=good.clone();changed["launch"]["host_instance"]=json!("invented-host-2");save(&path,&changed);
    assert!(server.send(&message,until()).is_err());save(&path,&good);assert!(server.send(&message,until()).is_err());
    assert_eq!(host.observe(&r,until()).unwrap(),good,"recovery observes same operation without relaunch");
    server.cancel(until()).unwrap();let deadline=until();
    while server.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert!(server.receive(until()).is_err(),"interruption/EOF never proves a turn or releases a slot");
    drop(server);std::fs::remove_dir_all(root).unwrap();
}
