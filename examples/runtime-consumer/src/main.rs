//! Plans one Claude and one Codex turn with optimization off, without any AgentQ installation.
use agentq_runtime::agent::{self, Client, Spec};
use agentq_runtime::optimization::{self, Intent};

fn main() {
    let root = std::env::temp_dir();
    for client in [Client::Claude, Client::Codex] {
        let mut spec = Spec { client, root: root.clone(), session: agent::new_session(client), resume: false, args: vec![], home: Some(root.join("no-home")), env: vec![] };
        let o = optimization::prepare(Intent::Off, "runtime-consumer", &mut spec, Some(&root.join("no-agentq")));
        let argv: Vec<_> = agent::args(&spec).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!((o.effective, o.reason), ("off", "launch_off"));
        println!("{}: {} | optimization {}", client.code(), argv.join(" "), o.json());
    }
}
