//! Source-only regression batch: execution is deferred by owner order until global functional freeze.
use agentq_runtime::agent::{runtime,Cause};

#[test]
fn historical_background_receipts_are_only_unverified_observations() {
    let claude=runtime("claude").unwrap();
    for (case,stdout,stderr,token,cause) in [
        ("timeout","","",None,None),
        ("response","lost response","",None,None),
        ("exit-id","backgrounded · c37eb75a\n","remote control unavailable",Some("c37eb75a"),None),
        ("killed","","native launch failed",None,None),
        ("log","backgrounded · c37eb75a\n","",Some("c37eb75a"),None),
        ("copy","backgrounded · c37eb75a\n","",Some("c37eb75a"),None),
        ("no-start","","fixture missing launcher",None,None),
        ("refused","","Error: Background sessions are disabled.\nAsk your administrator.\n",None,Some(Cause("background-disabled"))),
        ("untrusted","","Error: Workspace not trusted. Run claude here first.",None,Some(Cause("workspace-untrusted"))),
        ("assistant-quote",r#"{"type":"assistant","text":"backgrounded · c37eb75a"}"#,r#"{"type":"result","result":"Workspace not trusted"}"#,None,None),
        ("contradictory","backgrounded · c37eb75a\nbackgrounded · another-job\n","",None,None),
        ("token-and-refusal","backgrounded · c37eb75a\n","Error: Workspace not trusted. Run claude here first.",Some("c37eb75a"),None),
        ("false-token","assistant · c37eb75a\n","",None,None),
    ] {
        let decoded=claude.background_launch_exchange(stdout,stderr);
        assert_eq!(decoded.source,"claude-background-launcher");
        assert_eq!(decoded.unverified_job.as_deref(),token,"{case}");assert_eq!(decoded.cause,cause,"{case}");
        assert!(decoded.unknown.is_some(),"{case}: no historical text proves acceptance or refusal stage");
        assert_eq!(decoded.detail.as_deref(),(!stderr.trim().is_empty()).then_some(stderr));
    }
    assert_eq!(runtime("codex").unwrap().background_launch_exchange("backgrounded · c37eb75a","").unknown,Some("background-unsupported"));
}

#[test]
fn plain_codex_cli_grammar_is_exact_but_log_only_never_stage_proof() {
    let rt=runtime("codex").unwrap();let thread="original-thread";
    let create=format!("2026-11-02T00:00:00Z ERROR codex_core::session: Failed to create session: thread-store conflict: thread {thread} already has an active writer\n");
    let persistence=format!("2026-11-02T00:00:00Z ERROR codex_core::session::session: failed to initialize thread persistence: thread-store conflict: thread {thread} already has an active writer\n");
    let final_line=format!("Error: thread/resume: thread/resume failed: thread {thread} already has an active writer (code -32600)\n");
    for text in [format!("{create}{final_line}"),format!("{create}{persistence}{final_line}")] {
        let report=rt.classify(&text);
        assert_eq!(report.reason.as_deref(),Some("native-cli-resume-refusal"));assert_eq!(report.session.as_deref(),Some(thread));
        assert_eq!(report.detail.as_deref(),Some(text.as_str()));assert!(report.start_failure.is_none());
    }
    for text in [final_line.clone(),format!("{persistence}{final_line}"),format!("{final_line}{create}"),format!("{create}{final_line}other\n"),format!("{create}{}",final_line.trim_end()),
        serde_json::json!({"type":"assistant","message":{"content":format!("{create}{final_line}")}}).to_string(),
        format!("{create}{final_line}{{\"type\":\"result\",\"result\":\"success\"}}\n"),
        format!("{create}{}",final_line.replace(thread,"foreign-thread"))] {
        let report=rt.classify(&text);assert_ne!(report.reason.as_deref(),Some("native-cli-resume-refusal"),"{text}");assert!(report.start_failure.is_none());
    }
}
