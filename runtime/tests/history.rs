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
            "codex-app-server" => "{\"method\":\"turn/completed\",\"params\":{\"turn\":{\"id\":\"turn\",\"status\":\"completed\"}}}\n",
            _ => "{\"type\":\"PLANNER_RESPONSE\",\"step_index\":1,\"content\":\"answer\",\"status\":\"DONE\"}\n",
        };
        let path = root.join(format);
        fs::write(&path, source).unwrap();
        let session = match format {
            "agy-stream" => "redacted-id-1", "codex-exec"=>"redacted-id-2",
            "claude-native" | "claude-stream"=>"redacted-id-9", _=>"session",
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
