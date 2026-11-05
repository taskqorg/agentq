//! Runs one deterministic Claude and Codex worker through `agent::Runtime`, without an AgentQ installation or real client credentials.
use agentq_runtime::agent::{self, Launch, State};
use agentq_runtime::optimization::Intent;
use std::time::{Duration, Instant};

fn main() {
    if let Ok(client) = std::env::var("AGENTQ_RUNTIME_EXAMPLE_CLIENT") {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        assert_eq!(input, "fixture input");
        assert_eq!(std::env::var("AGENTQ_OPTIMIZATION").unwrap(), "off");
        assert!(["TASKQ_TASK", "AGENTQ_LAUNCHER"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty())));
        let argv: Vec<String> = std::env::args().skip(1).collect();
        std::fs::write(std::env::var("AGENTQ_RUNTIME_EXAMPLE_ARGV").unwrap(), argv.join(" ")).unwrap();
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
    for name in ["claude", "codex"] {
        let runtime = agent::runtime(name).unwrap();
        let argv = root.join(format!("{name}.argv"));
        let mut launch = Launch::new(&root, root.join(format!("{name}.log")), "fixture input", "runtime-consumer");
        launch.command = vec![std::env::current_exe().unwrap().into()];
        launch.optimization = Intent::Off;
        launch.home = Some(root.join("no-home"));
        launch.env = vec![("AGENTQ_RUNTIME_EXAMPLE_CLIENT".into(), name.into()), ("AGENTQ_RUNTIME_EXAMPLE_ARGV".into(), argv.clone().into())];
        let mut run = runtime.launch(&launch).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while runtime.poll(&mut run).unwrap() == State::Running {
            assert!(Instant::now() < until, "fixture turn ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        let (optimization, report) = (run.optimization().clone(), runtime.report(&run).unwrap());
        assert_eq!((optimization.effective, optimization.reason), ("off", "launch_off"));
        assert_eq!(report.outcome, "ok");
        assert_eq!(report.text.as_deref(), Some("fixture answer"));
        assert_eq!((report.usage.input, report.usage.output), (Some(1), Some(1)));
        println!("{name}: {} | optimization {} | turn {}", std::fs::read_to_string(&argv).unwrap(), optimization.json(), report.outcome);
    }
    std::fs::remove_dir_all(root).unwrap();
}
