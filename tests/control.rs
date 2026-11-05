//! Temporary controls through the real CLI, native runner and MCP transport, in private roots only.
use serde_json::{Value, json};
use std::{io::Write, path::{Path, PathBuf}, process::{Command, Output, Stdio}};

fn cli(root: &Path, args: &[&str], input: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_agentq"));
    c.args(args).current_dir(root)
        .env("HOME", root).env("USERPROFILE", root).env("LOCALAPPDATA", root).env("APPDATA", root)
        .env("AGENTQ_INSTALL", root.join("install"))
        .env("CODEX_HOME", root.join("codex")).env("CLAUDE_CONFIG_DIR", root.join("claude")).env("HERMES_HOME", root.join("hermes"))
        .env("XDG_CONFIG_HOME", root.join("config")).env("AGENTQ_AGENT", "claude")
        .env("CALLS", root.join("calls")).env("EXIT", "7")
        .env("PATH", std::env::join_paths(std::iter::once(root.join("bin")).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap())
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut p = c.spawn().unwrap();
    if let Some(s) = input { p.stdin.take().unwrap().write_all(s.as_bytes()).unwrap(); }
    drop(p.stdin.take());
    p.wait_with_output().unwrap()
}

fn value(root: &Path, args: &[&str]) -> Value {
    let out = cli(root, args, None);
    assert!(out.status.success(), "{args:?}: {out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

fn profile<'a>(v: &'a Value, kind: &str) -> &'a Value {
    v["profiles"].as_array().unwrap().iter().find(|p| p["kind"] == kind).unwrap()
}

fn setup() -> PathBuf {
    // macOS clock ticks can be shared by parallel tests in the same process.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!("agentq-control-cli-{}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    for dir in ["install", "bin", "codex", "claude", "hermes"] { std::fs::create_dir_all(root.join(dir)).unwrap(); }
    let desktop = if cfg!(target_os = "macos") { root.join("Library/Application Support/Claude") }
        else if cfg!(windows) { root.join("Claude") } else { root.join("config/Claude") };
    std::fs::create_dir_all(desktop).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_agentq"), root.join(format!("install/agentq{}", std::env::consts::EXE_SUFFIX))).unwrap();
    std::fs::write(root.join("codex/config.toml"), "model = \"fixture-native-model\"\n").unwrap();
    std::fs::write(root.join("claude/settings.json"), r#"{"model":"fixture-native-model","permissions":{"defaultMode":"default"}}"#).unwrap();
    std::fs::write(root.join("claude/.claude.json"), r#"{"mcpServers":{"foreign":{"command":"foreign-tool"}}}"#).unwrap();
    let projection = value(&root, &["configure"]);
    for kind in ["codex", "claude-code", "claude-desktop-chat"] {
        let id = profile(&projection, kind)["id"].as_str().unwrap();
        let args = if kind == "claude-desktop-chat" { vec!["configure", id, "mcp=registered", "--yes"] }
            else { vec!["configure", id, "mcp=registered", "hooks=registered", "--yes"] };
        let out = cli(&root, &args, None);
        assert!(out.status.success(), "{out:?}");
    }
    root
}

fn mcp(root: &Path, name: &str, args: Value) -> Value {
    let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}).to_string()+"\n";
    let out = cli(root, &["mcp", "claude"], Some(&req));
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A private recipe cannot grant ownership of another installation's loaded OS label.
#[cfg(target_os = "macos")]
#[test]
fn foreign_loaded_service_is_kept_even_when_the_recipe_file_is_ours() {
    use std::os::unix::fs::PermissionsExt;
    let root = setup();
    let ps = root.join("bin/ps");
    std::fs::write(&ps, "#!/bin/sh\ncase \"$*\" in *ppid*) echo '1 0 /sbin/init';; *) echo '1 /sbin/init';; esac\n").unwrap();
    let launchctl = root.join("bin/launchctl");
    std::fs::write(&launchctl, "#!/bin/sh\ncase \"$1\" in\nprint) case \"$2\" in */com.agentq.host) echo 'program = /foreign-install/agentq';; *) echo 'services = {'; echo '0 0 com.agentq.host'; echo '}';; esac;;\nbootout) touch \"$HOME/foreign-unloaded\";;\nesac\n").unwrap();
    for path in [&ps, &launchctl] { std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    let unit = root.join("Library/LaunchAgents/com.agentq.host.plist");
    std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
    let recipe = format!("<key>ProgramArguments</key><array><string>{}</string><string>service</string><string>run</string></array>", root.join("install/agentq").display());
    std::fs::write(&unit, &recipe).unwrap();
    let _ = cli(&root, &["disable"], None);
    assert!(!root.join("foreign-unloaded").exists(), "disable must inspect the loaded executable before touching the OS label");
    assert!(!cli(&root, &["service", "stop"], None).status.success());
    assert!(!root.join("foreign-unloaded").exists(), "service stop must preserve the same foreign registration");
    assert_eq!(std::fs::read_to_string(unit).unwrap(), recipe);
    std::fs::remove_dir_all(root).unwrap();
}

/// Service adapter proof only: no real launchd registration or working profile is changed.
#[cfg(target_os = "macos")]
#[test]
fn idle_registered_service_is_unloaded_once_and_restored_before_enable() {
    use std::os::unix::fs::PermissionsExt;
    let root = setup();
    let ps = root.join("bin/ps");
    std::fs::write(&ps, "#!/bin/sh\ncase \"$*\" in *ppid*) echo '1 0 /sbin/init';; *) echo '1 /sbin/init';; esac\n").unwrap();
    let launchctl = root.join("bin/launchctl");
    std::fs::write(&launchctl, "#!/bin/sh\ncase \"$1\" in\nprint) case \"$2\" in */com.agentq.host) echo \"program = $HOME/install/agentq\"; echo 'arguments = {'; echo \"$HOME/install/agentq\"; echo service; echo run; echo '}';; *) echo 'services = {'; if test -f \"$HOME/unit-loaded\"; then echo '0 0 com.agentq.host'; fi; echo '}';; esac;;\nbootout) rm -f \"$HOME/unit-loaded\"; echo unload >> \"$HOME/unit-actions\";;\nbootstrap) touch \"$HOME/unit-loaded\"; echo load >> \"$HOME/unit-actions\";;\n*) exit 2;;\nesac\n").unwrap();
    for path in [&ps, &launchctl] { std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    let unit = root.join("Library/LaunchAgents/com.agentq.host.plist");
    std::fs::create_dir_all(unit.parent().unwrap()).unwrap();
    std::fs::write(&unit, format!("<key>ProgramArguments</key><array><string>{}</string><string>service</string><string>run</string></array>", root.join("install/agentq").display())).unwrap();
    std::fs::write(root.join("unit-loaded"), "fixture").unwrap();
    let idle_ps = std::fs::read_to_string(&ps).unwrap();
    let running_ps = format!("#!/bin/sh\ncase \"$*\" in *ppid*) echo '1 0 /sbin/init'; echo '2 1 /usr/bin/python3';; *) echo '77 {}/install/releases/fixture/agentq service run';; esac\n", root.display());
    std::fs::write(&ps, running_ps).unwrap();
    std::fs::write(root.join("install/status.json"), r#"{"pid":77,"version":"fixture","at":1}"#).unwrap();
    assert_eq!(value(&root, &["status", "--json"])["service"]["state"], "running", "unknown client dependencies do not invalidate a fresh service PID/heartbeat");
    std::fs::write(&ps, "#!/bin/sh\nexit 1\n").unwrap();
    assert_eq!(value(&root, &["status", "--json"])["service"]["state"], "unknown", "failed service process inventory remains unknown");
    std::fs::write(&ps, &idle_ps).unwrap();
    value(&root, &["disable"]);
    let saved = std::fs::read(root.join("install/data/integration.json")).unwrap();
    let s: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(s["service"]["loaded"], true, "an idle loaded service must be resumed by enable");
    assert_eq!(s["service"]["state"], "stopped");
    assert!(unit.exists(), "the saved service recipe survives unload");
    assert_eq!(value(&root, &["status", "--json"])["service"]["state"], "stopped");
    std::fs::write(&ps, "#!/bin/sh\ncase \"$*\" in *ppid*) echo '1 0 /sbin/init'; echo '2 1 /usr/bin/python3';; *) echo '1 /sbin/init';; esac\n").unwrap();
    let with_interpreter = value(&root, &["status", "--json"]);
    assert_eq!(with_interpreter["service"]["state"], "stopped", "an unrelated interpreter cannot invalidate intentional service shutdown");
    assert_eq!(profile(&with_interpreter["configuration"], "claude-code")["integration"]["runtime"], "unknown", "the same interpreter still blocks safe drain/runtime claims");
    std::fs::write(&ps, &idle_ps).unwrap();
    // A half-finished stop is never promoted to a completed stop by process absence alone.
    let mut stopping = s.clone();
    stopping["service"]["state"] = "stopping".into();
    stopping["service"]["owner"] = std::process::id().into();
    std::fs::write(root.join("install/data/integration.json"), stopping.to_string()).unwrap();
    assert_eq!(value(&root, &["status", "--json"])["service"]["state"], "unknown");
    std::fs::write(root.join("install/data/integration.json"), saved).unwrap();
    value(&root, &["disable"]);
    assert_eq!(std::fs::read_to_string(root.join("unit-actions")).unwrap(), "unload\n", "repeated disable does not unload an absent registration");
    value(&root, &["enable"]);
    assert_eq!(std::fs::read_to_string(root.join("unit-actions")).unwrap(), "unload\nload\n");
    assert!(root.join("unit-loaded").exists());
    assert_eq!(value(&root, &["status", "--json"])["host_disabled"], false);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn switches_detach_recovery_and_native_output_cross_cli_boundaries() {
    let root = setup();
    let settings = std::fs::read(root.join("claude/settings.json")).unwrap();
    let original = root.join("original.txt");
    std::fs::write(&original, "original bytes survive both controls\n").unwrap();
    let indexed = mcp(&root, "index_document", json!({"path":original}));
    let indexed: Value = serde_json::from_str(indexed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    value(&root, &["optimization", "off", "claude"]);
    value(&root, &["optimization", "off"]);
    let paused = cli(&root, &["hook", "claude", "PreToolUse"], Some(r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#));
    assert_eq!(serde_json::from_slice::<Value>(&paused.stdout).unwrap(), json!({}), "pause must keep the native command and its approval classification unchanged");
    assert_ne!(mcp(&root, "index_document", json!({"path":original}))["result"]["isError"], true, "compression off keeps MCP available");
    assert_eq!(std::fs::read(root.join("claude/settings.json")).unwrap(), settings);
    let preview = value(&root, &["disable", "claude", "--dry-run"]);
    assert_eq!(preview["outcome"], "planned");
    assert_eq!(preview["blocked"], false);
    assert!(!root.join("install/data/integration.json").exists());
    assert_eq!(std::fs::read(root.join("claude/settings.json")).unwrap(), settings);
    value(&root, &["disable", "claude"]);
    let status = value(&root, &["status", "--json"]);
    let p = profile(&status["configuration"], "claude-code");
    assert_eq!(p["integration"]["desired"], "disabled");
    assert_eq!(p["integration"]["configuration"], "detached");
    assert_eq!(p["hooks"], "absent");
    assert_eq!(p["mcp"], "absent");
    assert_eq!(p["optimization_reason"], "integration_disabled");
    let hook = cli(&root, &["hook", "claude", "PreToolUse"], Some(r#"{"tool_name":"Bash","tool_input":{"command":"cargo test"}}"#));
    assert_eq!(serde_json::from_slice::<Value>(&hook.stdout).unwrap(), json!({}));
    assert_eq!(mcp(&root, "index_document", json!({"path":original}))["result"]["isError"], true);
    let recovered = mcp(&root, "get_original", json!({"ref": indexed["ref"]}));
    let recovered: Value = serde_json::from_str(recovered["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(recovered["content"], "original bytes survive both controls\n");
    let denied = cli(&root, &["onboard", "--yes", "claude"], None);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("integration_disabled"));
    let projection = value(&root, &["configure"]);
    let id = profile(&projection, "claude-code")["id"].as_str().unwrap();
    let denied = cli(&root, &["configure", id, "hooks=registered", "--yes"], None);
    assert!(!denied.status.success());
    assert!(String::from_utf8_lossy(&denied.stdout).contains("integration_disabled"));
    value(&root, &["disable"]);
    let denied = cli(&root, &["service", "install"], None);
    assert!(!denied.status.success(), "a disabled host must not reach the OS unit manager");
    assert!(String::from_utf8_lossy(&denied.stderr).contains("integration_disabled"));
    value(&root, &["enable", "codex"]);
    let s = value(&root, &["status", "--json"]);
    assert_eq!(profile(&s["configuration"], "codex")["integration"]["desired"], "enabled");
    assert_eq!(profile(&s["configuration"], "claude-code")["integration"]["desired"], "disabled");
    value(&root, &["enable"]);
    value(&root, &["disable", "claude-desktop"]);
    let req = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"index_document","arguments":{"path":original}}}).to_string()+"\n";
    let desktop = cli(&root, &["mcp", "claude-desktop"], Some(&req));
    assert_eq!(serde_json::from_slice::<Value>(&desktop.stdout).unwrap()["result"]["isError"], true);
    assert_ne!(mcp(&root, "index_document", json!({"path":original}))["result"]["isError"], true, "Desktop Chat disable keeps Claude Code MCP independent");
    value(&root, &["enable", "claude-desktop"]);
    value(&root, &["optimization", "on"]);
    let s = value(&root, &["status", "--json"]);
    assert_eq!(profile(&s["configuration"], "claude-code")["optimization_preference"], "disabled");
    let config: Value = serde_json::from_str(&std::fs::read_to_string(root.join("claude/.claude.json")).unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["foreign"]["command"], "foreign-tool");
    let settings: Value = serde_json::from_str(&std::fs::read_to_string(root.join("claude/settings.json")).unwrap()).unwrap();
    assert_eq!(settings["permissions"]["defaultMode"], "default");
    assert_eq!(settings["model"], "fixture-native-model");
    let native = root.join(format!("bin/cargo{}", std::env::consts::EXE_SUFFIX));
    assert!(Command::new("rustc").args(["--edition=2024", "tests/fixtures/compression-runner.rs", "-o"]).arg(&native).status().unwrap().success());
    let rtk = root.join(format!("install/engines/rtk-0.51.0/rtk{}", std::env::consts::EXE_SUFFIX));
    std::fs::create_dir_all(rtk.parent().unwrap()).unwrap();
    std::fs::copy(&native, rtk).unwrap();
    let out = cli(&root, &["run", "--", "cargo", "test"], None);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(out.stdout, "test result with preserved details\n".repeat(100).as_bytes());
    assert_eq!(out.stderr, b"native stderr\n");
    assert!(!root.join("install/data/runs").exists(), "disabled runs do not capture native output");
    value(&root, &["optimization", "on", "claude"]);
    let out = cli(&root, &["run", "--", "cargo", "test"], None);
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(out.stderr, b"native stderr\n");
    assert!(String::from_utf8_lossy(&out.stdout).contains("get_original"));
    assert_eq!(std::fs::read_to_string(root.join("calls")).unwrap().lines().count(), 2, "no native rerun");
    assert_eq!(value(&root, &["enable"])["outcome"], "unchanged");
    std::fs::remove_dir_all(root).unwrap();
}
