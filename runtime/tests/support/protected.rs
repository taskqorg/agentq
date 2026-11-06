//! M12.8 / #186 A3-A7: portable adapter doubles test the input gate, never Mac containment.
use super::*;
use agent::{ProtectedHost, ProtectedIdentity};
use agentq_runtime::optimization::Intent;
#[path="../../../docs/evidence/runtime-199/owner-route.rs"]
mod owner_route;

fn digest(path:&Path)->String {
    ring::digest::digest(&ring::digest::SHA256,&std::fs::read(path).unwrap()).as_ref().iter().map(|b|format!("{b:02x}")).collect()
}
fn read(path:&Path)->Value {serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()}
fn save(path:&Path,v:&Value) {coreq::write_atomic(path,&v.to_string()).unwrap();}
// A hang bound, not a speed claim: no scenario waits for it to pass, so it can stay well above an overloaded host's delays.
fn until()->Instant {Instant::now()+Duration::from_secs(60)}

const READBACK_TIMEOUT:&str="readback unknown: timeout";
/// #274: a host readback keeps its 5 s production cap and spawns this 60 MB test binary, so an overloaded host can exceed it.
/// That timeout is unknown authority, never a refusal: it proves no gate, sets no fence, and must not pass an `is_err` check.
fn refused<T,E:std::fmt::Display>(result:Result<T,E>)->bool {
    match result {Ok(_)=>false,Err(e)=>{let e=e.to_string();assert!(!e.contains(READBACK_TIMEOUT),"{e}");true}}
}
/// #274: `retained()` maps every process failure to one text, so only the time spent can show that its 5 s readback cap expired.
/// Each failure class under that time stops at once. A slower one, counting the adapter hash before the cap starts, also reruns and still fails on the last attempt.
fn retained_answer(call:impl FnOnce(Instant)->Result<Value,String>)->Value {
    let start=Instant::now();
    call(until()).unwrap_or_else(|e|{
        assert!(!(e.contains("retained recovery unknown")&&start.elapsed()>=Duration::from_secs(5)),"{e}: {READBACK_TIMEOUT}");
        panic!("{e}")
    })
}
/// A timed-out attempt reached no verdict, and its consumed operation cannot resume, so only that attempt reruns from a new root.
/// Any other failure stops at once, and a host that always times out still fails on the last attempt.
fn rerun_after_readback_timeout(scenario:fn()) {
    for _ in 1..5 {
        let Err(panic)=std::panic::catch_unwind(scenario) else {return};
        let message=panic.downcast_ref::<String>().map(String::as_str).or(panic.downcast_ref::<&str>().copied()).unwrap_or("");
        if !message.contains(READBACK_TIMEOUT) {std::panic::resume_unwind(panic);}
        println!("rerun after unfinished attempt: {message}");
    }
    scenario();
}
/// A timed-out attempt's frontend may still hold its root, so every attempt starts in its own.
fn fresh(tag:&str)->PathBuf {
    static ATTEMPT:std::sync::atomic::AtomicU32=std::sync::atomic::AtomicU32::new(0);
    scratch(&format!("{tag}-{}",ATTEMPT.fetch_add(1,std::sync::atomic::Ordering::Relaxed))).canonicalize().unwrap()
}

fn authority(root:&Path,revision:&Value,account:&Value)->Value {
    let exe=std::env::current_exe().unwrap().canonicalize().unwrap();let e=read(&root.join("expected.json"));let hash=digest(&exe);
    json!({"project":root,"runtime":"codex","profile_sha256":e["profile_sha256"],"requested":"workspace-write",
        "saved":{"revision":revision,"grant":{"runtime":"codex","mode":"workspace","project":root,"by":e["approver"],"at":1}},
        "resolved":{"runtime":"codex","requested":"workspace-write","grant":"workspace","effective":"workspace",
            "enforcement":"invented process double, no native enforcement","refused":null},"executable":exe,"executable_sha256":hash,
        "adapter":{"executable":exe,"sha256":hash},"account":account,"account_home":e["account_home"]})
}

pub fn fake_host() {
    let args:Vec<_>=std::env::args().collect();
    assert!(args[1..4]==["service","app-server","--action"] || args[1..4]==["agent","host","--action"]);
    assert_eq!(args[5],"--request"); assert_eq!(args.len(),7,"no native flags or caller environment");
    let wire:Value=serde_json::from_str(&args[6]).unwrap();
    if matches!(args[4].as_str(),"retained"|"retained-cancel") {
        assert_eq!(&args[1..3],["agent","host"],"retained recovery never invokes a TaskQ runtime");
        let checkpoint:Value=serde_json::from_str(wire["checkpoint"].as_str().unwrap()).unwrap();
        let root=PathBuf::from(checkpoint["slot"]["binding"]["root"].as_str().unwrap());
        assert_eq!(std::fs::read_to_string(root.join("retained-checkpoint")).unwrap(),wire["checkpoint"]);
        assert_eq!(std::fs::read_to_string(root.join("retained-receipt")).unwrap(),wire["receipt"]);
        println!("{}",json!({"schema":"agentq.retained-operation/1","checkpoint":wire["checkpoint"],"receipt":wire["receipt"],"state":"unknown","cleanup":{"requested":args[4]}}));return;
    }
    let modern=wire["schema"]=="agentq.app-server/1";
    let mut r=wire.clone();if modern {r["slot"]=r["permit"].take();r.as_object_mut().unwrap().remove("permit");}
    let root=PathBuf::from(r["slot"]["binding"]["root"].as_str().unwrap());
    let original=root.join("request.json"); let receipt=root.join("receipt.json");
    match args[4].as_str() {
        "prepare"=>{
            assert!(!original.exists(),"never derive preparation from launch");
            assert_eq!(r.as_object().unwrap().len(),2);
            let prepared=json!({"schema":if modern {"agentq.app-server-preparation/1"}else{"taskq.app-server-preparation/1"},"binding":r["slot"]["binding"],
                "authority":authority(&root,&json!(29),&json!("default")),"host_instance":"invented-host-1"});
            let hash=ring::digest::digest(&ring::digest::SHA256,prepared.to_string().as_bytes()).as_ref().iter().map(|b|format!("{b:02x}")).collect::<String>();
            let answer=json!({"schema":"taskq.app-server/1","state":"prepared","preparation":hash,"policy_revision":29,"account":"default","prepared":prepared});
            let path=root.join("prepared.json");
            if path.exists() {assert_eq!(read(&path),answer,"preparation drift cannot replace original authority");}else{save(&path,&answer);}
            println!("{}", wire_receipt(answer,modern));
        },
        "launch"=>{
            if root.join("prepared.json").exists() {
                let prepared=read(&root.join("prepared.json"));
                assert_eq!(r["preparation"],prepared["preparation"],"missing/stale preparation refuses before intent");
                assert_eq!(r["slot"]["binding"],prepared["prepared"]["binding"]);
                assert_eq!(authority(&root,&r["policy_revision"],&r["account"]),prepared["prepared"]["authority"],"launch revalidates host authority");
            }
            if root.join("owner-route").exists() {
                assert_eq!(read(&root.join("owner-route/launch-intent.json")),wire,"durable exact intent precedes dispatch");
            }
            assert!(!original.exists(),"uncertain launch must never replay"); save(&original,&r);
            if root.join("scope-mode").exists() {
                let frontend=coreq::process::identity(std::process::id()).unwrap().unwrap();
                save(&root.join("frontend.json"),&json!({"pid":frontend.pid,"started":frontend.started}));
                let status=Command::new(std::env::current_exe().unwrap()).args([if modern {"agent"}else{"service"},if modern {"host"}else{"app-server"},"--action","fake-scope","--request",&wire.to_string()]).status().unwrap();
                std::process::exit(status.code().unwrap_or(1));
            }
            native_scope(&r,&root,&receipt);
        },
        "fake-scope"=>native_scope(&r,&root,&receipt),
        "observe"=>{
            assert_eq!(read(&original),r,"host independently rejects changed operation/key");
            if root.join("lost-reply").exists() {print!("incomplete");return;}
            println!("{}",wire_receipt(read(&receipt),modern));
        },
        "reconcile"=>{
            if original.exists() {assert_eq!(read(&original),r,"recovery preserves original slot/key/operation");}
            if root.join("lost-reply").exists() {print!("incomplete");return;}
            let seen=if receipt.exists() {read(&receipt)}else{json!({"schema":"taskq.app-server/1","caller":r["slot"]["binding"]["caller"],
                "operation":r["slot"]["binding"]["op_id"],"state":"unknown","launch":null})};
            println!("{}",wire_receipt(seen,modern));
        },
        "cancel"=>{
            assert_eq!(read(&original),r);
            let id=read(&root.join("child.json"));
            coreq::process::terminate(&coreq::process::Identity{pid:id["pid"].as_u64().unwrap() as u32,started:id["started"].as_str().unwrap().into()}).unwrap();
            let mut seen=read(&receipt);seen["state"]=json!("unknown");seen["launch"]["cancel_requested"]=json!(true);save(&receipt,&seen);
            println!("{}",wire_receipt(seen,modern));
        },_=>panic!("unexpected adapter action"),
    }
}

fn native_scope(r:&Value,root:&Path,receipt:&Path) {
            let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
            let mut command=Command::new(&exe);
            command.args(["app-server","--disable","apps"]).env("COREQ_FAKE_AGENT","codex").env("COREQ_FAKE_APP_SERVER","1")
                .env_remove("COREQ_FAKE_LINGER").env_remove("COREQ_FAKE_HANG").current_dir(&root);
            let mut child=command.spawn().unwrap();
            let supervisor=coreq::process::identity(std::process::id()).unwrap().unwrap();let native=coreq::process::identity(child.id()).unwrap().unwrap();
            save(&root.join("child.json"),&json!({"pid":native.pid,"started":native.started}));
            let effective=json!({"state":"verified-effective","scope":"invented process double, no native enforcement",
                "workspace_write":true,"protected_write_create_unlink_denied":["invented-control"],"reads":"existing profile","network":"existing profile"});
            let authority=authority(root,&r["policy_revision"],&r["account"]);
            save(&receipt,&json!({"schema":"taskq.app-server/1","operation":r["slot"]["binding"]["op_id"],"caller":r["slot"]["binding"]["caller"],
                "state":"verified-effective","verified_effective":effective,"launch":{"schema":"taskq.app-server/1","policy_revision":r["policy_revision"],
                "account":r["account"],"authority":authority,"supervisor":{"pid":supervisor.pid,"started":supervisor.started},
                "native":{"pid":native.pid,"started":native.started},"host_instance":"invented-host-1","state":"verified-effective",
                "verified_effective":effective,"cancel_requested":false,"reason":null}}));
            if root.join("frontend.json").exists() {
                let mut seen=read(receipt);seen["launch"]["frontend"]=read(&root.join("frontend.json"));
                seen["launch"]["input_closed"]=json!(false);
                seen["launch"]["scope"]=json!({"binding":{"boot":r["slot"]["binding"]["boot"],"coalition":123},"dispatch_ack":true});
                save(receipt,&seen);
            }
            save(&root.join("intent.json"),&json!(std::env::var("AGENTQ_OPTIMIZATION").unwrap()));
            let _=child.wait();
}

// M12.8 / #199: a launchd supervisor differs from the consumer's transport frontend.
// Risk: valid published host readback is rejected before input despite exact live identities.
pub fn protected_frontend_is_not_scope_supervisor() { rerun_after_readback_timeout(frontend_scenario); }
fn frontend_scenario() {
    let root=fresh("protected-scope");let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
    let home=root.join("account");std::fs::create_dir(&home).unwrap();
    save(&root.join("expected.json"),&json!({"profile_sha256":"b".repeat(64),"account_home":home,"approver":"invented-operator"}));
    std::fs::write(root.join("scope-mode"),b"invented separate supervisor").unwrap();
    let host=ProtectedHost::new(&exe,&digest(&exe)).unwrap();let owner=coreq::process::identity(std::process::id()).unwrap().unwrap();
    let r=json!({"schema":"taskq.app-server/1","slot":{"binding":{"schema":"taskq.slots/1","caller":"invented-consumer","op_id":"job-scope",
        "root":root,"runtime":"codex","pid":owner.pid,"started":owner.started,"boot":1,"expires_at":9999999999u64},"key":"a".repeat(64)},"policy_revision":29,"account":"default"});
    let e=ProtectedIdentity{profile_sha256:"b".repeat(64),executable:exe.clone(),executable_sha256:digest(&exe),account_home:home,approver:"invented-operator".into()};
    let mut server=host.launch(r,e,Intent::Off,&root.join("private.log")).unwrap();
    let deadline=until();while !root.join("intent.json").exists() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    let result=server.verify(until());server.cancel(until()).unwrap();
    let deadline=until();while server.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    drop(server);std::fs::remove_dir_all(root).unwrap();
    result.expect("exact frontend and separate live scope supervisor must verify");
}

// M12.8 / #199 A1-A5: independent preparation, fresh input gate and original-operation recovery.
// Risk: stale preparation dispatches, absent roots/descendants or a partial cleanup release capacity, or recovery replays input.
pub fn protected_preparation_and_completion_recover_original_operation() { rerun_after_readback_timeout(||prepared_operation(false)); }
pub fn independent_signed_host_recovers_without_taskq() { rerun_after_readback_timeout(||prepared_operation(true)); }
fn prepared_operation(modern:bool) {
    let root=fresh("protected-prepared");let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
    let home=root.join("account");std::fs::create_dir(&home).unwrap();
    let expected=json!({"profile_sha256":"b".repeat(64),"account_home":home,"approver":"invented-operator"});
    save(&root.join("expected.json"),&expected);std::fs::write(root.join("scope-mode"),b"invented scope").unwrap();
    let host=if modern {signed_host(&exe)}else{ProtectedHost::new(&exe,&digest(&exe)).unwrap()};let owner=coreq::process::identity(std::process::id()).unwrap().unwrap();
    let mut slot=json!({"binding":{"schema":"taskq.slots/1","caller":"invented-consumer","op_id":"job-prepared",
        "root":root,"runtime":"codex","pid":owner.pid,"started":owner.started,"boot":10,"expires_at":9999999999u64},"key":"a".repeat(64)});
    if modern {slot["binding"]["schema"]=json!("pipe.permits/1");slot["signature"]=json!("a".repeat(128));}
    let field=if modern {"permit"}else{"slot"};
    let mut wrong=slot.clone();wrong["binding"]["started"]=json!("reused-consumer-pid");
    assert!(host.prepare(&wrong,until()).is_err());assert!(!root.join("prepared.json").exists());
    assert!(host.prepare(&slot,Instant::now()).is_err());assert!(!root.join("prepared.json").exists(),"expired readback never invokes host");
    let handoff=root.join("owner-route");std::fs::create_dir(&handoff).unwrap();
    let preparation=if modern {
        let preparation=host.prepare(&slot,until()).unwrap();
        save(&handoff.join("preparation.json"),&preparation.checkpoint());
        save(&handoff.join("launch-request.json"),preparation.request());
        preparation
    }else{owner_route::prepare(&host,&slot,&handoff).unwrap()};let checkpoint=preparation.checkpoint();
    assert_eq!(read(&handoff.join("preparation.json")),checkpoint);
    assert_eq!(read(&handoff.join("launch-request.json")),*preparation.request());
    if !modern {assert!(owner_route::prepare(&host,&slot,&handoff).is_err(),"one-shot checkpoint cannot repeat preparation");}
    assert_eq!(preparation.request()[field],slot);assert!(!root.join("request.json").exists(),"prepare cannot launch");
    assert_eq!(host.prepare(&slot,until()).unwrap().checkpoint(),checkpoint,"duplicate prepare keeps original authority");
    // Durable reload before intent never dispatches. Missing preparation cannot come from an operation receipt.
    let recovered=host.recover_preparation(&serde_json::from_str::<Value>(&checkpoint.to_string()).unwrap()).unwrap();
    if !modern {assert_eq!(owner_route::recover(&host,&handoff).unwrap().request(),recovered.request());}
    let mut foreign=recovered.request().clone();foreign[field]["key"]=json!("c".repeat(64));
    save(&handoff.join("launch-request.json"),&foreign);
    if !modern {assert!(owner_route::recover(&host,&handoff).is_err(),"request cannot replace original slot/key");}
    save(&handoff.join("launch-request.json"),recovered.request());
    assert_eq!(host.reconcile(&recovered,until()).unwrap()["state"],"unknown");
    assert!(!root.join("request.json").exists());
    for (pointer,value) in [
        ("/receipt/state",json!("verified-effective")),("/receipt/preparation",Value::Null),
        ("/receipt/preparation",json!("e".repeat(64))),
        ("/receipt/prepared/binding/started",json!("foreign-incarnation")),("/receipt/prepared/host_instance",Value::Null),
        ("/receipt/prepared/authority/saved/grant/mode",json!("full")),
        ("/receipt/prepared/authority/adapter/sha256",json!("c".repeat(64))),
        ("/receipt/policy_revision",json!(30)),("/receipt/account",json!("foreign-account")),
    ] {
        let mut changed=checkpoint.clone();*changed.pointer_mut(pointer).unwrap()=value;
        assert!(host.recover_preparation(&changed).is_err(),"{pointer}");
    }
    // The host, not a second AgentQ resolver, refuses drift between prepare and actual dispatch.
    let mut drift=expected.clone();drift["profile_sha256"]=json!("c".repeat(64));save(&root.join("expected.json"),&drift);
    assert!(refused(host.prepare(&slot,until())),"never replace stale preparation");
    let mut drifted=host.launch_prepared(&recovered,Intent::Off,&root.join("refused.log")).unwrap();
    let deadline=until();while drifted.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert!(!root.join("request.json").exists(),"drift refusal precedes intent/native launch");drop(drifted);
    save(&root.join("expected.json"),&expected);
    let mut server=if modern {
        save(&handoff.join("launch-intent.json"),recovered.request());
        host.launch_prepared(&recovered,Intent::Off,&root.join("private.log")).unwrap()
    }else{owner_route::launch(&host,&handoff,&root.join("private.log")).unwrap()};
    if !modern {assert!(owner_route::launch(&host,&handoff,&root.join("duplicate.log")).is_err(),"intent consumes dispatch before any retry");}
    let deadline=until();while !root.join("intent.json").exists() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    let path=root.join("receipt.json");let good=read(&path);let message=json!({"id":1,"method":"initialize","params":{}});
    for (pointer,value) in [
        ("/launch/frontend/started",json!("reused-frontend-pid")),("/launch/frontend",Value::Null),
        ("/launch/host_instance",json!("changed-host")),("/launch/input_closed",json!(true)),
        ("/launch/scope/dispatch_ack",json!(false)),("/launch/scope/binding/boot",json!(99)),
        ("/launch/scope/binding/coalition",json!(0)),
    ] {
        let mut changed=good.clone();*changed.pointer_mut(pointer).unwrap()=value;save(&path,&changed);
        assert!(refused(server.send(&message,until())),"{pointer}");assert!(!root.join("app-input.jsonl").exists());
    }
    save(&path,&good);server.verify(until()).unwrap();
    server.send(&message,until()).unwrap();assert_eq!(server.receive(until()).unwrap().unwrap()["id"],1);
    server.send(&json!({"method":"initialized"}),until()).unwrap();
    let deadline=until();let native_input=loop {
        let input=std::fs::read_to_string(root.join("app-input.jsonl")).unwrap_or_default();
        if input.ends_with('\n') && input.lines().count()==2 {break input;}
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
    };
    let mut changed=good.clone();changed["launch"]["scope"]["binding"]["coalition"]=json!(124);save(&path,&changed);
    assert!(refused(server.send(&message,until())),"changed live binding fences input");save(&path,&good);
    assert!(refused(server.send(&message,until())),"old observation cannot reopen fenced input");
    server.cancel(until()).unwrap();let deadline=until();
    while server.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    drop(server);
    let recovered=host.recover_preparation(&checkpoint).unwrap();
    assert_eq!(host.reconcile(&recovered,until()).unwrap()["state"],"unknown","supervisor/root exit is not descendant completion");
    std::fs::write(root.join("lost-reply"),b"interrupted readback after durable intent").unwrap();
    assert!(refused(host.reconcile(&recovered,until())));std::fs::remove_file(root.join("lost-reply")).unwrap();
    assert_eq!(wire_request(read(&root.join("request.json")),modern),*recovered.request(),"lost reply never changes slot/key/effect");
    // Invented host-state model, not a native certificate: each partial state retains unknown occupancy.
    let mut completed=read(&path);completed["state"]=json!("completed");completed["launch"]["input_closed"]=json!(true);
    let launch=&mut completed["launch"];
    launch["scope"]["certificate"]=json!({"schema":"taskq.native-completion/1","scope":launch["scope"]["binding"],
        "supervisor":launch["supervisor"],"native":launch["native"],"counts":[5,4]});
    for name in ["cleanup_intent","bootout_ack","cleanup_verified"] {launch["scope"][name]=json!(true);}
    for (pointer,value) in [
        ("/state",json!("exited")),("/operation",json!("another-effect")),("/caller",json!("another-caller")),
        ("/launch/authority/profile_sha256",json!("c".repeat(64))),("/launch/host_instance",json!("changed-host")),
        ("/launch/input_closed",json!(false)),("/launch/scope/dispatch_ack",json!(false)),
        ("/launch/scope/cleanup_intent",json!(false)),("/launch/scope/bootout_ack",json!(false)),("/launch/scope/cleanup_verified",json!(false)),
        ("/launch/scope/certificate",Value::Null),("/launch/scope/certificate/counts",json!([5,3])),
        ("/launch/scope/certificate/counts",json!([3,5])),("/launch/scope/certificate/counts",json!([0,0])),
        ("/launch/scope/certificate/native/started",json!("reused-native-pid")),
        ("/launch/scope/certificate/supervisor/started",json!("reused-supervisor-pid")),
        ("/launch/scope/certificate/scope/coalition",json!(124)),("/launch/scope/binding/boot",json!(99)),
    ] {
        let mut changed=completed.clone();*changed.pointer_mut(pointer).unwrap()=value;save(&path,&changed);
        assert!(refused(host.reconcile(&recovered,until())),"{pointer}: retain occupancy");
    }
    save(&path,&completed);assert_eq!(host.reconcile(&recovered,until()).unwrap(),wire_receipt(completed.clone(),modern));
    assert_eq!(host.reconcile(&host.recover_preparation(&checkpoint).unwrap(),until()).unwrap(),wire_receipt(completed.clone(),modern),"duplicate/recovered completion observes same operation");
    let mut old=completed.clone();old["state"]=json!("unknown");save(&path,&old);
    assert_eq!(host.reconcile(&recovered,until()).unwrap()["state"],"unknown","reordered unknown cannot authorize release");
    assert_eq!(std::fs::read_to_string(root.join("app-input.jsonl")).unwrap(),native_input,"recovery never replays even the non-generating handshake");
    assert_eq!(wire_request(read(&root.join("request.json")),modern),*recovered.request());std::fs::remove_dir_all(root).unwrap();
}

// Requirement: no input before fresh exact authority, no widening/replay from untrusted task/env/receipt text.
// Risk: a forged/stale/unknown receipt starts work, kills a foreign child or reuses an uncertain operation.
// #193: accept producer-shaped resolved.effective; absent/null/Full/mode-only evidence never opens stdin.
pub fn protected_launch_refuses_tampering_and_preserves_native_transport() { rerun_after_readback_timeout(launch_scenario); }
fn launch_scenario() {
    let root=fresh("protected");let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
    let home=root.join("account");std::fs::create_dir(&home).unwrap();
    let host=ProtectedHost::new(&exe,&digest(&exe)).unwrap();
    assert!(ProtectedHost::new(&exe,&"0".repeat(64)).is_err(),"untrusted adapter refused before execution");
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
        assert!(refused(server.send(&message,until())),"{pointer}");
        assert!(!root.join("app-input.jsonl").exists(),"{pointer}: no native input");
    }
    save(&path,&good);
    std::fs::write(root.join("lost-reply"),b"invented interrupted reply").unwrap();
    assert!(refused(server.send(&message,until())));assert!(!root.join("app-input.jsonl").exists());
    std::fs::remove_file(root.join("lost-reply")).unwrap();
    server.verify(until()).unwrap();
    let mut unknown=good.clone();unknown["state"]=json!("unknown");save(&path,&unknown);
    assert!(refused(server.send(&message,until())));assert!(!root.join("app-input.jsonl").exists());
    save(&path,&good);server.verify(until()).expect("same binding must recover from an unknown readback without resubmitting");
    std::fs::write(root.join("lost-reply"),b"invented interrupted reply").unwrap();
    assert!(refused(server.send(&message,until())));assert!(!root.join("app-input.jsonl").exists());
    std::fs::remove_file(root.join("lost-reply")).unwrap();server.verify(until()).unwrap();
    let mut changed=r.clone();changed["slot"]["key"]=json!("c".repeat(64));assert!(refused(host.observe(&changed,until())));
    let codex=agent::runtime("codex").unwrap();
    let mut l=agent::Launch::new(&root,root.join("run.log"),"","fixtureq");
    l.env=vec![("CODEX_HOME".into(),home.into())];l.model=Some("fixture-model".into());
    l.mode=agent::Mode::AppServer {initialize:json!({}),thread:json!({"approvalPolicy":"never","sandbox":"read-only","dynamicTools":[{"name":"caller-tool"}],"developerInstructions":"private rules"})};
    let frontend=server.id();
    let mut run=server.open(None,&l).unwrap();
    codex.input(&mut run,&[agent::Part::Text("protected request".into())]).unwrap();
    let deadline=until();
    while codex.poll(&mut run).unwrap()==agent::State::Running {
        for event in agent::ProtectedAppServer::requests(&mut run).unwrap() {
            if event["method"]=="item/tool/call" {agent::ProtectedAppServer::reply(&mut run,&json!({"id":event["id"],"result":{"success":true}}),until()).unwrap();}
        }
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(codex.report(&run).unwrap().outcome,"ok");
    let store=coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let context=Context{product:"fixtureq",version:"1",project:"other",task:None,route:Some("job-1"),account:"default"};
    run.record(&context,&store).unwrap().recorded.unwrap();
    let checkpoint=codex.checkpoint(&run).unwrap();codex.recover(&checkpoint,&l).unwrap().record(&context,&store).unwrap().recorded.unwrap();
    assert_eq!(observations(&store).len(),1,"one usage observation across recovery");drop(store);
    let mut duplicate=host.launch(r.clone(),ProtectedIdentity{profile_sha256:"b".repeat(64),executable:exe.clone(),executable_sha256:digest(&exe),
        account_home:root.join("account"),approver:"invented-operator".into()},Intent::Off,&root.join("duplicate.log")).unwrap();
    let deadline=until();while duplicate.poll().unwrap().is_none() {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert!(refused(duplicate.send(&message,until())),"consumed operation cannot launch again even with a copied receipt");drop(duplicate);
    // A valid-looking new generation must fence the already-bound transport permanently.
    let mut changed=good.clone();changed["launch"]["host_instance"]=json!("invented-host-2");save(&path,&changed);
    assert!(refused(codex.input(&mut run,&[agent::Part::Text("next".into())])));save(&path,&good);assert!(refused(codex.input(&mut run,&[agent::Part::Text("next".into())])));
    assert_eq!(host.observe(&r,until()).unwrap(),good,"recovery observes same operation without relaunch");
    assert!(!codex.cancel(&mut run).unwrap(),"a host cancel never proves exit");let deadline=until();
    // Process identity and the transport's exit/EOF observation arrive independently.
    while codex.poll(&mut run).unwrap()==agent::State::Running || matches!(coreq::process::identity(frontend),Ok(Some(_))) {assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));}
    assert_eq!(codex.poll(&mut run).unwrap(),agent::State::Unknown,"a fenced input and EOF never prove a turn or release a slot");
    drop(run);std::fs::remove_dir_all(root).unwrap();
}

fn wire_receipt(mut value:Value,modern:bool)->Value {
    if modern {
        if value["schema"]=="taskq.app-server/1" {value["schema"]=json!("agentq.app-server/1");}
        if value["prepared"]["schema"]=="taskq.app-server-preparation/1" {value["prepared"]["schema"]=json!("agentq.app-server-preparation/1");}
        if value["launch"]["schema"]=="taskq.app-server/1" {value["launch"]["schema"]=json!("agentq.app-server/1");}
        if value["launch"]["scope"]["certificate"]["schema"]=="taskq.native-completion/1" {value["launch"]["scope"]["certificate"]["schema"]=json!("agentq.native-completion/1");}
    }
    value
}
fn wire_request(mut value:Value,modern:bool)->Value {
    if modern {value["permit"]=value["slot"].take();value.as_object_mut().unwrap().remove("slot");}value
}
fn signed_host(exe:&Path)->ProtectedHost {
    use ring::signature::{Ed25519KeyPair,KeyPair};
    // Invented deterministic fixture key; never used for a published adapter or authority.
    let pair=Ed25519KeyPair::from_seed_unchecked(&[41;32]).unwrap();
    let hex=|bytes:&[u8]|bytes.iter().map(|v|format!("{v:02x}")).collect::<String>();
    let hash=digest(exe);let signature=hex(pair.sign(format!("agentq.protected-adapter/1\0{hash}").as_bytes()).as_ref());
    let public=hex(pair.public_key().as_ref());
    assert!(ProtectedHost::signed(exe,&hash,&public,&"00".repeat(64)).is_err(),"invalid signature refuses before dispatch");
    assert!(ProtectedHost::signed(exe,&"00".repeat(32),&public,&signature).is_err(),"signature cannot replace the digest pin");
    ProtectedHost::signed(exe,&hash,&public,&signature).unwrap()
}

// M12.8 / #241 A2/A3: immutable recovery/cancellation crosses only the signed independent command boundary.
pub fn signed_retained_recovery_preserves_original_bytes() { rerun_after_readback_timeout(retained_scenario); }
fn retained_scenario() {
    let root=fresh("signed-retained");let exe=std::env::current_exe().unwrap().canonicalize().unwrap();
    let host=signed_host(&exe);
    let checkpoint=json!({"slot":{"binding":{"root":root}}}).to_string();let receipt=r#"{ "invented": "retained receipt" }"#;
    std::fs::write(root.join("retained-checkpoint"),&checkpoint).unwrap();std::fs::write(root.join("retained-receipt"),receipt).unwrap();
    assert_eq!(retained_answer(|until|host.reconcile_retained(&checkpoint,receipt,until))["state"],"unknown");
    assert_eq!(retained_answer(|until|host.cancel_retained(&checkpoint,receipt,until))["state"],"unknown");
    assert!(!root.join("app-input.jsonl").exists());assert!(!root.join("request.json").exists());
    assert_eq!(std::fs::read_to_string(root.join("retained-receipt")).unwrap(),receipt);
    std::fs::remove_dir_all(root).unwrap();
}
