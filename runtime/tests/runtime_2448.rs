use agentq_runtime::agent::{Window, runtime};
use serde_json::{Value, json};

#[test]
fn window_rejection_is_independent_of_exhaustion() {
    let rt = runtime("claude").unwrap();
    let log=json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day","utilization":0.2,"unifiedWindows":{"five_hour":{"utilization":1.0},"seven_day":{"utilization":0.2}}}}).to_string();
    let windows = rt.log_limits(&log, None, None);
    let values = serde_json::to_value(&windows).unwrap();
    assert_eq!(values[0]["window"], "five_hour");
    assert_eq!(values[0]["exhausted"], true);
    assert_eq!(
        values[0]["rejected"], false,
        "exhaustion does not identify the binding window"
    );
    assert_eq!(values[1]["rejected"], true);
    assert_eq!(rt.classify(&log).reached, Some(None));
    for (status, used, rejected, exhausted) in [
        (json!("rejected"), 0.2, json!(true), true),
        (json!("allowed"), 1.0, json!(false), true),
        (json!("allowed_warning"), 0.3, json!(false), false),
        (Value::Null, 1.0, Value::Null, true),
    ] {
        let log=json!({"type":"rate_limit_event","rate_limit_info":{"status":status,"rateLimitType":"five_hour","utilization":used}}).to_string();
        let w = serde_json::to_value(rt.log_limits(&log, None, None)).unwrap();
        assert_eq!(w[0]["rejected"], rejected);
        assert_eq!(w[0]["exhausted"], exhausted);
    }
    let old:Window=serde_json::from_value(json!({"window":"five_hour","used":1.0,"resets_at":null,"event_id":null,"exhausted":true,"unknown":null})).unwrap();
    assert_eq!(serde_json::to_value(old).unwrap()["rejected"], Value::Null);
    let codex = runtime("codex").unwrap().log_limits(
        &json!({"rate_limits":{"primary":{"used_percent":100,"window_minutes":300}}}).to_string(),
        None,
        None,
    );
    assert_eq!(
        serde_json::to_value(codex).unwrap()[0]["rejected"],
        Value::Null
    );
}

#[test]
fn terminal_native_detail_survives_classification() {
    let claude = runtime("claude").unwrap();
    for subtype in [
        "error_max_turns",
        "error_during_execution",
        "error_max_budget_usd",
        "success",
    ] {
        let r = claude.classify(
            &json!({"type":"result","subtype":subtype,"is_error":subtype!="success"}).to_string(),
        );
        assert_eq!(r.detail.as_deref(), Some(subtype));
    }
    let r = runtime("codex").unwrap().classify(
        &json!({"type":"turn.failed","error":{"message":"native diagnostic"}}).to_string(),
    );
    assert_eq!(
        (r.error, r.detail.as_deref()),
        ("turn-failed", Some("native diagnostic"))
    );
    let failed = format!(
        "{}\n{}",
        json!({"type":"assistant","error":"authentication_failed","message":{"content":[{"type":"text","text":"native auth reason"}]}}),
        json!({"type":"result","is_error":true,"subtype":"error_during_execution"})
    );
    let r = claude.classify(&failed);
    assert_eq!(
        (r.error, r.detail.as_deref()),
        ("auth-rejected", Some("error_during_execution"))
    );
    assert_eq!(claude.classify(r#"{"type":"result"}"#).detail, None);
    assert_eq!(
        runtime("antigravity")
            .unwrap()
            .classify(r#"{"event":"result","result":{"status":"ERROR"}}"#)
            .detail,
        None
    );
}
