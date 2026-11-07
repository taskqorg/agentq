//! M12.18/M12.20: native kind and explicitly witnessed physical copies remain observations.
//! Source-only additions; execution awaits the coordinated global verification batch.
use agentq_runtime::agent::{self,Launch};
use serde_json::{json,Value};
use std::{fs,io::Write,path::{Path,PathBuf}};
const ORIGINAL:&str="aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
const DESTINATION:&str="11111111-2222-4333-8444-555555555555";
const FOREIGN:&str="99999999-8888-4777-8666-555555555555";
fn root(tag:&str)->PathBuf {let root=std::env::temp_dir().join(format!("agentq-claude-source-facts-{}-{tag}",std::process::id()));fs::create_dir_all(&root).unwrap();root}
fn path(home:&Path,session:&str)->PathBuf {home.join("projects/project").join(format!("{session}.jsonl"))}
fn rows(path:&Path,rows:&[Value]) {fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,rows.iter().map(|row|format!("{row}\n")).collect::<String>()).unwrap();}
fn message(session:&str)->Value {json!({"type":"user","sessionId":session,"message":{"content":"exact copied input"}})}

#[test]
fn disappeared_job_kind_is_record_local_and_missing_or_unrecognized_stays_unknown() {
    let home=root("kind");let file=path(&home,ORIGINAL);let rt=agent::runtime("claude").unwrap();
    let mut bg=message(ORIGINAL);bg["sessionKind"]=json!("bg");
    let mut native=vec![bg,message(ORIGINAL)];
    for value in [Value::Null,json!("future"),json!("foreground"),json!(true)] {let mut row=message(ORIGINAL);row["sessionKind"]=value;native.push(row);}
    rows(&file,&native);
    // No job locator exists: the surviving transcript supplies only its own typed native fact.
    let mut session=rt.history(ORIGINAL,Some(&home),None,None).unwrap();let page=session.poll().unwrap();
    assert_eq!(page.events[0]["presentation"]["session_kind"],"bg");
    for event in &page.events[1..] {assert!(event["presentation"]["session_kind"].is_null());assert!(event["presentation"]["unknown"]["session_kind"].is_string());}
    let reverse=rt.history(ORIGINAL,Some(&home),None,None).unwrap().poll_before().unwrap();assert_eq!(page.events,reverse.events);
    let steps=rt.history(ORIGINAL,Some(&home),None,None).unwrap().steps(u64::MAX).unwrap();
    assert_eq!(steps.steps[0].presentation["session_kind"],"bg");assert!(steps.steps[1..].iter().all(|step|step.presentation["session_kind"].is_null()));
    let selected=home.join("other-account");fs::create_dir_all(&selected).unwrap();assert!(rt.history(ORIGINAL,Some(&selected),None,None).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn copied_destination_resolves_only_with_explicit_original_and_keeps_native_identity_separate() {
    let home=root("copy");let file=path(&home,DESTINATION);let rt=agent::runtime("claude").unwrap();
    rows(&file,&[message(ORIGINAL)]);rows(&path(&home,ORIGINAL),&[message(ORIGINAL)]);
    assert!(rt.history(DESTINATION,Some(&home),None,None).is_err());
    let listed=rt.history_sessions(Some(&home)).unwrap();let physical=listed.iter().find(|source|source.path==file).unwrap();
    let mut copy=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).unwrap();
    assert_eq!(copy.source().path,file);assert_eq!(copy.source().session,DESTINATION);assert_eq!(copy.source().native_session,ORIGINAL);assert_eq!(copy.source().id,physical.id);
    let pristine=copy.checkpoint();assert_eq!(pristine["offset"],0);assert!(pristine["resolved"]["copy_witness"]["offset"].as_u64().unwrap()>0);
    let page=copy.poll().unwrap();assert_eq!(page.events[0]["session"],DESTINATION);assert_eq!(page.events[0]["native_session"],ORIGINAL);assert_eq!(page.events[0]["session_binding"],"alias");
    assert_eq!(copy.read_bytes(0,fs::metadata(&file).unwrap().len()).unwrap(),fs::read(&file).unwrap());
    let mut recovered=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&copy.checkpoint())).unwrap();assert!(recovered.poll().unwrap().events.is_empty());
    let mut reverse=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),Some(&file),None).unwrap();assert_eq!(reverse.poll_before().unwrap().events,page.events);
    let launch=Launch::new(home.clone(),home.join("unused-output"),"","fixture");assert!(copy.resume_command(&launch,&[]).is_err());assert!(copy.media().is_err());
    assert!(rt.history(DESTINATION,Some(&home),None,Some(&pristine)).is_err());
    let other=home.join("other-account");fs::create_dir_all(other.join("projects")).unwrap();
    assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&other),Some(&file),None).is_err());
    assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&other),None,Some(&pristine)).is_err());
    assert!(rt.history_with_native_session_alias(DESTINATION,FOREIGN,Some(&home),None,Some(&pristine)).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn physical_ambiguity_unknown_headers_and_unbound_or_conflicting_aliases_refuse() {
    let home=root("refusal");let file=path(&home,DESTINATION);let rt=agent::runtime("claude").unwrap();rows(&file,&[message(ORIGINAL)]);
    let duplicate=home.join("projects/another").join(format!("{DESTINATION}.jsonl"));rows(&duplicate,&[json!({"unknown":true})]);
    assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).is_err());fs::remove_file(duplicate).unwrap();
    for original in ["",DESTINATION,FOREIGN,"../foreign"] {assert!(rt.history_with_native_session_alias(DESTINATION,original,Some(&home),None,None).is_err());}
    assert!(agent::runtime("codex").unwrap().history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),Some(&file),None).is_err());
    for native in [vec![json!({"type":"user","message":{"content":"unbound"}})],vec![message(DESTINATION)],vec![message(FOREIGN)],vec![json!({"type":"user","sessionId":ORIGINAL,"session_id":FOREIGN})],vec![json!({"type":"user","sessionId":12})],vec![json!({"type":"user","sessionId":ORIGINAL,"agentId":ORIGINAL})],vec![json!({"type":"agentq_control","session_id":ORIGINAL})],vec![message(ORIGINAL),message(FOREIGN)]] {
        rows(&file,&native);assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).is_err());
    }
    fs::write(&file,format!("{{\"type\":\"user\",\"sessionId\":\"{ORIGINAL}\"}}")).unwrap();assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).is_err());
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn native_proof_is_bounded_and_later_foreign_records_fail_without_cursor_progress() {
    let home=root("bounds");let file=path(&home,DESTINATION);let rt=agent::runtime("claude").unwrap();
    let mut native=vec![json!({"type":"system","subtype":"metadata"});128];native.push(message(ORIGINAL));rows(&file,&native);
    assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).is_err());
    native=vec![message(ORIGINAL);128];native.push(message(FOREIGN));rows(&file,&native);
    let mut copy=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).unwrap();assert_eq!(copy.poll().unwrap().events.len(),128);
    let before=copy.checkpoint();assert!(copy.poll().is_err());assert_eq!(copy.checkpoint(),before);
    rows(&file,&[message(ORIGINAL)]);let mut copy=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).unwrap();copy.poll().unwrap();
    fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(format!("{}\n",message(DESTINATION)).as_bytes()).unwrap();assert_eq!(copy.poll().unwrap().events[0]["session_binding"],"native");
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn copy_witness_survives_recovery_and_refuses_replacement_rewrite_or_changed_binding() {
    let home=root("witness");let file=path(&home,DESTINATION);let rt=agent::runtime("claude").unwrap();rows(&file,&[message(ORIGINAL)]);
    let mut copy=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).unwrap();let fresh=copy.checkpoint();
    assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&fresh)).is_ok());
    for field in ["source","session","native_session","native_session_alias"] {
        let mut changed=fresh.clone();changed["resolved"]["copy_witness"][field]=json!(FOREIGN);assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&changed)).is_err());
    }
    let mut unbound=fresh.clone();unbound["resolved"].as_object_mut().unwrap().remove("copy_witness");assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&unbound)).is_err());
    let mut zero=fresh.clone();zero["resolved"]["copy_witness"]["offset"]=json!(0);assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&zero)).is_err());
    let original=fs::read(&file).unwrap();fs::rename(&file,home.join("retained.jsonl")).unwrap();fs::write(&file,&original).unwrap();
    assert!(copy.poll().is_err());assert_eq!(copy.checkpoint(),fresh);assert!(rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,Some(&fresh)).is_err());
    let mut copy=rt.history_with_native_session_alias(DESTINATION,ORIGINAL,Some(&home),None,None).unwrap();let before=copy.checkpoint();
    fs::write(&file,String::from_utf8(original).unwrap().replace("exact copied input","other copied input")).unwrap();
    assert!(copy.poll().is_err());assert_eq!(copy.checkpoint(),before);assert!(copy.read_bytes(0,1).is_err());assert!(copy.steps(1).is_err());
    fs::remove_dir_all(home).unwrap();
}
