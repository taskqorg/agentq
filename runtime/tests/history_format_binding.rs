//! M12.22: native protocols and history-only SourceSpecs bind public open/recovery.
use agentq_runtime::agent;
use serde_json::{json,Value};
use std::{fs,path::PathBuf};
fn root(tag:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-history-formats-{}-{tag}",std::process::id()));fs::create_dir_all(&p).unwrap();p}

#[test]
fn registered_source_formats_survive_recovery_and_refuse_foreign_formats() {
    let fixtures:Value=serde_json::from_str(include_str!("fixtures/history/agents.json")).unwrap();
    for (name,fixture) in fixtures.as_object().unwrap() {
        if fixture["rows"]["sqlite"]==true {continue;}
        let root=root(name);let file=root.join(fixture["path"].as_str().unwrap());fs::create_dir_all(file.parent().unwrap()).unwrap();
        let text=if fixture["jsonl"]==true {fixture["rows"].as_array().unwrap().iter().map(|r|format!("{r}\n")).collect::<String>()}else{fixture["rows"].to_string()};fs::write(&file,&text).unwrap();
        let rt=agent::runtime(name).unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();let source=&sources[0];
        let mut session=rt.history(&source.id,Some(&root),None,None).unwrap();let saved=session.steps(u64::MAX).unwrap().checkpoint;
        assert_eq!(saved["format"],source.format);assert!(rt.history(&source.id,None,None,Some(&saved)).unwrap().steps(1).unwrap().steps.is_empty());
        for foreign in ["acp","jsonl-rpc","history-unregistered",if source.format=="history-pi"{"history-hermes"}else{"history-pi"}] {
            if foreign==source.format || name=="antigravity" && foreign=="acp" {continue;}
            let mut forged=saved.clone();forged["format"]=json!(foreign);forged["resolved"]["source"]["format"]=json!(foreign);let before=forged.clone();
            let error=rt.history(&source.id,None,None,Some(&forged)).err().expect("foreign format admitted");assert_eq!(error,"history source format differs from runtime","{name}/{foreign}");assert_eq!(forged,before);
        }
        assert_eq!(fs::read_to_string(&file).unwrap(),text);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_runtime_cannot_select_a_foreign_transport() {
    for (runtime,row) in [
        ("codex",json!({"type":"acp","message":{"params":{"sessionId":"s"}}})),
        ("claude",json!({"type":"acp","message":{"params":{"sessionId":"s"}}})),
        ("codex",json!({"type":"response","command":"get_state","data":{"sessionFile":"s"}})),
        ("claude",json!({"type":"response","command":"get_state","data":{"sessionFile":"s"}})),
        ("antigravity",json!({"type":"response","command":"get_state","data":{"sessionFile":"s"}})),
    ] {
        let root=root(runtime);let file=root.join("foreign.jsonl");fs::write(&file,format!("{row}\n")).unwrap();
        assert_eq!(agent::runtime(runtime).unwrap().history("s",None,Some(&file),None).err().expect("foreign transport admitted"),"history source format differs from runtime");fs::remove_dir_all(root).unwrap();
    }
}
