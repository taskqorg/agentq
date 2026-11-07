//! M12.18: shared request content is attributable observation, never reply authority.
use agentq_runtime::agent::{Choice,Event,Question,RequestKind,history::{History,StepKind}};
use serde_json::{json,Value};
use std::{fs,io::Write,path::PathBuf};
fn path(name:&str)->PathBuf {std::env::temp_dir().join(format!("agentq-requests-{}-{name}.jsonl",std::process::id()))}
fn append(path:&std::path::Path,row:&Value) {writeln!(fs::OpenOptions::new().create(true).append(true).open(path).unwrap(),"{row}").unwrap();}
fn request(run:&str,generation:u64,state:&str)->Value {
    let event=Event::Request {id:json!("opaque"),kind:RequestKind::Question,prompt:Some("choose".into()),choices:vec![],questions:vec![Question {id:"q".into(),prompt:"Which?".into(),choices:vec![Choice {id:"a".into(),label:"A".into(),kind:"option".into()}],multiple:false,free_text:true}]};
    json!({"type":"agentq_control","session_id":"s","run_id":run,"generation":generation,"native_id":7,"turn_id":"t","event":event,"state":state,"native":{"id":7,"method":"native-request"}})
}
#[test]
fn request_revisions_are_authoritative_across_recovered_pages() {
    let p=path("revisions");fs::write(&p,"").unwrap();let mut h=History::new("codex-app-server","source","s").unwrap();
    let mut row=request("r",1,"pending");append(&p,&row);let first=h.steps(&p,u64::MAX).unwrap();let id=first.steps[0].id.clone();
    assert_eq!(first.steps[0].kind,StepKind::Question);assert_eq!(first.steps[0].state,"pending");assert_eq!(first.steps[0].data["event"]["Request"]["questions"][0]["id"],"q");
    let mut checkpoint=first.checkpoint;let mut revision=first.steps[0].revision;
    for (index,state) in ["reply-pending","reply-sent","unknown","cancelled"].into_iter().enumerate() {
        row["state"]=json!(state);row["event"]["Request"]["prompt"]=Value::Null;row["event"]["Request"]["questions"]=json!([]);row["native"]=Value::Null;row["turn_id"]=Value::Null;append(&p,&row);
        let mut recovered=History::recover(&checkpoint).unwrap();let page=recovered.steps(&p,1).unwrap();assert_eq!(page.steps.len(),1);let step=&page.steps[0];
        assert_eq!(step.id,id);assert_eq!(step.state,state);assert!(step.revision>revision);assert!(step.data["event"]["Request"]["prompt"].is_null());assert_eq!(step.data["event"]["Request"]["questions"],json!([]));assert!(step.data["native"].is_null());assert!(step.turn.native_id.is_none());assert_eq!(step.origins.len(),index+2);
        assert_eq!(recovered.read_bytes(&p,step.source["start"].as_u64().unwrap(),step.source["end"].as_u64().unwrap()-step.source["start"].as_u64().unwrap()).unwrap(),step.source["raw"].as_str().unwrap().as_bytes());
        checkpoint=page.checkpoint;revision=step.revision;
    }
    let fresh=History::new("codex-app-server","source","s").unwrap().steps(&p,u64::MAX).unwrap();assert_eq!(fresh.steps.len(),1);assert_eq!(fresh.steps[0].id,id);assert_eq!(fresh.steps[0].state,"cancelled");assert_eq!(fresh.steps[0].origins.len(),5);assert!(History::recover(&checkpoint).unwrap().steps(&p,1).unwrap().steps.is_empty());fs::remove_file(p).unwrap();
}
#[test]
fn reused_request_ids_are_distinct_for_runs_generations_and_unscoped_observations() {
    let p=path("reuse");fs::write(&p,"").unwrap();
    for (run,generation) in [("r1",1),("r1",2),("r2",1)] {
        let mut row=request(run,generation,"pending");if generation==2 {row["event"]["Request"]["kind"]=json!("Approval");row["event"]["Request"]["choices"]=json!([Choice {id:"deny".into(),label:"Deny".into(),kind:"reject".into()}]);row["event"]["Request"]["questions"]=json!([]);}append(&p,&row);
    }
    let mut unscoped=request("r",1,"pending");unscoped.as_object_mut().unwrap().remove("run_id");append(&p,&unscoped);append(&p,&unscoped);
    let page=History::new("claude-stream","source","s").unwrap().steps(&p,u64::MAX).unwrap();assert_eq!(page.steps.len(),5);
    let ids:std::collections::BTreeSet<_>=page.steps.iter().map(|s|&s.id).collect();assert_eq!(ids.len(),5);assert_eq!(page.steps[1].kind,StepKind::Approval);assert_eq!(page.steps[1].data["event"]["Request"]["choices"][0]["id"],"deny");
    for step in &page.steps[3..] {assert_eq!(step.state,"unknown");assert!(step.unknown.as_deref().is_some_and(|r|r.contains("scope")));assert!(step.data.get("run_id").is_none());}
    assert!(page.steps.iter().all(|s|s.data["native_id"]==7));fs::remove_file(p).unwrap();
}
#[test]
fn foreign_scope_refuses_transactionally_and_invalid_snapshots_stay_unknown() {
    let p=path("binding");let mut row=request("r",1,"pending");row["session_id"]=json!("foreign");fs::write(&p,format!("{row}\n")).unwrap();let mut h=History::new("claude-stream","source","s").unwrap();let before=h.checkpoint();assert_eq!(h.steps(&p,1).unwrap_err(),"history native session differs");assert_eq!(h.checkpoint(),before);
    row=request("r",1,"future-state");fs::write(&p,format!("{row}\n")).unwrap();let step=h.steps(&p,1).unwrap().steps.remove(0);assert_eq!(step.kind,StepKind::Unknown);assert_eq!(step.state,"unknown");assert!(step.unknown.is_some());assert_eq!(step.data["state"],"future-state");
    fs::write(&p,"").unwrap();let mut h=History::new("claude-stream","source","s").unwrap();append(&p,&request("r",1,"pending"));let prior=h.steps(&p,1).unwrap();let mut invalid=request("r",1,"pending");invalid["event"]["Request"]["questions"]=json!("malformed");append(&p,&invalid);let page=History::recover(&prior.checkpoint).unwrap().steps(&p,1).unwrap();assert_eq!(page.steps[0].id,prior.steps[0].id);assert_eq!(page.steps[0].kind,StepKind::Unknown);assert_eq!(page.steps[0].data["event"]["Request"]["questions"],"malformed");fs::remove_file(p).unwrap();
}
#[test]
fn control_origins_are_exact_without_inventing_native_physical_links() {
    let p=path("origins");let native=json!({"id":7,"method":"item/tool/requestUserInput","params":{"threadId":"s","turnId":"t","questions":[]}});let raw=format!("{native}\n");fs::write(&p,&raw).unwrap();let mut row=request("r",1,"pending");row["native"]=native;append(&p,&row);
    let mut h=History::new("codex-app-server","source","s").unwrap();let page=h.steps(&p,u64::MAX).unwrap();let step=page.steps.iter().find(|s|s.data["type"]=="agentq_control").unwrap();assert_eq!(step.origins.len(),1);assert_eq!(step.origins[0]["start"],raw.len());assert!(step.data["unknown"]["native_origin"].is_string());assert_eq!(h.read_bytes(&p,0,raw.len() as u64).unwrap(),raw.as_bytes());assert_eq!(step.data["native"]["id"],7);fs::remove_file(p).unwrap();
}
#[test]
fn native_only_controls_keep_occurrences_without_fabricating_scope() {
    for (format,row) in [("claude-stream",json!({"type":"control_request","request_id":"reused","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[]}}})),("acp",json!({"type":"acp","message":{"jsonrpc":"2.0","id":"reused","method":"session/request_permission","params":{"sessionId":"s","toolCall":{"title":"permission"},"options":[]}}}))] {
        let p=path(format);fs::write(&p,format!("{row}\n{row}\n")).unwrap();let page=History::new(format,"source","s").unwrap().steps(&p,u64::MAX).unwrap();assert_eq!(page.steps.len(),2);assert_ne!(page.steps[0].id,page.steps[1].id);for step in page.steps {assert_eq!(step.state,"unknown");assert!(step.unknown.as_deref().is_some_and(|r|r.contains("scope")));assert!(step.data["native"].is_object());}fs::remove_file(p).unwrap();
    }
}
#[test]
fn malformed_or_retired_scope_never_coerces_into_a_scoped_request() {
    let p=path("invalid-scope");fs::write(&p,"").unwrap();
    for generation in [json!("1"),json!(-1),Value::Null] {let mut row=request("r",1,"pending");row["generation"]=generation;append(&p,&row);}
    let page=History::new("acp","source","s").unwrap().steps(&p,u64::MAX).unwrap();assert_eq!(page.steps.len(),3);for step in page.steps {assert_eq!(step.state,"unknown");assert!(step.unknown.unwrap().contains("scope"));}fs::remove_file(p).unwrap();
}

#[test]
fn request_turn_snapshots_do_not_borrow_or_replace_native_message_context() {
    let p=path("turn-context");fs::write(&p,"").unwrap();append(&p,&json!({"type":"turn.started","turn_id":"ordinary"}));
    let mut row=request("r",1,"pending");append(&p,&row);row["turn_id"]=Value::Null;row["state"]=json!("reply-sent");append(&p,&row);
    append(&p,&json!({"type":"item.completed","item":{"id":"m","type":"agentMessage","text":"native text"}}));
    let page=History::new("codex-exec","source","s").unwrap().steps(&p,u64::MAX).unwrap();
    let request=page.steps.iter().find(|s|s.kind==StepKind::Question).unwrap();assert!(request.turn.native_id.is_none());assert!(request.data["unknown"]["turn_id"].is_string());
    let message=page.steps.iter().find(|s|s.kind==StepKind::Message).unwrap();assert_eq!(message.turn.native_id.as_deref(),Some("ordinary"));fs::remove_file(p).unwrap();
}

#[test]
fn public_runtime_opens_owned_control_logs_and_recovers_the_resolved_source() {
    for runtime in ["codex","claude","antigravity"] {
        let p=path(&format!("public-{runtime}"));fs::write(&p,"").unwrap();
        if runtime=="antigravity" {append(&p,&json!({"type":"acp","message":{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}}));}
        append(&p,&request("r",1,"pending"));let rt=agentq_runtime::agent::runtime(runtime).unwrap();
        let mut session=rt.history("s",None,Some(&p),None).unwrap();let page=session.steps(u64::MAX).unwrap();assert!(page.steps.iter().any(|s|s.kind==StepKind::Question));
        assert_eq!(page.checkpoint["format"],if runtime=="codex"{"codex-app-server"}else if runtime=="antigravity"{"acp"}else{"claude-native"});
        let mut recovered=rt.history("s",None,None,Some(&page.checkpoint)).unwrap();assert!(recovered.steps(1).unwrap().steps.is_empty());fs::remove_file(p).unwrap();
    }
}

#[test]
fn native_unknown_fields_never_become_runtime_annotations_or_panic() {
    use std::panic::{catch_unwind,AssertUnwindSafe};
    let mut panics=Vec::new();
    for (name,unknown) in [("string",json!("future native extension")),("number",json!(7)),("bool",json!(true)),("null",Value::Null),("array",json!(["extension"])),("object",json!({"request_scope":{"allow":true},"native-only":"retained"}))] {
        for format in ["acp","codex-app-server"] {
            let p=path(&format!("unknown-{format}-{name}"));
            let row=if format=="acp" {json!({"type":"acp","message":{"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"sessionId":"s","options":[],"unknown":unknown}}})}
                else{json!({"id":1,"method":"future/request","params":{"threadId":"s","unknown":unknown}})};
            let raw=format!("{row}\n");fs::write(&p,&raw).unwrap();let mut h=History::new(format,"source","s").unwrap();
            let projected=catch_unwind(AssertUnwindSafe(||h.steps(&p,u64::MAX)));
            match projected {
                Err(_)=>panics.push(format!("{format}/{name}")),
                Ok(result)=>{let page=result.unwrap();let step=&page.steps[0];assert_eq!(step.state,"unknown");assert_eq!(step.data["native"],row);assert_eq!(step.data["unknown"].as_object().map(|m|m.len()),Some(1));assert!(step.data["unknown"]["request_scope"].is_string());assert_eq!(step.source["raw"],raw);assert_eq!(step.origins.len(),1);assert_eq!(h.read_bytes(&p,0,raw.len() as u64).unwrap(),raw.as_bytes());}
            }
            fs::remove_file(p).unwrap();
        }
    }
    assert!(panics.is_empty(),"native unknown panics: {panics:?}");
}
#[test]
fn selected_runtime_refuses_foreign_formats_on_open_and_recovery() {
    let acp=json!({"type":"acp","message":{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}});
    let rpc=json!({"type":"response","command":"get_state","data":{"sessionFile":"s"}});
    let mut admitted=Vec::new();
    for runtime in ["codex","claude","antigravity"] {
        for (format,prefix) in [("acp",acp.clone()),("jsonl-rpc",rpc.clone())] {
            if runtime=="antigravity" && format=="acp" {continue;}
            let p=path(&format!("foreign-{runtime}-{format}"));fs::write(&p,format!("{prefix}\n{}\n",request("r",1,"pending"))).unwrap();
            let rt=agentq_runtime::agent::runtime(runtime).unwrap();if rt.history("s",None,Some(&p),None).is_ok(){admitted.push(format!("{runtime}/{format}"));}
            fs::remove_file(p).unwrap();
        }
    }
    // This is the resolved checkpoint an earlier Codex open incorrectly admitted.
    let p=path("foreign-recovered");fs::write(&p,format!("{acp}\n{}\n",request("r",1,"pending"))).unwrap();
    let native=agentq_runtime::agent::runtime("antigravity").unwrap();let mut allowed=native.history("s",None,Some(&p),None).unwrap();let page=allowed.steps(u64::MAX).unwrap();assert_eq!(page.checkpoint["format"],"acp");
    let mut old=page.checkpoint.clone();old["resolved"]["runtime"]=json!("codex");let codex=agentq_runtime::agent::runtime("codex").unwrap();if codex.history("s",None,None,Some(&old)).is_ok(){admitted.push("codex/recovery-acp".into());}
    assert!(native.history("s",None,None,Some(&page.checkpoint)).unwrap().steps(1).unwrap().steps.is_empty());fs::remove_file(p).unwrap();
    assert!(admitted.is_empty(),"foreign formats admitted: {admitted:?}");
}
