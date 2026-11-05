//! #132 A2 / agentq.md § 1 `hook`: a failed client's hook run answers `{}` and exits 0; a run that failed is recorded
//! with its error, doctor fails on it, and the next successful run clears it.
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

fn agentq(root: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut p = Command::new(env!("CARGO_BIN_EXE_agentq"))
        .args(args)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("LOCALAPPDATA", root)
        .env("APPDATA", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(input).unwrap();
    p.wait_with_output().unwrap()
}

fn record(root: &Path) -> PathBuf {
    let install = if cfg!(windows) { root.join("agentq") } else { root.join(".local/share/agentq") };
    install.join("data/evidence/codex.Stop.failed")
}

#[test]
fn hook_failure_is_answered_recorded_and_cleared() {
    let root = std::env::temp_dir().join(format!("agentq-hook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // invalid UTF-8 on stdin: the client still gets `{}` and exit 0, and the failure is kept
    let o = agentq(&root, &["hook", "codex", "Stop"], b"{\"x\": \"\xff\"}");
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).trim()), (Some(0), "{}"));
    let r = std::fs::read_to_string(record(&root)).unwrap();
    assert!(r.contains("stdin:"), "{r}");
    let d = agentq(&root, &["doctor"], b"");
    assert_eq!(d.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&d.stdout).contains("hooks: codex Stop (stdin:"), "{}", String::from_utf8_lossy(&d.stdout));

    // an unreadable record is reported, not hidden
    std::fs::write(record(&root), b"\xff\xfe").unwrap();
    let d = agentq(&root, &["doctor"], b"");
    assert!(String::from_utf8_lossy(&d.stdout).contains("hooks: codex Stop (unreadable failure record)"));

    // the next successful run clears it
    let o = agentq(&root, &["hook", "codex", "Stop"], b"{\"hook_event_name\": \"Stop\"}");
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).trim()), (Some(0), "{}"));
    assert!(!record(&root).exists());

    // Antigravity PreToolUse delegates with `decision: "allow"` on failure, unqualified input and `AGENTQ_OPTIMIZATION=off`
    let o = agentq(&root, &["hook", "antigravity", "PreToolUse"], b"{\"x\": \"\xff\"}");
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).trim()), (Some(0), "{\"decision\":\"allow\"}"));
    let mut p = Command::new(env!("CARGO_BIN_EXE_agentq"))
        .args(["hook", "antigravity", "PreToolUse"])
        .env("HOME", &root)
        .env("USERPROFILE", &root)
        .env("LOCALAPPDATA", &root)
        .env("APPDATA", &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("AGENTQ_OPTIMIZATION", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(b"{\"toolCall\":{\"name\":\"run_command\",\"args\":{\"CommandLine\":\"cargo test --offline\"}}}").unwrap();
    let o = p.wait_with_output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).trim()), (Some(0), "{\"decision\":\"allow\"}"));
    let _ = std::fs::remove_dir_all(&root);
}
