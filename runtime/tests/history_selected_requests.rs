//! S283 plus canonical285: selected native requests remain observation-only.
use agentq_runtime::agent::history::{History,CODEX_APP_SERVER_SELECTED};
use serde_json::json;
#[test]
fn selected_native_server_requests_keep_unknown_scope_and_exact_raw() {
    let path=std::env::temp_dir().join(format!("agentq-selected-request-{}.jsonl",std::process::id()));
    let row=json!({"id":7,"method":"future/request","params":{"threadId":"s","unknown":"future extension"}});
    let raw=format!("{row}\n");std::fs::write(&path,&raw).unwrap();
    let mut history=History::caller("codex",CODEX_APP_SERVER_SELECTED,"source","binding",Some("s")).unwrap();
    let page=history.steps(&path,u64::MAX).unwrap();let step=&page.steps[0];
    assert_eq!(step.state,"unknown");assert!(step.data["unknown"]["request_scope"].is_string());assert_eq!(step.data["native"],row);
    assert_eq!(step.source["format"],CODEX_APP_SERVER_SELECTED);assert_eq!(step.source["raw"],raw);assert!(step.id.ends_with(":request"));
    std::fs::remove_file(path).unwrap();
}
