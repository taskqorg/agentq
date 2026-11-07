//! Record-local TaskQ transcript parity: no media reads, native interpretation or UI policy in the consumer.
use agentq_runtime::agent::history::{History,Step};
use serde_json::{json,Value};
use std::fs;

fn observe(tag:&str,format:&str,rows:&[Value])->(Vec<Value>,Vec<Step>) {
    let path=std::env::temp_dir().join(format!("agentq-projection-{}-{tag}.jsonl",std::process::id()));
    fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let mut history=History::new(format,"source","fixture").unwrap();let mut events=vec![];
    loop {let page=history.poll(&path).unwrap();events.extend(page.events);if !page.more{break;}}
    let mut history=History::new(format,"source","fixture").unwrap();let mut backwards=vec![];
    loop {let page=history.poll_before(&path).unwrap();backwards.splice(0..0,page.events);if !page.more{break;}}
    assert_eq!(events,backwards,"typed output and exact physical provenance are direction independent");
    let steps=History::new(format,"source","fixture").unwrap().steps(&path,u64::MAX).unwrap().steps;
    fs::remove_file(path).unwrap();(events,steps)
}

#[test]
fn recorded_images_and_nested_result_parts_are_typed_without_reference_reads() {
    // Exact TaskQ recorded_images fixture; preview validation/limits stay in TaskQ.
    let png="iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=";
    let rows=[json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Review the image"},{"type":"input_image","image_url":format!("data:image/png;base64,{png}")} ]}})];
    let (events,steps)=observe("codex-images","codex-rollout",&rows);
    let image=&events[0]["blocks"][1]["image"];
    assert_eq!(image["data"],png);assert_eq!(image["encoding"],"base64");assert_eq!(image["mime"],"image/png");
    assert_eq!(steps[1].data["image"],*image);assert_eq!(steps[1].source["start"],events[0]["start"]);
    let rows=[json!({"type":"user","message":{"content":[{"type":"tool_result","content":[{"type":"text","text":"before"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":png}},{"type":"text","text":"after"}]},{"type":"image","source":{"type":"url","url":"file:///unrelated/private.png"}},{"type":"image","image_url":{"url":"https://example.invalid/private.png"},"alt":"Reported"},{"type":"image","source":{"type":"base64","data":"broken"}}]}})];
    let (events,steps)=observe("claude-images","claude-native",&rows);
    let blocks=&events[0]["blocks"];let parts=&blocks[0]["parts"];
    assert_eq!(parts[0]["text"],"before");assert_eq!(parts[1]["type"],"image");assert_eq!(parts[1]["image"]["data"],png);assert_eq!(parts[2]["text"],"after");
    assert_eq!(steps[0].data["parts"],*parts);
    assert_eq!(blocks[1]["image"]["url"],"file:///unrelated/private.png");assert!(blocks[1]["image"]["data"].is_null());assert!(blocks[1]["image"]["unknown"]["data"].is_string());
    assert_eq!(blocks[2]["image"]["name"],"Reported");assert_eq!(blocks[2]["image"]["url"],"https://example.invalid/private.png");assert!(blocks[2]["image"]["data"].is_null());
    assert_eq!(blocks[3]["image"]["data"],"broken","projection retains recorded bytes; consumer validates preview encoding");
    let referenced=std::env::temp_dir().join(format!("agentq-unrecorded-image-{}",std::process::id()));
    fs::write(&referenced,"UNRECORDED_PRIVATE_IMAGE_BYTES").unwrap();
    let (events,_)=observe("local-image","claude-native",&[json!({"type":"user","message":{"content":[{"type":"local_image","path":referenced}]}})]);
    assert!(events[0]["blocks"][0]["image"]["data"].is_null());assert!(!events[0].to_string().contains("UNRECORDED_PRIVATE_IMAGE_BYTES"));
    fs::remove_file(referenced).unwrap();
}

#[test]
fn antigravity_partial_final_and_owner_intent_never_claim_delivery() {
    let rows=[json!({"event":"init","conversation_id":"fixture"}),json!({"event":"user","message":{"content":"Owner input intent"}}),json!({"event":"step_update","step_update":{"conversation_id":"fixture","step_type":"user_input","state":"DONE"}}),json!({"event":"step_update","step_update":{"conversation_id":"fixture","step_type":"agent_response","text_delta":"partial answer"}}),json!({"event":"result","result":{"conversation_id":"fixture","status":"SUCCESS","response":"complete answer"}})];
    let (events,steps)=observe("agy","agy-stream",&rows);
    assert_eq!((events[1]["author"].as_str(),events[1]["kind"].as_str()),(Some("user"),Some("message")));
    assert_eq!(events[1]["blocks"][0]["text"],"Owner input intent");assert_eq!(events[1]["blocks"][0]["delivery"],"intent");assert_eq!(events[1]["state"],"unknown");
    assert_eq!(events[2]["state"],"completed");assert_eq!(events[3]["blocks"][0]["text_mode"],"delta");assert_eq!(events[4]["blocks"][0]["text_mode"],"complete");
    let final_text:Vec<_>=events.iter().filter(|e|e["author"]=="assistant").flat_map(|e|e["blocks"].as_array().unwrap()).filter(|b|b["text_mode"]=="complete").map(|b|b["text"].clone()).collect();
    assert_eq!(final_text,[json!("complete answer")]);
    assert!(steps.iter().any(|s|s.data["delivery"]=="intent" && s.state!="completed"));
    assert!(steps.iter().any(|s|s.data["text_mode"]=="delta" && s.data["text"]=="partial answer"));
    assert!(steps.iter().any(|s|s.data["text_mode"]=="complete" && s.data["text"]=="complete answer"));
    let malformed=[json!({"event":"user","message":{"content":42}}),json!({"event":"future","message":{"content":"not input"}})];
    assert!(observe("agy-unknown","agy-stream",&malformed).0.iter().all(|e|e["kind"]=="unknown" && e["state"]=="unknown"));
}

#[test]
fn codex_injected_context_is_adapter_scoped_and_markup_is_not_owner_proof() {
    let row=|text|json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}});
    let rows=[row("<environment_context>"),row("# AGENTS.md instructions for /fixture"),row("<strong>owner text</strong>"),row("ordinary owner text")];
    let (events,steps)=observe("origin","codex-rollout",&rows);
    assert_eq!(events[0]["blocks"][0]["origin"],"injected_context");assert_eq!(events[1]["blocks"][0]["origin"],"injected_context");
    for index in [2,3] {assert_eq!(events[index]["blocks"][0]["origin"],"native_user");}
    assert_eq!(steps[0].data["origin"],"injected_context");
    let (claude,_)=observe("origin-claude","claude-native",&[json!({"type":"user","message":{"content":"<environment_context>"}})]);
    assert_ne!(claude[0]["blocks"][0]["origin"],"injected_context");
}

#[test]
fn codex_custom_inputs_keep_taskq_command_summary_and_incomplete_string_tolerance() {
    let call=|name:&str,input:&str|json!({"type":"response_item","payload":{"type":"custom_tool_call","status":"completed","call_id":"c","name":name,"input":input}});
    let cases=[
        ("exec","text(await tools.exec_command({cmd:\"echo two\",max_output_tokens:2000}));\n","echo two"),
        ("exec","// @exec: {\"yield_time_ms\": 1000}\nconst r=await Promise.allSettled([\ntools.exec_command({cmd:'git log -1 --format=\\'%H\\'', workdir:w}),\ntools.exec_command({\"cmd\": `rg -n \"tools.exec_command(\" src`}),\ntools.write_stdin({session_id:7,chars:\"secret\\n\"})\n]); r.forEach((v,i)=>text({i,...v}));\n","git log -1 --format='%H' · rg -n \"tools.exec_command(\" src · write_stdin"),
        ("exec","text(await tools.apply_patch(\"*** Begin Patch\\n*** Update File: src/a.rs\\n@@\\n-x\\n+y\\n*** Add File: docs/b.md\\n+z\\n*** End Patch\\n\"));","apply_patch src/a.rs docs/b.md"),
        ("exec","const r=await tools.view_image({path:\"shots/a.png\"});image(r.image_url);","view_image shots/a.png"),
        ("exec","text(await tools.exec_command({cmd:\"GH_TOKEN=ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa gh api -H 'Authorization: Bearer q' x\"}));","GH_TOKEN=ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa gh api -H 'Authorization: Bearer q' x"),
        ("apply_patch","*** Begin Patch\n*** Delete File: old.txt\n*** End Patch\n","old.txt"),
        ("exec","let x = 1;","let x = 1;"),
        ("exec","text(await tools.exec_command({cmd:\"echo a","echo a"),
        ("exec","/* tools.exec_command({cmd:\"x\"}) 'it's */ text(await tools.exec_command({cmd:\"y\"}))","y"),
    ];
    let mut rows=cases.iter().map(|(name,input,_)|call(name,input)).collect::<Vec<_>>();
    rows.push(json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}"}}));
    let (events,steps)=observe("custom","codex-rollout",&rows);
    for (event,(_,input,summary)) in events.iter().zip(&cases) {let block=&event["blocks"][0];assert_eq!(block["input_kind"],"free_text");assert_eq!(block["input"],*input);assert_eq!(block["summary"],*summary);}
    let malformed=[json!({"type":"response_item","payload":{"type":"custom_tool_call","name":"exec","input":{"cmd":"not native free text"}}})];
    let malformed=observe("custom-malformed","codex-rollout",&malformed).0;
    assert_eq!(malformed[0]["blocks"][0]["input_kind"],"unknown");assert!(malformed[0]["blocks"][0]["summary"].is_null());
    assert_eq!(events.last().unwrap()["blocks"][0]["input_kind"],"structured");assert_eq!(events.last().unwrap()["blocks"][0]["input"]["cmd"],"ls");
    assert!(steps.iter().any(|s|s.data["summary"]=="y"),"reused native call ID retains the last physical revision, including its typed summary");
    let js="/* c */ const r=await tools.exec_command({cmd:'a \\'b\\' \\u00e9',\"workdir\": `w`}); // x\ntools.view_image({path:\"p\"})";
    let cuts=js.char_indices().map(|(cut,_)|call("exec",&js[..cut])).collect::<Vec<_>>();
    assert_eq!(observe("custom-cuts","codex-rollout",&cuts).0.len(),cuts.len());
}

#[test]
fn tool_result_parts_and_unknown_tool_name_stay_at_their_physical_record() {
    let rows=(0..150).flat_map(|n|[json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":format!("c{n}"),"arguments":"{\"cmd\":\"ls\"}"}}),json!({"type":"response_item","payload":{"type":"function_call_output","call_id":format!("c{n}"),"output":format!("out {n}")}})]).collect::<Vec<_>>();
    let (events,_)=observe("results","codex-rollout",&rows);
    for (n,pair) in events.chunks_exact(2).enumerate() {assert_eq!(pair[1]["blocks"][0]["parts"][0]["text"],format!("out {n}"));assert!(pair[1]["start"].as_u64().unwrap()>pair[0]["start"].as_u64().unwrap());}
    let missing=observe("result-unknown","codex-rollout",&[json!({"type":"response_item","payload":{"type":"function_call_output","output":42}})]).0;
    assert!(missing[0]["blocks"][0]["parts"].is_null());assert!(missing[0]["blocks"][0]["unknown"]["parts"].is_string());
    let rows=[json!({"event":"step_update","step_update":{"step_type":"tool","tool_info":{"name":"known_tool"}}})];
    let (events,_)=observe("unknown-tool","agy-stream",&rows);
    assert_eq!(events[0]["blocks"][0]["type"],"unknown");assert_eq!(events[0]["blocks"][0]["name"],"known_tool");
}

#[test]
fn runtime_resolved_session_keeps_physical_acp_paging_and_recovery_bound() {
    let path=std::env::temp_dir().join(format!("agentq-resolved-acp-{}.jsonl",std::process::id()));
    let rows=(0..260).map(|i|json!({"type":"acp","message":{"method":"session/update","params":{"sessionId":"fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":i.to_string()}}}}})).collect::<Vec<_>>();
    fs::write(&path,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();
    let runtime=agentq_runtime::agent::runtime("antigravity").unwrap();
    let mut session=runtime.history("fixture",None,Some(&path),None).unwrap();let mut forwards=vec![];
    loop {let page=session.poll().unwrap();forwards.extend(page.events);if !page.more{break;}}
    let mut session=runtime.history("fixture",None,Some(&path),None).unwrap();let last=session.poll_before().unwrap();
    assert!(last.more);let checkpoint=session.checkpoint();
    let mut session=runtime.history("fixture",None,Some(&path),Some(&checkpoint)).unwrap();let mut reverse=last.events;
    loop {let page=session.poll_before().unwrap();reverse.splice(0..0,page.events);if !page.more{break;}}
    assert_eq!(reverse,forwards);assert_eq!(reverse.len(),260);
    fs::write(&path,"{}\n").unwrap();assert!(runtime.history("fixture",None,Some(&path),Some(&checkpoint)).is_err());
    fs::remove_file(path).unwrap();
}
