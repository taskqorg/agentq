//! agentq.md § 12 (moved from coreq.md § Agent runtimes) with fake clients: this binary is also the fake `claude` and `codex` when
//! `COREQ_FAKE_AGENT` names one, so start, resume, end detection and observations run on every OS without a real CLI.
use agentq_runtime::agent::{self, Context};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
#[path="support/protected.rs"]
mod protected;

fn main() {
    if std::env::var_os("AGENTQ_FAKE_QUOTA_HTTP").is_some() { return fake_quota_http(); }
    let wrapper: Vec<_> = std::env::args_os().skip(1).collect();
    if wrapper.first().is_some_and(|s| s == "--wrap") {
        std::process::exit(Command::new(&wrapper[1]).args(&wrapper[2..]).status().unwrap().code().unwrap_or(1));
    }
    if std::env::var_os("AGENTQ_FAKE_AGENTAPI").is_some() { return fake_agentapi(); }
    if matches!(std::env::args().nth(1).as_deref(), Some("service"|"agent")) { return protected::fake_host(); }
    if let Some(file) = ["CLAUDE_CONFIG_DIR", "CODEX_HOME"].iter().find_map(|k| std::env::var_os(k)).map(|h| PathBuf::from(h).join("fake-ready.json")).filter(|f| f.is_file()) {
        return fake_ready(&file);
    }
    if let Ok(client) = std::env::var("COREQ_FAKE_AGENT") { return fake(&client); }
    if std::env::var_os("AGENTQ_LIVE_CLAUDE_BACKGROUND").is_some() { return live_claude_background_link(); }
    let tests: &[(&str, fn())] = &[
        ("automatic_codex_turns_select_transport", automatic_codex_turns_select_transport),
        ("quota_account_read_guards_credentials", quota_account_read_guards_credentials),
        ("catalogue_and_account_reads_need_no_turn", catalogue_and_account_reads_need_no_turn),
        ("rollout_progress_survives_fast_resume_and_recovery", rollout_progress_survives_fast_resume_and_recovery),
        ("antigravity_wrapped_submission_keeps_prompt_and_tail", antigravity_wrapped_submission_keeps_prompt_and_tail),
        ("antigravity_native_submission_tails_and_resumes", antigravity_native_submission_tails_and_resumes),
        ("claude_starts_and_resumes_one_session", claude_starts_and_resumes_one_session),
        ("codex_resumes_its_thread_in_the_account_home", codex_resumes_its_thread_in_the_account_home),
        ("lingering_process_is_stopped_after_its_turn_ends", lingering_process_is_stopped_after_its_turn_ends),
        ("deadline_stops_a_turn_that_never_ends", deadline_stops_a_turn_that_never_ends),
        ("background_commands_preserve_native_mode", background_commands_preserve_native_mode),
        ("native_control_readiness_and_large_history_stay_owned", native_control_readiness_and_large_history_stay_owned),
        ("background_job_found_when_cwd_listing_misses_it", background_job_found_when_cwd_listing_misses_it),
        ("app_transport_preserves_native_tools_drafts_and_catalogue", app_transport_preserves_native_tools_drafts_and_catalogue),
        ("app_input_forwards_images_once_and_reads_account_only", app_input_forwards_images_once_and_reads_account_only),
        ("app_resume_frames_stay_bounded_and_named", app_resume_frames_stay_bounded_and_named),
        ("protected_launch_refuses_tampering_and_preserves_native_transport", protected::protected_launch_refuses_tampering_and_preserves_native_transport),
        ("signed_retained_recovery_preserves_original_bytes", protected::signed_retained_recovery_preserves_original_bytes),
        ("independent_signed_host_recovers_without_taskq", protected::independent_signed_host_recovers_without_taskq),
        ("protected_frontend_is_not_scope_supervisor", protected::protected_frontend_is_not_scope_supervisor),
        ("protected_preparation_and_completion_recover_original_operation", protected::protected_preparation_and_completion_recover_original_operation),
        ("json_turns_run_plain_without_the_optimizer", json_turns_run_plain_without_the_optimizer),
        ("off_reaches_codex_queue_and_app_server", off_reaches_codex_queue_and_app_server),
        ("external_consumer_builds_and_plans_turns", external_consumer_builds_and_plans_turns),
        ("runtime_trait_runs_every_runtime_by_descriptor", runtime_trait_runs_every_runtime_by_descriptor),
        ("cli_turn_recovers_its_running_child", cli_turn_recovers_its_running_child),
        ("process_hooks_shape_the_child", process_hooks_shape_the_child),
        ("runtime_trait_exposes_native_seams", runtime_trait_exposes_native_seams),
        ("ready_reads_native_login_and_minimum_version", ready_reads_native_login_and_minimum_version),
        ("host_args_reach_codex_turns_only", host_args_reach_codex_turns_only),
        ("settings_env_reaches_the_session", settings_env_reaches_the_session),
        ("settings_env_preserves_subscription_policy", settings_env_preserves_subscription_policy),
        ("settings_env_off_precedes_state_read", settings_env_off_precedes_state_read),
        ("settings_env_keeps_inherited_launch_markers", settings_env_keeps_inherited_launch_markers),
        ("launcher_output_is_handed_over_once", launcher_output_is_handed_over_once),
    ];
    let filters:Vec<_>=std::env::args().skip(1).collect();
    let selected:Vec<_>=tests.iter().filter(|(name,_)|filters.is_empty()||filters.iter().any(|f|name.contains(f))).collect();
    for (name, test) in &selected {
        test();
        println!("test {name} ... ok");
    }
    println!("test result: ok. {} passed", selected.len());
}

fn append(path:&Path, events:&[Value]) {
    use std::io::Write;
    let mut f=std::fs::File::options().create(true).append(true).open(path).unwrap();
    for event in events {writeln!(f,"{event}").unwrap();}
}

// #246 A1: caller needs select one per-turn session; plain workers retain exec/recovery.
// Risk: relay input is refused or a caller must implement a second native transport.
fn automatic_codex_turns_select_transport() {
    let root = scratch("automatic-transport");
    let codex = agent::runtime("codex").unwrap();
    let mut launch = fake_launch("codex", &root, "private request", &[("COREQ_FAKE_APP_SERVER", "1".as_ref()), ("AGENTQ_FAKE_ONE_TURN", "1".as_ref())]);
    launch.deltas = true;
    let mut run = codex.launch(&launch).unwrap_or_else(|e| panic!("neutral deltas launch refused: {e}"));
    let until = Instant::now() + Duration::from_secs(10);
    while codex.poll(&mut run).unwrap() == agent::State::Running {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(codex.report(&run).unwrap().text.as_deref(), Some("final answer"));
}

fn fake_quota_http() {
    let mut input=String::new(); std::io::stdin().read_to_string(&mut input).unwrap();
    assert!(input.contains("Authorization: Bearer invented-token"));
    assert!(std::env::args().all(|a|!a.contains("invented-token")),"credentials never enter argv");
    if let Some(path)=std::env::var_os("AGENTQ_FAKE_QUOTA_CHANGE") {std::fs::write(path,b"invented changed credentials").unwrap();}
    println!("{}",json!({"seven_day":{"utilization":61,"resets_at":"2026-11-09T00:00:00Z"},"limits":[{"group":"weekly","percent":99,"resets_at":"2026-11-09T00:00:00Z","scope":{"model":{"display_name":"Fixture"}}}]}));
}

// #246 A3/M12.11: capture a rollout boundary before spawn and retain both progress cursors across recovery.
// Risk: a fast child loses events, an unreadable supplemental source consumes messages, or recovery emits prior turns.
fn rollout_progress_survives_fast_resume_and_recovery() {
    let root=scratch("rollout-progress"); let home=root.join("account"); let dir=home.join("sessions/2026/11/06");std::fs::create_dir_all(&dir).unwrap();
    let path=dir.join("rollout-native-thread.jsonl");
    let quota=|used|json!({"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":used,"window_minutes":300,"resets_at":1900000000}},"info":{"last_token_usage":{"total_tokens":12},"model_context_window":100}}});
    append(&path,&[quota(10)]);
    let mut l=fake_launch("codex",&root,"x",&[("COREQ_FAKE_HANG","1".as_ref()),("CODEX_HOME",home.as_os_str())]);
    let file=path.clone();let row=quota(80);
    l.process.spawned=Some(std::sync::Arc::new(move |_|{append(&file,std::slice::from_ref(&row));Ok(())}));
    let runtime=agent::runtime("codex").unwrap();let mut run=runtime.resume("native-thread",&l).unwrap();
    let events=runtime.progress(&mut run).unwrap();
    let quotas:Vec<_>=events.iter().filter_map(|e|if let agent::Event::Quota(w)=e {Some(w[0].used)} else {None}).collect();
    assert_eq!(quotas,vec![Some(0.8)],"new rows survive launch; earlier turns do not leak");
    append(&l.log,&[json!({"type":"item.completed","item":{"type":"agent_message","text":"retained message"}})]);
    let bytes=std::fs::read(&path).unwrap();std::fs::remove_file(&path).unwrap();
    assert!(runtime.progress(&mut run).is_err());std::fs::write(&path,bytes).unwrap();
    assert!(runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,agent::Event::Message{text,..} if text=="retained message")));
    let saved=runtime.checkpoint(&run).unwrap();let mut cold=runtime.recover(&saved,&l).unwrap();
    assert!(runtime.progress(&mut cold).unwrap().is_empty(),"recovery retains delivered progress and rollout boundary");
    let mut legacy=saved.clone();legacy.as_object_mut().unwrap().remove("rollout_progress");
    let mut legacy=runtime.recover(&legacy,&l).unwrap();assert!(runtime.progress(&mut legacy).unwrap().is_empty(),"legacy recovery never attributes whole thread history to this turn");
    runtime.cancel(&mut run).unwrap();let until=Instant::now()+Duration::from_secs(5);
    while runtime.poll(&mut run).unwrap()==agent::State::Running {assert!(Instant::now()<until);std::thread::sleep(Duration::from_millis(10));}
}

// #246 A4: every catalogue page uses one short session; account windows need no worker or product protocol.
fn catalogue_and_account_reads_need_no_turn() {
    let root=scratch("native-reads"); let bin=root.join(if cfg!(windows) {"codex.exe"} else {"codex"});
    std::fs::copy(std::env::current_exe().unwrap(),&bin).unwrap();
    std::fs::write(root.join("auth.json"),b"invented credential binding").unwrap();
    let keys=["PATH","COREQ_FAKE_AGENT","COREQ_FAKE_APP_SERVER","COREQ_FAKE_CATALOGUE_PAGES"];
    let saved:Vec<_>=keys.iter().map(|k|(*k,std::env::var_os(k))).collect();
    let old=std::env::var_os("PATH").unwrap_or_default();
    let path=std::env::join_paths(std::iter::once(root.clone()).chain(std::env::split_paths(&old))).unwrap();
    // SAFETY: the custom harness is serial; the read session is closed before restoring the environment.
    unsafe {std::env::set_var("PATH",path);std::env::set_var("COREQ_FAKE_AGENT","codex");std::env::set_var("COREQ_FAKE_APP_SERVER","1");std::env::set_var("COREQ_FAKE_CATALOGUE_PAGES","1");}
    let cwd=std::env::current_dir().unwrap();std::env::set_current_dir(&root).unwrap();
    let runtime=agent::runtime("codex").unwrap();
    let models=runtime.catalogue(Some(&root)).unwrap();
    assert_eq!(models.iter().map(|m|m.id.as_str()).collect::<Vec<_>>(),["fixture-model-1","fixture-model-2"]);
    let windows=runtime.account_limits(Some(&root),1800000000);
    assert_eq!(windows.len(),2);assert_eq!(windows[1].used,Some(0.9));
    let requests=std::fs::read_to_string(root.join("app-input.jsonl")).unwrap();
    let rows:Vec<Value>=requests.lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.iter().filter(|v|v["method"]=="initialize").count(),2,"one catalogue session, one account session");
    assert!(rows.iter().all(|v|!matches!(v["method"].as_str(),Some("thread/start"|"thread/resume"|"turn/start"))));
    std::env::set_current_dir(cwd).unwrap();
    for (key,value) in saved {unsafe {match value {Some(value)=>std::env::set_var(key,value),None=>std::env::remove_var(key)}}}
}

// #246 A4: a read binds the credential before/after HTTP, never refreshes, and keeps every model window.
fn quota_account_read_guards_credentials() {
    // Native Keychain access needs its own platform proof; this exercises the file credential route.
    #[cfg(not(target_os="macos"))]
    {
        let root=scratch("quota-http"); let bin=root.join(if cfg!(windows) {"curl.exe"} else {"curl"});
        std::fs::copy(std::env::current_exe().unwrap(),&bin).unwrap();
        let path=root.join(".credentials.json"); let original=json!({"claudeAiOauth":{"accessToken":"invented-token","expiresAt":2000000000000i64}}).to_string();
        std::fs::write(&path,&original).unwrap();
        let old=std::env::var_os("PATH").unwrap_or_default();
        let new=std::env::join_paths(std::iter::once(root.clone()).chain(std::env::split_paths(&old))).unwrap();
        // SAFETY: the custom harness runs serially; subprocesses are awaited before environment restoration.
        unsafe {std::env::set_var("PATH",&new);std::env::set_var("AGENTQ_FAKE_QUOTA_HTTP","1");}
        let runtime=agent::runtime("claude").unwrap();
        let windows=runtime.account_limits(Some(&root),1800000000);
        assert_eq!(windows.len(),2,"{windows:?}");assert_eq!(windows[0].used,Some(0.61));assert_eq!(windows[1].used,Some(0.99));
        assert_eq!(std::fs::read_to_string(&path).unwrap(),original,"no refresh or credential write");
        unsafe {std::env::set_var("AGENTQ_FAKE_QUOTA_CHANGE",&path);}
        assert_eq!(runtime.account_limits(Some(&root),1800000000)[0].unknown.as_deref(),Some("credential-changed"));
        unsafe {std::env::set_var("PATH",old);std::env::remove_var("AGENTQ_FAKE_QUOTA_HTTP");std::env::remove_var("AGENTQ_FAKE_QUOTA_CHANGE");}
    }
}

// ---------- fake clients ----------

/// One line per event; the session store is `<cwd>/fake-sessions.json` (turn count per session).
fn fake(client: &str) {
    if std::env::var("COREQ_FAKE_APP_SERVER").is_ok() {return fake_app_server();}
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    if client == "antigravity" { input = serde_json::from_str::<Value>(&input).unwrap()["message"]["content"].as_str().unwrap().to_string(); }
    if std::env::var("COREQ_FAKE_CONTROL").is_ok() && args.first().is_some_and(|a| matches!(a.as_str(), "agents" | "stop" | "queue")) {
        let record=json!({"args":args,"home":std::env::var(if client=="claude" {"CLAUDE_CONFIG_DIR"} else {"CODEX_HOME"}).ok()});
        std::fs::write("native-control.json",record.to_string()).unwrap();
        // Claude's `--cwd` filter lists nothing for roots under `.claude/worktrees` (#222).
        let missed=std::env::var_os("FAKE_CWD_FILTER_EMPTY").is_some() && args.iter().any(|a|a=="--cwd");
        if args.first().is_some_and(|s|s=="agents") {println!("{}",if missed {"[]".into()} else {std::env::var("FAKE_JOB_ROWS").unwrap()});}
        return;
    }
    if std::env::var("COREQ_FAKE_BACKGROUND").is_ok() {
        assert_eq!(args.first().map(String::as_str), Some("--bg"), "background worker must not switch to -p");
        std::fs::write("background-command.json", json!({"args":args,"input":input,"home":std::env::var("CLAUDE_CONFIG_DIR").ok(),
            "inherited":std::env::var("CLAUDE_CODE_SESSION_ID").is_ok(),"extra":std::env::var("FAKE_EXTRA").ok()}).to_string()).unwrap();
        if std::env::var("FAKE_LAUNCH_REFUSED").is_ok() { eprintln!("Error: Workspace not trusted"); std::process::exit(1); }
        // Claude Code 2.1.317 launcher output: the job ID is not the last token (#216).
        let name = args.iter().position(|a| a == "--name").and_then(|i| args.get(i + 1)).map(|n| format!(" · {n}")).unwrap_or_default();
        println!("backgrounded · c37eb75a{name}\n  claude agents             list sessions\n  claude stop c37eb75a      stop this session\n");
        return;
    }
    let store = PathBuf::from("fake-sessions.json");
    let mut sessions: Value = std::fs::read_to_string(&store).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    let after = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
    let emit = |v: Value| println!("{v}");
    if std::env::var("COREQ_FAKE_HANG").is_ok() { std::thread::sleep(Duration::from_secs(60)); }
    let (session, resumed) = match client {
        "claude" => {
            assert_eq!(&args[..4], ["-p", "--output-format", "stream-json", "--verbose"], "claude print mode");
            match (after("--session-id"), after("--resume")) { (Some(s), None) => (s, false), (None, Some(s)) => (s, true), _ => panic!("claude session flag") }
        }
        "antigravity" => {
            assert_eq!(&args[..4], ["--input-format", "stream-json", "--output-format", "stream-json"]);
            match after("--conversation") {Some(s)=>(s,true),None=>(format!("ag-{}",std::process::id()),false)}
        }
        _ => {
            assert_eq!(&args[..2], ["exec", "--json"], "codex exec");
            assert_eq!(args.last().map(String::as_str), Some("-"), "codex reads stdin");
            match after("resume") { Some(t) => (t, true), None => (format!("t-{}", std::process::id()), false) }
        }
    };
    if resumed && sessions.get(&session).is_none() { eprintln!("unknown session {session}"); std::process::exit(2); }
    if !resumed && sessions.get(&session).is_some() { eprintln!("session {session} already exists"); std::process::exit(2); }
    let turn = sessions[&session].as_u64().unwrap_or(0) + 1;
    sessions[&session] = json!(turn);
    std::fs::write(&store, sessions.to_string()).unwrap();
    // The record lets the test read argv, stdin and environment the child got.
    let home_var = match client { "claude" => "CLAUDE_CONFIG_DIR", "antigravity" => "ANTIGRAVITY_HOME", _ => "CODEX_HOME" };
    let record = json!({"args": args, "input": input, "home": std::env::var(home_var).ok(),
        "inherited": std::env::var("CLAUDE_CODE_SESSION_ID").is_ok() || std::env::var("CODEX_THREAD_ID").is_ok() || std::env::var("ANTIGRAVITY_CONVERSATION_ID").is_ok(),
        "extra": std::env::var("FAKE_EXTRA").ok(),
        "optimization": std::env::var("AGENTQ_OPTIMIZATION").ok(), "launcher": std::env::var("AGENTQ_LAUNCHER").ok()});
    std::fs::write(format!("fake-turn-{turn}.json"), record.to_string()).unwrap();
    // A gated turn runs until the test creates the gate file, so the caller can checkpoint and recover it mid-turn.
    if let Ok(gate) = std::env::var("COREQ_FAKE_GATE") { while !Path::new(&gate).exists() { std::thread::sleep(Duration::from_millis(20)); } }
    if client == "claude" {
        emit(json!({"type": "system", "subtype": "init", "session_id": session}));
        emit(json!({"type": "assistant", "message": {"id": format!("m{turn}"), "model": "claude-opus-5-5",
            "usage": {"input_tokens": 3, "cache_read_input_tokens": 100, "cache_creation_input_tokens": 20, "output_tokens": 7}}}));
        emit(json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1792386000u64, "rateLimitType": "five_hour", "utilization": 0.25}}));
        emit(json!({"type": "result", "subtype": "success", "is_error": false, "result": format!("turn {turn}"), "session_id": session}));
    } else if client == "antigravity" {
        emit(json!({"event":"init", "conversation_id":session, "init":{}}));
        emit(json!({"event":"step_update", "step_update":{"step_index":turn,"step_type":"agent_response","state":"DONE", "usage":{"input_tokens":12,"cache_read_tokens":60,"output_tokens":8}}}));
        emit(json!({"event":"result", "result":{"conversation_id":session,"status":"SUCCESS","response":format!("turn {turn}")}}));
    } else {
        if !resumed { emit(json!({"type": "thread.started", "thread_id": session})); }
        if let Ok(home) = std::env::var("CODEX_HOME") {
            let dir = Path::new(&home).join("sessions/2026/11/01");
            std::fs::create_dir_all(&dir).unwrap();
            let line = json!({"type": "event_msg", "payload": {"type": "token_count", "rate_limits": {"primary": {"used_percent": 30.0 + turn as f64, "window_minutes": 300, "resets_at": 1793700000u64}, "secondary": null}}});
            use std::io::Write;
            writeln!(std::fs::File::options().create(true).append(true).open(dir.join(format!("rollout-2026-11-01T00-00-00-{session}.jsonl"))).unwrap(), "{line}").unwrap();
        }
        emit(json!({"type": "turn.started"}));
        emit(json!({"type": "item.completed", "item": {"type": "agent_message", "text": format!("turn {turn}")}}));
        emit(json!({"type": "turn.completed", "usage": {"input_tokens": 50, "cached_input_tokens": 40, "output_tokens": 5}}));
    }
    if std::env::var("COREQ_FAKE_LINGER").is_ok() { std::thread::sleep(Duration::from_secs(60)); }
}

fn fake_app_server() {
    use std::io::BufRead;
    let args:Vec<_>=std::env::args().skip(1).collect();
    assert_eq!(args[0],"app-server");
    std::fs::write("app-transport.json",json!({"home":std::env::var("CODEX_HOME").ok(),"inherited":std::env::var("CODEX_THREAD_ID").is_ok(),"args":args}).to_string()).unwrap();
    let mut turn=0;
    println!("fixture startup diagnostic");
    for line in std::io::stdin().lock().lines() {
        let value:Value=serde_json::from_str(&line.unwrap()).unwrap();
        append(Path::new("app-input.jsonl"), &[value.clone()]);
        let id=&value["id"];
        match value["method"].as_str() {
            Some("initialize")=>println!("{}",json!({"id":id,"result":{}})),
            Some("initialized")=>{},
            Some("model/list")=>{
                let pages=std::env::var_os("COREQ_FAKE_CATALOGUE_PAGES").is_some();
                let first=value["params"]["cursor"].is_null();
                println!("{}",json!({"id":id,"result":{"data":[{"model":if pages {if first {"fixture-model-1"} else {"fixture-model-2"}} else {"fixture-model"},"hidden":false}],"nextCursor":if pages && first {json!("second")} else {Value::Null}}}));
            },
            Some("account/rateLimits/read")=>println!("{}",json!({"id":id,"result":{"rateLimits":{"primary":{"usedPercent":20,"windowDurationMins":300,"resetsAt":1900000000},"secondary":{"usedPercent":90,"windowDurationMins":10080,"resetsAt":1900000000}}}})),
            Some("account/read")=>{
                assert_eq!(value["params"],json!({"refreshToken":false}),"identity read never refreshes a token");
                println!("{}",json!({"id":id,"result":{"account":{"type":"chatgpt","email":"private@example.invalid","planType":"plus"},"requiresOpenaiAuth":true}}));
            },
            Some("thread/start")=>{
                if std::env::var_os("AGENTQ_FAKE_ONE_TURN").is_none() { assert_eq!(value["params"]["dynamicTools"][0]["name"],"caller-tool"); }
                if let Some(instructions)=value["params"]["developerInstructions"].as_str() {assert_eq!(instructions,"private rules");}
                if std::env::var_os("AGENTQ_FAKE_ONE_TURN").is_none() {assert!(matches!(value["params"]["sandbox"].as_str(),Some("read-only"|"workspace-write")),"policy maps to thread params");}
                std::fs::write("app-native-thread","native-thread").unwrap();
                println!("{}",json!({"id":id,"result":{"thread":{"id":"native-thread"},"model":"fixture-model"}}));
            },
            Some("thread/resume")=>{
                assert_eq!(value["params"]["threadId"],std::fs::read_to_string("app-native-thread").unwrap());
                // Native semantics measured in docs/research/253.md: `thread.turns` carries the history unless excluded.
                if let Ok(frame)=std::env::var("AGENTQ_FAKE_RESUME_FRAME") {
                    use std::io::Write;
                    let head=json!({"id":id,"result":{"thread":{"id":"native-thread"},"model":"fixture-model","pad":""}}).to_string();
                    let padded=|total:usize| head.replace("\"pad\":\"\"",&format!("\"pad\":\"{}\"","x".repeat(total-head.len()-1)));
                    const LIMIT:usize=8*1024*1024;
                    match frame.as_str() {
                        "over"=>println!("{}",padded(LIMIT+1)),
                        "native" if value["params"]["excludeTurns"]!=true=>println!("{}",padded(LIMIT+1)),
                        "exact"=>println!("{}",padded(LIMIT)),
                        "cut"=>{print!("{}",&head[..head.len()-1]);std::io::stdout().flush().unwrap();std::process::exit(0);},
                        // Valid UTF-8 near the bound: a two-byte character closes an exact-limit pad, or straddles the bound.
                        "exact-utf8"=>println!("{}",padded(LIMIT).replace("xx\"","é\"")),
                        "over-utf8"=>{let pad=head.find("\"pad\":\"").unwrap()+7;println!("{}",head.replace("\"pad\":\"\"",&format!("\"pad\":\"{}é\"","x".repeat(LIMIT-pad))));},
                        // Raw bytes: a stream cut inside a character, and a complete record that is not UTF-8.
                        "cut-utf8"=>{let mut out=std::io::stdout();out.write_all(&head.as_bytes()[..head.len()-1]).unwrap();out.write_all(&[0xC3]).unwrap();out.flush().unwrap();std::process::exit(0);},
                        "invalid"=>{let mut out=std::io::stdout();out.write_all(b"{\"pad\":\"\xFF\"}\n").unwrap();out.flush().unwrap();},
                        _=>println!("{head}"),
                    }
                    continue;
                }
                println!("{}",json!({"id":id,"result":{"thread":{"id":"native-thread"},"model":"fixture-model"}}));
            },
            Some("turn/start")=>{
                turn+=1;
                if std::env::var_os("AGENTQ_FAKE_ONE_TURN").is_some() {
                    turn=std::fs::read_to_string("app-turn-count").ok().and_then(|s|s.parse::<u64>().ok()).unwrap_or(0)+1;
                    std::fs::write("app-turn-count",turn.to_string()).unwrap();
                }
                println!("{}",json!({"id":id,"result":{"turn":{"id":format!("turn-{turn}")}}}));
                println!("{}",json!({"id":999,"method":"item/tool/call","params":{"threadId":"native-thread","turnId":format!("turn-{turn}"),"tool":"caller-tool","arguments":{"private":"tool input"}}}));
                println!("{}",json!({"method":"item/agentMessage/delta","params":{"threadId":"native-thread","turnId":format!("turn-{turn}"),"itemId":"draft","delta":"private draft"}}));
            },
            None if id==999=>{
                if std::env::var_os("AGENTQ_FAKE_ONE_TURN").is_some() {assert_eq!(value["error"]["code"],-32601);}
                else {assert_eq!(value["result"]["success"],true,"only caller's handler can answer native tool requests");}
                println!("{}",json!({"method":"item/completed","params":{"threadId":"native-thread","turnId":format!("turn-{turn}"),"item":{"type":"agentMessage","text":"final answer"}}}));
                println!("{}",json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":format!("turn-{turn}"),"status":"completed"}}}));
            },
            _=>panic!("unexpected native protocol request"),
        }
    }
}

// #246 A1/A3: one Launch carries relay data; the adapter owns the native protocol and stop.
// Protected native tools retain their authority checks in support/protected.rs.
fn app_transport_preserves_native_tools_drafts_and_catalogue() {
    let root = scratch("relay-turn");
    let codex = agent::runtime("codex").unwrap();
    let instructions = root.join("instructions.txt");
    let mcp = root.join("mcp.json");
    std::fs::write(&instructions, "private rules").unwrap();
    std::fs::write(&mcp, json!({"mcpServers":{"relay":{"command":"invented-relay","args":[],"env":{"TOKEN":"invented-only"}}}}).to_string()).unwrap();
    let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let ctx = Context { product:"fixtureq", version:"1", project:"other", task:None, route:None, account:"default" };
    let mut session: Option<String> = None;
    for _ in 0..2 {
        let mut l = fake_launch("codex", &root, "private request", &[("COREQ_FAKE_APP_SERVER","1".as_ref()),("AGENTQ_FAKE_ONE_TURN","1".as_ref())]);
        l.deltas = true; l.instructions = Some(instructions.clone()); l.mcp = Some(mcp.clone());
        l.policy.write = agent::Scope::None; l.policy.approval = agent::Approval::Deny; l.policy.isolated = true;
        let mut run = match &session { None => codex.launch(&l), Some(s) => codex.resume(s, &l) }.unwrap();
        let mut events = vec![];
        let until = Instant::now() + Duration::from_secs(10);
        while codex.poll(&mut run).unwrap() == agent::State::Running {
            events.extend(codex.progress(&mut run).unwrap());
            assert!(Instant::now() < until); std::thread::sleep(Duration::from_millis(10));
        }
        events.extend(codex.progress(&mut run).unwrap());
        assert!(events.iter().any(|e| matches!(e, agent::Event::Text { delta, .. } if delta == "private draft")));
        assert!(events.iter().any(|e| matches!(e, agent::Event::Message { text, .. } if text == "final answer")));
        assert_eq!(codex.report(&run).unwrap().text.as_deref(), Some("final answer"));
        assert!(codex.progress(&mut run).unwrap().is_empty());
        assert!(matches!(codex.input(&mut run, &[agent::Part::Text("x".into())]), Err(agent::Failure::Refused(agent::Cause("input-unsupported")))));
        assert!(agent::ProtectedAppServer::requests(&mut run).is_err(), "ordinary turns cannot access the protected protocol");
        run.record(&ctx, &store).unwrap().recorded.unwrap();
        assert!(run.record(&ctx, &store).is_err(), "one observation per owned turn");
        session = codex.session_id(&run);
        let saved = codex.checkpoint(&run).unwrap();
        let cold = codex.recover(&saved, &l).unwrap();
        assert_eq!(codex.report(&cold).unwrap().text.as_deref(), Some("final answer"));
    }
    assert_eq!(observations(&store).len(), 2);
    let requests = std::fs::read_to_string(root.join("app-input.jsonl")).unwrap();
    let rows: Vec<Value> = requests.lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    let threads: Vec<_> = rows.iter().filter(|v| matches!(v["method"].as_str(),Some("thread/start"|"thread/resume"))).collect();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0]["params"]["config"]["mcp_servers"]["relay"]["env"]["TOKEN"], "invented-only");
    assert_eq!(threads[1]["params"]["excludeTurns"], true);
    assert!(rows.iter().filter(|v|v["id"]==999).all(|v|v["error"]["code"]==-32601));
    let metadata = records(&store).iter().map(Value::to_string).collect::<String>();
    for private in ["private draft", "private request", "invented-only", "final answer"] { assert!(!metadata.contains(private)); }
}

// #253 A2/A3: a thread/resume reply over the bound and a stream cut inside a record read as different failures before any
// input; `excludeTurns` keeps a long native history inside the bound. The bound counts bytes, so a valid character across it
// still reads as size, not invalid UTF-8. Risk: both collapse into one message again, the record bound drifts, decoding runs
// before the byte checks, or the ordinary resume starts hydrating history.
fn app_resume_frames_stay_bounded_and_named() {
    let codex = agent::runtime("codex").unwrap();
    for (frame, refusal) in [("native", None), ("exact", None), ("exact-utf8", None),
        ("over", Some("app-server record exceeds 8 MiB")), ("over-utf8", Some("app-server record exceeds 8 MiB")),
        ("cut", Some("app-server stream ended inside a record after ")), ("cut-utf8", Some("app-server stream ended inside a record after ")),
        ("invalid", Some("app-server record invalid"))] {
        let root = scratch(&format!("resume-frame-{frame}"));
        std::fs::write(root.join("app-native-thread"), "native-thread").unwrap();
        let mut l = fake_launch("codex", &root, "private request", &[("COREQ_FAKE_APP_SERVER","1".as_ref()),("AGENTQ_FAKE_ONE_TURN","1".as_ref()),("AGENTQ_FAKE_RESUME_FRAME",frame.as_ref())]);
        l.deltas = true;
        // The fake server still appends while a run is open, so requests are read only after it ended.
        let requests = || -> Vec<Value> { std::fs::read_to_string(root.join("app-input.jsonl")).unwrap().lines().map(|l|serde_json::from_str(l).unwrap()).collect() };
        match (codex.resume("native-thread", &l), refusal) {
            (Ok(mut run), None) => assert_eq!(finish(codex, &mut run), agent::State::Finished, "{frame}"),
            (Err(e), Some(refusal)) => {
                let e = e.to_string();
                assert!(e.contains(refusal), "{frame}: {e}");
                assert!(!requests().iter().any(|v|v["method"]=="turn/start"), "{frame}: no input follows a failed open");
            }
            (opened, _) => panic!("{frame}: {:?}", opened.map(|_| "opened").map_err(|e| e.to_string())),
        }
        assert_eq!(requests().iter().find(|v|v["method"]=="thread/resume").unwrap()["params"]["excludeTurns"], true, "{frame}");
    }
}

// #246 A1 and retained #217 protection: ordered images are submitted exactly once; invalid input never spawns.
fn app_input_forwards_images_once_and_reads_account_only() {
    let root = scratch("parts-turn");
    let codex = agent::runtime("codex").unwrap();
    let png = root.join("ref.png"); std::fs::write(&png,b"invented image bytes").unwrap();
    let mut l = fake_launch("codex", &root, "unused", &[("COREQ_FAKE_APP_SERVER","1".as_ref()),("AGENTQ_FAKE_ONE_TURN","1".as_ref())]);
    l.parts = vec![agent::Part::Image("relative.png".into())];
    assert!(codex.launch(&l).is_err()); assert!(!root.join("app-input.jsonl").exists());
    l.parts = vec![agent::Part::Text("describe".into()), agent::Part::Image(png.clone())];
    let mut run = codex.launch(&l).unwrap();
    let until = Instant::now()+Duration::from_secs(10);
    while codex.poll(&mut run).unwrap()==agent::State::Running { assert!(Instant::now()<until); std::thread::sleep(Duration::from_millis(10)); }
    let saved = codex.checkpoint(&run).unwrap(); let mut cold = codex.recover(&saved,&l).unwrap();
    assert!(codex.input(&mut cold,&l.parts).is_err()); assert!(codex.account(&mut cold).is_err());
    let rows:Vec<Value>=std::fs::read_to_string(root.join("app-input.jsonl")).unwrap().lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    let turns:Vec<_>=rows.iter().filter(|v|v["method"]=="turn/start").collect();
    assert_eq!(turns.len(),1); assert_eq!(turns[0]["params"]["input"],json!([{"type":"text","text":"describe"},{"type":"localImage","path":png}]));
}

// ---------- helpers ----------

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("coreq-agent-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn record(root: &Path, turn: u64) -> Value { serde_json::from_str(&std::fs::read_to_string(root.join(format!("fake-turn-{turn}.json"))).unwrap()).unwrap() }

fn observations(store: &coreq::telemetry::Store) -> Vec<Value> {
    records(store).into_iter().filter(|o| o["kind"] == "agent.turn").collect()
}

fn records(store: &coreq::telemetry::Store) -> Vec<Value> {
    store.export(0, 0, 100).unwrap()["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()).collect()
}

fn exe() -> std::ffi::OsString { std::env::current_exe().unwrap().into() }

/// A launch of the fake `name` client through the trait; `env` adds fake switches and the account home.
fn fake_launch(name: &str, root: &Path, input: &str, env: &[(&str, &std::ffi::OsStr)]) -> agent::Launch {
    let mut l = agent::Launch::new(root, root.join("worker.log"), input, "fixtureq");
    l.command = vec![exe()];
    l.env = std::iter::once(("COREQ_FAKE_AGENT".into(), name.into())).chain(env.iter().map(|(k, v)| (k.into(), v.into()))).collect();
    l
}

fn finish(r: &dyn agent::Runtime, run: &mut agent::Run) -> agent::State {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let s = r.poll(run).unwrap();
        if s != agent::State::Running { return s; }
        assert!(Instant::now() < until, "run ended");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn control(root: &Path) -> Value { serde_json::from_str(&std::fs::read_to_string(root.join("native-control.json")).unwrap()).unwrap() }

// R48/R49/R36 through the trait: job discovery, stop acknowledgment, native readiness and large history cannot retire a
// background or queue run, and checkpoints taken before or after discovery recover it without resubmission.
// Risk: account/root confusion stops foreign work, or a bounded reader repeats input / accepts a stale terminal.
fn native_control_readiness_and_large_history_stay_owned() {
    use agent::State::{Finished, Running};
    let root=scratch("native-control");
    let account=root.join("account");
    let claude=agent::runtime("claude").unwrap();
    let row=json!({"id":"c37eb75a","kind":"background","cwd":root,"sessionId":"native-session","state":"working","pid":42});
    let rows=json!([row]).to_string();
    let mut l=fake_launch("claude",&root,"owned",&[("COREQ_FAKE_BACKGROUND","1".as_ref()),("COREQ_FAKE_CONTROL","1".as_ref()),
        ("FAKE_JOB_ROWS",rows.as_ref()),("CLAUDE_CONFIG_DIR",account.as_os_str())]);
    l.mode=agent::Mode::Background;
    let mut run=claude.launch(&l).unwrap();
    assert_eq!((claude.session_id(&run),claude.transcript(&run).unwrap_err()),(None,agent::Cause("transcript-unknown")),"launcher output is not a session");
    let early=claude.checkpoint(&run).unwrap();
    assert_eq!(claude.poll(&mut run).unwrap(),Running,"readiness without native history is not completion");
    assert_eq!(control(&root)["args"],json!(["agents","--json","--all","--cwd",root]));assert_eq!(control(&root)["home"],json!(account));
    assert_eq!(claude.session_id(&run).as_deref(),Some("native-session"));
    let history=claude.transcript(&run).unwrap();
    assert!(history.starts_with(account.join("projects")) && history.ends_with("native-session.jsonl"),"{}",history.display());
    let saved=claude.checkpoint(&run).unwrap();
    let mut cold=claude.recover(&saved,&l).unwrap();
    assert!(!claude.cancel(&mut cold).unwrap(),"a stop request never proves exit");
    assert_eq!(control(&root)["args"],json!(["stop","c37eb75a"]));
    assert_eq!(claude.poll(&mut cold).unwrap(),Running,"stop acknowledgment without native history never becomes completion");
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    append(&history,&[json!({"type":"user","sessionId":"native-session","message":{"content":"owned"}})]);
    // Multiple bounded passes, each individual native record below the record bound.
    let filler=json!({"type":"progress","data":"x".repeat(1024*1024)});
    append(&history,&vec![filler;9]);
    append(&history,&[json!({"type":"system","subtype":"turn_duration","sessionId":"native-session"})]);
    assert_eq!(claude.poll(&mut cold).unwrap(),Running,"scan budget is pending, never an early success");
    assert_eq!(claude.poll(&mut cold).unwrap(),Finished);
    assert_eq!(claude.report(&cold).unwrap().outcome,"ok");
    let mut before=claude.recover(&early,&l).unwrap();
    assert_eq!(finish(claude,&mut before),Finished,"a checkpoint from before discovery rediscovers the same job");
    let mut changed=l.clone();changed.policy.approval=agent::Approval::Auto;assert!(claude.recover(&saved,&changed).is_err());
    let late=claude.checkpoint(&cold).unwrap();
    let replacement=root.join("replacement.jsonl");std::fs::copy(&history,&replacement).unwrap();
    std::fs::remove_file(&history).unwrap();std::fs::rename(&replacement,&history).unwrap();
    assert!(claude.poll(&mut claude.recover(&late,&l).unwrap()).is_err(),"a copied transcript has another file identity");
    let codex=agent::runtime("codex").unwrap();
    let path=root.join("app.jsonl");
    append(&path,&[json!({"type":"session_meta","payload":{"id":"native-thread","cwd":root}})]);
    let mut q=fake_launch("codex",&root,"caller input reference",&[("COREQ_FAKE_CONTROL","1".as_ref()),("CODEX_HOME",account.as_os_str())]);
    q.mode=agent::Mode::Queue {transcript:path.clone()};
    assert!(codex.launch(&q).is_err(),"queue needs a running thread");
    let mut app=codex.resume("native-thread",&q).unwrap();
    assert_eq!(control(&root)["args"],json!(["queue","--thread","native-thread","--message","caller input reference"]));assert_eq!(control(&root)["home"],json!(account));
    let usage=json!({"type":"event_msg","payload":{"type":"token_count","turn_id":"native-turn","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":5,"output_tokens":2},"total_token_usage":{"input_tokens":110,"cached_input_tokens":55,"output_tokens":12}}}});
    append(&path,&[json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"native-turn"}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","thread_id":"native-thread","turn_id":"native-turn","item":{"type":"UserMessage","content":[{"type":"text","text":"caller input reference\n"}]}}}),
        usage.clone(),usage,
        json!({"type":"event_msg","payload":{"type":"agent_message","message":"app answer"}}),
        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"native-turn"}})]);
    assert_eq!(codex.poll(&mut app).unwrap(),Finished);
    let report=codex.report(&app).unwrap();assert_eq!(report.usage.input,Some(5));assert_eq!(report.text.as_deref(),Some("app answer"));
    assert!(codex.progress(&mut app).unwrap().len()==6 && codex.progress(&mut app).unwrap().is_empty(),"progress reads the native transcript once");
    assert!(codex.cancel(&mut app).unwrap_err().starts_with("cancel-unsupported"));
    assert!(codex.resume("unsafe & command",&q).is_err());
    assert_eq!(claude.resume("native-thread",&q).err().as_deref(),Some("mode app-queue unsupported by claude"));
    std::fs::remove_dir_all(root).unwrap();
}

fn soon() -> Option<Instant> { Some(Instant::now() + Duration::from_secs(30)) }

// R47/R48: the native background path maps the abstract launch to start-only flags and keeps the resume argv exact.
// Risk: workers silently move to print mode or resume with changed saved permissions/rules.
fn background_commands_preserve_native_mode() {
    let root = scratch("background-command");
    let account = root.join("account");
    let claude = agent::runtime("claude").unwrap();
    let row = json!([{"id":"c37eb75a","kind":"background","cwd":root,"sessionId":"native-session","state":"idle","pid":null}]).to_string();
    let mut l = fake_launch("claude", &root, "read caller-owned input file", &[("COREQ_FAKE_BACKGROUND","1".as_ref()),("COREQ_FAKE_CONTROL","1".as_ref()),
        ("FAKE_JOB_ROWS",row.as_ref()),("CLAUDE_CONFIG_DIR",account.as_os_str()),("FAKE_EXTRA","preserved".as_ref())]);
    l.mode = agent::Mode::Background; l.title = Some("fixture name".into()); l.policy.approval = agent::Approval::Auto; l.compact_hook = Some("rules".into());
    // SAFETY: this harness runs its tests one at a time on the main thread.
    unsafe { std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent"); }
    let mut run = claude.launch(&l).unwrap();
    unsafe { std::env::remove_var("CLAUDE_CODE_SESSION_ID"); }
    let record:Value = serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap();
    let args: Vec<_> = record["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect();
    assert_eq!(&args[..7], ["--bg","--permission-mode","auto","--name","fixture name","--remote-control","fixture name"]);
    assert_eq!((args[7], args.len()), ("--settings", 9));
    assert_eq!(record["input"],"read caller-owned input file");
    assert_eq!(record["home"],json!(account));
    assert_eq!(record["inherited"],false); assert_eq!(record["extra"],"preserved");
    assert_eq!(claude.checkpoint(&run).unwrap()["job"], "c37eb75a", "the launcher names the job");
    claude.poll(&mut run).unwrap();
    // #213 A2/A3: a background run names the Claude log cause, and its recovered run reads the job's URL without a launch.
    let seen = claude.observe(&mut run);
    assert_eq!((seen.session.as_deref(), seen.cause, seen.url), (Some("native-session"), Some(agent::Cause("unlogged")), None));
    std::fs::create_dir_all(account.join("debug")).unwrap();
    std::fs::write(account.join("debug/native-session.txt"), "[bridge:repl] Init failed: Remote Control requires a claude.ai subscription.; consecutive failures: 1\n").unwrap();
    let seen = claude.observe(&mut run);
    assert_eq!((seen.cause, seen.detail.as_deref()), (Some(agent::Cause("disabled")), Some("Remote Control requires a claude.ai subscription.")));
    let mut recovered = claude.recover(&claude.checkpoint(&run).unwrap(), &l).unwrap();
    std::fs::create_dir_all(account.join("jobs/c37eb75a")).unwrap();
    std::fs::write(account.join("jobs/c37eb75a/state.json"), json!({"bridgeSessionId":"cse_01Recovered"}).to_string()).unwrap();
    let seen = claude.observe(&mut recovered);
    assert_eq!((seen.url.as_deref(), seen.cause), (Some("https://claude.ai/code/session_01Recovered"), None));
    let history = claude.transcript(&run).unwrap();
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    append(&history, &[json!({"type":"system","subtype":"init","sessionId":"native-session"})]);
    claude.resume("native-session", &l).unwrap();
    let record:Value = serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap();
    assert_eq!(record["args"],json!(["--bg","--resume","native-session"]));
    assert_eq!(agent::runtime("codex").unwrap().launch(&l).err().as_deref(), Some("mode background unsupported by codex"));
    std::fs::remove_dir_all(root).unwrap();
}

// #240 A1: host-configured Codex arguments sit in the Arguments position of start and resume, and only there.
// Risk: TaskQ keeps a native launch path, or host flags silently override a Policy the adapter claims to enforce.
fn host_args_reach_codex_turns_only() {
    let root = scratch("host-args");
    let home = root.join("home");
    let codex = agent::runtime("codex").unwrap();
    let mut l = fake_launch("codex", &root, "x", &[("CODEX_HOME", home.as_os_str())]);
    l.host_args = ["--sandbox", "workspace-write", "-c", "approval_policy=never"].map(Into::into).to_vec();
    let mut run = codex.launch(&l).unwrap();
    finish(codex, &mut run);
    assert_eq!(record(&root, 1)["args"], json!(["exec", "--json", "--sandbox", "workspace-write", "-c", "approval_policy=never", "-C", root, "-"]));
    let thread = codex.session_id(&run).unwrap();
    let mut next = codex.resume(&thread, &l).unwrap();
    finish(codex, &mut next);
    assert_eq!(record(&root, 2)["args"], json!(["exec", "--json", "--sandbox", "workspace-write", "-c", "approval_policy=never", "resume", thread, "-"]));
    let refused = |l: &agent::Launch, name: &str| agent::runtime(name).unwrap().launch(l).err();
    let mut policy = l.clone();
    policy.policy.write = agent::Scope::Project;
    assert_eq!(refused(&policy, "codex").as_deref(), Some("host-args require the native policy on codex"));
    assert_eq!(refused(&l, "claude").as_deref(), Some("host-args unsupported by claude"));
    assert_eq!(refused(&l, "antigravity").as_deref(), Some("host-args unsupported by antigravity"));
    let mut queue = l.clone();
    queue.mode = agent::Mode::Queue { transcript: root.join("t.jsonl") };
    assert_eq!(codex.resume(&thread, &queue).err().as_deref(), Some("host-args unsupported by codex"));
    assert!(!root.join("native-control.json").exists(), "a refusal starts nothing");
    std::fs::remove_dir_all(root).unwrap();
}

fn settings_of(args: &Value) -> Value {
    let args = args.as_array().unwrap();
    let at: Vec<_> = args.iter().enumerate().filter(|(_, a)| *a == "--settings").map(|(i, _)| i).collect();
    assert_eq!(at.len(), 1, "one --settings");
    serde_json::from_str(args[at[0] + 1].as_str().unwrap()).unwrap()
}

// #240 A1: caller settings reach the session: Claude through one `--settings` that survives the daemon, Codex through its child.
// Risk: a `--bg` session misses TaskQ's variables or `off`, a second `--settings` drops the compact hook, or settings values leak into child env.
fn settings_env_reaches_the_session() {
    use agentq_runtime::optimization::Intent;
    // This fixture supplies its own launcher identity; inherited TaskQ identity has a separate regression below.
    let task = std::env::var_os("TASKQ_TASK");
    unsafe { std::env::remove_var("TASKQ_TASK"); }
    let root = scratch("settings-env");
    let account = root.join("account");
    let claude = agent::runtime("claude").unwrap();
    // An explicit child marker: `prepare` also reads the test process's own environment, which a worker sets.
    let mut l = fake_launch("claude", &root, "x", &[("CLAUDE_CONFIG_DIR", account.as_os_str()), ("AGENTQ_LAUNCHER", "child".as_ref())]);
    l.settings_env = vec![("FAKE_EXTRA".into(), "settings".into()), ("AGENTQ_LAUNCHER".into(), "caller".into())];
    l.compact_hook = Some("rules".into());
    l.optimization = Intent::Off;
    let mut run = claude.launch(&l).unwrap();
    finish(claude, &mut run);
    let turn = record(&root, 1);
    let settings = settings_of(&turn["args"]);
    assert_eq!(settings["env"], json!({"FAKE_EXTRA": "settings", "AGENTQ_LAUNCHER": "child", "AGENTQ_OPTIMIZATION": "off"}), "the optimizer's own values win");
    assert_eq!(settings["hooks"]["SessionStart"][0]["hooks"][0]["command"], "rules");
    assert_eq!(turn["extra"], Value::Null, "settings values stay out of the child environment");

    let mut bg = fake_launch("claude", &root, "reference", &[("COREQ_FAKE_BACKGROUND", "1".as_ref()), ("COREQ_FAKE_CONTROL", "1".as_ref()), ("FAKE_JOB_ROWS", "[]".as_ref()), ("CLAUDE_CONFIG_DIR", account.as_os_str()), ("AGENTQ_LAUNCHER", "child".as_ref())]);
    bg.mode = agent::Mode::Background;
    bg.optimization = Intent::Off;
    bg.settings_env = vec![("FAKE_EXTRA".into(), "settings".into())];
    claude.launch(&bg).unwrap();
    let launched: Value = serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap();
    assert_eq!(settings_of(&launched["args"])["env"], json!({"FAKE_EXTRA": "settings", "AGENTQ_LAUNCHER": "child", "AGENTQ_OPTIMIZATION": "off"}), "off reaches the daemon's session");
    bg.settings_env.clear();
    claude.launch(&bg).unwrap();
    let plain: Value = serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap();
    assert_eq!(plain["args"], json!(["--bg"]), "a launch without settings keeps its native arguments");

    let codex = agent::runtime("codex").unwrap();
    let home = root.join("codex-home");
    let codex_root = scratch("settings-env-codex");
    let mut c = fake_launch("codex", &codex_root, "x", &[("CODEX_HOME", home.as_os_str()), ("FAKE_EXTRA", "child".as_ref())]);
    c.settings_env = vec![("FAKE_EXTRA".into(), "settings".into())];
    let mut run = codex.launch(&c).unwrap();
    finish(codex, &mut run);
    let turn = record(&codex_root, 1);
    assert_eq!(turn["extra"], "settings", "a settings value wins over the child value");
    assert!(!turn["args"].as_array().unwrap().iter().any(|a| a == "--settings"), "Codex has no settings flag");
    let mut queue = c.clone();
    queue.mode = agent::Mode::Queue { transcript: codex_root.join("t.jsonl") };
    assert_eq!(codex.resume(&codex.session_id(&run).unwrap(), &queue).err().as_deref(), Some("settings-env unsupported by codex"));
    assert_eq!(agent::runtime("antigravity").unwrap().launch(&c).err().as_deref(), Some("settings-env unsupported by antigravity"));
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(codex_root).unwrap();
    if let Some(task) = task { unsafe { std::env::set_var("TASKQ_TASK", task); } }
}

// #240 A1 / M12.10: settings must not restore API auth after subscription policy cleared the child.
fn settings_env_preserves_subscription_policy() {
    let root = scratch("settings-subscription");
    let claude = agent::runtime("claude").unwrap();
    for mode in [agent::Mode::Turn, agent::Mode::Background] {
        let mut l = fake_launch("claude", &root, "x", &[]);
        l.mode = mode.clone();
        if mode == agent::Mode::Background { l.env.push(("COREQ_FAKE_BACKGROUND".into(), "1".into())); }
        l.policy.subscription = true;
        l.settings_env = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"].map(|k| (k.into(), "invented-fixture".into())).to_vec();
        let mut run = claude.launch(&l).unwrap();
        let turn = if mode == agent::Mode::Turn {
            finish(claude, &mut run);
            record(&root, 1)
        } else { serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap() };
        let settings = settings_of(&turn["args"]);
        for key in ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] {
            assert_eq!(settings["env"][key], "", "subscription policy wins over settings: {mode:?} {key}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

// #240 A1 / M12.5-M12.7: settings off must be observed before installed state and reach both native sessions.
fn settings_env_off_precedes_state_read() {
    for name in ["claude", "codex"] {
        let root = scratch(&format!("settings-off-{name}"));
        let home = root.join("home");
        let install = root.join("install");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(install.join("data")).unwrap();
        std::fs::write(home.join("config.toml"), "model_provider = \"agentq\"\n").unwrap();
        std::fs::write(home.join("settings.json"), r#"{"hooks":{"PreToolUse":[{"hooks":[{"command":"/i/agentq hook claude PreToolUse"}]}]}}"#).unwrap();
        let runtime = agent::runtime(name).unwrap();
        let mut l = fake_launch(name, &root, "x", &[("AGENTQ_OPTIMIZATION", "on".as_ref())]);
        l.home = Some(home);
        l.install = Some(install.clone());
        l.settings_env = vec![("AGENTQ_OPTIMIZATION".into(), "off".into())];
        for invalid in [false, true] {
            if invalid { std::fs::write(install.join("data/config.json"), "not JSON").unwrap(); }
            let mut run = runtime.launch(&l).unwrap();
            finish(runtime, &mut run);
            assert_eq!((run.optimization().effective, run.optimization().reason), ("off", "launch_off"), "{name} invalid={invalid}");
            let turn = record(&root, 1);
            assert_eq!(turn["optimization"], "off");
            if name == "claude" { assert_eq!(settings_of(&turn["args"])["env"]["AGENTQ_OPTIMIZATION"], "off"); }
            else { assert!(turn["args"].as_array().unwrap().iter().any(|a| a == "model_providers.agentq.http_headers.x-agentq-optimization=\"off\"")); }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

// #240 A1 / M12.4, M6.9: a daemon session must retain inherited launcher identity to prevent double counting.
fn settings_env_keeps_inherited_launch_markers() {
    let keys = ["TASKQ_TASK", "AGENTQ_LAUNCHER", "AGENTQ_OPTIMIZATION"];
    let saved = keys.map(|k| (k, std::env::var_os(k)));
    for key in keys { unsafe { std::env::remove_var(key); } }
    let root = scratch("settings-markers");
    let claude = agent::runtime("claude").unwrap();
    for key in ["TASKQ_TASK", "AGENTQ_LAUNCHER"] {
        unsafe { std::env::set_var(key, "inherited-fixture"); }
        let mut l = fake_launch("claude", &root, "x", &[("COREQ_FAKE_BACKGROUND", "1".as_ref())]);
        l.mode = agent::Mode::Background;
        l.settings_env = vec![(key.into(), "caller-fixture".into())];
        claude.launch(&l).unwrap();
        let turn: Value = serde_json::from_str(&std::fs::read_to_string(root.join("background-command.json")).unwrap()).unwrap();
        assert_eq!(settings_of(&turn["args"])["env"][key], "inherited-fixture");
        unsafe { std::env::remove_var(key); }
    }
    for (key, value) in saved { unsafe { match value { Some(v) => std::env::set_var(key, v), None => std::env::remove_var(key) } } }
    std::fs::remove_dir_all(root).unwrap();
}

// #240 A1, TaskQ #1555/#1557: the caller reads the launcher's own stdout, stderr and status from a launched run, once.
// Risk: a refused launch reads only as an uncertain fault, so a confirmed no-start blocks its task forever.
fn launcher_output_is_handed_over_once() {
    use coreq::process::Failure;
    let root = scratch("launcher-output");
    let account = root.join("account");
    let claude = agent::runtime("claude").unwrap();
    let codex = agent::runtime("codex").unwrap();
    let env = [("COREQ_FAKE_BACKGROUND", "1".as_ref()), ("COREQ_FAKE_CONTROL", "1".as_ref()), ("FAKE_JOB_ROWS", "[]".as_ref()), ("CLAUDE_CONFIG_DIR", account.as_os_str())];
    let mut l = fake_launch("claude", &root, "reference", &env);
    l.mode = agent::Mode::Background;
    let mut run = claude.launch(&l).unwrap();
    let saved = claude.checkpoint(&run).unwrap();
    let out = claude.launcher_output(&mut run).unwrap().unwrap();
    assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).starts_with("backgrounded · c37eb75a"));
    assert_eq!(claude.launcher_output(&mut run).err(), Some(agent::Cause("launcher-output-unavailable")), "handed over once");
    assert_eq!(codex.launcher_output(&mut run).err(), Some(agent::Cause("foreign-run")));
    let mut recovered = claude.recover(&saved, &l).unwrap();
    assert_eq!(claude.launcher_output(&mut recovered).err(), Some(agent::Cause("launcher-output-unavailable")), "not part of the checkpoint");

    l.env.push(("FAKE_LAUNCH_REFUSED".into(), "1".into()));
    let mut refused = claude.launch(&l).unwrap();
    let out = match claude.launcher_output(&mut refused).unwrap() { Ok(o) | Err(Failure::Exit(o)) => o, Err(e) => panic!("launcher exchange: {e}") };
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!((out.status.success(), out.stdout.is_empty(), claude.launch_refusal(&stderr)), (false, true, Some(agent::Cause("workspace-untrusted"))));
    assert_eq!(claude.poll(&mut refused).unwrap(), agent::State::Unknown, "the run itself never resubmits");

    let mut turn = fake_launch("claude", &root, "x", &[("CLAUDE_CONFIG_DIR", account.as_os_str())]);
    turn.log = root.join("turn.log");
    let mut run = claude.launch(&turn).unwrap();
    finish(claude, &mut run);
    assert_eq!(claude.launcher_output(&mut run).err(), Some(agent::Cause("no-launcher")));
    let codex_root = scratch("launcher-output-codex");
    let mut run = codex.launch(&fake_launch("codex", &codex_root, "x", &[("CODEX_HOME", codex_root.join("home").as_os_str())])).unwrap();
    finish(codex, &mut run);
    assert_eq!(codex.launcher_output(&mut run).err(), Some(agent::Cause("no-launcher")));
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(codex_root).unwrap();
}

// #222 A1: Claude's `agents --cwd <root>` lists nothing for a root under `.claude/worktrees`.
// Risk: the job stays unattached, so observe reads session-unknown and cancel finds no job; or a foreign job attaches.
fn background_job_found_when_cwd_listing_misses_it() {
    let base=scratch("cwd-miss");
    let root=base.join(".claude").join("worktrees").join("w1");
    std::fs::create_dir_all(&root).unwrap();
    let claude=agent::runtime("claude").unwrap();
    let foreign=json!({"id":"0badc0de","kind":"background","cwd":root,"sessionId":"foreign-session","state":"working","pid":7});
    let row=json!({"id":"c37eb75a","kind":"background","cwd":root,"sessionId":"native-session","state":"working","pid":42});
    let rows=json!([foreign,row]).to_string();
    let mut l=fake_launch("claude",&root,"owned",&[("COREQ_FAKE_BACKGROUND","1".as_ref()),("COREQ_FAKE_CONTROL","1".as_ref()),
        ("FAKE_CWD_FILTER_EMPTY","1".as_ref()),("FAKE_JOB_ROWS",rows.as_ref()),("CLAUDE_CONFIG_DIR",root.join("account").as_os_str())]);
    l.mode=agent::Mode::Background;
    let mut run=claude.launch(&l).unwrap();
    assert_eq!(claude.poll(&mut run).unwrap(),agent::State::Running);
    assert_eq!(control(&root)["args"],json!(["agents","--json","--all"]),"a filtered miss retries the unfiltered listing");
    assert_eq!(claude.session_id(&run).as_deref(),Some("native-session"),"attached by exact job id and cwd");
    assert!(!claude.cancel(&mut run).unwrap());
    assert_eq!(control(&root)["args"],json!(["stop","c37eb75a"]),"cancel stops the attached job");
    std::fs::remove_dir_all(base).unwrap();
}

// #213 A3 / #216 native proof, not CI: launches one real Claude background job with Remote Control through the trait, reads
// its URL through `observe`, then rebuilds the run from its checkpoint and reads the same URL without a launch.
// Run: AGENTQ_LIVE_CLAUDE_BACKGROUND=1 cargo test --locked -p agentq-runtime --test agent
fn live_claude_background_link() {
    let claude = agent::runtime("claude").unwrap();
    println!("claude version: {:?}", claude.ready(None, None).version);
    // A trusted project folder: Claude refuses `--bg` in an untrusted workspace, and accepting trust edits client configuration.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("target").join(format!("live-bg-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut l = agent::Launch::new(&root, root.join("caller.log"), "Reply with the single word OK and stop.", "agentq-live");
    l.mode = agent::Mode::Background;
    l.title = Some("agentq live 216 remote control".into());
    l.model = Some("haiku".into());
    let mut run = claude.launch(&l).unwrap();
    let until = Instant::now() + Duration::from_secs(180);
    let seen = loop {
        let seen = claude.observe(&mut run);
        println!("observe: {seen:?}");
        if seen.url.is_some() || Instant::now() >= until { break seen; }
        std::thread::sleep(Duration::from_secs(5));
    };
    let saved = claude.checkpoint(&run).unwrap();
    println!("checkpoint: {saved}");
    drop(run);
    let mut recovered = claude.recover(&saved, &l).unwrap();
    let again = claude.observe(&mut recovered);
    println!("recovered observe: {again:?}");
    let stopped = claude.cancel(&mut recovered);
    println!("stop: {stopped:?}");
    let url = seen.url.expect("a live Remote Control background job names its URL");
    assert!(url.starts_with("https://claude.ai/code/session_"), "{url}");
    assert_eq!(again.url.as_deref(), Some(url.as_str()), "the recovered run reads the same URL without a launch");
    println!("live result: ok {url}");
}

// ---------- tests ----------

// Chain: coreq.md R47-R50, task A2-A4: one Claude session per conversation, one process per turn, resumed by session
// ID; each turn appends one observation with tokens, model, limit and duration.
// Risk: a resume opens a second session, the parent's session leaks into the child, or a turn goes unobserved.
fn claude_starts_and_resumes_one_session() {
    let root = scratch("claude");
    let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let claude = agent::runtime("claude").unwrap();
    let mut l = fake_launch("claude", &root, "brief", &[("FAKE_EXTRA", "x".as_ref())]);
    l.log = root.join("logs").join("T1-1.log");
    l.policy.approval = agent::Approval::Auto;
    let ctx = Context { product: "fixtureq", version: "1.0.0", project: "taskqorg/coreq", task: Some((1, 1)), route: None, account: "default" };
    // SAFETY: this harness runs its tests one at a time on the main thread.
    unsafe { std::env::set_var("CLAUDE_CODE_SESSION_ID", "parent"); }
    let mut run = claude.launch(&l).unwrap();
    finish(claude, &mut run);
    let first = run.record(&ctx, &store).unwrap();
    let session = claude.session_id(&run).expect("caller-chosen session");
    assert_eq!((first.outcome, first.session.as_deref(), first.text.as_deref(), first.exit), ("ok", Some(session.as_str()), Some("turn 1"), Some(0)));
    first.recorded.as_ref().unwrap();
    let r = record(&root, 1);
    assert_eq!((r["input"].as_str(), r["inherited"].as_bool(), r["extra"].as_str()), (Some("brief"), Some(false), Some("x")), "stdin brief, no parent session, caller env");
    l.input = "next".into();
    let mut next = claude.resume(&session, &l).unwrap();
    finish(claude, &mut next);
    unsafe { std::env::remove_var("CLAUDE_CODE_SESSION_ID"); }
    let second = next.record(&ctx, &store).unwrap();
    assert_eq!((second.outcome, second.text.as_deref()), ("ok", Some("turn 2")), "the resume continues the same session");
    assert!(record(&root, 2)["args"].as_array().unwrap().iter().any(|a| a == "--resume"));
    assert_eq!(second.usage.input, Some(3), "a resume reads only its own turn from the shared log");
    let seen = observations(&store);
    assert_eq!(seen.len(), 2, "one observation per turn");
    let o = &seen[1];
    assert_eq!((o["kind"].as_str(), o["runtime"].as_str(), o["model"].as_str(), o["task"].as_u64()), (Some("agent.turn"), Some("claude"), Some("claude-opus-5-5"), Some(1)));
    assert_eq!((o["input_tokens"].as_u64(), o["cache_read_tokens"].as_u64(), o["cache_write_tokens"].as_u64(), o["output_tokens"].as_u64()), (Some(3), Some(100), Some(20), Some(7)));
    assert_eq!((o["limit_used_percent"].as_u64(), o["limit_resets_at"].as_u64(), o["limit_window"].as_str()), (Some(25), Some(1792386000), Some("five_hour")));
    assert!(o["duration_us"].as_u64().is_some());
    // A2, A3: a real child turn through `Run::record`. Risk: the fields exist in unit tests but never reach the store.
    assert_eq!((o["schema"].as_u64(), o["exit_code"].as_u64(), o["session_id"].as_str().is_some()), (Some(1), Some(0), true));
    let measured = o["peak_rss_bytes"].as_u64().is_some() && o["user_cpu_us"].as_u64().is_some() && o["system_cpu_us"].as_u64().is_some();
    assert!(measured || o["unknown"]["peak_rss_bytes"] == "not-sampled", "{o}");
    let calls: Vec<_> = records(&store).into_iter().filter(|r| r["kind"] == "agent.call").collect();
    assert!(calls.len() >= 2, "each turn's model requests are recorded: {calls:?}");
    let last = calls.last().unwrap();
    assert_eq!((last["runtime"].as_str(), last["model"].as_str(), last["session_id"].clone(), last["task"].as_u64()), (Some("claude"), Some("claude-opus-5-5"), o["session_id"].clone(), Some(1)));
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

// Chain: coreq.md R48, R50, R51: Codex names its thread on start; the resume uses that thread and keeps the account's
// login home; the limit snapshot comes from the thread rollout in that home. Risk: a resume lands in another account
// or the snapshot reads another account's limits.
fn codex_resumes_its_thread_in_the_account_home() {
    let root = scratch("codex");
    let home = root.join("codex-home").join("fixtureq-accounts").join("work");
    let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let codex = agent::runtime("codex").unwrap();
    let mut l = fake_launch("codex", &root, "hello", &[("CODEX_HOME", home.as_os_str())]);
    l.log = root.join("T2-1.log");
    l.policy.write = agent::Scope::Project;
    let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: None, route: Some("manager"), account: "work" };
    let mut run = codex.launch(&l).unwrap();
    finish(codex, &mut run);
    let first = run.record(&ctx, &store).unwrap();
    let thread = first.session.clone().expect("thread.started names the thread");
    assert_eq!((first.outcome, first.text.as_deref()), ("ok", Some("turn 1")));
    assert_eq!(record(&root, 1)["home"].as_str().map(PathBuf::from), Some(home.clone()), "the account home reaches the child");
    let args = record(&root, 1)["args"].clone();
    assert_eq!(args, json!(["exec", "--json", "--sandbox", "workspace-write", "-C", root.to_string_lossy(), "-"]));
    l.input = "again".into();
    let mut next = codex.resume(&thread, &l).unwrap();
    finish(codex, &mut next);
    let second = next.record(&ctx, &store).unwrap();
    assert_eq!((second.outcome, second.text.as_deref()), ("ok", Some("turn 2")));
    assert_eq!(record(&root, 2)["args"], json!(["exec", "--json", "--sandbox", "workspace-write", "resume", thread, "-"]));
    assert_eq!((second.limit.used_percent, second.limit.resets_at, second.limit.window.as_deref()), (Some(32), Some(1793700000), Some("300m")), "latest rollout snapshot");
    let o = &observations(&store)[1];
    assert_eq!((o["route"].as_str(), o["account"].as_str(), o["input_tokens"].as_u64(), o["cache_read_tokens"].as_u64()), (Some("manager"), Some("work"), Some(10), Some(40)));
    assert_eq!((o["model"].as_str(), o["unknown"]["model"].as_str(), o["unknown"]["task"].as_str()), (None, Some("not-reported"), Some("no-task")));
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

// Chain: TaskQ CliTurn end detection (unchanged): a turn-end event ends the turn even when the process lingers.
// Risk: a client that keeps running after its turn holds the caller's slot until the deadline.
fn lingering_process_is_stopped_after_its_turn_ends() {
    let root = scratch("linger");
    let codex = agent::runtime("codex").unwrap();
    let mut l = fake_launch("codex", &root, "x", &[("COREQ_FAKE_LINGER", "1".as_ref())]);
    l.deadline = soon();
    let started = Instant::now();
    let mut run = codex.launch(&l).unwrap();
    finish(codex, &mut run);
    assert!(started.elapsed() < Duration::from_secs(20), "stopped after its turn ended, not at the deadline");
    assert_eq!(codex.report(&run).unwrap().outcome, "ok", "the ended turn keeps its outcome");
    let _ = std::fs::remove_dir_all(root);
}

// #210 A1/A3/A4: a caller drives every runtime only through `agent::runtime(name)`: an abstract policy, launch, progress,
// poll, report, resume, session ID, named session-URL causes and one recorded observation, without matching on a runtime.
// Risk: a caller needs native flags or the client enum, progress repeats records, or an unsupported URL reads as success.
fn runtime_trait_runs_every_runtime_by_descriptor() {
    unsafe { std::env::remove_var("TASKQ_TASK"); std::env::remove_var("AGENTQ_LAUNCHER"); std::env::remove_var("AGENTQ_OPTIMIZATION"); }
    for r in agent::runtimes() {
        let d = r.descriptor();
        let root = scratch(&format!("trait-{}", d.name));
        let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
        let mut l = agent::Launch::new(&root, root.join("worker.log"), "brief", "fixtureq");
        l.command = vec![std::env::current_exe().unwrap().into()];
        l.env = vec![("COREQ_FAKE_AGENT".into(), d.name.into())];
        l.policy.approval = agent::Approval::Bypass;
        if d.name == "codex" { l.policy.write = agent::Scope::Any; }
        let before = r.running();
        let mut run = r.launch(&l).unwrap();
        assert_eq!((run.runtime(), r.running()), (d.name, before + 1));
        assert_eq!(r.report(&run).unwrap_err(), "running");
        let mut seen = vec![];
        let deadline = Instant::now() + Duration::from_secs(30);
        let state = loop {
            seen.extend(r.progress(&mut run).unwrap());
            let s = r.poll(&mut run).unwrap();
            if s != agent::State::Running { break s; }
            assert!(Instant::now() < deadline, "{} turn ended", d.name);
            std::thread::sleep(Duration::from_millis(20));
        };
        seen.extend(r.progress(&mut run).unwrap());
        assert_eq!((state, r.running()), (agent::State::Finished, before), "{}", d.name);
        assert!(!seen.is_empty() && r.progress(&mut run).unwrap().is_empty(), "{}: progress reads each record once", d.name);
        let flags = record(&root, 1)["args"].clone();
        let native = match d.name { "claude" => "bypassPermissions", "codex" => "--dangerously-bypass-approvals-and-sandbox", _ => "--dangerously-skip-permissions" };
        assert!(flags.as_array().unwrap().iter().any(|a| a == native), "{}: policy maps to {native}: {flags}", d.name);
        let session = r.session_id(&run).expect("native session named");
        let cause = if d.has("session-url") { "not-requested" } else { "no-session-url" };
        assert_eq!(r.session_url(&run), Err(agent::Cause(cause)), "{}", d.name);
        let seen = r.observe(&mut run);
        assert_eq!((seen.state, seen.error, seen.session.as_deref(), seen.url, seen.cause), (agent::State::Finished, None, Some(session.as_str()), None, Some(agent::Cause(cause))), "{}", d.name);
        assert_eq!(r.transcript(&run).unwrap(), root.join("worker.log"));
        let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: Some((210, 1)), route: None, account: "default" };
        assert_eq!(run.record(&ctx, &store).unwrap().text.as_deref(), Some("turn 1"));
        assert!(run.record(&ctx, &store).is_err(), "one observation per run");
        let mut next = r.resume(&session, &l).unwrap();
        while r.poll(&mut next).unwrap() == agent::State::Running { std::thread::sleep(Duration::from_millis(20)); }
        let second = r.report(&next).unwrap();
        assert_eq!((second.outcome, second.text.as_deref(), second.session.as_deref()), ("ok", Some("turn 2"), Some(session.as_str())), "{}", d.name);
        if d.has("session-url") {
            // #213 A2 review: a finished transcript replaced by a directory reads `evidence-unreadable` with its error, never `session-unknown`.
            let log = r.transcript(&next).unwrap();
            let kept = root.join("worker.log.kept");
            std::fs::rename(&log, &kept).unwrap();
            std::fs::create_dir(&log).unwrap();
            let seen = r.observe(&mut next);
            assert_eq!(seen.cause, Some(agent::Cause("evidence-unreadable")), "{}: {seen:?}", d.name);
            assert!(seen.detail.as_deref().is_some_and(|t| t.starts_with("transcript: ")), "{}: {seen:?}", d.name);
            std::fs::remove_dir(&log).unwrap();
            std::fs::rename(&kept, &log).unwrap();
        }
        // #217 A1: a one-process CLI run refuses further input before any native write; images need capability `image-input`.
        let refused = |c: &'static str| Err(agent::Failure::Refused(agent::Cause(c)));
        assert_eq!(r.input(&mut next, &[agent::Part::Text("more".into())]), refused("input-unsupported"), "{}", d.name);
        let image = agent::Part::Image(root.join("worker.log"));
        assert_eq!(r.input(&mut next, &[image]), refused(if d.has("image-input") { "input-unsupported" } else { "image-unsupported" }), "{}", d.name);
        assert_eq!(r.account(&mut next).unwrap_err(), agent::Failure::Refused(agent::Cause("account-unsupported")), "{}", d.name);
        assert_eq!(observations(&store)[0]["runtime"].as_str(), Some(d.name));
        let mut refused = l.clone();
        refused.policy = agent::Policy { read: agent::Scope::Project, ..agent::Policy::default() };
        assert!(r.launch(&refused).is_err_and(|e| e.starts_with("policy read unsupported")), "{}", d.name);
        refused.policy = agent::Policy::default();
        refused.accounts = Some(agent::Accounts { list: vec![agent::Entry { label: "default".into(), enabled: false }], limits: Default::default() });
        assert_eq!(r.launch(&refused).err().as_deref(), Some("quota-exhausted"));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
}

fn wait_for(path: &Path) {
    let until = Instant::now() + Duration::from_secs(30);
    while !path.exists() { assert!(Instant::now() < until, "{} appeared", path.display()); std::thread::sleep(Duration::from_millis(20)); }
}

// #212 A1: a consumer that re-executes drops its Run while the CLI turn child keeps running; the checkpoint rebuilds an
// observation-only run of that child (identity, log offset, session) without a native call, and cancel reaches only
// that incarnation. Risk: a re-exec loses or resubmits live turns, double counts them, or signals a reused PID.
fn cli_turn_recovers_its_running_child() {
    use agent::State::{Finished, Running};
    unsafe { std::env::remove_var("TASKQ_TASK"); std::env::remove_var("AGENTQ_LAUNCHER"); std::env::remove_var("AGENTQ_OPTIMIZATION"); }
    for r in agent::runtimes() {
        let name = r.descriptor().name;
        let root = scratch(&format!("turn-recover-{name}"));
        let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
        let gate = root.join("gate");
        let l = fake_launch(name, &root, "brief", &[("COREQ_FAKE_GATE", gate.as_os_str())]);
        let mut run = r.launch(&l).unwrap();
        let pid = run.id();
        wait_for(&root.join("fake-turn-1.json"));
        let saved = r.checkpoint(&run).unwrap();
        assert_eq!((saved["kind"].as_str(), saved["turn"]["pid"].as_u64()), (Some("turn"), pid.map(u64::from)), "{name}");
        assert_eq!(r.poll(&mut run).unwrap(), Running);
        drop(run);
        let mut moved = l.clone();
        moved.log = root.join("other.log");
        assert_eq!(r.recover(&saved, &moved).err().as_deref(), Some("turn recovery scope differs"), "{name}: another log is another turn");
        let mut cold = r.recover(&saved, &l).unwrap();
        assert_eq!((cold.id(), r.poll(&mut cold).unwrap()), (pid, Running), "{name}: the live child is observed, not restarted");
        std::fs::write(&gate, "").unwrap();
        assert_eq!(finish(r, &mut cold), Finished, "{name}");
        let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: Some((212, 1)), route: None, account: "default" };
        let report = cold.record(&ctx, &store).unwrap();
        assert_eq!((report.outcome, report.text.as_deref()), ("ok", Some("turn 1")), "{name}");
        // A re-executed Unix parent reaps its own child and reads the code; elsewhere an unobserved exit stays unknown.
        assert_eq!(report.exit, if cfg!(unix) { Some(0) } else { None }, "{name}");
        assert!(r.session_id(&cold).is_some() && !root.join("fake-turn-2.json").exists(), "{name}: recovery made no native call");
        assert_eq!(observations(&store).len(), 1, "{name}: one observation");
        assert_eq!(r.checkpoint(&cold).unwrap_err(), agent::Cause("turn-ended"));
        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }
    let root = scratch("turn-recover-cancel");
    let claude = agent::runtime("claude").unwrap();
    let l = fake_launch("claude", &root, "x", &[("COREQ_FAKE_HANG", "1".as_ref())]);
    let run = claude.launch(&l).unwrap();
    let saved = claude.checkpoint(&run).unwrap();
    drop(run);
    let mut cold = claude.recover(&saved, &l).unwrap();
    claude.cancel(&mut cold).unwrap();
    finish(claude, &mut cold);
    let r = claude.report(&cold).unwrap();
    assert_eq!((r.outcome, r.error), ("cancelled", "interrupted"), "the identity-bound stop ends the recovered child");
    let _ = std::fs::remove_dir_all(root);
}

// #212 A2: caller hooks run on the child Command after the adapter's own setup and on the spawned Child before input,
// so creation flags, process groups and Job Objects survive; a failing hook fails the launch.
// Risk: the adapter overwrites caller process setup, or a child the caller could not contain keeps running.
fn process_hooks_shape_the_child() {
    use std::sync::{Arc, Mutex};
    let root = scratch("process-hooks");
    let gate = root.join("gate");
    let codex = agent::runtime("codex").unwrap();
    let mut l = fake_launch("codex", &root, "x", &[("FAKE_EXTRA", "adapter".as_ref()), ("COREQ_FAKE_GATE", gate.as_os_str())]);
    let seen = Arc::new(Mutex::new(None));
    let spawned = seen.clone();
    l.process.configure = Some(Arc::new(|c: &mut Command| {
        c.env("FAKE_EXTRA", "hooked");
        #[cfg(unix)] { use std::os::unix::process::CommandExt; c.process_group(0); }
        #[cfg(windows)] { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000 | 0x0000_0200); }
    }));
    l.process.spawned = Some(Arc::new(move |child: &std::process::Child| { *spawned.lock().unwrap() = Some(child.id()); Ok(()) }));
    let mut run = codex.launch(&l).unwrap();
    assert_eq!(run.id(), *seen.lock().unwrap(), "the spawned hook sees the child");
    wait_for(&root.join("fake-turn-1.json"));
    #[cfg(unix)]
    assert_eq!(unsafe { libc::getpgid(run.id().unwrap() as libc::pid_t) }, run.id().unwrap() as libc::pid_t, "the caller's process group survives");
    std::fs::write(&gate, "").unwrap();
    assert_eq!(finish(codex, &mut run), agent::State::Finished);
    assert_eq!(record(&root, 1)["extra"], "hooked", "configure runs after the adapter's environment");
    let before = codex.running();
    l.process.spawned = Some(Arc::new(|_: &std::process::Child| Err("job object refused".into())));
    assert_eq!(codex.launch(&l).err().as_deref(), Some("job object refused"));
    assert_eq!(codex.running(), before);
    let _ = std::fs::remove_dir_all(root);
}

// Chain: coreq.md R49: a turn past its deadline is stopped and observed as cancelled/timeout, never ok; a caller
// cancel reads cancelled/interrupted. Risk: a hung client blocks its caller forever or reads as a finished turn.
fn deadline_stops_a_turn_that_never_ends() {
    let root = scratch("hang");
    let claude = agent::runtime("claude").unwrap();
    let store = coreq::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
    let mut l = fake_launch("claude", &root, "x", &[("COREQ_FAKE_HANG", "1".as_ref())]);
    l.deadline = Some(Instant::now() + Duration::from_millis(500));
    let mut run = claude.launch(&l).unwrap();
    finish(claude, &mut run);
    let r = claude.report(&run).unwrap();
    assert_eq!((r.outcome, r.error), ("cancelled", "timeout"));
    let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: Some((4, 1)), route: None, account: "default" };
    run.record(&ctx, &store).unwrap().recorded.expect("a cancelled turn is a valid observation");
    l.deadline = None;
    let mut run = claude.launch(&l).unwrap();
    claude.cancel(&mut run).unwrap();
    finish(claude, &mut run);
    let r = claude.report(&run).unwrap();
    assert_eq!((r.outcome, r.error), ("cancelled", "interrupted"));
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}

// Chain: agentq.md M12.3-M12.6, task #182 A3/A5/A6: the JSON command runs and resumes both clients without an
// optimizer installation; `off` reaches the child, the launcher marker prevents a second count, one observation per turn.
// Risk: a plain turn needs the optimizer, writes the installation, or a launch counts twice or loses `off`.
fn json_turns_run_plain_without_the_optimizer() {
    // A worker that runs these tests may itself carry a launcher marker; the child would inherit it.
    // SAFETY: this harness runs its tests one at a time on the main thread.
    unsafe { std::env::remove_var("TASKQ_TASK"); std::env::remove_var("AGENTQ_LAUNCHER"); std::env::remove_var("AGENTQ_OPTIMIZATION"); }
    let root = scratch("json");
    let install = root.join("no-install");
    let exe = std::env::current_exe().unwrap();
    for (client, intent) in [("claude", "off"), ("codex", "inherit"), ("antigravity", "off")] {
        let dir = root.join(client);
        std::fs::create_dir_all(&dir).unwrap();
        let mut request = json!({"client": client, "root": dir, "log": dir.join("turns.log"), "input": "brief", "timeout_s": 30,
            "command": [exe], "env": {"COREQ_FAKE_AGENT": client, "FAKE_EXTRA": "x"}, "home": dir.join("home"),
            "optimization": intent, "store": dir.join("telemetry.db"),
            "context": {"product": "fixtureq", "version": "1.0.0", "project": "other", "task": null, "route": "manager", "account": "default"}});
        let first = agentq_runtime::cli::turn(&request.to_string(), Some(&install)).unwrap();
        assert_eq!((first["outcome"].as_str(), first["text"].as_str(), first["recorded"].as_str()), (Some("ok"), Some("turn 1"), Some("ok")));
        let expected = if intent == "off" { json!({"intent": "off", "effective": "off", "reason": "launch_off"}) } else { json!({"intent": "inherit", "effective": "off", "reason": "not_installed"}) };
        assert_eq!(first["optimization"], expected);
        let seen = record(&dir, 1);
        assert_eq!((seen["optimization"].as_str(), seen["launcher"].as_str()), (if intent == "off" { Some("off") } else { None }, Some("fixtureq")));
        // Recovery path: the next turn resumes the native session the report named.
        request["resume"] = json!(true);
        request["session"] = first["session"].clone();
        let second = agentq_runtime::cli::turn(&request.to_string(), Some(&install)).unwrap();
        assert_eq!((second["outcome"].as_str(), second["text"].as_str(), &second["session"]), (Some("ok"), Some("turn 2"), &first["session"]));
        let store = coreq::telemetry::Store::open(&dir.join("telemetry.db")).unwrap();
        assert_eq!(observations(&store).len(), 2, "one observation per turn");
        assert!(agentq_runtime::cli::turn(&json!({"client": "other"}).to_string(), None).is_err());
    }
    assert!(!install.exists(), "a plain turn never creates the installation");
    let _ = std::fs::remove_dir_all(root);
}

// Chain: agentq.md § 12 and RUNTIME.md § Adoption and rollback: a consumer outside the workspace builds against the
// runtime alone and plans both clients without an installation. Risk: the published crate needs workspace-only state.
fn external_consumer_builds_and_plans_turns() {
    let publication = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/runtime_publication.py");
    let out = Command::new(if cfg!(windows) { "python" } else { "python3" }).arg(publication).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/runtime-consumer");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/runtime-consumer");
    let out = Command::new(env!("CARGO")).args(["run", "--locked", "-q", "--manifest-path"]).arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", target).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("claude: -p --output-format stream-json") && text.contains("codex: exec --json"), "{text}");
}

// M12.6/A5: off survives Codex queue and app-server adaptation; native mode and caller policy stay intact.
fn off_reaches_codex_queue_and_app_server() {
    use agentq_runtime::optimization::Intent;
    let root = scratch("off-modes");
    std::fs::write(root.join("config.toml"), "model_provider = \"agentq\"\n").unwrap();
    let path = root.join("app.jsonl");
    append(&path, &[json!({"type":"session_meta","payload":{"id":"native-thread","cwd":root}})]);
    let mut q = fake_launch("codex", &root, "owned input reference", &[("COREQ_FAKE_CONTROL", "1".as_ref()), ("CODEX_HOME", root.as_os_str())]);
    q.mode = agent::Mode::Queue { transcript: path };
    q.optimization = Intent::Off;
    let run = agent::runtime("codex").unwrap().resume("native-thread", &q).unwrap();
    assert_eq!(run.optimization().reason, "launch_off");
    let queue = control(&root);
    assert!(queue["args"].as_array().unwrap().iter().any(|a| a.as_str().is_some_and(|s| s.contains("x-agentq-optimization"))), "queue lost launch off: {queue}");
    let mut l = fake_launch("codex", &root, "owned request", &[("COREQ_FAKE_APP_SERVER", "1".as_ref()), ("AGENTQ_FAKE_ONE_TURN", "1".as_ref()), ("CODEX_HOME", root.as_os_str())]);
    l.log = root.join("private.log");
    l.optimization = Intent::Off;
    l.deltas = true;
    let mut run = agent::runtime("codex").unwrap().launch(&l).unwrap();
    let args = serde_json::from_str::<Value>(&std::fs::read_to_string(root.join("app-transport.json")).unwrap()).unwrap()["args"].clone();
    assert!(args[0] == "app-server" && args.as_array().unwrap().iter().any(|s| s.as_str().is_some_and(|s| s.contains("x-agentq-optimization"))), "app-server lost launch off: {args}");
    agent::runtime("codex").unwrap().cancel(&mut run).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

// M12.1/M12.6: a caller-owned executable wrapper must preserve agentapi prompt/tailing and reject server off.
fn antigravity_wrapped_submission_keeps_prompt_and_tail() {
    let root = scratch("wrapped-agentapi");
    let bin = root.join(if cfg!(windows) {"agentapi.exe"} else {"agentapi"});
    std::fs::copy(std::env::current_exe().unwrap(), &bin).unwrap();
    let ag = agent::runtime("antigravity").unwrap();
    let home = root.join("home");
    let mut l = agent::Launch::new(&root, root.join("turn.log"), "first", "fixtureq");
    l.command = vec![exe(), "--wrap".into(), bin.clone().into()];
    l.env = vec![("AGENTQ_FAKE_AGENTAPI".into(),"1".into()),("AGENTQ_OPTIMIZATION".into(),"inherit".into()),("ANTIGRAVITY_HOME".into(),home.clone().into())];
    let mut run = ag.launch(&l).unwrap();
    let history = home.join("brain/native-conv/.system_generated/logs/transcript.jsonl");
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    append(&history, &[json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","content":"wrapper answer"})]);
    finish(ag, &mut run);
    let r = ag.report(&run).unwrap();
    assert_eq!((r.outcome,r.text.as_deref()),("ok",Some("wrapper answer")));
    l.env.push(("AGENTQ_OPTIMIZATION".into(),"off".into()));
    assert!(ag.launch(&l).is_err(),"submission env cannot suppress existing server hooks");
    std::fs::remove_dir_all(root).unwrap();
}

// M12.1/M12.3: agentapi acknowledges submission before terminal transcript evidence; resume consumes only new steps.
// Risk: exit 0 is mistaken for completed work, stale history counts twice, or the caller's prompt/args are lost.
fn antigravity_native_submission_tails_and_resumes() {
    let root = scratch("native-agentapi");
    let bin = root.join(if cfg!(windows) { "agentapi.exe" } else { "agentapi" });
    std::fs::copy(std::env::current_exe().unwrap(), &bin).unwrap();
    let ag = agent::runtime("antigravity").unwrap();
    let home = root.join("home");
    let mut l = agent::Launch::new(&root, root.join("turns.log"), "first", "fixtureq");
    l.command = vec![bin.clone().into()];
    l.env = vec![("AGENTQ_FAKE_AGENTAPI".into(),"1".into()),("ANTIGRAVITY_HOME".into(),home.clone().into())];
    let mut run = ag.launch(&l).unwrap();
    // The double exits after receipt. Supply the terminal later, as a separately owned server would.
    wait_agentapi_receipt(ag, &mut run, &l.log, "first");
    assert_eq!(ag.poll(&mut run).unwrap(), agent::State::Running, "submission is not completion");
    let history = home.join("brain/native-conv/.system_generated/logs/transcript.jsonl");
    std::fs::create_dir_all(history.parent().unwrap()).unwrap();
    append(&history,&[json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","content":"first done","input_tokens":3,"cache_read_tokens":2,"cache_write_tokens":0,"output_tokens":1,"tool_calls":[]})]);
    finish(ag, &mut run); let report=ag.report(&run).unwrap();
    assert_eq!((report.outcome,report.text.as_deref(),report.session.as_deref()),("ok",Some("first done"),Some("native-conv")));
    let session=report.session.unwrap();
    l.input="second".into();
    let mut run=ag.resume(&session,&l).unwrap();
    wait_agentapi_receipt(ag, &mut run, &l.log, "second");
    assert_eq!(ag.poll(&mut run).unwrap(), agent::State::Running);
    append(&history,&[json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","content":"second done","input_tokens":7,"cache_read_tokens":4,"cache_write_tokens":0,"output_tokens":2})]);
    finish(ag, &mut run);let report=ag.report(&run).unwrap();
    assert_eq!((report.outcome,report.usage.input,report.usage.calls,report.text.as_deref()),("ok",Some(7),Some(1),Some("second done")));
    l.input="timeout".into(); l.deadline=Some(Instant::now()+Duration::from_millis(100));
    let mut run=ag.resume(&session,&l).unwrap();
    finish(ag, &mut run);
    let report=ag.report(&run).unwrap();
    assert_eq!((report.outcome,report.error),("cancelled","timeout"));
    std::fs::remove_dir_all(root).unwrap();
}

fn wait_agentapi_receipt(ag: &dyn agent::Runtime, run: &mut agent::Run, log: &Path, prompt: &str) {
    let until = Instant::now()+Duration::from_secs(5);
    loop {
        assert_eq!(ag.poll(run).unwrap(), agent::State::Running, "receipt alone is not a terminal");
        if std::fs::read_to_string(log).unwrap_or_default().contains(&format!("\"prompt\":\"{prompt}\"")) {break;}
        assert!(Instant::now()<until); std::thread::sleep(Duration::from_millis(10));
    }
}

fn fake_agentapi() {
    let args:Vec<String>=std::env::args().skip(1).collect();
    assert!(matches!(args.first().map(String::as_str),Some("new-conversation" | "send-message")));
    assert!(matches!(args.last().map(String::as_str),Some("first" | "second" | "timeout")),"native prompt is argv, not '-' or stdin");
    if args[0]=="send-message" {assert_eq!(args[1],"native-conv");}
    use std::io::Write;
    let receipt = serde_json::to_string_pretty(&json!({"response":{"newConversation":{"conversationId":"native-conv"}},"prompt":args.last()})).unwrap();
    let half=receipt.len()/2;
    print!("{}",&receipt[..half]); std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_millis(40));
    print!("{}",&receipt[half..]); std::io::stdout().flush().unwrap();
}

// taskq#2271: a caller that builds its own native session reaches every native seam through the trait, and an optional
// one refuses by capability. Risk: a consumer reimplements a native parser or command because the trait lacks it.
fn runtime_trait_exposes_native_seams() {
    use agent::{NativeSpec, Process};
    use agentq_runtime::optimization::Intent;
    let root=scratch("native-seams");
    let (claude,codex)=(agent::runtime("claude").unwrap(),agent::runtime("codex").unwrap());
    let env=|extra:&[(&str,&str)]| std::iter::once(("COREQ_FAKE_AGENT","claude")).chain(extra.iter().copied()).map(|(k,v)|(k.into(),v.into())).collect::<Vec<_>>();
    let session="0c0ffee0-0000-4000-8000-000000000001";
    let spec=NativeSpec {root:root.clone(),session:session.into(),env:env(&[]),..Default::default()};

    // command and turn: the native start with the caller's spec, classified like a launched turn.
    let args:Vec<_>=claude.command(Command::new(exe()),&spec).unwrap().get_args().map(|a|a.to_string_lossy().into_owned()).collect();
    assert_eq!(args[args.len()-2..],["--session-id".to_string(),session.to_string()]);
    let log=root.join("seam.log");
    let mut turn=claude.turn(Command::new(exe()),&spec,&log,"hello",&Process::default()).unwrap();
    assert!(turn.id()>0);
    assert_eq!(turn.wait(Some(Instant::now()+Duration::from_secs(30))).unwrap(),0);
    let r=turn.report();
    assert_eq!((r.outcome,r.session.as_deref()),("ok",Some(session)));
    let c=claude.classify(&std::fs::read_to_string(&log).unwrap());
    assert_eq!((c.outcome,c.session.as_deref(),c.text),("ok",Some(session),r.text));

    // classify: one event is its own log; native reasons arrive in `detail`.
    let capacity=codex.classify(&json!({"type":"turn.failed","error":{"message":"Selected model is at capacity"}}).to_string());
    assert_eq!((capacity.error,capacity.detail.as_deref()),("model-capacity",Some("Selected model is at capacity")));
    let auth=codex.classify(&json!({"type":"error","message":"401 Unauthorized"}).to_string());
    assert_eq!((auth.error,auth.detail.as_deref()),("auth-rejected",Some("401 Unauthorized")));
    assert_eq!(codex.classify(&json!({"type":"thread.started","thread_id":"t-1"}).to_string()).session.as_deref(),Some("t-1"));
    assert_eq!(codex.classify(&json!({"type":"error","message":"You've hit your usage limit"}).to_string()).reached,Some(None));

    // executable, home and session transcript.
    let bin=root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"),"").unwrap();
    assert_eq!(claude.executable(Some(bin.as_os_str())).unwrap(),bin.join("claude"));
    assert_eq!(claude.executable(Some("".as_ref())).unwrap(),PathBuf::from("claude"),"unresolved stays bare");
    assert_eq!(claude.home(),claude.protected_paths(None).into_iter().next());
    let home=root.join("codex-home");
    let rollout=home.join("sessions/2026/11/03/rollout-2026-11-03T00-00-00-t-1.jsonl");
    std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    std::fs::write(&rollout,"").unwrap();
    assert_eq!(codex.session_transcript("t-1",Some(&home)),Some(rollout));
    assert_eq!(codex.session_transcript("t-2",Some(&home)),None);
    assert_eq!(claude.session_transcript(session,Some(&home)),None);

    // prepare: `off` reaches the child environment of the caller's spec.
    let mut off=spec.clone();
    assert_eq!(claude.prepare(Intent::Off,"fixtureq",&mut off,None).intent,Intent::Off);
    assert!(off.env.iter().any(|(k,v)|k=="AGENTQ_OPTIMIZATION"&&v=="off"));
    assert!(off.env.iter().any(|(k,v)|k=="AGENTQ_LAUNCHER"&&v=="fixtureq"));

    // background (Claude only): launch, list, validate and stop one job.
    let row=json!({"id":"c37eb75a","kind":"background","cwd":root,"sessionId":"native-session","state":"working","pid":42});
    let rows=json!([row]).to_string();
    let bg=NativeSpec {env:env(&[("COREQ_FAKE_BACKGROUND","1"),("COREQ_FAKE_CONTROL","1"),("FAKE_JOB_ROWS",&rows)]),..spec.clone()};
    let out=claude.background_launch(Command::new(exe()),&bg,"owned input",Duration::from_secs(30)).unwrap().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("backgrounded · c37eb75a"));
    assert_eq!(claude.jobs(Command::new(exe()),&bg,true,Duration::from_secs(30)).unwrap(),vec![row.clone()]);
    assert_eq!(control(&root)["args"][3],"--cwd");
    let job=claude.job(&row,&bg,"c37eb75a").unwrap();
    assert_eq!((job.id.as_str(),job.session.as_str(),job.pid),("c37eb75a","native-session",Some(42)));
    claude.stop_job(Command::new(exe()),&bg,&job,Duration::from_secs(30)).unwrap();
    assert_eq!(control(&root)["args"],json!(["stop","c37eb75a"]));
    assert_eq!(claude.launch_refusal("Error: Workspace not trusted").map(|c|c.0),Some("workspace-untrusted"));
    assert_eq!(codex.launch_refusal("Error: Workspace not trusted"),None);
    for (rt,text,cause) in [(&claude,"Not logged in · Please run /login",Some("login-required")),(&claude,"No conversation found with session ID: x",Some("session-missing")),
        (&codex,"unexpected status 401 Unauthorized",Some("login-required")),(&codex,"Please Log In again",Some("login-required")),
        (&codex,"app-server thread/resume refused: gone",Some("session-missing")),(&codex,"rate limited",None),(&claude,"Please log in",None)] {
        assert_eq!(rt.launch_refusal(text).map(|c|c.0),cause,"{text}");
    }
    assert_eq!(codex.background_launch(Command::new(exe()),&bg,"x",Duration::from_secs(1)).unwrap_err(),"background unsupported by codex");
    assert_eq!(codex.jobs(Command::new(exe()),&bg,false,Duration::from_secs(1)).unwrap_err(),"background unsupported by codex");

    // app queue and app-server (Codex only).
    let thread=NativeSpec {session:"t-1".into(),resume:true,args:vec!["--flag".into()],env:vec![("COREQ_FAKE_AGENT".into(),"codex".into()),("COREQ_FAKE_CONTROL".into(),"1".into())],..spec.clone()};
    codex.app_queue(Command::new(exe()),&thread,"queued input",Duration::from_secs(30)).unwrap();
    assert_eq!(control(&root)["args"],json!(["queue","--thread","t-1","--message","queued input"]));
    let server:Vec<_>=codex.app_server_command(Command::new(exe()),&thread).unwrap().get_args().map(|a|a.to_string_lossy().into_owned()).collect();
    assert_eq!(server,["app-server","--flag"]);
    assert_eq!(claude.app_queue(Command::new(exe()),&thread,"x",Duration::from_secs(1)).unwrap_err(),"app-queue unsupported by claude");
    assert_eq!(claude.app_server_command(Command::new(exe()),&thread).unwrap_err(),"app-server unsupported by claude");
    std::fs::remove_dir_all(root).unwrap();
}

// Readiness fake: `fake-ready.json` in the login home maps `--version` and the status command to their output.
fn fake_ready(file: &Path) {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let key = if args == ["--version"] { "version" } else { "status" };
    if key == "status" { assert_eq!(args, v["expect"].as_array().unwrap().iter().map(|a| a.as_str().unwrap()).collect::<Vec<_>>()); }
    print!("{}", v[key].as_str().unwrap_or(""));
    std::process::exit(v["code"].as_i64().unwrap_or(0) as i32);
}

// pilotq#363 / taskq#2324: `ready` reads the native login status and minimum version through the trait, so products keep
// no status command, login text or version floor. Risk: an API-key login or an old CLI reads as ready.
fn ready_reads_native_login_and_minimum_version() {
    let home = std::env::temp_dir().join(format!("agentq-ready-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let claude_status = ["auth", "status", "--json"];
    let codex_status = ["-c", "forced_login_method=\"chatgpt\"", "login", "status"];
    let cases: &[(&str, &[&str], &str, &str, &str, &str)] = &[
        ("claude", &claude_status, "2.1.5 (Claude Code)", r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#, "ok", "ok"),
        ("claude", &claude_status, "2.0.9 (Claude Code)", r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#, "outdated", "api-key"),
        ("claude", &claude_status, "2.1.5", r#"{"loggedIn":false}"#, "ok", "missing"),
        ("codex", &codex_status, "codex-cli 0.162.0", "Logged in using ChatGPT\n", "ok", "ok"),
        ("codex", &codex_status, "codex-cli 0.161.9", "Logged in using an API key - sk-***\n", "outdated", "api-key"),
        ("codex", &codex_status, "codex-cli 0.170.0", "Not logged in\n", "ok", "missing"),
    ];
    for (name, expect, version, status, want_version, want_login) in cases {
        std::fs::write(home.join("fake-ready.json"), json!({"version": version, "status": status, "expect": expect}).to_string()).unwrap();
        let r = agent::runtime(name).unwrap().ready(Some(Path::new(&exe())), Some(&home));
        assert_eq!((r.install.state, r.version.state, r.login.state), ("ok", *want_version, *want_login), "{name} {version} {status}");
    }
    let r = agent::runtime("codex").unwrap().ready(Some(&home.join("absent")), Some(&home));
    assert_eq!((r.install.state, r.login.state), ("missing", "unknown"));
    std::fs::remove_dir_all(&home).unwrap();
}
