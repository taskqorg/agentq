//! Runs one deterministic Claude and Codex worker without an AgentQ installation or real client credentials.
use agentq_runtime::agent::{self, Client, Spec, Turn};
use agentq_runtime::optimization::{self, Intent};
use std::process::Command;
use std::time::{Duration, Instant};

fn main() {
    if let Ok(client) = std::env::var("AGENTQ_RUNTIME_EXAMPLE_CLIENT") {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        assert_eq!(input, "fixture input");
        assert_eq!(std::env::var("AGENTQ_OPTIMIZATION").unwrap(), "off");
        assert!(["TASKQ_TASK", "AGENTQ_LAUNCHER"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty())));
        if client == "claude" {
            println!(r#"{{"type":"assistant","message":{{"id":"fixture","model":"fixture","usage":{{"input_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":1}}}}}}"#);
            println!(r#"{{"type":"result","is_error":false,"result":"fixture answer"}}"#);
        } else {
            println!(r#"{{"type":"thread.started","thread_id":"fixture-thread"}}"#);
            println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"fixture answer"}}}}"#);
            println!(r#"{{"type":"turn.completed","usage":{{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}}}}"#);
        }
        return;
    }
    let root = std::env::temp_dir().join(format!("agentq-external-consumer-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for client in [Client::Claude, Client::Codex] {
        let mut spec = Spec { client, root: root.clone(), session: agent::new_session(client), resume: false, args: vec![],
            home: Some(root.join("no-home")), env: vec![("AGENTQ_RUNTIME_EXAMPLE_CLIENT".into(), client.code().into())] };
        let o = optimization::prepare(Intent::Off, "runtime-consumer", &mut spec, Some(&root.join("no-agentq")));
        let argv: Vec<_> = agent::args(&spec).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!((o.effective, o.reason), ("off", "launch_off"));
        let mut turn = Turn::start(Command::new(std::env::current_exe().unwrap()), &spec, &root.join(format!("{}.log", client.code())), "fixture input").unwrap();
        turn.wait(Some(Instant::now() + Duration::from_secs(10))).unwrap();
        let report = turn.report();
        assert_eq!(report.outcome, "ok");
        assert_eq!(report.text.as_deref(), Some("fixture answer"));
        assert_eq!((report.usage.input, report.usage.output), (Some(1), Some(1)));
        assert!(!root.join("no-agentq").exists());
        println!("{}: {} | optimization {} | turn {}", client.code(), argv.join(" "), o.json(), report.outcome);
    }
    std::fs::remove_dir_all(root).unwrap();
}
