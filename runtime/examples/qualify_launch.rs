//! #284 A3/A4 native qualification: the real `codex` and `claude` on PATH, isolated account homes only. No working
//! profile is read or written, except a read-only `ready` contrast of the client's own login. Offline without arguments.
use agentq_runtime::agent::{self, AccountScope, Failure};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn main() {
    let Some(base) = std::env::args().nth(1).map(PathBuf::from) else { return };
    let soon = || Instant::now() + Duration::from_secs(30);
    let project = base.join("project");
    std::fs::create_dir_all(project.join(".codex")).unwrap();
    let git = std::process::Command::new("git").args(["init", "-q"]).current_dir(&project).status().unwrap();
    assert!(git.success());
    let project_hook = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"project-layer-hook","timeout":5}]}]}}"#;
    std::fs::write(project.join(".codex/hooks.json"), project_hook).unwrap();

    let codex = agent::runtime("codex").unwrap();
    let codex_home = base.join("codex-account");
    std::fs::create_dir_all(&codex_home).unwrap();
    let scope = AccountScope { home: Some(codex_home.clone()), ..Default::default() };
    let r = codex.ready_for(&scope);
    println!("codex isolated ready: install {} version {} ({}) login {} ({})", r.install.state, r.version.state, r.version.detail, r.login.state, r.login.detail);
    let own = codex.ready(None, None);
    println!("codex own login contrast: {}", own.login.state);
    let start = Instant::now();
    let again = codex.ready_for(&scope);
    println!("codex repeat ready: version {} in {} ms", again.version.state, start.elapsed().as_millis());
    let layer = |scope: &AccountScope| codex.hooks(&project, scope, soon()).unwrap().iter().any(|h| h.command.as_deref() == Some("project-layer-hook"));
    println!("codex project hook listed before trust: {}", layer(&scope));
    println!("codex trust_folder: {:?}", codex.trust_folder(&project, &scope, soon()));
    println!("codex project hook listed after trust: {}", layer(&scope));
    println!("codex trust_folder repeat: {:?}", codex.trust_folder(&project, &scope, soon()));
    println!("codex config.toml: {}", redact(&std::fs::read_to_string(codex_home.join("config.toml")).unwrap(), &base));
    for folder in [home_of(), home_of().ancestors().last().unwrap().to_path_buf()] {
        println!("codex trust home or root: {:?}", codex.trust_folder(&folder, &scope, soon()).err().map(cause));
    }
    let catalogue = codex.catalogue_for(&scope);
    println!("codex isolated catalogue: {}", catalogue.as_ref().map_or_else(|e| format!("error {e}"), |m| format!("{} models", m.len())));
    let limits = codex.account_limits_for(&scope, 1_800_000_000);
    println!("codex isolated limits: {:?}", limits.iter().map(|w| w.unknown.as_deref().unwrap_or("known")).collect::<Vec<_>>());

    let claude = agent::runtime("claude").unwrap();
    // Any Claude start creates its configuration file, so the fresh case runs first.
    let fresh = AccountScope { home: Some(base.join("claude-fresh")), ..Default::default() };
    std::fs::create_dir_all(base.join("claude-fresh")).unwrap();
    println!("claude trust before first run: {:?}", claude.trust_folder(&project, &fresh, soon()).err().map(cause));
    let claude_home = base.join("claude-account");
    std::fs::create_dir_all(&claude_home).unwrap();
    // The first working Claude the script found: a host may hold a stale npm shim on PATH.
    let executable = std::env::var_os("AGENTQ_QUAL_CLAUDE").map(PathBuf::from);
    let scope = AccountScope { home: Some(claude_home.clone()), executable: executable.clone(), ..Default::default() };
    let r = claude.ready_for(&scope);
    println!("claude isolated ready: install {} version {} ({}) login {} ({})", r.install.state, r.version.state, r.version.detail, r.login.state, r.login.detail);
    println!("claude own login contrast: {}", claude.ready(executable.as_deref(), None).login.state);
    let file = claude_home.join(".claude.json");
    std::fs::write(&file, r#"{"numStartups":1}"#).unwrap();
    let lock = claude_home.join(".claude.json.lock");
    std::fs::create_dir(&lock).unwrap();
    println!("claude trust with held lock: {:?}", claude.trust_folder(&project, &scope, Instant::now() + Duration::from_millis(300)).err().map(cause));
    std::fs::remove_dir(&lock).unwrap();
    println!("claude trust_folder: {:?}", claude.trust_folder(&project, &scope, soon()));
    println!("claude trust_folder repeat: {:?}", claude.trust_folder(&project, &scope, soon()));
    println!("claude .claude.json: {}", redact(&std::fs::read_to_string(&file).unwrap().replace('\n', " "), &base));
    println!("claude trust home or root: {:?}", claude.trust_folder(&home_of(), &scope, soon()).err().map(cause));
}

fn home_of() -> PathBuf { PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap()).canonicalize().unwrap() }

fn cause(f: Failure) -> String { f.to_string() }

fn redact(text: &str, base: &Path) -> String {
    let lower = base.display().to_string();
    text.replace(&lower, "<base>").replace(&lower.replace('\\', "/"), "<base>").replace(&lower.to_ascii_lowercase(), "<base>")
}
