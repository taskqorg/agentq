//! S283-SELECTED-20261107-1: distinct representation, shared decoding, caller-only scope.
use agentq_runtime::agent::{self,history::{self,History,StepKind,CODEX_APP_SERVER_SELECTED as SELECTED}};
use serde_json::json;
use std::{fs,path::PathBuf};
fn root(tag:&str)->PathBuf {let path=std::env::temp_dir().join(format!("agentq-selected-history-{}-{tag}",std::process::id()));fs::create_dir_all(&path).unwrap();path.canonicalize().unwrap()}
fn caller(format:&str)->History {History::caller("codex",format,"selected-source","attempt",Some("s")).unwrap()}

#[test]
fn selected_format_reuses_app_decoding_but_keeps_its_own_raw_identity() {
    let root=root("decode");let path=root.join("selected.jsonl");
    let raw=concat!(
        "{ \"method\" : \"turn/started\", \"params\" : {\"threadId\":\"s\",\"turn\":{\"id\":\"t\"}} }\r\n",
        "{\"method\":\"item/completed\",\"params\":{\"threadId\":\"s\",\"turnId\":\"t\",\"item\":{\"type\":\"commandExecution\",\"id\":\"command\",\"command\":\"printf \\\"λ\\\"\",\"aggregatedOutput\":\"λ\\n\",\"exitCode\":0,\"status\":\"completed\"}}}\r\n",
        "{\"method\":\"item/completed\",\"params\":{\"threadId\":\"s\",\"item\":{\"type\":\"agentMessage\",\"id\":\"reply\",\"text\":\"answer\"}}}\n",
        "{\"method\":\"thread/tokenUsage/updated\",\"params\":{\"threadId\":\"s\",\"turnId\":\"t\",\"tokenUsage\":{\"last\":{\"inputTokens\":10,\"outputTokens\":2,\"cachedInputTokens\":0,\"totalTokens\":12},\"total\":{\"inputTokens\":10,\"outputTokens\":2,\"cachedInputTokens\":0,\"totalTokens\":12},\"modelContextWindow\":100}}}\n"
    );fs::write(&path,raw).unwrap();
    let base=caller("codex-app-server").poll(&path).unwrap();let selected=caller(SELECTED).poll(&path).unwrap();assert_eq!(base.events.len(),selected.events.len());
    for (old,new) in base.events.iter().zip(&selected.events) {
        assert_ne!(old["id"],new["id"]);assert_eq!(new["format"],SELECTED);
        for key in ["raw","sha256","start","end","author","kind","blocks","native_session"] {assert_eq!(old[key],new[key],"{key}");}
    }
    assert!(selected.events[0]["raw"].as_str().unwrap().ends_with("\r\n"));
    let base=caller("codex-app-server").steps(&path,u64::MAX).unwrap();let mut h=caller(SELECTED);let page=h.steps(&path,u64::MAX).unwrap();assert_eq!(base.steps.len(),page.steps.len());
    for (old,new) in base.steps.iter().zip(&page.steps) {
        assert_ne!(old.id,new.id);assert_eq!(new.source["format"],SELECTED);assert_eq!((old.kind,&old.author,&old.state,&old.data),(new.kind,&new.author,&new.state,&new.data));assert_eq!(old.origins,new.origins);
    }
    let shell=page.steps.iter().find(|s|s.kind==StepKind::Shell).unwrap();assert_eq!(shell.native_id.as_deref(),Some("command"));assert_eq!(shell.data["output"],"λ\n");assert!(page.steps.iter().any(|s|s.kind==StepKind::Usage));
    assert_eq!(page.checkpoint["format"],SELECTED);assert_eq!(h.read_bytes(&path,0,raw.len() as u64).unwrap(),raw.as_bytes());
    let mut recovered=History::recover(&page.checkpoint).unwrap();assert_eq!(recovered.checkpoint()["format"],SELECTED);assert!(recovered.steps(&path,1).unwrap().steps.is_empty());fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_creation_and_recovery_never_grant_native_session_scope() {
    let root=root("admission");let dir=root.join("sessions");fs::create_dir_all(&dir).unwrap();let path=dir.join("selected.jsonl");fs::write(&path,"{\"method\":\"item/completed\",\"params\":{\"threadId\":\"s\",\"item\":{\"id\":\"i\",\"type\":\"agentMessage\",\"text\":\"answer\"}}}\n").unwrap();
    assert!(History::new(SELECTED,"source","s").is_err());assert!(History::with_native_session_alias(SELECTED,"source","s","other").is_err());assert!(History::caller("claude",SELECTED,"source","s",Some("s")).is_err());assert!(History::caller("codex","codex-app-server-selected/2","source","s",None).is_err());
    let mut h=caller(SELECTED);let saved=h.steps(&path,u64::MAX).unwrap().checkpoint;let mut no_caller=saved.clone();no_caller.as_object_mut().unwrap().remove("caller_runtime");assert!(History::recover(&no_caller).is_err());let mut foreign=saved.clone();foreign["caller_runtime"]=json!("claude");assert!(History::recover(&foreign).is_err());
    let unknown=root.join("identity-free.jsonl");fs::write(&unknown,"{\"method\":\"item/completed\",\"params\":{\"item\":{\"id\":\"u\",\"type\":\"agentMessage\",\"text\":\"unbound\"}}}\n").unwrap();
    let mut unbound=History::caller("codex",SELECTED,"source","binding",None).unwrap();let page=unbound.steps(&unknown,u64::MAX).unwrap();assert!(page.checkpoint["native_session"].is_null());assert!(page.steps[0].source["native_session"].is_null());assert_eq!(History::recover(&page.checkpoint).unwrap().checkpoint()["format"],SELECTED);
    let rt=agent::runtime("codex").unwrap();assert!(rt.history("s",Some(&root),Some(&path),None).is_err());assert!(rt.history_sessions(Some(&root)).unwrap().is_empty());
    let native=dir.join("native.jsonl");fs::write(&native,"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s\"}}\n").unwrap();let mut session=rt.history("s",None,Some(&native),None).unwrap();let mut forged=session.steps(u64::MAX).unwrap().checkpoint;
    forged["format"]=json!(SELECTED);forged["resolved"]["source"]["format"]=json!(SELECTED);
    for scoped in [false,true] {let mut value=forged.clone();if scoped{value["caller_runtime"]=json!("codex");}assert!(rt.history("s",None,None,Some(&value)).err().unwrap().contains("format differs"));}
    assert_eq!(rt.history_sessions(Some(&root)).unwrap().len(),1);fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_capabilities_are_observation_only() {
    let caps=history::capabilities(SELECTED,None,std::env::consts::OS).unwrap();assert_eq!(caps["format"],SELECTED);assert_eq!(caps["features"]["history"]["status"],"supported");
    for feature in ["launch","resume","session","input","ask_reply","heartbeat","cancel_recovery","model_effort_accounts_permissions","usage_errors"] {assert_eq!(caps["features"][feature]["status"],"unsupported","{feature}");}
    assert_eq!(caps["features"]["files_images"]["status"],"limited");assert_eq!(caps["live_output"]["status"],"limited");assert!(history::capabilities("codex-app-server-selected/2",None,"unknown").is_err());
}

#[test]
fn selected_media_and_native_identity_keep_existing_integrity_guards() {
    let root=root("media");let path=root.join("sessions/selected.jsonl");fs::create_dir_all(path.parent().unwrap()).unwrap();let image=root.join("image.png");fs::write(&image,b"\x89PNG\r\n\x1a\n").unwrap();let generated=root.join("generated_images/s/unreferenced.png");fs::create_dir_all(generated.parent().unwrap()).unwrap();fs::write(&generated,b"\x89PNG\r\n\x1a\nprivate").unwrap();
    let row=json!({"method":"item/completed","params":{"threadId":"s","item":{"id":"image","type":"imageGeneration","savedPath":image}}});let raw=format!("{row}\r\n");fs::write(&path,&raw).unwrap();let mut h=caller(SELECTED);let saved=h.steps(&path,u64::MAX).unwrap().checkpoint;
    let media=h.media(&path).unwrap();assert_eq!(media.items.len(),1);assert_eq!(media.items[0].origin["format"],SELECTED);assert_eq!(media.read(&media.items[0].id,0,8).unwrap(),b"\x89PNG\r\n\x1a\n");assert!(media.read(generated.to_str().unwrap(),0,1).is_err());
    fs::write(&image,b"\x89PNG\r\n\x1a\nchanged").unwrap();assert!(media.read(&media.items[0].id,0,1).is_err());
    let foreign=json!({"method":"item/completed","params":{"threadId":"foreign","item":{"id":"other","type":"agentMessage","text":"foreign"}}});fs::write(&path,format!("{raw}{foreign}\n")).unwrap();let before=h.checkpoint();assert!(h.steps(&path,u64::MAX).is_err());assert!(h.media(&path).is_err());assert_eq!(h.checkpoint(),before);assert!(History::recover(&saved).unwrap().media(&path).is_err());
    fs::write(&path,b"{}\n").unwrap();assert!(h.read_bytes(&path,0,1).is_err());fs::remove_dir_all(root).unwrap();
}
