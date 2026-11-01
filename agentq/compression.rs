//! Tool result -> protected output -> retained original -> local record (agentq.md §3).
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Private runner replay: native execution already finished; RTK receives the same streams and exit status.
pub fn replay() -> Option<i32> {
    let dir = std::env::var_os("AGENTQ_RUN_REPLAY")?;
    let dir = Path::new(&dir);
    let exe = std::env::current_exe().ok()?;
    if exe.parent()? != dir.join("bin") {
        return None;
    }
    let replay = || -> Result<i32, std::io::Error> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join("replay-started"))?;
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if std::env::var("AGENTQ_RUN_DIGEST").ok().as_deref()
            != Some(crate::configure::digest(&json!(args)).as_str())
        {
            return Err(std::io::Error::other("recipe changed native arguments"));
        }
        let stdout = std::fs::read(dir.join("stdout"))?;
        let stderr = std::fs::read(dir.join("stderr"))?;
        let code = std::fs::read_to_string(dir.join("exit"))?
            .parse()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::io::stdout().write_all(&stdout)?;
        std::io::stderr().write_all(&stderr)?;
        std::fs::write(dir.join("replayed"), b"")?;
        Ok(code)
    };
    Some(replay().unwrap_or_else(|_| {
        let _ = std::fs::write(dir.join("replay-error"), b"");
        127
    }))
}

fn exit(s: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(sig) = std::os::unix::process::ExitStatusExt::signal(&s) {
        return 128 + sig;
    }
    s.code().unwrap_or(1)
}

/// Pinned RTK 0.51.0 pytest prepends display flags; the replay still returns the original native capture.
fn replay_args(cmd: &[String]) -> Vec<String> {
    let mut args = vec![];
    if cmd[0] == "pytest" {
        let native = &cmd[1..];
        if !native.iter().any(|a| a.starts_with("--tb")) { args.push("--tb=short".into()); }
        if !native.iter().any(|a| a == "-q" || a == "--quiet") { args.push("-q".into()); }
        if !native.iter().any(|a| a.starts_with("-r") && !a.starts_with("--")) { args.push("-rxX".into()); }
    }
    args.extend_from_slice(&cmd[1..]);
    args
}

/// A bounded RTK recipe over an already executed native capture; never re-executes the user's command.
fn filter(
    install: &Path,
    cmd: &[String],
    raw: &[u8],
    stderr: &[u8],
    code: i32,
    id: &str,
) -> Result<Vec<u8>, &'static str> {
    let start = Instant::now();
    let rtk = crate::engines::rtk_path(install);
    if !rtk.is_file() {
        return Err("no_engine");
    }
    let dir = install.join("data/runs").join(id);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).map_err(|_| "error")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [install.join("data/runs"), dir.clone()] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "error")?;
        }
    }
    let setup = || -> Result<(), std::io::Error> {
        std::fs::write(dir.join("stdout"), raw)?;
        std::fs::write(dir.join("stderr"), stderr)?;
        std::fs::write(dir.join("exit"), code.to_string())?;
        // RTK's databases store command arguments. A file parent makes either database unavailable.
        std::fs::write(dir.join("no-engine-db"), b"")?;
        let exe = std::env::current_exe()?;
        std::fs::copy(
            exe,
            bin.join(format!("{}{}", cmd[0], std::env::consts::EXE_SUFFIX)),
        )?;
        Ok(())
    };
    setup().map_err(|_| "error")?;
    let stdout = std::fs::File::create(dir.join("filtered")).map_err(|_| "error")?;
    let mut c = Command::new(rtk);
    // Only the private replay can resolve on PATH; the engine cannot find a native runner to execute again.
    c.args(cmd)
        .current_dir(&bin)
        .env("PATH", &bin)
        .env("AGENTQ_RUN_REPLAY", &dir)
        .env(
            "AGENTQ_RUN_DIGEST",
            crate::configure::digest(&json!(replay_args(cmd))),
        )
        .env("RTK_DB_PATH", dir.join("no-engine-db/gain.db"))
        .env("RTK_RECALL_DB", dir.join("no-engine-db/recall.db"))
        .env("RTK_TELEMETRY_DISABLED", "1")
        .env("RTK_SUPPRESS_HOOK_WARNING", "1")
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    if start.elapsed() >= Duration::from_secs(3) {
        return Err("timeout");
    }
    let mut child = coreq::quiet(&mut c).spawn().map_err(|_| "error")?;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                if exit(s) != code {
                    return Err("error");
                }
                // Require proof that only the captured invocation was replayed.
                if !dir.join("replayed").is_file() || dir.join("replay-error").exists() {
                    return Err("error");
                }
                let output = std::fs::read(dir.join("filtered")).map_err(|_| "error")?;
                return if start.elapsed() < Duration::from_secs(3) {
                    Ok(output)
                } else {
                    Err("timeout")
                };
            }
            Ok(None) if start.elapsed() < Duration::from_secs(3) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                #[cfg(windows)]
                {
                    let _ = coreq::quiet(
                        Command::new("taskkill")
                            .args(["/PID", &child.id().to_string(), "/T", "/F"])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null()),
                    )
                    .status();
                }
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err("timeout");
            }
        }
    }
}

pub fn run(install: &Path, cmd: &[String]) -> i32 {
    let start = Instant::now();
    let data = install.join("data");
    let agent = std::env::var("AGENTQ_AGENT").unwrap_or_else(|_| "unknown".into());
    let ids = json!({"session":std::env::var("AGENTQ_SESSION").ok(),"turn":std::env::var("AGENTQ_TURN").ok(),"tool_call":std::env::var("AGENTQ_TOOL_CALL").ok()});
    let qualified = crate::engines::qualified(cmd);
    let c = policy(&data, &agent);
    let id = crate::mcp::new_id();
    let run_dir = install.join("data/runs").join(&id);
    let mut bypass = match &c {
        Err(_) => Some("error"),
        Ok(c) if c["enabled"] == false || c["agents"][&agent] == false => Some("disabled"),
        _ if !qualified => Some("not_qualified"),
        _ if !crate::engines::rtk_path(install).is_file() => Some("no_engine"),
        _ => None,
    };
    if bypass.is_none() && std::fs::create_dir_all(&run_dir).is_err() {
        bypass = Some("error");
    }
    if let Some(reason) = bypass {
        let status = Command::new(&cmd[0])
            .args(&cmd[1..])
            .env_remove("AGENTQ_RUN_REPLAY")
            .status();
        let code = match &status {
            Ok(s) => exit(*s),
            Err(e) => {
                eprintln!("agentq: {}: {e}", cmd[0]);
                127
            }
        };
        let mut r = record(&agent, "run", "shell", &ids, 0, crate::now());
        if qualified {
            r["runner"] = json!(if cmd[0] == "pytest" {
                "pytest".into()
            } else {
                cmd[..2].join(" ")
            });
        }
        r["reason"] = json!(if status.is_err() { "error" } else { reason });
        r["outcome"] = json!(
            if status.is_err() || ["error", "no_engine"].contains(&reason) {
                "failed"
            } else {
                "passed"
            }
        );
        for k in ["bytes_in", "bytes_out", "est_tokens_in", "est_tokens_out"] {
            r[k] = Value::Null;
        }
        r["exit_code"] = json!(code);
        r["duration_ms"] = json!(start.elapsed().as_millis() as u64);
        if let Err(e) = save(&data, &c.unwrap_or(Value::Null), &r) {
            eprintln!("agentq: stats unknown: {e}");
        }
        return code;
    }
    let step = Instant::now();
    let native = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::inherit())
        .env_remove("AGENTQ_RUN_REPLAY")
        .output();
    crate::profile::trace(
        &data,
        "compress.run",
        "native",
        step.elapsed(),
        native.as_ref().err().map(|_| "error"),
    );
    let native = match native {
        Ok(o) => o,
        Err(e) => {
            eprintln!("agentq: {}: {e}", cmd[0]);
            let _ = std::fs::remove_dir(&run_dir);
            let mut r = record(&agent, "run", "shell", &ids, 0, crate::now());
            r["outcome"] = json!("failed");
            r["reason"] = json!("error");
            r["exit_code"] = json!(127);
            if crate::engines::qualified(cmd) {
                r["runner"] = json!(if cmd[0] == "pytest" {
                    "pytest".into()
                } else {
                    cmd[..2].join(" ")
                });
            }
            r["duration_ms"] = json!(start.elapsed().as_millis() as u64);
            if let Err(e) = save(&data, &c.unwrap_or(Value::Null), &r) {
                eprintln!("agentq: stats unknown: {e}");
            }
            return 127;
        }
    };
    let code = exit(native.status);
    let mut r = record(
        &agent,
        "run",
        "shell",
        &ids,
        native.stdout.len(),
        crate::now(),
    );
    r["exit_code"] = json!(code);
    if qualified {
        r["runner"] = json!(if cmd[0] == "pytest" {
            "pytest".into()
        } else {
            cmd[..2].join(" ")
        });
    }
    let reason = match &c {
        Ok(c) => guard(c, &agent, &native.stdout),
        Err(_) => Some("error"),
    };
    let mut text = native.stdout.clone();
    let mut reason = reason.unwrap_or(if qualified {
        "no_engine"
    } else {
        "not_qualified"
    });
    if reason == "no_engine" && qualified {
        let rtk = crate::engines::rtk_path(install).is_file();
        r["engine"] = json!(if rtk { "rtk" } else { "none" });
        r["engine_version"] = json!(rtk.then_some(crate::engines::RTK));
        let step = Instant::now();
        let filtered = filter(install, cmd, &native.stdout, &native.stderr, code, &id);
        crate::profile::trace(
            &data,
            "compress.run",
            "filter",
            step.elapsed(),
            filtered.as_ref().err().copied(),
        );
        match filtered {
            Ok(mut candidate) => {
                let reference = format!("rtk:{id}");
                candidate.extend_from_slice(
                    format!("\n[AgentQ original ref: {reference}; use get_original.]\n")
                        .as_bytes(),
                );
                if candidate.len() < native.stdout.len()
                    && std::str::from_utf8(&native.stdout).is_ok()
                    && std::str::from_utf8(&candidate).is_ok()
                {
                    text = candidate;
                    reason = "compressed";
                    r["ref"] = json!(reference);
                } else {
                    reason = "not_smaller";
                }
            }
            Err(why) => reason = why,
        }
        // Replays are executable scratch, never retained with user originals.
        let bin = install.join("data/runs").join(&id).join("bin");
        let _ = std::fs::remove_dir_all(bin);
    }
    if run_dir.exists() {
        crate::engines::prune(&install.join("data/runs"));
    }
    if reason == "compressed" && !run_dir.join("stdout").is_file() {
        text = native.stdout.clone();
        reason = "error";
        r["ref"] = Value::Null;
    }
    r["reason"] = json!(reason);
    r["outcome"] = json!(if reason == "compressed" {
        "compressed"
    } else if ["error", "timeout", "no_engine"].contains(&reason) {
        "failed"
    } else {
        "passed"
    });
    r["bytes_out"] = json!(text.len());
    r["est_tokens_out"] = json!(text.len().div_ceil(4));
    r["duration_ms"] = json!(start.elapsed().as_millis() as u64);
    if let Err(e) = save(&data, &c.unwrap_or(Value::Null), &r) {
        text = native.stdout.clone();
        eprintln!("agentq: stats unknown: {e}; native output preserved");
    }
    let _ = std::io::stdout().write_all(&text);
    let _ = std::io::stderr().write_all(&native.stderr);
    code
}

fn config(data: &Path) -> Result<Value, String> {
    match std::fs::read(data.join("config.json")) {
        Ok(b) => {
            let v: Value = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
            if !v.is_object() {
                return Err("config requires object".into());
            }
            let Some(c) = v.get("compression") else {
                return Ok(Value::Null);
            };
            let m = c.as_object().ok_or("compression config requires object")?;
            if m.get("enabled").is_some_and(|v| !v.is_boolean())
                || m.get("min_bytes").is_some_and(|v| v.as_u64().is_none())
                || m.get("stats")
                    .is_some_and(|v| !matches!(v.as_str(), Some("off" | "queue")))
            {
                return Err("invalid compression defaults".into());
            }
            if let Some(agents) = m.get("agents") {
                if !agents
                    .as_object()
                    .is_some_and(|a| a.values().all(Value::is_boolean))
                {
                    return Err("compression agents requires boolean preferences".into());
                }
            }
            Ok(c.clone())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(e) => Err(e.to_string()),
    }
}

pub(crate) fn policy(data: &Path, agent: &str) -> Result<Value, String> {
    let mut c = config(data)?;
    // § 12 M12.7: a launch with optimization `off` disables compression for its session only.
    if crate::control::disabled(data, agent)? || std::env::var(agentq_runtime::optimization::ENV).is_ok_and(|v| v == "off") {
        if c.is_null() { c = json!({}); }
        c["enabled"] = false.into();
    }
    Ok(c)
}

fn guard(c: &Value, agent: &str, text: &[u8]) -> Option<&'static str> {
    if c["enabled"] == false || c["agents"][agent] == false {
        return Some("disabled");
    }
    if text.len() < c["min_bytes"].as_u64().unwrap_or(2048) as usize {
        return Some("short");
    }
    if serde_json::from_slice::<Value>(text).is_ok() {
        return Some("json");
    }
    let s = String::from_utf8_lossy(text);
    // Fail closed on code markers; no qualified recipe operates on source text.
    if s.contains("```")
        || s.lines().any(|l| {
            [
                "fn ",
                "pub ",
                "def ",
                "class ",
                "function ",
                "import ",
                "#include",
                "const ",
                "let ",
            ]
            .iter()
            .any(|p| l.trim_start().starts_with(p))
        })
        || [
            " must ", " MUST ", " never ", " NEVER ", " shall ", " SHALL ",
        ]
        .iter()
        .any(|p| s.contains(p))
    {
        return Some("code");
    }
    None
}

fn record(agent: &str, adapter: &str, tool: &str, input: &Value, bytes: usize, at: u64) -> Value {
    json!({"schema":1,"event_id":crate::mcp::new_id().replace('-',""),"time":at,
        "agent":agent,"adapter":adapter,"tool":tool,"runner":null,"engine":"none",
        "outcome":"passed","reason":"not_qualified","bytes_in":bytes,"bytes_out":bytes,
        "est_tokens_in":bytes.div_ceil(4),"est_tokens_out":bytes.div_ceil(4),"duration_ms":0,"ref":null,
        "session":input["session"],"turn":input["turn"],"tool_call":input["tool_call"],
        "version":env!("CARGO_PKG_VERSION"),"engine_version":null})
}

pub(crate) fn stats_date(name: &str) -> Option<&str> {
    let d = name.strip_suffix(".jsonl")?;
    (d.len() == 10
        && d.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        }))
    .then_some(d)
}

/// The producer's coverage journal in `data/stats/` (ACCOUNTING O3).
pub const COVERAGE: &str = "coverage.json";
/// Prefix of markers left in `data/` when a save fails before or at its journal write: the journal no longer proves
/// coverage until the next written journal resets it. Each failure writes its own file, so a reset removes only the
/// markers it saw. They live outside `data/stats/`, so an unwritable stats folder still leaves evidence.
pub const FAILED: &str = "coverage.failed";

/// Failure markers held in `data`.
pub fn failed(data: &Path) -> Vec<std::path::PathBuf> {
    let es = std::fs::read_dir(data).into_iter().flatten().flatten();
    es.filter(|e| e.file_name().to_string_lossy().starts_with(FAILED)).map(|e| e.path()).collect()
}

/// Durable unknown for a save that may have lost its record; works without the lifecycle lock.
fn mark_failed(data: &Path, at: u64) {
    let name = format!("{FAILED}.{}.{}", std::process::id(), crate::mcp::new_id());
    if let Err(e) = std::fs::create_dir_all(data).and_then(|()| std::fs::write(data.join(name), at.to_string())) {
        eprintln!("agentq: stats coverage marker: {e}");
    }
}

/// Coverage proven by this store: `since` is its first record; every later call wrote a record unless `gaps` holds its
/// second; `counts` holds the records each day file must still contain, so a lost or truncated file is detectable. A missing journal over held records, or an unreadable one, restarts `since` at the next second and adds a reset: the
/// older records stay countable but unproven, and `counts` restarts from the records held now.
pub(crate) fn coverage(journal: Option<&[u8]>, held: bool, lines: &serde_json::Map<String, Value>, at: u64, reset: u64) -> Value {
    let fresh = |since: u64, resets: Vec<u64>| json!({"schema":1,"since":since,"resets":resets,"gaps":[],"counts":lines});
    // A reset proves only the next second on: an earlier call in its own second may already be lost.
    match journal {
        None if !held => fresh(at, vec![]),
        None => fresh(reset + 1, vec![reset]),
        Some(b) => match serde_json::from_slice::<Value>(b) {
            Ok(v) if journal_valid(&v) => v,
            _ => fresh(reset + 1, vec![reset]),
        },
    }
}

/// The whole journal shape, as the producer writes it; anything else proves no coverage.
pub fn journal_valid(v: &Value) -> bool {
    v["schema"] == 1
        && v["since"].is_u64()
        && v["resets"]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_u64))
        && v["gaps"].as_array().is_some_and(|a| {
            a.iter().all(|g| {
                g.as_array().is_some_and(|p| p.len() == 2)
                    && g[0].as_u64().zip(g[1].as_u64()).is_some_and(|(x, y)| x < y)
            })
        })
        && v["counts"]
            .as_object()
            .is_some_and(|m| m.iter().all(|(k, n)| stats_date(&format!("{k}.jsonl")).is_some() && n.is_u64()))
}

/// Local writes serialize with the existing lifecycle lock; sink errors never change returned tool bytes. A failure before
/// the journal write (lock timeout, unreadable store) marks coverage unknown, since the record may be lost.
fn save(data: &Path, c: &Value, r: &Value) -> Result<(), String> {
    let mut journaled = false;
    let res = save_locked(data, c, r, &mut journaled);
    if res.is_err() && !journaled {
        mark_failed(data, r["time"].as_u64().unwrap_or_else(crate::now));
    }
    res
}

fn save_locked(data: &Path, c: &Value, r: &Value, journaled: &mut bool) -> Result<(), String> {
    let _lock = crate::executor::lock(data)?;
    let at = r["time"].as_u64().ok_or("missing time")?;
    let failed = failed(data);
    crate::local::save_locked(data, r, !failed.is_empty())?;
    *journaled = true;
    for marker in failed { let _ = std::fs::remove_file(marker); }
    drop(_lock);
    if c["stats"] != "off" {
        let mut a = serde_json::Map::new();
        for (k, v) in r.as_object().ok_or("invalid record")? {
            a.insert(format!("agentq.{k}"), v.clone());
        }
        a.insert("agentq.est_tokens_kind".into(), json!("estimate"));
        if ["session", "turn", "tool_call"]
            .iter()
            .any(|k| r[k].is_null())
        {
            a.insert(
                "agentq.ids_unknown".into(),
                json!("native IDs not supplied"),
            );
        }
        if crate::telemetry::emit_at(
            data,
            "agentq.compress",
            "end",
            Value::Object(a),
            at as u128 * 1_000_000_000,
        )
        .is_none()
        {
            eprintln!("agentq: compression queue unavailable; local record retained");
        }
    }
    Ok(())
}

/// Reuse the qualified RTK capture replay; no command in JSON is ever executed.
pub fn compress(data: &Path, agent: &str, input: &str) -> Result<Value, String> {
    let start = Instant::now();
    if agent.is_empty() {
        return Err("compression requires agent".into());
    }
    let v: Value =
        serde_json::from_str(input).map_err(|e| format!("invalid compression input: {e}"))?;
    let text = v["text"]
        .as_str()
        .ok_or("compression input requires text string")?;
    let adapter = v["adapter"]
        .as_str()
        .ok_or("compression input requires adapter")?;
    if !["hook", "provider"].contains(&adapter) {
        return Err("result compression requires hook or provider; run uses agentq run".into());
    }
    let tool = v["tool"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("compression input requires tool")?;
    if !v["command"].is_null() && !v["command"].is_string() {
        return Err("command requires string or null".into());
    }
    for key in ["session", "turn", "tool_call"] {
        if !v[key].is_null() && !v[key].is_string() {
            return Err(format!("{key} requires string or null"));
        }
    }
    if !v["exit_code"].is_null() && v["exit_code"].as_i64().and_then(|n| i32::try_from(n).ok()).is_none() {
        return Err("exit_code requires a 32-bit integer or null".into());
    }
    let mut r = record(agent, adapter, tool, &v, text.len(), crate::now());
    let words: Vec<String> = v["command"].as_str().unwrap_or("").split_ascii_whitespace().map(String::from).collect();
    let qualified = (["terminal", "shell", "exec_command"].contains(&tool) || (agent == "claude" && adapter == "hook" && tool == "Bash")) && crate::engines::qualified(&words);
    if qualified {
        r["runner"] = json!(if words[0] == "pytest" { "pytest".into() } else { words[..2].join(" ") });
    }
    let c = policy(data, agent);
    let mut output = text.to_owned();
    let mut reference = Value::Null;
    let mut reason = match &c {
        Ok(c) => guard(c, agent, text.as_bytes()).unwrap_or("not_qualified"),
        Err(_) => "error",
    };
    if reason == "not_qualified" && qualified {
        if let (Some(install), Some(code)) = (data.parent(), v["exit_code"].as_i64()) {
            let id = crate::mcp::new_id();
            r["engine"] = json!("rtk");
            r["engine_version"] = json!(crate::engines::RTK);
            match filter(install, &words, text.as_bytes(), b"", code as i32, &id) {
                Ok(bytes) => {
                    let original = format!("rtk:{id}");
                    match String::from_utf8(bytes) {
                        Ok(candidate) => {
                            let candidate = format!("{candidate}\n[AgentQ original ref: {original}; use get_original.]\n");
                            if candidate.len() < text.len() {
                                output = candidate; reference = json!(original); reason = "compressed";
                            } else { reason = "not_smaller"; }
                        }
                        Err(_) => reason = "error",
                    }
                }
                Err(why) => reason = why,
            }
            let _ = std::fs::remove_dir_all(data.join("runs").join(&id).join("bin"));
            crate::engines::prune(&data.join("runs"));
        }
    }
    r["ref"] = reference.clone();
    r["reason"] = json!(reason);
    r["outcome"] = json!(if reason == "compressed" { "compressed" } else if ["error", "timeout", "no_engine"].contains(&reason) { "failed" } else { "passed" });
    r["bytes_out"] = json!(output.len());
    r["est_tokens_out"] = json!(output.len().div_ceil(4));
    r["duration_ms"] = json!(start.elapsed().as_millis() as u64);
    if let Err(e) = save(data, &c.unwrap_or(Value::Null), &r) {
        eprintln!("agentq: stats unknown: {e}; original preserved");
        return Ok(json!({"text":text,"ref":null}));
    }
    Ok(json!({"text":output,"ref":reference}))
}

pub(crate) fn validate_record(v: &Value) -> Result<(), String> {
    v["agent"].as_str().ok_or("stats unknown: missing agent")?;
    v["engine"]
        .as_str()
        .filter(|e| ["none", "rtk", "headroom"].contains(e))
        .ok_or("stats unknown: invalid engine")?;
    v["outcome"]
        .as_str()
        .filter(|o| ["compressed", "passed", "failed"].contains(o))
        .ok_or("stats unknown: invalid outcome")?;
    if v["schema"] != 1 {
        return Err("stats unknown: unsupported schema".into());
    }
    if !v["event_id"]
        .as_str()
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        || ["time", "duration_ms"]
            .iter()
            .any(|k| v[k].as_u64().is_none())
        || ["bytes_in", "bytes_out", "est_tokens_in", "est_tokens_out"]
            .iter()
            .any(|k| {
                v.get(k).is_none()
                    || (v[k].as_u64().is_none()
                        && !(v["adapter"] == "run"
                            && v[k].is_null()
                            && v["outcome"] != "compressed"))
            })
        || !v["adapter"]
            .as_str()
            .is_some_and(|s| ["run", "hook", "provider"].contains(&s))
        || v["tool"].as_str().is_none()
        || !v["reason"]
            .as_str()
            .and_then(crate::accounting::outcome_of)
            .is_some_and(|o| v["outcome"] == o)
        || ["runner", "ref", "session", "turn", "tool_call"]
            .iter()
            .any(|k| v.get(k).is_none() || (!v[k].is_null() && !v[k].is_string()))
        || (v["adapter"] == "run" && v["exit_code"].as_i64().is_none())
    {
        return Err("stats unknown: malformed record shape".into());
    }
    Ok(())
}

pub fn records(data: &Path, cutoff: u64) -> Result<Vec<Value>, String> {
    let snapshot = crate::local::snapshot(data)?;
    if let Some(reason) = snapshot["error"].as_str() { return Err(format!("stats unknown: {reason}")); }
    let mut out = vec![];
    for v in snapshot["records"].as_array().ok_or("stats unknown: invalid snapshot")? {
        validate_record(v)?;
        if v["time"].as_u64().is_some_and(|at| at >= cutoff) { out.push(v.clone()); }
    }
    if out.is_empty() { return Err("stats unknown: no current stats file".into()); }
    Ok(out)
}

pub fn stats(data: &Path) -> Result<Value, String> {
    let mut groups = std::collections::BTreeMap::<(String, String), Value>::new();
    for v in records(data, crate::now().saturating_sub(7 * 86400))? {
        // records() checked these strings
        let (agent, engine, outcome) = (
            v["agent"].as_str().unwrap_or_default(),
            v["engine"].as_str().unwrap_or_default(),
            v["outcome"].as_str().unwrap_or_default(),
        );
        let g=groups.entry((agent.into(),engine.into())).or_insert_with(||json!({"agent":agent,"engine":engine,"calls":0,"compressed":0,"passed":0,"failed":0,"bytes_in":0,"bytes_out":0}));
        for (k, add) in [("calls", 1), (outcome, 1)] {
            g[k] = json!(
                g[k].as_u64()
                    .unwrap()
                    .checked_add(add)
                    .ok_or("stats unknown: count overflow")?
            );
        }
        for k in ["bytes_in", "bytes_out"] {
            g[k] = match (g[k].as_u64(), v[k].as_u64()) {
                (Some(a), Some(b)) => {
                    json!(a.checked_add(b).ok_or("stats unknown: byte overflow")?)
                }
                _ => Value::Null,
            };
            if g[k].is_null() {
                g["bytes_unknown"] = json!("native output passed through without capture");
            }
        }
    }
    Ok(json!({"schema":1,"groups":groups.into_values().collect::<Vec<_>>()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// §3 / VERIFICATION A2,A4: result -> protected bytes -> one metadata-only record -> aggregate; corrupt/missing is unknown.
    #[test]
    fn pinned_pytest_replay_injection_matches_exact_native_args() {
        let args = |s: &str| s.split_ascii_whitespace().map(String::from).collect::<Vec<_>>();
        assert_eq!(replay_args(&args("pytest -v test_native.py")), args("--tb=short -q -rxX -v test_native.py"));
        assert_eq!(replay_args(&args("pytest --tb=no --quiet -ra test_native.py")), args("--tb=no --quiet -ra test_native.py"));
        assert_eq!(replay_args(&args("pytest --randomly-seed=1")), args("--tb=short -q -rxX --randomly-seed=1"));
        assert_eq!(replay_args(&args("cargo test --color never")), args("test --color never"));
    }

    #[test]
    fn result_stats_chain() {
        let d = std::env::temp_dir().join(format!("agentq-compression-{}", crate::mcp::new_id()));
        assert!(stats(&d).unwrap_err().contains("unknown"));
        for bad in [
            "{",
            r#"{"text":3}"#,
            r#"{"text":"x","adapter":"run","tool":"shell"}"#,
        ] {
            assert!(compress(&d, "hermes", bad).is_err());
        }
        assert!(!d.exists());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("config.json"),
            r#"{"compression":{"min_bytes":0,"stats":"off"}}"#,
        )
        .unwrap();
        for (text, why) in [
            ("{}", "json"),
            ("fn f() {}", "code"),
            ("a long tool result", "not_qualified"),
        ] {
            let input = json!({"text":text,"adapter":"hook","tool":"terminal","command":"secret argument","session":"s"});
            assert_eq!(
                compress(&d, "hermes", &input.to_string()).unwrap(),
                json!({"text":text,"ref":null})
            );
            let snapshot = crate::local::snapshot(&d).unwrap();
            let r = snapshot["records"].as_array().unwrap().last().unwrap();
            let s = r.to_string();
            assert_eq!(r["reason"], why);
            assert_eq!(r.as_object().unwrap().len(), 21);
            assert_eq!(r["version"],crate::VERSION);
            assert!(r["engine_version"].is_null());
            assert!(!s.contains("secret") && !s.contains(text));
        }
        assert_eq!(stats(&d).unwrap()["groups"][0]["calls"], 3);
        let at = crate::now();
        let mut expired = record(
            "hermes",
            "hook",
            "terminal",
            &Value::Null,
            10,
            at - 7 * 86400 - 120,
        );
        let boundary = d
            .join("stats")
            .join(format!("{}.jsonl", crate::date(at - 7 * 86400)));
        let ancient = d
            .join("stats")
            .join(format!("{}.jsonl", crate::date(at - 8 * 86400)));
        // Align expired data to the boundary partition even when UTC is within two minutes of midnight.
        expired["time"] = json!(at - 7 * 86400 - 120);
        std::fs::create_dir_all(d.join("stats")).unwrap();
        std::fs::write(&boundary, format!("{expired}\n")).unwrap();
        std::fs::write(&ancient, "expired data\n").unwrap();
        assert_eq!(stats(&d).unwrap()["groups"][0]["calls"], 3);
        let input = json!({"text":"result","adapter":"provider","tool":"shell","command":"cargo test --private-arg"});
        std::fs::write(
            d.join("config.json"),
            r#"{"compression":{"min_bytes":0,"stats":"queue","agents":{"codex":false}}}"#,
        )
        .unwrap();
        assert_eq!(
            compress(&d, "codex", &input.to_string()).unwrap()["text"],
            "result"
        );
        assert!(ancient.exists(), "migration preserves legacy files");
        assert_eq!(std::fs::read_to_string(&boundary).unwrap(),format!("{expired}\n"));
        assert!(crate::local::snapshot(&d).unwrap()["records"].as_array().unwrap().iter().all(|r|r["time"].as_u64().unwrap()>=at-7*86400));
        let q: Vec<_> = std::fs::read_dir(d.join("queue"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|s| s == "json"))
            .collect();
        assert_eq!(q.len(), 1);
        let span = std::fs::read_to_string(q[0].path()).unwrap();
        assert!(
            span.contains("agentq.compress")
                && span.contains("estimate")
                && span.contains("disabled")
        );
        assert!(!span.contains("private-arg"));
        let p = d
            .join("stats")
            .join(format!("{}.jsonl", crate::date(crate::now())));
        std::fs::write(p, "bad\n").unwrap();
        assert!(stats(&d).unwrap_err().contains("malformed"));
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: retention trims expired lines without hiding an earlier loss on the oldest day.
    #[test]
    fn retention_keeps_loss_chain() {
        let d = std::env::temp_dir().join(format!("agentq-retention-{}", crate::mcp::new_id()));
        let stats_dir = d.join("stats");
        std::fs::create_dir_all(&stats_dir).unwrap();
        std::fs::write(d.join("config.json"), r#"{"compression":{"min_bytes":0,"stats":"off"}}"#).unwrap();
        let at = crate::now();
        let day = crate::date(at - 7 * 86400);
        let line = |t: u64| record("hermes", "hook", "terminal", &Value::Null, 10, t).to_string();
        // Producer wrote 3 lines; one unexpired line is lost, one expired line remains to be trimmed.
        std::fs::write(
            stats_dir.join(format!("{day}.jsonl")),
            format!("{}\n{}\n", line(at - 7 * 86400 - 120), line(at - 7 * 86400 + 3600)),
        )
        .unwrap();
        std::fs::write(
            stats_dir.join(COVERAGE),
            json!({"schema":1,"since":at - 7 * 86400 - 200,"resets":[],"gaps":[],"counts":{&day:3}}).to_string(),
        )
        .unwrap();
        let input = json!({"text":"result","adapter":"hook","tool":"terminal"});
        compress(&d, "hermes", &input.to_string()).unwrap();
        let j = crate::local::snapshot(&d).unwrap()["journal"].clone();
        assert_eq!(j["counts"][&day], 2, "{j}");
        let a = crate::accounting::local(&d, crate::now());
        assert!(a.to_string().contains("stats records lost"), "{a}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: a failed journal write stays unknown even after the day file is truncated to match it.
    #[test]
    fn journal_write_failure_chain() {
        let d = std::env::temp_dir().join(format!("agentq-journal-fail-{}", crate::mcp::new_id()));
        let stats_dir = d.join("stats");
        std::fs::create_dir_all(&stats_dir).unwrap();
        std::fs::write(d.join("config.json"), r#"{"compression":{"min_bytes":0,"stats":"off"}}"#).unwrap();
        let input = json!({"text":"result","adapter":"hook","tool":"terminal"}).to_string();
        compress(&d, "hermes", &input).unwrap();
        let db = rusqlite::Connection::open(d.join("telemetry.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_journal BEFORE UPDATE ON meta WHEN new.key='journal' BEGIN SELECT RAISE(ABORT,'invented journal failure'); END;").unwrap();
        compress(&d, "hermes", &input).unwrap();
        assert_eq!(crate::local::snapshot(&d).unwrap()["records"].as_array().unwrap().len(),1,"failed coverage update rolls back the event");
        db.execute("DELETE FROM events WHERE kind='stats'",[]).unwrap();
        let a = crate::accounting::local(&d, crate::now());
        assert_eq!(a["unknown"], "stats write failed", "{a}");
        // The next written journal resets coverage and clears the marker.
        db.execute_batch("DROP TRIGGER fail_journal").unwrap();
        drop(db);
        compress(&d, "hermes", &input).unwrap();
        assert!(failed(&d).is_empty());
        let j = crate::local::snapshot(&d).unwrap()["journal"].clone();
        assert_eq!(j["resets"].as_array().unwrap().len(), 1, "{j}");
        assert_eq!(crate::accounting::local(&d, crate::now())["unknown"], crate::accounting::UNPROVEN);
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: a reset leaves its own second unproven, so a call lost earlier in that second is never complete.
    #[test]
    fn same_second_reset_chain() {
        let d = std::env::temp_dir().join(format!("agentq-same-second-{}", crate::mcp::new_id()));
        let stats_dir = d.join("stats");
        std::fs::create_dir_all(&stats_dir).unwrap();
        let c = json!({"stats":"off"});
        let t = crate::now();
        let save_at = || save(&d, &c, &record("hermes", "hook", "terminal", &Value::Null, 10, t));
        save_at().unwrap();
        let db = rusqlite::Connection::open(d.join("telemetry.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_journal BEFORE UPDATE ON meta WHEN new.key='journal' BEGIN SELECT RAISE(ABORT,'invented journal failure'); END;").unwrap();
        assert!(save_at().is_err());
        db.execute("DELETE FROM events WHERE kind='stats'",[]).unwrap();
        db.execute_batch("DROP TRIGGER fail_journal").unwrap(); drop(db);
        save_at().unwrap();
        let mut input = json!({"window": {"start": t, "end": t + 1}, "now": t});
        input["sources"] = json!([crate::accounting::local(&d, t)]);
        let a = crate::accounting::aggregate(&input).unwrap();
        assert_ne!(a["state"], "complete", "{a}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: a save that times out on the lifecycle lock writes no record; coverage stays unknown.
    #[test]
    fn lock_timeout_chain() {
        let d = std::env::temp_dir().join(format!("agentq-lock-timeout-{}", crate::mcp::new_id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"compression":{"min_bytes":0,"stats":"off"}}"#).unwrap();
        let input = json!({"text":"result","adapter":"hook","tool":"terminal"}).to_string();
        compress(&d, "hermes", &input).unwrap();
        let held = crate::executor::lock(&d).unwrap();
        // Another process holds the lock: the std file lock is per open file, so this thread times out like one.
        compress(&d, "hermes", &input).unwrap();
        drop(held);
        let a = crate::accounting::local(&d, crate::now());
        assert_eq!(a["unknown"], "stats write failed", "{a}");
        // A marker written by a call this save never saw survives its reset.
        let late = d.join(format!("{FAILED}.late"));
        compress(&d, "hermes", &input).unwrap();
        assert_eq!(crate::accounting::local(&d, crate::now() + 1)["unknown"], crate::accounting::UNPROVEN);
        std::fs::write(&late, "0").unwrap();
        assert_eq!(crate::accounting::local(&d, crate::now())["unknown"], "stats write failed");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: an immutable stats folder and day file lose the call; evidence outside them keeps it unknown.
    #[cfg(target_os = "macos")]
    #[test]
    fn unwritable_stats_chain() {
        let d = std::env::temp_dir().join(format!("agentq-unwritable-{}", crate::mcp::new_id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"compression":{"min_bytes":0,"stats":"off"}}"#).unwrap();
        let input = json!({"text":"result","adapter":"hook","tool":"terminal"}).to_string();
        compress(&d, "hermes", &input).unwrap();
        let day = d.join("telemetry.sqlite3");
        let flags = |f: &str| {
            let ok = Command::new("chflags").arg(f).arg(&day).status().unwrap().success();
            assert!(ok);
        };
        flags("uchg");
        compress(&d, "hermes", &input).unwrap();
        flags("nouchg");
        let a = crate::accounting::local(&d, crate::now());
        assert_eq!(a["unknown"], "stats write failed", "{a}");
        std::fs::remove_dir_all(d).unwrap();
    }

    /// ACCOUNTING O3 / A4: with data, stats and the day file immutable a call leaves no evidence; a later call must
    /// not read the period as complete.
    #[cfg(target_os = "macos")]
    #[test]
    fn no_evidence_chain() {
        let d = std::env::temp_dir().join(format!("agentq-no-evidence-{}", crate::mcp::new_id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"compression":{"min_bytes":0,"stats":"off"}}"#).unwrap();
        let input = json!({"text":"result","adapter":"hook","tool":"terminal"}).to_string();
        let start = crate::now();
        compress(&d, "hermes", &input).unwrap();
        let day = d.join("telemetry.sqlite3");
        let flags = |f: &str| {
            let ok = Command::new("chflags").arg(f).args([&day, &d]).status().unwrap().success();
            assert!(ok);
        };
        flags("uchg");
        compress(&d, "hermes", &input).unwrap();
        flags("nouchg");
        assert!(failed(&d).is_empty(), "the lost call left no marker");
        compress(&d, "hermes", &input).unwrap();
        let now = crate::now();
        let mut q = json!({"window": {"start": start, "end": now + 1}, "now": now});
        q["sources"] = json!([crate::accounting::local(&d, now)]);
        let a = crate::accounting::aggregate(&q).unwrap();
        assert_eq!((&a["state"], &a["lower_bound"], &a["calls"]), (&json!("partial"), &json!(true), &json!(2)), "{a}");
        assert_eq!(a["coverage"]["sources"][0]["reasons"][0]["reason"], "unproven", "{a}");
        std::fs::remove_dir_all(d).unwrap();
    }
}
