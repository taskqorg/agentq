//! A283-CALLER-MEDIA-1: explicit caller sources are observation, never native discovery.
use agentq_runtime::agent::{self,history::History};
use serde_json::{json,Value};
use std::{fs,path::PathBuf};
fn root(tag:&str)->PathBuf {let path=std::env::temp_dir().join(format!("agentq-caller-media-{}-{tag}",std::process::id()));fs::create_dir_all(&path).unwrap();path.canonicalize().unwrap()}
fn rows(path:&std::path::Path,records:&[Value]) {fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,records.iter().map(|r|format!("{r}\n")).collect::<String>()).unwrap();}

#[test]
fn identity_free_logs_keep_caller_binding_and_unknown_native_identity() {
    for (runtime,format,row) in [
        ("codex","codex-exec",json!({"type":"item.completed","item":{"id":"image","type":"imageGeneration","savedPath":"IMAGE"}})),
        ("codex","codex-app-server",json!({"method":"item/completed","params":{"item":{"id":"image","type":"imageGeneration","savedPath":"IMAGE"}}})),
        ("antigravity","acp",json!({"type":"acp","message":{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"image","mimeType":"image/png","data":"iVBORw0KGgo="}}}}})),
    ] {
        let root=root(format);let path=root.join("owned.jsonl");let image=root.join("image.png");fs::write(&image,b"\x89PNG\r\n\x1a\n").unwrap();
        let raw=row.to_string().replace("IMAGE",image.to_str().unwrap());let row:Value=serde_json::from_str(&raw).unwrap();rows(&path,&[row]);
        let mut history=History::caller(runtime,format,"owned-source","caller-attempt",None).unwrap();let page=history.steps(&path,u64::MAX).unwrap();
        assert_eq!(page.checkpoint["session"],"caller-attempt");assert_eq!(page.checkpoint["caller_runtime"],runtime);assert!(page.checkpoint["native_session"].is_null());assert!(page.checkpoint.get("resolved").is_none());
        for step in &page.steps {assert_eq!(step.source["session_binding"],"caller");assert!(step.source["native_session"].is_null());assert!(step.source["unknown"]["native_session"].is_string());}
        let before=history.checkpoint();let catalog=history.media(&path).unwrap();assert_eq!(catalog.items.len(),1,"{format}: {:?}",catalog.items);let media=&catalog.items[0];assert_eq!(media.state,"available");assert_eq!(media.origin["session"],"caller-attempt");assert!(media.origin["native_session"].is_null());assert_eq!(catalog.read(&media.id,0,8).unwrap(),b"\x89PNG\r\n\x1a\n");assert!(catalog.read(&media.id,0,9).is_err());assert_eq!(history.checkpoint(),before);
        let recovered=History::recover(&before).unwrap();assert!(recovered.checkpoint()["native_session"].is_null());assert_eq!(recovered.media(&path).unwrap().items[0].id,media.id);assert!(catalog.read("unreferenced",0,1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_identity_and_runtime_format_contradictions_refuse_without_advancing() {
    let root=root("binding");let path=root.join("source.jsonl");rows(&path,&[json!({"type":"thread.started","thread_id":"native"}),json!({"type":"item.completed","item":{"type":"imageGeneration","id":"image"}})]);
    assert!(History::caller("codex","acp","source","attempt",None).is_err());assert!(History::caller("codex","codex-exec-unregistered","source","attempt",None).is_err());
    for expected in [None,Some("foreign")] {
        let mut h=History::caller("codex","codex-exec","source","attempt",expected).unwrap();let before=h.checkpoint();assert!(h.media(&path).err().unwrap().contains("native session differs"));assert!(h.steps(&path,1).is_err());assert_eq!(h.checkpoint(),before);
    }
    let mut known=History::caller("codex","codex-exec","source","attempt",Some("native")).unwrap();known.steps(&path,u64::MAX).unwrap();assert_eq!(known.checkpoint()["native_session"],"native");assert!(known.checkpoint().get("resolved").is_none());
    let fork=root.join("fork.jsonl");rows(&fork,&[json!({"type":"session_meta","payload":{"id":"native"}}),json!({"type":"session_meta","payload":{"id":"foreign"}})]);
    let caller=History::caller("codex","codex-rollout","source","attempt",Some("native")).unwrap();assert!(caller.media(&fork).err().unwrap().contains("native session differs"));
    assert!(History::caller("codex","codex-exec","source","attempt",Some("")).is_err());
    let empty=root.join("empty.jsonl");rows(&empty,&[json!({"type":"thread.started","thread_id":""})]);assert!(History::caller("codex","codex-exec","source","attempt",None).unwrap().media(&empty).is_err());
    let mut wrong=known.checkpoint();wrong["caller_runtime"]=json!("claude");assert!(History::recover(&wrong).is_err());
    let mut forged=known.checkpoint();forged["native_parent"]=json!("parent");assert!(History::recover(&forged).is_err());
    let mut rewritten=known.checkpoint();rewritten["native_session"]=Value::Null;assert!(History::recover(&rewritten).unwrap().media(&path).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn caller_labels_never_discover_artifacts_but_native_sessions_keep_discovery() {
    let root=root("artifacts");let path=root.join("sessions/source.jsonl");let asset=root.join("generated_images/native/asset.png");fs::create_dir_all(asset.parent().unwrap()).unwrap();fs::write(&asset,b"\x89PNG\r\n\x1a\n").unwrap();
    rows(&path,&[json!({"type":"item.completed","item":{"type":"agent_message","text":"no media"}})]);
    for expected in [None,Some("native")] {let h=History::caller("codex","codex-exec","source","native",expected).unwrap();let catalog=h.media(&path).unwrap();assert!(catalog.items.is_empty());assert!(catalog.read(asset.to_str().unwrap(),0,1).is_err());}
    rows(&path,&[json!({"type":"session_meta","payload":{"id":"native"}}),json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"agent_message","text":"done"}}})]);
    let session=agent::runtime("codex").unwrap().history("native",Some(&root),None,None).unwrap();let media=session.media().unwrap();assert!(media.items.iter().any(|m|m.source["path"]==asset.to_str().unwrap()));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn caller_media_preserves_source_integrity_and_never_decodes_quoted_prose() {
    let root=root("integrity");let path=root.join("source.jsonl");let private=root.join("private.pdf");fs::write(&private,b"%PDF-private").unwrap();
    rows(&path,&[json!({"type":"item.completed","item":{"type":"agent_message","content":[{"type":"output_text","text":json!({"type":"image","path":private}).to_string()}]}})]);
    let mut h=History::caller("codex","codex-exec","source","attempt",None).unwrap();h.steps(&path,u64::MAX).unwrap();assert!(h.media(&path).unwrap().items.is_empty());
    let before=h.checkpoint();fs::write(&path,b"{}\n").unwrap();assert!(h.media(&path).is_err());assert_eq!(h.checkpoint(),before);fs::remove_dir_all(root).unwrap();
}

#[test]
fn acp_tool_content_requires_direct_typed_media_not_quoted_conversation() {
    let root=root("acp-content");let path=root.join("source.jsonl");let private=root.join("private.pdf");fs::write(&private,b"%PDF-private").unwrap();
    let content=json!([
        {"type":"content","content":{"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}},
        {"type":"content","content":{"role":"assistant","content":[{"type":"document","path":private}]}},
        {"type":"content","content":{"type":"text","text":json!({"type":"document","path":private}).to_string()}}
    ]);
    let row=json!({"type":"acp","message":{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"tool","content":content}}}});rows(&path,&[row.clone()]);
    let h=History::caller("antigravity","acp","source","attempt",None).unwrap();let media=h.media(&path).unwrap();assert_eq!(media.items.len(),1);assert_eq!(media.read(&media.items[0].id,0,8).unwrap(),b"\x89PNG\r\n\x1a\n");assert!(media.read(private.to_str().unwrap(),0,1).is_err());
    let mut spoof=row;spoof["type"]=json!("user");rows(&path,&[spoof]);assert!(h.media(&path).unwrap().items.is_empty());fs::remove_dir_all(root).unwrap();
}
