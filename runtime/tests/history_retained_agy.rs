//! M12.18: retained sources resolve native identity without caller filename/format inference.
use agentq_runtime::agent;
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
const SESSION:&str="aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
fn root(tag:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-retained-agy-{}-{tag}",std::process::id()));fs::create_dir_all(&p).unwrap();p}
fn rows(path:&Path,values:&[Value]) {fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,values.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();}
fn init()->Value {json!({"event":"init","conversation_id":SESSION})}
fn result()->Value {json!({"event":"result","result":{"conversation_id":SESSION,"status":"SUCCESS","response":"saved answer"}})}

#[test]
fn request_local_and_worker_logs_keep_native_identity_physical_pages_and_recovery() {
    let root=root("local");let rt=agent::runtime("antigravity").unwrap();
    for relative in ["install/setup/request/native.jsonl","worker/output.jsonl","copied/history.jsonl"] {
        let path=root.join(relative);let values=if relative.starts_with("worker") {vec![json!({"event":"user","message":{"content":"intent"}}),init(),result()]}else{vec![init(),result()]};
        rows(&path,&values);let original=fs::read(&path).unwrap();
        let mut session=rt.history(SESSION,None,Some(&path),None).unwrap();
        let page=session.poll().unwrap();assert_eq!(page.events.len(),values.len());
        assert!(page.events.iter().all(|e|e["format"]=="agy-stream"));assert_eq!(page.events.last().unwrap()["blocks"][0]["text"],"saved answer");
        assert_eq!(page.events[0]["start"],0);assert_eq!(page.events.last().unwrap()["end"],original.len());
        let backwards=rt.history(SESSION,None,Some(&path),None).unwrap().poll_before().unwrap();assert_eq!(backwards.events,page.events);
        let saved=session.checkpoint();assert_eq!(saved["resolved"]["source"]["native_session"],SESSION);
        assert!(rt.history(SESSION,None,Some(&path),Some(&saved)).unwrap().poll().unwrap().events.is_empty());
        assert!(rt.history("foreign",None,Some(&path),None).is_err());assert_eq!(fs::read(&path).unwrap(),original);
        let steps=rt.history(SESSION,None,Some(&path),None).unwrap().steps(u64::MAX).unwrap();assert!(steps.steps.iter().any(|s|s.data["text"]=="saved answer"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_headers_require_native_shape_and_consistent_valid_identity() {
    let root=root("invalid");let path=root.join("request/native.jsonl");let rt=agent::runtime("antigravity").unwrap();
    for value in [json!({"conversation_id":SESSION}),json!({"event":"unknown","conversation_id":SESSION}),json!({"event":"init"}),json!({"event":"init","conversation_id":12}),json!({"event":"init","conversation_id":"../foreign"}),json!({"event":"init","conversation_id":SESSION,"conversationId":"foreign"}),json!({"event":"init","conversation_id":SESSION,"type":"result","session_id":"foreign"}),json!({"event":"init","conversation_id":SESSION,"response":{"newConversation":{"conversationId":"foreign"}}}),json!({"event":"init","conversation_id":SESSION,"result":{"conversation_id":null}}),json!({"event":"user","message":{"content":"intent"}}),json!({"event":"user","conversation_id":SESSION,"message":{"content":"intent"}})] {
        rows(&path,&[value]);assert!(rt.history(SESSION,None,Some(&path),None).is_err());
    }
    for bytes in ["{broken}\n", "{\"event\":\"init\",\"conversation_id\":\"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\"}"] {fs::write(&path,bytes).unwrap();assert!(rt.history(SESSION,None,Some(&path),None).is_err());}
    rows(&path,&[json!({"event":"user","message":{"content":"intent"}}),json!({"unknown":true}),init()]);assert!(rt.history(SESSION,None,Some(&path),None).is_err());
    rows(&path,&[init(),json!({"event":"result","result":{"conversation_id":"foreign","status":"SUCCESS"}})]);
    let mut session=rt.history(SESSION,None,Some(&path),None).unwrap();let before=session.checkpoint();assert!(session.poll().is_err());assert_eq!(session.checkpoint(),before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_acp_and_native_sidecar_stay_distinct_from_legacy_intent() {
    let root=root("formats");let path=root.join("request/native.jsonl");let rt=agent::runtime("antigravity").unwrap();
    let acp=json!({"type":"acp","message":{"method":"session/update","params":{"sessionId":SESSION,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"ACP answer"}}}}});
    rows(&path,&[acp.clone()]);let mut session=rt.history(SESSION,None,Some(&path),None).unwrap();assert_eq!(session.poll().unwrap().events[0]["format"],"acp");
    rows(&path,&[json!({"event":"user","message":{"content":"legacy intent"}}),acp.clone()]);assert!(rt.history(SESSION,None,Some(&path),None).is_err());
    for values in [vec![init(),acp.clone()],vec![acp,init()]] {rows(&path,&values);let mut session=rt.history(SESSION,None,Some(&path),None).unwrap();assert!(session.poll().is_err());}
    let sidecar=root.join(format!("antigravity-cli/brain/{SESSION}/.system_generated/logs/transcript.jsonl"));
    rows(&sidecar,&[json!({"type":"PLANNER_RESPONSE","step_index":0,"content":"sidecar answer"})]);let mut session=rt.history(SESSION,None,Some(&sidecar),None).unwrap();assert_eq!(session.poll().unwrap().events[0]["format"],"antigravity-sidecar");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retained_recovery_refuses_copy_replacement_and_prefix_rewrite() {
    let root=root("guards");let path=root.join("request/native.jsonl");let rt=agent::runtime("antigravity").unwrap();rows(&path,&[init(),result()]);
    let mut session=rt.history(SESSION,None,Some(&path),None).unwrap();session.poll().unwrap();let saved=session.checkpoint();let original=fs::read(&path).unwrap();
    let copy=root.join("copy.jsonl");fs::write(&copy,&original).unwrap();assert!(rt.history(SESSION,None,Some(&copy),Some(&saved)).is_err());
    fs::rename(&path,root.join("old.jsonl")).unwrap();fs::write(&path,&original).unwrap();assert!(rt.history(SESSION,None,Some(&path),Some(&saved)).is_err());
    let mut fresh=rt.history(SESSION,None,Some(&path),None).unwrap();fresh.poll().unwrap();let fresh_saved=fresh.checkpoint();
    let changed=String::from_utf8(original).unwrap().replace("saved answer","other answer");fs::write(&path,changed).unwrap();assert!(rt.history(SESSION,None,Some(&path),Some(&fresh_saved)).is_err());
    fs::remove_dir_all(root).unwrap();
}
