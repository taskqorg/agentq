//! M12.8 / #241 A1-A4: signature/permit trust, durable no-replay and immutable retained recovery.
use super::*;
use ring::signature::{Ed25519KeyPair,KeyPair};
fn encode(bytes:&[u8])->String {bytes.iter().map(|v|format!("{v:02x}")).collect()}
fn sign(key:&Ed25519KeyPair,domain:&str,bytes:&[u8])->String {
    let mut message=domain.as_bytes().to_vec();message.push(0);message.extend_from_slice(bytes);encode(key.sign(&message).as_ref())
}
fn save(path:&Path,value:&Value) {fs::write(path,value.to_string()).unwrap();}
struct Fixture {root:PathBuf,key:Ed25519KeyPair,permit:Value}
impl Drop for Fixture {fn drop(&mut self){let _=fs::remove_dir_all(&self.root);}}
impl Fixture {
    fn new()->Self {
        let root=std::env::temp_dir().join(format!("agentq-host-test-{}",random_id().unwrap()));fs::create_dir(&root).unwrap();
        let root=root.canonicalize().unwrap();fs::create_dir(root.join("policies")).unwrap();fs::create_dir(root.join("retained")).unwrap();
        // Invented test seed, never a production signing key.
        let key=Ed25519KeyPair::from_seed_unchecked(&[73;32]).unwrap();
        let artifact_digest=file_digest(&std::env::current_exe().unwrap()).unwrap();
        save(&root.join("trust.json"),&json!({"schema":"agentq.protected-adapter/1","sha256":artifact_digest,"public_key":encode(key.public_key().as_ref()),"signature":sign(&key,"agentq.protected-adapter/1",artifact_digest.as_bytes())}));
        save(&root.join("initial.json"),&json!({"schema":"agentq.host-initial/1","operations":0}));
        let mut f=Self{root,key,permit:Value::Null};
        let policy=json!({"schema":"agentq.protected-policy/1","authority":{"project":f.root},"permit_public_keys":{"pipe":encode(f.key.public_key().as_ref())}});
        f.signed(&policy_path(&f.root,&f.root),"agentq.protected-policy/1",&policy);
        f.permit=json!({"binding":{"schema":"pipe.permits/1","caller":"pipe","op_id":"invented-original-job","root":f.root,"runtime":"codex","pid":17,"started":"invented-start","boot":10,"expires_at":9999999999u64},"key":"a".repeat(64),"signature":""});
        let payload=json!({"binding":f.permit["binding"],"key_sha256":digest(f.permit["key"].as_str().unwrap().as_bytes())});
        f.permit["signature"]=json!(sign(&f.key,"pipe.permits/1",payload.to_string().as_bytes()));f
    }
    fn signed(&self,path:&Path,domain:&str,value:&Value) {
        let payload=value.to_string();save(path,&json!({"payload":payload,"signature":sign(&self.key,domain,payload.as_bytes())}));
    }
}
// Risk: a valid digest without a signature, another issuer, changed payload or corrupt journal opens an effect gate.
#[test]
fn protected_independent_signatures_permits_and_missing_state_refuse() {
    let f=Fixture::new();let original=fs::read(f.root.join("trust.json")).unwrap();
    verify_artifact(&f.root).unwrap();permits::validate(&f.root,&f.permit).unwrap();
    for field in ["sha256","public_key","signature"] {
        let mut trust:Value=serde_json::from_slice(&original).unwrap();trust[field]=json!("00".repeat(if field=="signature" {64}else{32}));save(&f.root.join("trust.json"),&trust);
        assert!(permits::validate(&f.root,&f.permit).is_err(),"{field}");fs::write(f.root.join("trust.json"),&original).unwrap();
    }
    for pointer in ["/signature","/key","/binding/op_id","/binding/caller","/binding/started","/binding/schema"] {
        let mut changed=f.permit.clone();*changed.pointer_mut(pointer).unwrap()=json!("forged");
        assert!(permits::validate(&f.root,&changed).is_err(),"{pointer}");
    }
    resources::within(&f.root,resources::WAIT,|s|{assert!(s.ops.is_empty());Ok(())}).unwrap();
    fs::remove_file(f.root.join("operations.json")).unwrap();
    assert!(resources::within(&f.root,resources::WAIT,|_|Ok(())).is_err(),"lost journal never resets state");
    fs::write(f.root.join("operations.json"),b"corrupt").unwrap();
    assert!(resources::within(&f.root,resources::WAIT,|_|Ok(())).is_err(),"corrupt journal never resets state");
}
fn model()->(resources::State,Request,Value,coreq::process::Identity) {
    let r=Request{schema:SCHEMA.into(),permit:json!({"binding":{"caller":"pipe","op_id":"invented-job","boot":10}}),policy_revision:29,account:"default".into(),preparation:String::new()};
    let a=json!({"project":"invented-project","profile_sha256":"invented-profile","saved":{"revision":29,"grant":{"by":"invented-operator"}},"account":"default","executable_sha256":"invented-image"});
    let mut s=resources::State::default();
    s.ops.insert("op".into(),resources::Op{bound:json!({"local_permit":{"binding":r.permit["binding"],"key_hash":"invented-key","prepared":preparation(&r,&a,&json!("invented-host"))}}),receipt:json!({"instance":"invented-host","occupied":true})});
    let mut r=r;r.preparation=digest(s.ops["op"].bound["local_permit"]["prepared"].to_string().as_bytes());
    (s,r,a,coreq::process::Identity{pid:42,started:"invented-supervisor".into()})
}
// Risk: drift, crash, duplicate/reordered counts or partial cleanup releases occupancy or allows a second dispatch.
#[test]
fn protected_independent_adversarial_recovery_keeps_original_occupancy() {
    let (s,r,a,id)=model();
    for field in ["project","profile_sha256","saved","account","executable_sha256"] {
        let mut changed=a.clone();changed[field]=json!("changed");let mut trial=s.clone();
        assert!(consume(&mut trial,"op",&r,&changed,&id).is_err());assert_eq!(trial,s);
    }
    let mut s=s;consume(&mut s,"op",&r,&a,&id).unwrap();
    s=serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    for state in ["unknown","verified-effective","cancelled","completed"] {
        let mut trial=s.clone();trial.ops.get_mut("op").unwrap().bound["local_permit"]["launch"]["state"]=json!(state);let before=trial.clone();
        assert!(consume(&mut trial,"op",&r,&a,&id).is_err());assert_eq!(trial,before);
    }
    cancel(&mut s,"op",&r).unwrap();assert!(input_gate(&s,"op",&r).is_err());
    let scope=json!({"boot":10,"coalition":101});
    let launch=&mut s.ops.get_mut("op").unwrap().bound["local_permit"]["launch"];
    launch["native"]=json!({"pid":43,"started":"invented-native"});launch["input_closed"]=json!(true);
    launch["scope"]=json!({"binding":scope,"dispatch_ack":true,"cleanup_intent":false,"cleanup_verified":false});
    assert!(!certify(&mut s,"op",&r,&id,&scope,[9,6]).unwrap());assert!(release_ready(&s.ops["op"]).is_err());
    let before=s.clone();for counts in [[8,7],[9,10],[8,8]] {assert!(certify(&mut s,"op",&r,&id,&scope,counts).is_err());assert_eq!(s,before);}
    assert!(certify(&mut s,"op",&r,&id,&scope,[9,8]).unwrap());
    let sealed=s.clone();assert!(certify(&mut s,"op",&r,&id,&scope,[9,8]).unwrap());assert_eq!(s,sealed);
    assert!(certify(&mut s,"op",&r,&id,&scope,[10,9]).is_err());assert_eq!(s,sealed);
    for field in ["cleanup_intent","bootout_ack","cleanup_verified"] {
        assert!(release_ready(&s.ops["op"]).is_err());s.ops.get_mut("op").unwrap().bound["local_permit"]["launch"]["scope"][field]=json!(true);
    }
    release_ready(&s.ops["op"]).unwrap();assert!(consume(&mut s,"op",&r,&a,&id).is_err());
    assert_eq!(s.ops["op"].bound["local_permit"]["key_hash"],"invented-key");
}
// Risk: importing a receipt as its own authority, changing an operation's original bytes or replacing its adapter identity.
#[test]
fn protected_retained_import_authenticates_exact_original_bytes() {
    let f=Fixture::new();let slot=json!({"binding":{"schema":"taskq.slots/1","caller":"pipe","op_id":"invented-original-job","root":f.root,"runtime":"codex","pid":17,"started":"invented-start","boot":10,"expires_at":9999999999u64},"key":"a".repeat(64)});
    let independent=json!({"schema":"taskq.app-server-preparation/1","binding":slot["binding"],"authority":{"adapter":"invented-original-adapter","project":f.root,"runtime":"codex","saved":{"grant":{"mode":"workspace"}}},"host_instance":"invented-original-host"});
    let prepared=json!({"schema":"taskq.app-server/1","state":"prepared","policy_revision":29,"account":"default","preparation":digest(independent.to_string().as_bytes()),"prepared":independent});
    let checkpoint=json!({"slot":slot,"receipt":prepared}).to_string();
    let receipt=json!({"schema":"taskq.app-server/1","caller":"pipe","operation":"invented-original-job","launch":{"authority":prepared["prepared"]["authority"],"host_instance":"invented-original-host","policy_revision":29,"account":"default"}}).to_string();
    let input=json!({"checkpoint":checkpoint,"receipt":receipt});
    assert!(retained_original(&f.root,&input).is_err(),"unsigned copied receipts grant nothing");
    let key=operation_id(&Request{schema:SCHEMA.into(),permit:slot,policy_revision:29,account:"default".into(),preparation:String::new()}).unwrap();
    let path=f.root.join("retained").join(format!("{key}.json"));f.signed(&path,"agentq.retained-operation/1",&input);
    let before=fs::read(&path).unwrap();let recovered=retained_original(&f.root,&input).unwrap();
    assert_eq!(recovered["authority"]["adapter"],"invented-original-adapter");assert_eq!(fs::read(&path).unwrap(),before);
    let mut changed=input.clone();changed["receipt"]=json!(format!("{} ",receipt));
    assert!(retained_original(&f.root,&changed).is_err(),"even equivalent JSON cannot rewrite retained bytes");
}
// Risk: an imported signed snapshot silently widens a grant or loses the account/image/profile binding.
#[test]
fn protected_existing_authority_snapshot_refuses_drift_before_launch() {
    let f=Fixture::new();let install=f.root.join("host");let project=f.root.join("project");let home=f.root.join("home");
    fs::create_dir_all(install.join("policies")).unwrap();fs::create_dir(&project).unwrap();fs::create_dir_all(home.join(".codex")).unwrap();
    fs::write(home.join(".codex/config.toml"),b"# invented native config").unwrap();fs::copy(f.root.join("trust.json"),install.join("trust.json")).unwrap();
    let native=std::env::current_exe().unwrap().canonicalize().unwrap();let profile="(version 1)\n(allow default)\n";
    let authority=json!({"project":project,"runtime":"codex","account":"default","account_home":home.join(".codex"),"executable":native,"executable_sha256":file_digest(&native).unwrap(),"profile_sha256":digest(profile.as_bytes()),"requested":"workspace-write","resolved":{"effective":"workspace"},"saved":{"revision":29,"grant":{"mode":"workspace","runtime":"codex","project":project,"by":"invented-operator","at":1}}});
    let policy=json!({"schema":"agentq.protected-policy/1","authority":authority,"profile":profile,"args":["-c","sandbox_mode=\"danger-full-access\"","-c","approval_policy=\"never\""],"permit_public_keys":{}});
    let path=policy_path(&install,&project);f.signed(&path,"agentq.protected-policy/1",&policy);
    let r=Request{schema:SCHEMA.into(),permit:json!({"binding":{"root":project,"caller":"pipe","op_id":"invented-job"}}),policy_revision:29,account:"default".into(),preparation:String::new()};
    let planned=resolve(&install,&home,&r,"macos",&native).unwrap();assert!(planned.access.contains("deny file-write*"));
    for (pointer,value) in [("/authority/saved/grant/mode",json!("full")),("/authority/saved/grant/by",json!("unknown")),("/authority/saved/revision",json!(30)),("/authority/account",json!("changed")),("/authority/executable_sha256",json!("0".repeat(64))),("/profile",json!("changed profile"))] {
        let mut changed=policy.clone();*changed.pointer_mut(pointer).unwrap()=value;f.signed(&path,"agentq.protected-policy/1",&changed);
        assert!(resolve(&install,&home,&r,"macos",&native).is_err(),"{pointer}");
    }
    f.signed(&path,"agentq.protected-policy/1",&policy);
    for os in ["windows","linux"] {assert!(resolve(&install,&home,&r,os,&native).is_err());}
    fs::remove_file(home.join(".codex/config.toml")).unwrap();assert!(resolve(&install,&home,&r,"macos",&native).is_err());
}
// Risk: a crash after exact-job bootout intent is mistaken for acknowledgment and repeats cleanup.
#[cfg(target_os="macos")]
#[test]
fn protected_retained_lost_cleanup_ack_never_retries() {
    let f=Fixture::new();let binding=json!({"boot":10,"coalition":101});let supervisor=json!({"pid":17,"started":"invented-supervisor"});let native=json!({"pid":18,"started":"invented-native"});
    let original=json!({"supervisor":supervisor,"native":native,"input_closed":true,"scope":{"binding":binding,"dispatch_ack":true,"label":"org.taskq.protected.invented","domain":format!("gui/{}",unsafe{libc::geteuid()}),"certificate":{"schema":"taskq.native-completion/1","scope":binding,"supervisor":supervisor,"native":native,"counts":[9,8]}}});
    resources::within(&f.root,resources::WAIT,|s| {s.ops.insert("retained-invented-archive".into(),resources::Op{bound:json!({"original_sha256":digest(original.to_string().as_bytes())}),receipt:json!({"cleanup_intent":true})});Ok(())}).unwrap();
    let before=fs::read(f.root.join("operations.json")).unwrap();
    let outcome=scope::cleanup_retained(&f.root,&original,"invented-archive").unwrap();
    assert_eq!(outcome["state"],"unknown");assert_eq!(outcome["reason"],"retained cleanup unacknowledged; never repeat");
    assert_eq!(fs::read(f.root.join("operations.json")).unwrap(),before,"lost acknowledgment never rewrites receipt or consumes cleanup twice");
}
