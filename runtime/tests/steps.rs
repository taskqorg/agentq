//! M12.18/M12.19: fixtures are scrubbed real transcripts (fixtures/steps/provenance.json). Each test names the
//! mapping it guards; before this task every record below read as `unknown` or was not read at all.
use agentq_runtime::agent::{self, Failure, history::{History, capabilities}, steps::{Body, Output, Shell, Status, Step, Timeline}};
use serde_json::{Value, json};
use std::{fs, path::{Path, PathBuf}, sync::atomic::{AtomicUsize, Ordering}};

macro_rules! fixture { ($name:literal) => { include_str!(concat!("fixtures/steps/", $name, ".jsonl")) } }

const ROLLOUT: &str = fixture!("codex-0.162.1-rollout");
const COMPACTION: &str = fixture!("codex-0.162.1-compaction");
const SUBAGENT: &str = fixture!("codex-0.162.1-subagent");
const PARENT: &str = fixture!("codex-0.162.1-parent");
const FORK: &str = fixture!("codex-0.162.1-fork");
const APP: &str = fixture!("codex-0.162.1-app-server");
const CLAUDE: &str = fixture!("claude-2.1.318");
const CLAUDE_OLD: &str = fixture!("claude-2.1.295");
const CLAUDE_COMPACT: &str = fixture!("claude-2.1.318-compaction");
const CLAUDE_AGENT: &str = fixture!("claude-2.1.318-agent");
const CLAUDE_CHILD: &str = fixture!("claude-2.1.318-subagent");

fn timeline(runtime: &str, log: &str) -> Vec<Step> {
    let mut timeline = Timeline::default();
    for step in agent::runtime(runtime).unwrap().log_steps(log) { timeline.apply(step); }
    timeline.steps().into_iter().cloned().collect()
}
fn kind(step: &Step) -> String { serde_json::to_value(step).unwrap()["kind"].as_str().unwrap().into() }
fn of<'a>(steps: &'a [Step], name: &str) -> Vec<&'a Step> { steps.iter().filter(|s| kind(s) == name).collect() }
fn shells(steps: &[Step]) -> Vec<(&Step, &Shell)> { steps.iter().filter_map(|s| match &s.body { Body::Shell(shell) => Some((s, shell)), _ => None }).collect() }
fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("agentq-steps-{}-{}-{name}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn rollout_home(files: &[(&str, &str)]) -> PathBuf {
    let home = temp("codex");
    let dir = home.join("sessions/2000/01/01");
    fs::create_dir_all(&dir).unwrap();
    for (id, text) in files { fs::write(dir.join(format!("rollout-2000-01-01T00-00-00-{id}.jsonl")), text).unwrap(); }
    home
}
fn session_of(log: &str) -> String {
    log.lines().find_map(|l| { let v: Value = serde_json::from_str(l).ok()?; (v["type"] == "session_meta").then(|| v["payload"]["id"].as_str().map(String::from)).flatten() }).unwrap()
}
fn read_all(runtime: &str, session: &str, home: &Path) -> (Vec<Step>, agent::steps::Session) {
    let runtime = agent::runtime(runtime).unwrap();
    let (mut saved, mut timeline, mut pages) = (None, Timeline::default(), 0);
    loop {
        let page = runtime.history(session, Some(home), saved.as_ref(), None).unwrap();
        for step in page.steps { timeline.apply(step); }
        pages += 1;
        assert!(pages < 100);
        saved = Some(page.checkpoint);
        if !page.more { return (timeline.steps().into_iter().cloned().collect(), page.session); }
    }
}

// A1: an unmapped record keeps its bytes and never reads as a guessed kind.
#[test]
fn a1_unknown_keeps_raw_bytes() {
    let future = r#"{"type":"event_msg","payload":{"type":"future_kind","x":"  spaced  "}}"#;
    let item = r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FutureItem","id":"f"}}}"#;
    let claude = r#"{"type":"future-record","uuid":"u","x":1}"#;
    let log = format!("{future}\n{item}\n");
    let steps = timeline("codex", &log);
    assert_eq!(steps.len(), 2);
    assert!(matches!(&steps[0].body, Body::Unknown { record, raw } if record == "event_msg:future_kind" && raw.trim_end() == future), "{:?}", steps[0]);
    assert!(matches!(&steps[1].body, Body::Unknown { raw, .. } if raw.trim_end() == item));
    let steps = timeline("claude", &format!("{claude}\n"));
    assert!(matches!(&steps[0].body, Body::Unknown { raw, .. } if raw.trim_end() == claude));
    // Every kind name of the contract serializes as written.
    for (body, name) in [
        (Body::Message { role: None, text: None }, "message"), (Body::Thinking { text: None, encrypted: None }, "thinking"), (Body::Shell(Shell::default()), "shell"),
        (Body::FileChange { files: vec![], output: None, stderr: None }, "file_change"), (Body::Tool { name: None, input: None, output: None, duration_ms: None }, "tool"),
        (Body::Mcp { server: None, tool: None, input: None, output: None, duration_ms: None }, "mcp"), (Body::Web { action: None, query: None, url: None, output: None, duration_ms: None }, "web"),
        (Body::Subagent { action: None, agent: None, sessions: vec![], input: None, output: None }, "subagent"), (Body::Image { path: None, media_type: None }, "image"),
        (Body::Question { prompt: None, answer: None }, "question"), (Body::Approval { request: None, decision: None }, "approval"),
        (Body::Compaction { trigger: None, tokens_before: None, tokens_after: None }, "compaction"),
        (Body::Usage { input: None, cache_read: None, cache_write: None, output: None, reasoning: None, total: None, context_window: None, model: None }, "usage"),
        (Body::Unknown { record: "r".into(), raw: "{}".into() }, "unknown"),
    ] {
        let step = Step { id: "i".into(), body, order: 0, revision: 0, turn: None, parent: None, status: Status::Unknown, started_ms: None, ended_ms: None, source: None, unknown: Default::default() };
        let value = serde_json::to_value(&step).unwrap();
        assert_eq!(value["kind"], name);
        assert_eq!(serde_json::from_value::<Step>(value).unwrap(), step, "a step survives JSON, so a later archive can hold it");
    }
}

// A2 + A3: Codex item_completed CommandExecution read as unknown; its console was unreachable.
#[test]
fn a3_codex_command_execution_is_a_shell_step_with_its_console() {
    let steps = timeline("codex", ROLLOUT);
    let (step, shell) = shells(&steps).into_iter().find(|(_, s)| s.exit_code.is_some()).expect("real command with exit code");
    assert!(shell.command.is_some() && shell.argv.is_some());
    assert!(shell.cwd.as_deref().is_some_and(|c| !c.starts_with("file:")), "a file URL reads as its path: {:?}", shell.cwd);
    assert!(shell.duration_ms.is_some());
    let output = shell.output.as_ref().expect("output");
    assert_eq!(output.source.as_ref().unwrap().pointer.as_deref(), Some("/payload/item/aggregated_output"));
    assert_eq!(step.unknown.get("stdout").map(String::as_str), Some("merged-output"), "Codex merges streams; null names why");
    assert!(step.status != Status::Running && step.started_ms.is_some() && step.ended_ms.is_some());
}

// A3: Codex Reasoning and its response item were two unknown records; they are one step.
#[test]
fn a3_codex_reasoning_converges_to_one_thinking_step() {
    let steps = timeline("codex", ROLLOUT);
    let thinking = of(&steps, "thinking");
    assert!(!thinking.is_empty());
    let ids: std::collections::BTreeSet<_> = thinking.iter().map(|s| &s.id).collect();
    assert_eq!(ids.len(), thinking.len(), "one step per native id");
    assert!(thinking.iter().all(|s| matches!(&s.body, Body::Thinking { text: None, .. }) == s.unknown.contains_key("text")), "null text names its reason");
}

// A3: Codex FileChange read as unknown; paths and diffs were unreachable.
#[test]
fn a3_codex_file_change_lists_files_and_diffs() {
    let steps = timeline("codex", ROLLOUT);
    let change = of(&steps, "file_change");
    let Body::FileChange { files, .. } = &change.first().expect("real file change").body else { unreachable!() };
    assert!(!files.is_empty() && files[0].action.is_some());
    let diff = files.iter().find_map(|f| f.diff.as_ref()).expect("diff");
    let pointer = diff.source.as_ref().unwrap().pointer.as_deref().unwrap();
    assert!(pointer.ends_with("/unified_diff") || pointer.ends_with("/content"), "{pointer}");
}

// A3: Codex McpToolCall read as unknown.
#[test]
fn a3_codex_mcp_call_keeps_server_tool_and_result() {
    let steps = timeline("codex", ROLLOUT);
    let Body::Mcp { server, tool, input, output, duration_ms } = &of(&steps, "mcp").first().expect("real mcp call").body else { unreachable!() };
    assert!(server.is_some() && tool.is_some() && input.is_some() && output.is_some() && duration_ms.is_some());
}

// A3: ContextCompaction and the compacted record were two records of one event.
#[test]
fn a3_codex_compaction_is_one_step() {
    let steps = timeline("codex", COMPACTION);
    let compaction = of(&steps, "compaction");
    let natives = COMPACTION.matches("\"ContextCompaction\"").count();
    assert!(natives > 0);
    assert_eq!(compaction.len(), natives, "the compacted record joins its item");
}

// A3: user text appears once although the rollout holds it twice, and injected text stays visible.
#[test]
fn a3_codex_messages_do_not_duplicate() {
    let steps = timeline("codex", ROLLOUT);
    let messages = of(&steps, "message");
    assert!(messages.iter().any(|s| matches!(&s.body, Body::Message { role: Some(r), .. } if r == "assistant")));
    assert!(messages.iter().any(|s| matches!(&s.body, Body::Message { role: Some(r), .. } if r == "developer")), "injected text is not dropped");
    let log = [
        json!({"type":"response_item","payload":{"type":"message","id":"msg_u","role":"user","content":[{"type":"input_text","text":"hello"}]}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":"t","turn_id":"x","item":{"type":"UserMessage","id":"other","content":[{"type":"text","text":"hello"}]}}}),
        json!({"type":"response_item","payload":{"type":"message","id":"msg_a","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":"t","turn_id":"x","item":{"type":"AgentMessage","id":"msg_a","content":[{"type":"Text","text":"answer"}]}}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let steps = timeline("codex", &log);
    assert_eq!(steps.len(), 2, "{steps:?}");
}

// A3 + A2: Claude toolUseResult stdout and stderr were only native blocks.
#[test]
fn a3_claude_bash_keeps_stdout_and_stderr_apart() {
    for log in [CLAUDE, CLAUDE_OLD] {
        let steps = timeline("claude", log);
        let (step, shell) = shells(&steps).into_iter().find(|(s, sh)| sh.stdout.is_some() && sh.origin.is_none() && s.status == Status::Completed).expect("real Bash result");
        assert!(shell.command.is_some() && shell.cwd.is_some());
        assert_eq!(shell.stdout.as_ref().unwrap().source.as_ref().unwrap().pointer.as_deref(), Some("/toolUseResult/stdout"));
        assert_eq!(shell.stderr.as_ref().unwrap().source.as_ref().unwrap().pointer.as_deref(), Some("/toolUseResult/stderr"));
        assert_eq!(shell.exit_code, None, "success reports no code and never reads 0");
        assert_eq!(step.unknown.get("exit_code").map(String::as_str), Some("not-reported"));
        assert!(shell.duration_ms.is_some(), "from the record times");
    }
}

// A2: a failed Claude call reports its code only in text.
#[test]
fn a2_claude_failed_call_reads_its_exit_code() {
    let steps = timeline("claude", CLAUDE);
    let failed: Vec<_> = shells(&steps).into_iter().filter(|(s, sh)| s.status == Status::Failed && sh.origin.is_none()).collect();
    assert!(failed.iter().any(|(_, sh)| sh.exit_code.is_some()), "real `Exit code <n>` result");
}

// A3: Claude thinking blocks were unsupported content.
#[test]
fn a3_claude_thinking_is_a_step() {
    let steps = timeline("claude", CLAUDE);
    let thinking = of(&steps, "thinking");
    assert!(!thinking.is_empty());
    assert!(thinking.iter().all(|s| matches!(&s.body, Body::Thinking { text: Some(_), .. }) || s.unknown["text"] == "not-retained"), "empty native text names its reason");
}

// A3: Claude attachments read as raw blocks; hook consoles and external edits were unreachable.
#[test]
fn a3_claude_useful_attachments_map() {
    let steps = timeline("claude", CLAUDE);
    let hook = shells(&steps).into_iter().find(|(_, s)| s.origin.as_deref() == Some("hook")).expect("real hook");
    assert!(hook.1.exit_code.is_some() && hook.1.duration_ms.is_some() && hook.1.stdout.is_some() && hook.1.stderr.is_some());
    let log = format!("{}\n", json!({"type":"attachment","uuid":"e","attachment":{"type":"edited_text_file","filename":"a.txt","snippet":"1\tx"}}));
    assert!(matches!(&timeline("claude", &log)[0].body, Body::FileChange { files, .. } if files[0].action.as_deref() == Some("external")));
    // Context attachments add no step, and a new attachment type is never dropped.
    let log = format!("{}\n{}\n", json!({"type":"attachment","uuid":"d","attachment":{"type":"date"}}), json!({"type":"attachment","uuid":"n","attachment":{"type":"future"}}));
    let steps = timeline("claude", &log);
    assert_eq!(steps.len(), 1);
    assert!(matches!(&steps[0].body, Body::Unknown { record, .. } if record == "attachment:future"));
}

// A3: edits carry their structured patch.
#[test]
fn a3_claude_edit_is_a_file_change() {
    let steps = timeline("claude", CLAUDE);
    assert!(of(&steps, "file_change").iter().any(|s| matches!(&s.body, Body::FileChange { files, .. } if files.iter().any(|f| f.diff.as_ref().is_some_and(|d| d.source.as_ref().unwrap().pointer.as_deref() == Some("/toolUseResult/structuredPatch"))))));
}

// A3: Claude compaction boundary and usage.
#[test]
fn a3_claude_compaction_and_usage_map() {
    let steps = timeline("claude", CLAUDE_COMPACT);
    assert!(matches!(&of(&steps, "compaction")[0].body, Body::Compaction { trigger: Some(_), tokens_before: Some(_), tokens_after: Some(_) }));
    let usage = of(&steps, "usage");
    assert!(matches!(&usage[0].body, Body::Usage { input: Some(_), output: Some(_), model: Some(_), .. }));
    assert_eq!(usage[0].unknown.get("reasoning").map(String::as_str), Some("not-reported"));
}

// A4: a background job revises its starting step until its native notification, in every source order.
#[test]
fn a4_background_work_stays_one_step_and_links_by_id() {
    let steps = timeline("claude", CLAUDE);
    let job = shells(&steps).into_iter().find(|(_, s)| s.background.as_ref().is_some_and(|b| b.task.is_some())).expect("real background job");
    assert!(job.0.id.starts_with("toolu_"), "the job's step is the call that started it");
    let notified = shells(&steps).into_iter().filter(|(s, sh)| sh.background.is_some() && s.status.ne(&Status::Running)).count();
    assert!(notified > 0, "a real notification closes a job");
    let start = json!({"type":"assistant","uuid":"a","timestamp":"2000-01-01T00:00:00.000Z","message":{"id":"m","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"sleep","run_in_background":true}}]}});
    let result = json!({"type":"user","uuid":"b","timestamp":"2000-01-01T00:00:01.000Z","toolUseResult":{"stdout":"","stderr":"","backgroundTaskId":"bg"},"message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"with ID: bg"}]}});
    let note = |status: &str, summary: &str| json!({"type":"queue-operation","operation":"enqueue","timestamp":"2000-01-01T00:00:09.000Z","content":format!("<task-notification>\n<task-id>bg</task-id>\n<tool-use-id>toolu_1</tool-use-id>\n<output-file>o</output-file>\n<status>{status}</status>\n<summary>{summary}</summary>\n</task-notification>")});
    let log = format!("{start}\n{result}\n");
    let running = timeline("claude", &log);
    assert_eq!((running.len(), running[0].status), (1, Status::Running), "{running:?}");
    let finished = timeline("claude", &format!("{log}{}\n{}\n", note("completed", "done (exit code 0)"), json!({"type":"attachment","uuid":"q","attachment":{"type":"queued_command","prompt":note("completed", "done (exit code 0)")["content"]}})));
    let shell = shells(&finished);
    assert_eq!(shell.len(), 1, "three native notifications revise one step");
    assert_eq!((shell[0].0.status, shell[0].1.exit_code), (Status::Completed, Some(0)));
    assert_eq!(shell[0].1.background.as_ref().unwrap().output_file.as_deref(), Some("o"));
    assert_eq!((shell[0].1.duration_ms, shell[0].0.started_ms.is_some()), (Some(9000), true), "a job's duration ends at its notification");
    let late = agent::runtime("claude").unwrap().log_steps(&format!("{}\n", note("completed", "done")));
    let mut merged = Timeline::default();
    for step in agent::runtime("claude").unwrap().log_steps(&log) { merged.apply(step); }
    let start = merged.steps()[0].started_ms;
    for step in late { merged.apply(step); }
    assert_eq!(merged.steps()[0].started_ms, start, "a late revision keeps the real start");
    assert!(merged.steps()[0].unknown.values().all(|why| why != "bg"), "the unknown map holds reasons only");
}

// A4: Codex subagent activity revises the spawning call and names its child; the child names its parent.
#[test]
fn a4_subagent_links_parent_and_child() {
    let steps = timeline("codex", PARENT);
    let subagent = of(&steps, "subagent");
    let linked = subagent.iter().find_map(|s| match &s.body { Body::Subagent { sessions, .. } if !sessions.is_empty() => Some(sessions), _ => None }).expect("real child thread");
    assert!(!linked.is_empty());
    assert!(!of(&steps, "tool").iter().any(|s| subagent.iter().any(|a| a.id == s.id)));
    let child = session_of(SUBAGENT);
    let home = rollout_home(&[(&child, SUBAGENT)]);
    let (steps, session) = read_all("codex", &child, &home);
    assert!(!steps.is_empty());
    assert_eq!(session.id.as_deref(), Some(child.as_str()));
    assert!(session.parent.is_some() && session.root.is_some());
    assert_eq!(session.parent, session.root);
    assert_eq!(session.relation.as_deref(), Some("subagent"));
    assert_eq!(session.unknown.get("parent_step").map(String::as_str), Some("not-reported"));
    fs::remove_dir_all(home).unwrap();
    // Claude: the parent step names the child file, whose records carry the parent's session.
    let steps = timeline("claude", CLAUDE_AGENT);
    let agent = of(&steps, "subagent");
    assert!(agent.iter().any(|s| matches!(&s.body, Body::Subagent { sessions, output, .. } if sessions.iter().all(|c| c.starts_with("agent-")) && !sessions.is_empty() && output.is_some())));
    let child: Value = serde_json::from_str(CLAUDE_CHILD.lines().next().unwrap()).unwrap();
    let id = format!("agent-{}", child["agentId"].as_str().unwrap());
    let home = temp("claude");
    let dir = home.join("projects/p").join(child["sessionId"].as_str().unwrap()).join("subagents");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{id}.jsonl")), CLAUDE_CHILD).unwrap();
    let (steps, session) = read_all("claude", &id, &home);
    assert!(!steps.is_empty());
    assert_eq!((session.id.as_deref(), session.parent.as_deref(), session.relation.as_deref()), (Some(id.as_str()), child["sessionId"].as_str(), Some("subagent")));
    let refused = agent::runtime("claude").unwrap().resume_command(&id, Some(&home));
    assert!(matches!(refused, Err(Failure::Refused(c)) if c.0 == "resume-unsupported"), "the native client resumes only the parent");
    fs::remove_dir_all(home).unwrap();
}

// A4: repeated delivery, partial revisions and any order give one identical step.
#[test]
fn a4_repeated_delivery_converges() {
    for (runtime, log) in [("codex", ROLLOUT), ("codex", APP), ("claude", CLAUDE)] {
        let revisions = agent::runtime(runtime).unwrap().log_steps(log);
        let once = |steps: &[Step]| { let mut t = Timeline::default(); for s in steps { t.apply(s.clone()); } t.steps().into_iter().cloned().collect::<Vec<_>>() };
        let expected = once(&revisions);
        let twice: Vec<Step> = revisions.iter().chain(revisions.iter()).cloned().collect();
        assert_eq!(once(&twice), expected, "repeat");
        let reversed: Vec<Step> = revisions.iter().rev().cloned().collect();
        assert_eq!(once(&reversed), expected, "any order");
        // A second full read of the same source never adds a step.
        let mut ids: Vec<_> = expected.iter().map(|s| s.id.clone()).collect();
        ids.dedup();
        assert_eq!(ids.len(), expected.len());
    }
    // A partial result never erases a known command: null does not replace.
    let call = json!({"type":"assistant","uuid":"a","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}]}});
    let result = json!({"type":"user","uuid":"b","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"out"}]}});
    let runtime = agent::runtime("claude").unwrap();
    let mut timeline = Timeline::default();
    for step in runtime.log_steps(&format!("{call}\n")) { timeline.apply(step); }
    // The result arrives through a reader that has not seen the call.
    for step in runtime.log_steps(&format!("{}\n{result}\n", " ".repeat(200))) { timeline.apply(step); }
    let steps = timeline.steps();
    assert!(steps[0].unknown.keys().all(|field| field != "name" && field != "input"), "a generic revision leaves no reason for a field of another kind: {:?}", steps[0].unknown);
    let Body::Shell(shell) = &steps[0].body else { panic!("a generic result never replaces the shell kind: {:?}", steps[0]) };
    assert_eq!((shell.command.as_deref(), shell.output.as_ref().map(|o| o.text.as_str()), steps[0].status), (Some("ls"), Some("out"), Status::Completed));
}

// A2: output is bounded inline, and the byte range returns the whole value.
#[test]
fn a2_output_is_bounded_and_reads_by_range() {
    let text = "é".repeat(agent::steps::INLINE);
    let session = "11111111-1111-1111-1111-111111111111";
    let meta = json!({"type":"session_meta","payload":{"id":session,"session_id":session,"cwd":"file:///tmp/a%20b","cli_version":"0.162.1"}});
    let row = json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":session,"turn_id":"t","item":{"type":"CommandExecution","id":"c","command":["sh","-c","run"],"cwd":"file:///tmp/a%20b","aggregated_output":text,"exit_code":3,"status":"failed","duration":{"secs":2,"nanos":500000000}},"started_at_ms":1,"completed_at_ms":2}});
    let home = rollout_home(&[(session, &format!("{meta}\n{row}\n"))]);
    let codex = agent::runtime("codex").unwrap();
    let page = codex.history(session, Some(&home), None, None).unwrap();
    assert_eq!(page.session.cwd.as_deref(), Some("/tmp/a b"));
    let Body::Shell(shell) = &page.steps[0].body else { panic!() };
    assert_eq!((shell.command.as_deref(), shell.exit_code, shell.duration_ms, shell.cwd.as_deref()), (Some("run"), Some(3), Some(2500), Some("/tmp/a b")));
    let Output { text: inline, truncated, bytes, source } = shell.output.clone().unwrap();
    assert!(truncated && inline.len() <= agent::steps::INLINE && inline.is_char_boundary(inline.len()));
    assert_eq!(bytes, Some(text.len() as u64));
    let source = source.unwrap();
    assert_eq!(codex.read_source(session, Some(&home), &source).unwrap(), text, "full text by byte range");
    let inside = agent::steps::Source { start: source.start + 1, ..source.clone() };
    assert!(codex.read_source(session, Some(&home), &inside).is_err(), "a range inside a record refuses");
    fs::remove_dir_all(home).unwrap();
}

// A5: only the app-server has a proven live source, and a delta never outlives the native result.
#[test]
fn a5_live_output_uses_native_deltas_only() {
    let steps = agent::runtime("codex").unwrap().log_steps(APP);
    let first = steps.iter().find(|s| matches!(&s.body, Body::Shell(_))).expect("real command");
    let Body::Shell(shell) = &first.body else { unreachable!() };
    assert_eq!(first.status, Status::Failed, "the native result replaced the running state");
    assert!(shell.output.as_ref().unwrap().source.is_some(), "the native item replaced the deltas");
    assert!(APP.contains("item/commandExecution/outputDelta"), "fixture is a real delta capture");
    // Before the item ends, output is the native delta text and nothing else.
    let running: String = APP.lines().filter(|l| !l.contains("\"item/completed\"") && !l.contains("\"turn/completed\"")).collect::<Vec<_>>().join("\n") + "\n";
    let steps = agent::runtime("codex").unwrap().log_steps(&running);
    let live = steps.iter().find(|s| matches!(&s.body, Body::Shell(_))).unwrap();
    assert_eq!(live.status, Status::Running);
    let deltas: String = APP.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["method"] == "item/commandExecution/outputDelta").filter_map(|v| v["params"]["delta"].as_str().map(String::from)).collect();
    let Body::Shell(shell) = &live.body else { unreachable!() };
    assert_eq!(shell.output.as_ref().unwrap().text, deltas);
    // A source without deltas shows no output until the step ends.
    let call = format!("{}\n", json!({"type":"assistant","uuid":"a","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}]}}));
    let steps = timeline("claude", &call);
    let (step, shell) = shells(&steps)[0];
    assert_eq!((shell.output.is_none(), step.unknown.get("output").map(String::as_str)), (true, Some("running")));
    for (format, status) in [("codex-app-server", "supported"), ("codex-exec", "unsupported"), ("codex-rollout", "unsupported"), ("claude-stream", "unsupported"), ("claude-native", "unsupported"), ("agy-stream", "unsupported"), ("antigravity-sidecar", "unsupported")] {
        assert_eq!(capabilities(format, None, "linux").unwrap()["features"]["live_output"]["status"], status, "{format}");
    }
    assert!(agent::runtime("codex").unwrap().descriptor().has("live-output"));
    for name in ["claude", "antigravity"] { assert!(!agent::runtime(name).unwrap().descriptor().has("live-output")); }
}

// A6: pages resume from a checkpoint, across a tool result on a later page, and refuse a rewritten source.
#[test]
fn a6_pages_resume_from_a_checkpoint() {
    let session = session_of(ROLLOUT);
    let home = rollout_home(&[(&session, ROLLOUT)]);
    let codex = agent::runtime("codex").unwrap();
    let (paged, info) = read_all("codex", &session, &home);
    assert_eq!(info.version.as_deref(), Some("0.162.1"));
    let whole = timeline("codex", ROLLOUT);
    let strip = |steps: &[Step]| steps.iter().map(|s| { let mut v = serde_json::to_value(s).unwrap(); v.as_object_mut().unwrap().remove("revision"); v }).collect::<Vec<_>>();
    assert_eq!(strip(&paged), strip(&whole), "paged and whole reads give one list");
    let first = codex.history(&session, Some(&home), None, None).unwrap();
    let saved = serde_json::to_string(&first.checkpoint).unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    let path = codex.session_transcript(&session, Some(&home)).unwrap();
    assert!(codex.history("22222222-2222-2222-2222-222222222222", Some(&home), Some(&saved), None).is_err(), "a checkpoint binds its session");
    fs::write(&path, "{}\n").unwrap();
    assert!(codex.history(&session, Some(&home), Some(&saved), None).is_err(), "a rewritten source refuses");
    assert!(matches!(codex.history("33333333-3333-3333-3333-333333333333", Some(&home), None, None), Err(Failure::Refused(c)) if c.0 == "session-not-found"));
    assert!(matches!(codex.history("../x", Some(&home), None, None), Err(Failure::Refused(c)) if c.0 == "session-invalid"));
    fs::remove_dir_all(home).unwrap();
}

// A6: the JSON command prints one page and a range read; errors name the flag.
#[test]
fn a6_json_command_prints_a_page() {
    let session = session_of(ROLLOUT);
    let home = rollout_home(&[(&session, ROLLOUT)]);
    let home_text = home.display().to_string();
    let args = ["--runtime", "codex", "--session", session.as_str(), "--home", home_text.as_str()];
    let page = agentq_runtime::cli::history(&args).unwrap();
    assert_eq!((&page["schema"], &page["runtime"], &page["format"]), (&json!(1), &json!("codex"), &json!("codex-rollout")));
    for key in ["session", "turns", "steps", "covered", "more", "pending_bytes", "checkpoint", "state", "resume", "resume_unknown", "capabilities"] { assert!(page.get(key).is_some(), "{key}"); }
    let checkpoint = home.join("checkpoint.json");
    fs::write(&checkpoint, page["checkpoint"].to_string()).unwrap();
    let checkpoint_text = checkpoint.display().to_string();
    let mut resumed = args.to_vec();
    resumed.extend(["--checkpoint", checkpoint_text.as_str()]);
    agentq_runtime::cli::history(&resumed).unwrap();
    let output = page["steps"].as_array().unwrap().iter().find_map(|s| s["output"]["source"].as_object().filter(|o| o.contains_key("pointer"))).expect("a source");
    let range = format!("{}:{}:{}", output["start"], output["end"], output["pointer"].as_str().unwrap());
    let mut read = args.to_vec();
    read.extend(["--read", range.as_str()]);
    let full = agentq_runtime::cli::history(&read).unwrap();
    assert!(full["text"].is_string() && full["bytes"].is_u64());
    assert!(agentq_runtime::cli::history(&["--runtime", "codex"]).unwrap_err().contains("--session"));
    assert!(agentq_runtime::cli::history(&["--runtime", "none", "--session", "x"]).unwrap_err().contains("unknown"));
    assert!(agentq_runtime::cli::history(&["--future", "x"]).unwrap_err().contains("--future"));
    fs::remove_dir_all(home).unwrap();
}

// A6: the descriptor names the native command, and running evidence refuses a second writer.
#[test]
fn a6_resume_command_refuses_a_second_writer() {
    for (name, resume) in [("claude", ["claude", "--resume", "{session}"]), ("codex", ["codex", "resume", "{session}"]), ("antigravity", ["agy", "--conversation", "{session}"])] {
        assert_eq!(agent::runtime(name).unwrap().descriptor().resume, resume);
    }
    let session = "44444444-4444-4444-4444-444444444444";
    let meta = json!({"type":"session_meta","payload":{"id":session,"session_id":session,"cwd":"/work"}});
    let started = json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"t"}});
    let complete = json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"t"}});
    let home = rollout_home(&[(session, &format!("{meta}\n{started}\n"))]);
    let codex = agent::runtime("codex").unwrap();
    assert_eq!(codex.session_state(session, Some(&home)).state, "running", "an open turn with a recent write");
    assert!(matches!(codex.resume_command(session, Some(&home)), Err(Failure::Refused(c)) if c.0 == "session-running"));
    let path = codex.session_transcript(session, Some(&home)).unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
    drop(file);
    let stale = codex.resume_command(session, Some(&home)).unwrap();
    assert_eq!((stale.state.state, stale.state.reason.as_str()), ("unknown", "open-turn-stale"), "a killed client stays resumable and is named unknown");
    fs::write(&path, format!("{meta}\n{started}\n{complete}\n")).unwrap();
    let command = codex.resume_command(session, Some(&home)).unwrap();
    assert_eq!(command.argv, ["codex", "resume", session]);
    assert_eq!(command.cwd.as_deref(), Some("/work"));
    assert_eq!(command.env, [("CODEX_HOME".to_string(), home.display().to_string())]);
    assert_eq!(command.state.state, "idle");
    fs::remove_dir_all(&home).unwrap();
    // Claude: the native registry and the live process decide. Key files beside it stay unread.
    let home = temp("claude-state");
    let id = "55555555-5555-5555-5555-555555555555";
    fs::create_dir_all(home.join("projects/p")).unwrap();
    fs::write(home.join("projects/p").join(format!("{id}.jsonl")), format!("{}\n", json!({"type":"user","uuid":"u","sessionId":id,"cwd":"/c","message":{"content":"x"}}))).unwrap();
    let claude = agent::runtime("claude").unwrap();
    assert_eq!(claude.session_state(id, Some(&home)).reason, "registry-unreadable", "an absent registry is unknown, never idle");
    fs::create_dir_all(home.join("sessions")).unwrap();
    fs::write(home.join("sessions/1.key"), "{\"sessionId\":\"55555555-5555-5555-5555-555555555555\",\"pid\":1}").unwrap();
    assert_eq!(claude.session_state(id, Some(&home)).state, "idle");
    fs::write(home.join("sessions/live.json"), json!({"sessionId":id,"pid":std::process::id()}).to_string()).unwrap();
    assert_eq!(claude.session_state(id, Some(&home)).state, "running");
    assert!(matches!(claude.resume_command(id, Some(&home)), Err(Failure::Refused(c)) if c.0 == "session-running"));
    fs::remove_file(home.join("sessions/live.json")).unwrap();
    assert_eq!(claude.resume_command(id, Some(&home)).unwrap().cwd.as_deref(), Some("/c"));
    fs::remove_dir_all(&home).unwrap();
    // Antigravity has no native state, so unknown names its reason.
    assert_eq!(agent::runtime("antigravity").unwrap().session_state("x", None).reason, "no-native-state");
}

// A7: every shape of real rollout reads under its own thread ID; binding the root ID to a child is the probe's failure.
#[test]
fn a7_forked_and_subagent_rollouts_read_under_their_own_id() {
    let (parent, child, fork) = (session_of(ROLLOUT), session_of(SUBAGENT), session_of(FORK));
    let home = rollout_home(&[(&parent, ROLLOUT), (&child, SUBAGENT), (&fork, FORK)]);
    for id in [&parent, &child, &fork] { assert!(!read_all("codex", id, &home).0.is_empty(), "{id}"); }
    // A child's records carry its own thread; its `session_id` is the root. Binding the root refuses, by design.
    let meta: Value = serde_json::from_str(SUBAGENT.lines().next().unwrap()).unwrap();
    let root = meta["payload"]["session_id"].as_str().unwrap();
    assert_ne!(root, child);
    let path = agent::runtime("codex").unwrap().session_transcript(&child, Some(&home)).unwrap();
    let mut wrong = History::new("codex-rollout", "probe", root).unwrap();
    assert_eq!(wrong.poll(&path).unwrap_err(), "history native session differs");
    // A fork names its base; its own source holds only records after the fork.
    let (_, session) = read_all("codex", &fork, &home);
    let base = session.fork.expect("native history_base");
    assert!(base.session.is_some() && base.end_byte.is_some() && base.end_ordinal.is_some());
    assert_eq!(session.relation.as_deref(), Some("fork"));
    fs::remove_dir_all(home).unwrap();
}

// A7: `--until-byte` ends a base read at the fork point.
#[test]
fn a7_until_byte_reads_a_fork_base() {
    let session = session_of(ROLLOUT);
    let home = rollout_home(&[(&session, ROLLOUT)]);
    let limit = ROLLOUT.lines().take(5).map(|l| l.len() as u64 + 1).sum::<u64>();
    let page = agent::runtime("codex").unwrap().history(&session, Some(&home), None, Some(limit)).unwrap();
    assert!(!page.more);
    assert!(page.steps.iter().all(|s| s.revision <= limit));
    fs::remove_dir_all(home).unwrap();
}

// A3: History events of the same records no longer read as unknown.
#[test]
fn a3_history_events_map_the_records() {
    let dir = temp("history");
    let path = dir.join("rollout.jsonl");
    fs::write(&path, ROLLOUT).unwrap();
    let mut history = History::new("codex-rollout", "test", &session_of(ROLLOUT)).unwrap();
    let mut kinds = std::collections::BTreeMap::<String, String>::new();
    loop {
        let page = history.poll(&path).unwrap();
        for event in &page.events {
            let v: Value = serde_json::from_str(event["raw"].as_str().unwrap()).unwrap();
            if v["payload"]["type"] == "item_completed" { kinds.insert(v["payload"]["item"]["type"].as_str().unwrap().into(), event["kind"].as_str().unwrap().into()); }
        }
        if !page.more { break; }
    }
    for (item, kind) in [("CommandExecution", "tool"), ("FileChange", "tool"), ("McpToolCall", "tool"), ("Reasoning", "thinking")] {
        assert_eq!(kinds.get(item).map(String::as_str), Some(kind), "{item}");
    }
    fs::remove_dir_all(dir).unwrap();
}

// A8: Antigravity reads through the same shape, and its limits read as unknown with reasons.
#[test]
fn a8_antigravity_limits_are_stated() {
    let agy = timeline("antigravity", include_str!("fixtures/history/agy-stream.jsonl"));
    assert!(matches!(&of(&agy, "message")[0].body, Body::Message { text: None, .. }));
    assert_eq!(of(&agy, "message")[0].unknown.get("text").map(String::as_str), Some("text-not-emitted"));
    assert!(!of(&agy, "tool").is_empty() && !of(&agy, "usage").is_empty());
    assert!(!of(&agy, "unknown").is_empty(), "checkpoint step keeps its raw record");
    let sidecar = timeline("antigravity", include_str!("fixtures/history/antigravity-sidecar.jsonl"));
    assert!(of(&sidecar, "unknown").iter().any(|s| matches!(&s.body, Body::Unknown { record, .. } if record == "GENERIC")), "GENERIC has no known author");
    assert!(!of(&sidecar, "message").is_empty());
    let command = format!("{}\n{}\n", json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","tool_calls":[{"id":"c","name":"run_command","args":{"CommandLine":"ls","Cwd":"/w"}}]}), json!({"type":"TOOL_RESPONSE","step_index":2,"status":"DONE","tool_call_id":"c","content":"out"}));
    let steps = timeline("antigravity", &command);
    let (step, shell) = shells(&steps)[0];
    assert_eq!((shell.command.as_deref(), shell.cwd.as_deref(), shell.output.as_ref().map(|o| o.text.as_str()), step.status), (Some("ls"), Some("/w"), Some("out"), Status::Completed));
    assert_eq!(step.unknown.get("exit_code").map(String::as_str), Some("not-reported"));
    let done = format!("{}\n", json!({"event":"step_update","step_update":{"conversation_id":"c","step_index":0,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"pwd"},"output":"/x"}}}));
    let (step, shell) = (|steps: Vec<Step>| { let (s, sh) = shells(&steps)[0]; (s.clone(), sh.clone()) })(timeline("antigravity", &done));
    assert_eq!((shell.command.as_deref(), shell.output.map(|o| o.text), step.status), (Some("pwd"), Some("/x".to_string()), Status::Completed));
}

// A8: schema-only mappings stay honest about their source; approval and question revise by request.
#[test]
fn a8_schema_only_approval_and_question_map() {
    let log = [
        json!({"id":7,"method":"item/commandExecution/requestApproval","params":{"itemId":"c","threadId":"t"}}),
        json!({"id":8,"method":"item/tool/requestUserInput","params":{"itemId":"q","questions":[]}}),
        json!({"method":"serverRequest/resolved","params":{"threadId":"t","requestId":7}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let steps = timeline("codex", &log);
    assert!(matches!(&steps[0].body, Body::Approval { request: Some(_), .. }) && steps[0].status == Status::Completed);
    assert!(matches!(&steps[1].body, Body::Question { prompt: Some(_), .. }) && steps[1].status == Status::Running);
    let log = format!("{}\n{}\n", json!({"type":"control_request","request_id":"r","request":{"subtype":"can_use_tool","tool_name":"Bash"}}), json!({"type":"control_response","response":{"subtype":"success","request_id":"r","response":{"behavior":"allow"}}}));
    let steps = timeline("claude", &log);
    assert_eq!(steps.len(), 1);
    assert!(matches!(&steps[0].body, Body::Approval { request: Some(_), decision: Some(d) } if d == "allow"));
    let log = format!("{}\n{}\n", json!({"type":"assistant","uuid":"a","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"AskUserQuestion","input":{"questions":[]}}]}}), json!({"type":"user","uuid":"b","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"yes"}]}}));
    let steps = timeline("claude", &log);
    assert!(matches!(&steps[0].body, Body::Question { prompt: Some(_), answer: Some(a) } if a.text == "yes"));
}

// A8: other record shapes map per contract: web, image, images in user content, exec JSON.
#[test]
fn a8_remaining_kinds_map() {
    let steps = timeline("codex", include_str!("fixtures/history/codex-exec.jsonl"));
    assert!(!shells(&steps).is_empty() && !of(&steps, "mcp").is_empty() && !of(&steps, "file_change").is_empty(), "exec JSON reads through one mapper");
    let log = [
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"Extension","kind":"web.search","id":"w","query":"q","action":{"type":"openPage","url":"u"},"results":[]}}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"ImageView","id":"i","path":"file:///x.png"}}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
    let steps = timeline("codex", &log);
    assert!(matches!(&steps[0].body, Body::Web { query: Some(q), url: Some(u), .. } if q == "q" && u == "u"));
    assert!(matches!(&steps[1].body, Body::Image { path: Some(p), .. } if p == "/x.png"));
    let log = format!("{}\n", json!({"type":"user","uuid":"u","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}}]}}));
    let steps = timeline("claude", &log);
    assert!(matches!(&steps[0].body, Body::Image { media_type: Some(m), path: None } if m == "image/png"));
    assert!(!serde_json::to_string(&steps[0].body).unwrap().contains("AAAA"), "inline image data stays in the source");
    let log = format!("{}\n", json!({"type":"assistant","uuid":"a","message":{"id":"m","content":[{"type":"tool_use","id":"w","name":"WebFetch","input":{"url":"u"}},{"type":"tool_use","id":"p","name":"mcp__srv__do","input":{"a":1}}]}}));
    let steps = timeline("claude", &log);
    assert!(matches!(&steps[0].body, Body::Web { url: Some(_), .. }));
    assert!(matches!(&steps[1].body, Body::Mcp { server: Some(s), tool: Some(t), .. } if s == "srv" && t == "do"));
}

// A8: fixtures stay scrubbed.
#[test]
fn a8_fixtures_hold_no_private_values() {
    for text in [ROLLOUT, COMPACTION, SUBAGENT, PARENT, FORK, APP, CLAUDE, CLAUDE_OLD, CLAUDE_COMPACT, CLAUDE_AGENT, CLAUDE_CHILD] {
        for marker in ["alexk", "gmail", "C:\\\\Users", "/Users/", "Bangkok"] { assert!(!text.contains(marker), "{marker}"); }
        assert!(text.lines().all(|l| l.parse::<Value>().is_ok()));
    }
}
