//! M12.1/M12.3: observation must retain source content without replay or invented parity.
use agentq_runtime::agent::history::{History, capabilities};
use serde_json::{Value, json};
use std::{fs, io::Write};

#[test]
fn history_preserves_sources_and_recovers_without_replay() {
    let root = std::env::temp_dir().join(format!("agentq-history-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    for format in ["codex-exec", "codex-rollout", "codex-app-server", "claude-stream", "claude-native", "agy-stream", "antigravity-sidecar"] {
        let source = match format {
            "codex-exec" => include_str!("fixtures/history/codex-exec.jsonl"),
            "codex-rollout" => include_str!("fixtures/history/codex-rollout.jsonl"),
            "claude-stream" | "claude-native" => include_str!("fixtures/history/claude-native.jsonl"),
            "agy-stream" => include_str!("fixtures/history/agy-stream.jsonl"),
            "codex-app-server" => include_str!("fixtures/history/codex-app-server.jsonl"),
            _ => "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"content\":\"answer\",\"status\":\"DONE\"}\n",
        };
        let path = root.join(format);
        fs::write(&path, source).unwrap();
        let session = match format {
            "agy-stream" => "redacted-id-1", "codex-exec"=>"redacted-id-2",
            "claude-native" | "claude-stream"=>"redacted-id-9", "codex-app-server"=>"fixture-id-1", "codex-rollout"=>"redacted-id-18", _=>"session",
        };
        let mut h = History::new(format, "caller-source", session).unwrap();
        let p = h.poll(&path).unwrap();
        assert!(!p.events.is_empty());
        assert_eq!(p.events.iter().map(|v| v["raw"].as_str().unwrap()).collect::<String>(), source);
        let saved = h.checkpoint();
        let mut h = History::recover(&saved).unwrap();
        assert!(h.poll(&path).unwrap().events.is_empty());
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"event\":").unwrap();
        let partial=h.poll(&path).unwrap();
        assert!(partial.events.is_empty(), "partial source waits");
        assert!(partial.pending_bytes>0);
        assert!(partial.more, "partial write must not terminate incremental observation");
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"\"future\"}\n").unwrap();
        let next = h.poll(&path).unwrap();
        assert_eq!(next.events.len(), 1);
        assert_eq!(next.events[0]["author"], "unknown");
        let before = h.checkpoint();
        fs::write(&path, b"{}\n").unwrap();
        assert!(h.poll(&path).is_err());
        assert_eq!(h.checkpoint(), before, "failure must not consume source");
        let cap = capabilities(format, None, "macos").unwrap();
        assert_eq!(cap["native"]["status"], "untested");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn adapters_keep_native_blocks_and_explicit_limits() {
    for (format, source, author, kind) in [
        ("claude-stream", json!({"type":"assistant","uuid":"m","timestamp":"exact-time","message":{"content":[{"type":"text","text":"```rs\n<&>\n```"},{"type":"image","source":{"type":"base64","data":"invented"}}]}}), "assistant", "message"),
        ("claude-native", json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"result"}]}}), "user", "message"),
        ("codex-exec", json!({"type":"item.completed","item":{"id":"i","type":"command_execution","command":"echo x","aggregated_output":"x"}}), "tool", "tool"),
        ("codex-rollout", json!({"type":"response_item","timestamp":"exact-time","payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"invented://image"}]}}), "user", "message"),
        ("codex-app-server", json!({"method":"item/completed","params":{"item":{"id":"i","type":"agentMessage","text":"answer"}}}), "assistant", "message"),
        ("agy-stream", json!({"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","state":"ACTIVE","text_delta":"**exact**"}}), "assistant", "message"),
        ("antigravity-sidecar", json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","content":"partial","truncated_fields":["content"]}), "assistant", "message"),
    ] {
        let path = std::env::temp_dir().join(format!("agentq-history-{}-{format}.jsonl", std::process::id()));
        let raw = format!("{source}\n{source}\n");
        fs::write(&path, &raw).unwrap();
        let mut h = History::new(format, "source", "session").unwrap();
        let p = h.poll(&path).unwrap();
        assert_eq!(p.events.len(), 2, "physical duplicates retain order");
        assert_eq!(p.events[0]["author"], author);
        assert_eq!(p.events[0]["kind"], kind);
        assert_ne!(p.events[0]["id"], p.events[1]["id"]);
        assert!(p.events[0]["blocks"].as_array().is_some());
        assert_eq!(p.events[0]["raw"], format!("{source}\n"));
        match format {
            "claude-stream" => {
                assert_eq!(p.events[0]["time"], "exact-time");
                assert_eq!(p.events[0]["blocks"][0]["text"], "```rs\n<&>\n```");
                assert_eq!(p.events[0]["blocks"][1]["type"], "image");
                assert_eq!(p.events[0]["blocks"][1]["native"], source["message"]["content"][1]);
            }
            "claude-native" => assert_eq!(p.events[0]["blocks"][0]["type"], "tool_result"),
            "codex-rollout" => assert_eq!(p.events[0]["blocks"][0]["native"], source["payload"]["content"][0]),
            "codex-exec" => assert_eq!(p.events[0]["blocks"][0]["output"], "x"),
            "codex-app-server" => assert_eq!(p.events[0]["blocks"][0]["text"], "answer"),
            "agy-stream" => assert_eq!(p.events[0]["blocks"][0]["text"], "**exact**"),
            _ => {}
        }
        if format == "antigravity-sidecar" { assert_eq!(p.events[0]["limited"], "truncated-source"); }
        fs::remove_file(path).unwrap();
    }
    assert!(History::new("unknown", "s", "x").is_err());
    assert!(History::recover(&json!({})).is_err());
    let _: Value = capabilities("agy-stream", Some("1.4.12"), "macos").unwrap();
}

// M12.1: corrupt/changed history cannot silently advance; long histories stay bounded and ordered.
#[test]
fn history_bounds_and_corruption_do_not_lose_records() {
    let path=std::env::temp_dir().join(format!("agentq-history-bounds-{}.jsonl",std::process::id()));
    let row="{\"type\":\"future\"}\n";
    fs::write(&path,row.repeat(300)).unwrap();
    let mut h=History::new("codex-exec","source","session").unwrap();
    let first=h.poll(&path).unwrap();assert_eq!(first.events.len(),128);assert!(first.more);
    let saved=h.checkpoint();
    let mut recovered=History::recover(&saved).unwrap();
    let second=recovered.poll(&path).unwrap();assert_eq!(second.events.len(),128);assert!(second.more);
    assert!(second.events[0]["order"].as_u64().unwrap()>first.events[127]["order"].as_u64().unwrap());
    let third=recovered.poll(&path).unwrap();assert_eq!(third.events.len(),44);assert!(!third.more);
    let mut rewrite=fs::read(&path).unwrap();rewrite[2]=b'X';fs::write(&path,&rewrite).unwrap();
    assert!(recovered.poll(&path).is_err(),"consumed prefix rewrite");
    fs::rename(&path,path.with_extension("old")).unwrap();fs::write(&path,row.repeat(300)).unwrap();
    assert!(h.poll(&path).is_err(),"same bytes in a replacement file are not original source");
    for data in [b"not JSON\n".to_vec(),b"[]\n".to_vec(),vec![255,b'\n'],vec![b'x';1024*1024+1],b"{\"event\":\"init\",\"conversation_id\":\"foreign\"}\n".to_vec()] {
        fs::write(&path,data).unwrap();
        let mut h=History::new("agy-stream","s","x").unwrap();let before=h.checkpoint();
        assert!(h.poll(&path).is_err());assert_eq!(h.checkpoint(),before);
    }
    fs::remove_file(&path).unwrap();fs::remove_file(path.with_extension("old")).unwrap();
}

// A3/A4: unknown media and source errors remain exact, never converted into invented text.
#[test]
fn sidecar_media_errors_and_future_fields_remain_observable() {
    let path=std::env::temp_dir().join(format!("agentq-history-sidecar-{}.jsonl",std::process::id()));
    let rows=[json!({"type":"USER_INPUT","step_index":0,"content":"user **text**","attachments":[{"uri":"invented://file"}]}),
        json!({"type":"TOOL_RESPONSE","step_index":1,"status":"ERROR","content":"failed","tool_call_id":"tool-1"}),
        json!({"type":"FUTURE_MEDIA","step_index":2,"future":{"image":"invented://image"}})];
    fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let page=History::new("antigravity-sidecar","s","x").unwrap().poll(&path).unwrap();
    assert_eq!(page.events[0]["author"],"user");
    assert_eq!(page.events[0]["limited"],"attachments-raw-only");
    assert_eq!(page.events[0]["blocks"][0]["text"],"user **text**");
    assert_eq!(page.events[1]["blocks"][0]["type"],"tool_result");
    assert_eq!(page.events[1]["blocks"][0]["native"],rows[1]);
    assert_eq!(page.events[2]["author"],"unknown");
    assert_eq!(page.events[2]["limited"],"unsupported-source-shape");
    assert_eq!(serde_json::from_str::<Value>(page.events[2]["raw"].as_str().unwrap()).unwrap(),rows[2]);
    fs::remove_file(path).unwrap();
}

// A3/P2: final result text must be visible without losing the error/usage envelope.
#[test]
fn claude_result_exposes_text_and_lifecycle() {
    let v = observe("claude-stream", json!({"type":"result","result":"**answer**","is_error":true,"errors":["native error"]}));
    assert_eq!(v["kind"], "lifecycle");
    assert_eq!(v["blocks"][0]["type"], "text");
    assert_eq!(v["blocks"][0]["text"], "**answer**");
    assert_eq!(v["blocks"][1]["native"]["errors"][0], "native error");
}

// A3: a native item ID must not break call/result correlation.
#[test]
fn codex_tools_correlate_by_call_id() {
    for (call, output) in [("function_call", "function_call_output"), ("custom_tool_call", "custom_tool_call_output")] {
        let a = observe("codex-rollout", json!({"type":"response_item","payload":{"type":call,"id":"item-1","call_id":"call-1","name":"tool","arguments":"{}"}}));
        let b = observe("codex-rollout", json!({"type":"response_item","payload":{"type":output,"call_id":"call-1","output":"done"}}));
        assert_eq!(a["item_id"], "item-1");
        assert_eq!(a["blocks"][0]["id"], b["blocks"][0]["id"]);
        assert_eq!(a["blocks"][0]["id"], "call-1");
    }
}

// A3/A4/P2: unknown steps/media cannot claim complete normalization.
#[test]
fn agy_unknown_steps_keep_unknown_meaning() {
    let v = observe("agy-stream", json!({"event":"step_update","step_update":{"step_index":1,"step_type":"future_media","state":"DONE"}}));
    assert_eq!(v["author"], "unknown");
    assert_eq!(v["kind"], "unknown");
    assert_eq!(v["limited"], "unsupported-source-shape");
    for known in ["system_message", "checkpoint"] {
        let v = observe("agy-stream", json!({"event":"step_update","step_update":{"step_index":2,"step_type":known,"state":"DONE"}}));
        assert_eq!(v["kind"], "lifecycle");
        assert_eq!(v["limited"], "none");
    }
}

#[test]
fn claude_unknown_media_is_explicitly_limited() {
    for format in ["claude-stream", "claude-native"] {
        let source = json!({"type":"assistant","message":{"content":[{"type":"future_media","uri":"invented://media"}]}});
        let v = observe(format, source.clone());
        assert_eq!(v["blocks"][0]["type"], "unknown");
        assert_eq!(v["blocks"][0]["native"], source["message"]["content"][0]);
        assert_eq!(v["limited"], "unsupported-content-block");
    }
}

// A1/A4: readback cannot advertise more than the transport matrix.
#[test]
fn capability_readback_keeps_transport_limits() {
    for format in ["agy-stream", "antigravity-sidecar"] {
        let c = capabilities(format, Some("caller-observed"), "macos").unwrap();
        assert_eq!(c["features"]["history"]["status"], "limited");
        assert!(!c["features"]["history"]["reason"].as_str().unwrap().is_empty());
        assert_eq!(c["native"]["status"], "untested");
    }
}

fn observe(format: &str, source: Value) -> Value {
    let path = std::env::temp_dir().join(format!("agentq-history-probe-{}-{:?}.jsonl", std::process::id(), std::thread::current().id()));
    fs::write(&path, format!("{source}\n")).unwrap();
    let v = History::new(format, "s", "session").unwrap().poll(&path).unwrap().events.remove(0);
    fs::remove_file(path).unwrap();
    v
}

// A5: actual no-model control captures protect envelopes independently of fake turn tests.
#[test]
fn native_control_captures_keep_source_time_and_request_identity() {
    let rows: Vec<Value> = include_str!("fixtures/history/codex-app-server.jsonl").lines().map(|r|serde_json::from_str(r).unwrap()).collect();
    let source = rows.iter().find(|v|v["method"] == "thread/started").unwrap();
    let path = std::env::temp_dir().join(format!("agentq-app-native-{}.jsonl",std::process::id()));
    fs::write(&path, format!("{source}\n")).unwrap();
    let v = History::new("codex-app-server","app",source["params"]["thread"]["id"].as_str().unwrap()).unwrap().poll(&path).unwrap().events.remove(0);
    assert_eq!(v["kind"],"lifecycle");
    assert_eq!(v["session_binding"],"native");
    assert_eq!(v["time"],source["emittedAtMs"]);
    fs::remove_file(path).unwrap();
    for row in include_str!("fixtures/history/claude-control.jsonl").lines() {
        let source: Value = serde_json::from_str(row).unwrap();
        let v = observe("claude-stream",source.clone());
        assert_eq!(v["kind"],"lifecycle");
        assert_eq!(v["item_id"],source["response"]["request_id"]);
        assert_eq!(v["blocks"][0]["native"],source);
    }
}

// A5: retained Claude lifecycle and projected sidecar content are source evidence,
// not proof of missing original sidecar fields or native turn execution.
#[test]
fn retained_lifecycle_and_sidecar_projection_keep_native_meaning() {
    for row in include_str!("fixtures/history/claude-stream.jsonl").lines() {
        let source: Value = serde_json::from_str(row).unwrap();
        let session = source["sessionId"].as_str().or(source["session_id"].as_str()).unwrap();
        let path = std::env::temp_dir().join(format!("agentq-claude-native-{}.jsonl",std::process::id()));
        fs::write(&path,format!("{source}\n")).unwrap();
        let v = History::new("claude-stream","native",session).unwrap().poll(&path).unwrap().events.remove(0);
        assert_eq!(v["kind"],"lifecycle");
        if source["type"] == "result" {assert_eq!(v["blocks"][0]["text"],source["result"]);}
        fs::remove_file(path).unwrap();
    }
    for row in include_str!("fixtures/history/sidecar-projection.jsonl").lines() {
        let source: Value = serde_json::from_str(row).unwrap();
        let v = observe("antigravity-sidecar",source.clone());
        assert_eq!(v["item_id"],source["step_index"]);
        assert_eq!(serde_json::from_str::<Value>(v["raw"].as_str().unwrap()).unwrap(),source);
        if source["type"] == "PLANNER_RESPONSE" {
            assert_eq!(v["author"],"assistant");
            assert_eq!(v["blocks"][0]["text"],source["content"]);
            if source["tool_calls"].as_array().unwrap().is_empty() {
                assert_eq!(v["limited"],"none","null truncated_fields does not mark intact content truncated");
            } else {assert_eq!(v["limited"],"unsupported-content-block");}
        } else {
            assert_eq!(v["kind"],"unknown");
            assert_eq!(v["limited"],"unsupported-source-shape");
        }
    }
}

// A3/A5: the real projection uses null for intact fields; empty lists also omit nothing.
#[test]
fn sidecar_truncation_marker_distinguishes_missing_fields() {
    for kind in ["PLANNER_RESPONSE", "TOOL_RESPONSE"] {
        for (marker, expected) in [(Value::Null,"none"),(json!([]),"none"),(json!(["content"]),"truncated-source"),(json!(42),"unsupported-truncation-field")] {
            let v = observe("antigravity-sidecar",json!({"type":kind,"step_index":1,"content":"intact","truncated_fields":marker}));
            assert_eq!(v["limited"],expected);
        }
    }
}

// A3/A5: real rollout event_msg UserMessage uses a different case from app-server.
#[test]
fn codex_rollout_native_user_message_is_attributable() {
    let source: Value = include_str!("fixtures/history/codex-rollout.jsonl").lines()
        .map(|r|serde_json::from_str::<Value>(r).unwrap())
        .find(|v|v["payload"]["item"]["type"] == "UserMessage").unwrap();
    let path=std::env::temp_dir().join(format!("agentq-rollout-native-{}.jsonl",std::process::id()));
    fs::write(&path,format!("{source}\n")).unwrap();
    let session=source["payload"]["thread_id"].as_str().unwrap();
    let v=History::new("codex-rollout","native",session).unwrap().poll(&path).unwrap().events.remove(0);
    assert_eq!(v["author"],"user");
    assert_eq!(v["kind"],"message");
    assert_eq!(v["blocks"][0]["text"],source["payload"]["item"]["content"][0]["text"]);
    assert_eq!(v["session_binding"],"native");
    assert!(History::new("codex-rollout","native","foreign").unwrap().poll(&path).is_err());
    fs::remove_file(path).unwrap();
}
