//! Loopback routing (agentq.md § 4, § 5): fixed ports in `data/config.json`, headroom workers owned by the service,
//! per-agent routing state and drift.
//!
//! ponytail: Codex only. Claude Code and Hermes stay direct until their routing lands; Hermes telemetry stays on its own Langfuse exporter.
//! ponytail: one worker per port, respawned each pass; the § 5 engine-pin worker swap joins with the next pin change.

use coreq::write_atomic;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const ROUTED: [&str; 1] = ["codex"];
/// A proxy silent this long reads failed (agentq.md M5.6).
pub const PROXY_LIMIT: u64 = 300;
/// A worker this old and silent this long is replaced. Per worker, because a replacement discards a slow start's progress.
const PROXY_REPLACE: u64 = 900;
/// A reply this recent needs no second probe: the service already probed in this pass.
const PROXY_FRESH: u64 = 5;

fn config(data: &Path) -> Value {
    std::fs::read_to_string(data.join("config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

/// The agent's loopback port, if one was fixed at install.
pub fn port(data: &Path, agent: &str) -> Option<u16> {
    config(data)["ports"][agent].as_u64().and_then(|p| u16::try_from(p).ok())
}

/// The agent's port, picked from a free one on first install and kept across updates (§ 4).
pub fn fix_port(data: &Path, agent: &str) -> Result<u16, String> {
    if let Some(p) = port(data, agent) {
        return Ok(p);
    }
    let p = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|l| l.local_addr()).map_err(|e| format!("no free loopback port: {e}"))?.port();
    let mut c = crate::control::config(data)?;
    c["ports"][agent] = p.into();
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    write_atomic(&data.join("config.json"), &serde_json::to_string_pretty(&c).unwrap_or_default())?;
    Ok(p)
}

pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

fn answers(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(1)).is_ok()
}

/// Whether the proxy on `port` returns 200 to `GET /livez`. A TCP accept proves nothing: the kernel accepts while the interpreter stalls.
fn responds(port: u16) -> bool {
    let probe = || -> std::io::Result<bool> {
        let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(1))?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        s.set_write_timeout(Some(Duration::from_secs(5)))?;
        s.write_all(b"GET /livez HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")?;
        let mut head = [0; 12];
        s.read_exact(&mut head)?;
        Ok(head.starts_with(b"HTTP/1.") && &head[8..] == b" 200")
    };
    probe().unwrap_or(false)
}

/// Whether `port` answers within `limit`, polled twice a second.
fn answers_within(port: u16, limit: Duration) -> bool {
    let end = std::time::Instant::now() + limit;
    while !answers(port) {
        if std::time::Instant::now() >= end {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    true
}

/// `onboard --yes` (#137): each port answers within `limit`, else `restart` runs once and each silent port gets
/// `limit` again. The ports that still do not answer.
pub fn await_proxies(ports: &[u16], limit: Duration, restart: &mut dyn FnMut()) -> Vec<u16> {
    let silent: Vec<u16> = ports.iter().copied().filter(|p| !answers_within(*p, limit)).collect();
    if silent.is_empty() {
        return silent;
    }
    restart();
    silent.into_iter().filter(|p| !answers_within(*p, limit)).collect()
}

/// Package-owned extension assets; the pinned Headroom installation stays immutable.
fn provider_adapter(install: &Path) -> Result<std::path::PathBuf, String> {
    let dir = install.join("releases").join(crate::VERSION).join("provider");
    let meta = dir.join(format!("agentq_provider-{}.dist-info", crate::VERSION));
    std::fs::create_dir_all(&meta).map_err(|e| e.to_string())?;
    let files = [
        (dir.join("agentq_provider.py"), include_str!("../integrations/provider/agentq_provider.py").to_string()),
        (meta.join("METADATA"), format!("Metadata-Version: 2.1\nName: agentq-provider\nVersion: {}\n", crate::VERSION)),
        (meta.join("entry_points.txt"), "[headroom.proxy_extension]\nagentq = agentq_provider:install\n".into()),
    ];
    for (path, text) in files {
        match std::fs::read_to_string(&path) {
            Ok(old) if old == text => {},
            Ok(_) => return Err("immutable provider assets differ".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => write_atomic(&path, &text)?,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(dir)
}

/// Raises this process from the Task Scheduler default (below-normal CPU, low I/O, low memory) to normal, so that
/// the proxy workers it starts inherit normal priority (agentq.md M5.5, evidence in docs/evidence/proxy-275.md).
#[cfg(windows)]
pub fn normal_priority() -> Result<(), String> {
    use std::ffi::c_void;
    const NORMAL_PRIORITY_CLASS: u32 = 0x20;
    const PROCESS_IO_PRIORITY: u32 = 33;
    let io_normal: u32 = 2;
    const PROCESS_MEMORY_PRIORITY: i32 = 0;
    let memory_normal: u32 = 5;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
        fn SetProcessInformation(process: *mut c_void, class: i32, info: *const c_void, size: u32) -> i32;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtSetInformationProcess(process: *mut c_void, class: u32, info: *const c_void, size: u32) -> i32;
    }
    // SAFETY: the pseudo handle needs no close; each call reads one u32 that lives across the call.
    unsafe {
        let me = GetCurrentProcess();
        if SetPriorityClass(me, NORMAL_PRIORITY_CLASS) == 0 {
            return Err(format!("CPU class: {}", std::io::Error::last_os_error()));
        }
        let status = NtSetInformationProcess(me, PROCESS_IO_PRIORITY, (&raw const io_normal).cast(), 4);
        if status != 0 {
            return Err(format!("I/O priority: NTSTATUS {status:#x}"));
        }
        if SetProcessInformation(me, PROCESS_MEMORY_PRIORITY, (&raw const memory_normal).cast(), 4) == 0 {
            return Err(format!("memory priority: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn normal_priority() -> Result<(), String> {
    Ok(())
}

/// Ends a worker and its descendants: on Windows the venv `python.exe` is only the parent of the real interpreter.
fn end_worker(c: &mut Child) {
    if cfg!(windows) {
        let mut kill = Command::new("taskkill");
        kill.args(["/PID", &c.id().to_string(), "/T", "/F"]);
        // An already ended worker fails here as well; the wait below settles it.
        let _ = crate::host::output(&mut kill);
    }
    let _ = c.kill();
    let _ = c.wait();
}

fn proxy_records(data: &Path) -> Value {
    std::fs::read_to_string(data.join("proxy.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).filter(Value::is_object).unwrap_or(json!({}))
}

/// Service side: keep one passthrough headroom worker per fixed port; a dead, missing or silent one is started again.
pub fn supervise(install: &Path, workers: &mut Vec<(u16, Child)>) {
    let data = install.join("data");
    workers.retain_mut(|(p, c)| {
        if ROUTED.iter().any(|a| port(&data, a) == Some(*p) && !crate::control::transport(&data, a)) {
            end_worker(c);
            false
        } else { true }
    });
    workers.retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
    let now = crate::now();
    let mut records = proxy_records(&data);
    for agent in ROUTED {
        let Some(p) = port(&data, agent).filter(|_| crate::control::transport(&data, agent)) else {
            if let Some(r) = records.as_object_mut() { r.remove(agent); }
            continue;
        };
        let Some(i) = workers.iter().position(|w| w.0 == p) else { continue };
        let r = &mut records[agent];
        if r.is_null() { *r = json!({"port": p, "silent_since": now}); }
        r["at"] = now.into();
        if responds(p) {
            r["answered"] = now.into();
            r["silent_since"] = Value::Null;
            continue;
        }
        let since = r["silent_since"].as_u64().unwrap_or(now);
        r["silent_since"] = since.into();
        let started = r["started"].as_u64().unwrap_or(since);
        if now.saturating_sub(since.max(started)) >= PROXY_REPLACE {
            eprintln!("agentq: headroom on port {p}: no HTTP reply for {} s; replacing the worker", now.saturating_sub(since));
            end_worker(&mut workers.remove(i).1);
        }
    }
    let Some(exe) = crate::engines::headroom_exe(install) else {
        if let Err(e) = write_atomic(&data.join("proxy.json"), &records.to_string()) { eprintln!("agentq: proxy record: {e}"); }
        return;
    };
    for agent in ROUTED {
        if !crate::control::transport(&data, agent) { continue; }
        let Some(p) = port(&install.join("data"), agent) else { continue };
        if workers.iter().any(|w| w.0 == p) {
            continue;
        }
        // Headroom keeps history passthrough; our supported extension adapts new tool results only.
        let adapter = match provider_adapter(install) { Ok(p) => p, Err(e) => { eprintln!("agentq: provider adapter: {e}"); continue; } };
        let python = exe.parent().unwrap().join(format!("python{}", std::env::consts::EXE_SUFFIX));
        let bootstrap = "import sys; sys.path.insert(0,sys.argv.pop(1)); from headroom.cli import main; main()";
        let args = ["proxy", "--host", "127.0.0.1", "--port", &p.to_string(), "--no-optimize", "--no-cache", "--no-rate-limit", "--no-ccr", "--no-subscription-tracking", "--proxy-extension", "agentq"];
        let started = coreq::quiet(Command::new(&python).args(["-I", "-c", bootstrap]).arg(&adapter).args(args))
            .env("AGENTQ_INSTALL", install)
            .env("AGENTQ_BINARY", install.join(format!("agentq{}", std::env::consts::EXE_SUFFIX)))
            .env("OPENBLAS_NUM_THREADS", "1")
            .env("OMP_NUM_THREADS", "1")
            .env("HEADROOM_BEACON", "off")
            .env("DO_NOT_TRACK", "1")
            .env("HEADROOM_TELEMETRY", "off")
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn();
        match started {
            Ok(c) => {
                workers.push((p, c));
                // The silence clock survives a replacement, so a worker that never replies keeps reading failed.
                let old = &records[agent];
                let current = old["at"].as_u64().is_some_and(|at| now.saturating_sub(at) <= 3 * crate::PASS);
                let since = old["silent_since"].as_u64().filter(|_| current).unwrap_or(now);
                records[agent] = json!({"port": p, "started": now, "silent_since": since, "at": now});
            }
            Err(e) => eprintln!("agentq: headroom on port {p}: {e}"),
        }
    }
    if let Err(e) = write_atomic(&data.join("proxy.json"), &records.to_string()) { eprintln!("agentq: proxy record: {e}"); }
}

pub fn stop(workers: &mut Vec<(u16, Child)>) {
    for (_, c) in workers.iter_mut() {
        end_worker(c);
    }
    workers.clear();
}

/// Proxy state and cause from the service's record and a fresh probe (agentq.md M5.6).
fn proxy_read(record: &Value, responded: bool, now: u64) -> (&'static str, Option<&'static str>) {
    if responded {
        return ("answering", None);
    }
    if !record["at"].as_u64().is_some_and(|at| now.saturating_sub(at) <= 3 * crate::PASS) {
        return ("failed", Some("proxy_not_started"));
    }
    if record["answered"].is_u64() {
        return ("failed", Some("proxy_not_responding"));
    }
    if now.saturating_sub(record["silent_since"].as_u64().unwrap_or(0)) < PROXY_LIMIT {
        return ("starting", Some("proxy_starting"));
    }
    ("failed", Some("proxy_start_timeout"))
}

/// `proxy` of one agent's `status.json` and `status` entry; never a path or request data.
pub fn proxy(data: &Path, agent: &str, port: u16, now: u64) -> Value {
    let records = proxy_records(data);
    let record = &records[agent];
    let fresh = record["silent_since"].is_null() && record["answered"].as_u64().is_some_and(|a| now.saturating_sub(a) < PROXY_FRESH);
    let (state, cause) = proxy_read(record, fresh || responds(port), now);
    let mut out = json!({"state": state, "port": port});
    if let Some(cause) = cause {
        out["cause"] = cause.into();
        if let Some(since) = record["silent_since"].as_u64().or(record["answered"].as_u64()) {
            out["silent_for"] = now.saturating_sub(since).into();
        }
    }
    out
}

/// The `status` and `doctor` line of an agent's `proxy` entry, with the fallback when it failed.
pub fn proxy_line(agent: &str, entry: &Value) -> Option<String> {
    let p = entry.get("proxy")?;
    let (state, port) = (p["state"].as_str().unwrap_or("unknown"), p["port"].as_u64().unwrap_or(0));
    let Some(cause) = p["cause"].as_str() else { return Some(format!("{agent}: proxy {state} (127.0.0.1:{port})")) };
    let silent = p["silent_for"].as_u64().map_or(String::new(), |s| format!(" for {s} s"));
    let next = if state == "failed" { format!("; run `agentq service restart`, or `agentq direct {agent}` for direct access") } else { String::new() };
    Some(format!("{agent}: proxy {state} ({cause}, no HTTP reply on 127.0.0.1:{port}{silent}){next}"))
}

/// Established TCP connections with an end on one of `ports` (§ 5 restart drain): Linux `/proc/net/tcp{,6}`, macOS `lsof`
/// (its `netstat` lists no TCP sockets on recent releases), Windows `netstat -an`.
pub fn open_connections(ports: &[u16]) -> Result<usize, String> {
    if ports.is_empty() {
        return Ok(0);
    }
    let list = if cfg!(target_os = "linux") {
        ["/proc/net/tcp", "/proc/net/tcp6"].iter().map(|p| std::fs::read_to_string(p).map_err(|_| "connection list unavailable".to_string())).collect::<Result<Vec<_>, _>>()?.concat()
    } else {
        let (cmd, args) = if cfg!(target_os = "macos") { ("lsof", &["-nP", "-iTCP", "-sTCP:ESTABLISHED"][..]) } else { ("netstat", &["-an"][..]) };
        let mut c = Command::new(cmd);
        c.args(args);
        let o = crate::host::output(&mut c).map_err(|e| format!("{cmd}: {e}"))?;
        // lsof exits 1 with no output when nothing matches.
        if empty_connections(cmd, o.status.code(), &o.stdout, &o.stderr).map_err(|e| crate::host::failed(&c, &o, &e))? {
            return Ok(0);
        }
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    if list.is_empty() {
        return Err("no connection list".into());
    }
    Ok(established_in(&list, ports))
}

fn empty_connections(cmd: &str, code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> Result<bool, String> {
    if cmd == "lsof" && code == Some(1) && stdout.is_empty() && stderr.is_empty() { return Ok(true); }
    if code != Some(0) || !stderr.is_empty() { return Err("connection inventory failed".into()); }
    Ok(false)
}

/// Counts `netstat -an` lines (`127.0.0.1.8787`, `127.0.0.1:8787`), `lsof` names (`127.0.0.1:8787->127.0.0.1:54321`)
/// and `/proc/net/tcp` rows (`0100007F:2253 .. 01`).
fn established_in(list: &str, ports: &[u16]) -> usize {
    let hit = |a: &str, sep: char, hex: bool| {
        a.rsplit_once(sep).and_then(|(_, p)| if hex { u16::from_str_radix(p, 16).ok() } else { p.parse().ok() }).is_some_and(|p| ports.contains(&p))
    };
    list.lines()
        .filter(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if l.contains("ESTABLISHED") {
                f.iter().flat_map(|a| a.split("->")).any(|a| hit(a, ':', false) || hit(a, '.', false))
            } else {
                f.len() > 3 && f[3] == "01" && f[0].ends_with(':') && (hit(f[1], ':', true) || hit(f[2], ':', true))
            }
        })
        .count()
}

/// `routing` (routed, direct, bypassed, unknown) and its reason for one agent (§ 5).
pub fn state(agent: &str, root: &Path, port: Option<u16>) -> (&'static str, String) {
    match agent {
        "codex" => {
            let toml = std::fs::read_to_string(root.join("config.toml")).unwrap_or_default();
            if !toml.lines().take_while(|l| !l.trim_start().starts_with('[')).any(|l| l.trim() == "model_provider = \"agentq\"") {
                return ("direct", "config.toml selects no agentq provider".into());
            }
            match port {
                Some(p) if toml.contains(&format!("base_url = \"{}\"", base_url(p))) && answers(p) => ("routed", format!("127.0.0.1:{p}")),
                Some(p) if toml.contains(&format!("base_url = \"{}\"", base_url(p))) => ("unknown", format!("proxy on 127.0.0.1:{p} not answering")),
                _ => ("bypassed", "agentq provider is not the fixed loopback port".into()),
            }
        }
        "claude" => match std::fs::read_to_string(root.join("settings.json")).ok().and_then(|s| serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')).ok()) {
            Some(s) if s["env"]["ANTHROPIC_BASE_URL"].is_string() => ("bypassed", "settings.json env.ANTHROPIC_BASE_URL is foreign".into()),
            _ => ("direct", "not routed yet".into()),
        },
        _ => ("direct", "not routed yet".into()),
    }
}

/// Records `agentq direct` for `agents` in `config.json` `direct` (`on`), or clears it on `onboard --yes`.
pub fn set_direct(data: &Path, agents: &[&str], on: bool) -> Result<(), String> {
    // a corrupt config.json stops here instead of being replaced, so its ports and settings survive
    let mut c = read_json(&data.join("config.json"), config_shape).map_err(|_| format!("{} unreadable", data.join("config.json").display()))?.unwrap_or(json!({}));
    let mut list: Vec<String> = c["direct"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|a| !agents.contains(a)).map(String::from).collect();
    if on {
        list.extend(agents.iter().map(|a| a.to_string()));
    }
    if list.is_empty() && c["direct"].is_null() {
        return Ok(());
    }
    c["direct"] = list.into();
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    write_atomic(&data.join("config.json"), &serde_json::to_string_pretty(&c).unwrap_or_default())
}

/// A JSON file: `None` when missing, `Err` when unreadable, corrupt or not of the expected shape.
fn read_json(p: &Path, shape: fn(&Value) -> bool) -> Result<Option<Value>, ()> {
    match std::fs::read_to_string(p) {
        Ok(s) => serde_json::from_str(&s).ok().filter(shape).map(Some).ok_or(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

/// config.json: an object whose `direct`, when present, lists agent names.
fn config_shape(c: &Value) -> bool {
    c.is_object() && (c["direct"].is_null() || c["direct"].as_array().is_some_and(|l| l.iter().all(Value::is_string)))
}

/// owned.json: the onboard journal, a list of entries that each name their `file`.
fn journal_shape(j: &Value) -> bool {
    j.as_array().is_some_and(|l| l.iter().all(|e| e["file"].is_string()))
}

/// Whether the agent should be routed (#132): an onboarded agent with shipped routing and no `agentq direct`.
/// ponytail: Claude Code and Hermes are intended direct until their routing ships (owner decision on #132).
pub fn intended(data: &Path, agent: &str, root: &Path) -> (&'static str, &'static str) {
    match crate::control::disabled(data, agent) {
        Ok(true) => return ("disabled", "`agentq disable`"),
        Err(_) => return ("unknown", "integration.json unreadable"),
        Ok(false) => {},
    }
    if !ROUTED.contains(&agent) {
        return ("direct", "routing not shipped");
    }
    let Ok(c) = read_json(&data.join("config.json"), config_shape) else { return ("unknown", "config.json unreadable") };
    if c.is_some_and(|c| c["direct"].as_array().is_some_and(|l| l.iter().any(|a| a == agent))) {
        return ("direct", "`agentq direct`");
    }
    let Ok(j) = read_json(&data.join("owned.json"), journal_shape) else { return ("unknown", "owned.json unreadable") };
    if !j.as_ref().and_then(Value::as_array).into_iter().flatten().any(|e| e["file"].as_str().is_some_and(|f| Path::new(f).starts_with(root))) {
        return ("direct", "not onboarded");
    }
    ("routed", "onboarded")
}

/// status.json `agents`: the routing of each agent whose config root exists, and whether it is intended.
pub fn agents(data: &Path) -> Value {
    let mut out = serde_json::Map::new();
    for a in crate::onboard::AGENTS {
        let root = crate::onboard::root(a);
        if root.exists() {
            let (routing, reason) = state(a, &root, port(data, a));
            let (intended, why) = intended(data, a, &root);
            let mut entry = json!({"routing": routing, "reason": reason, "intended": intended, "intended_reason": why});
            if let Some(p) = port(data, a).filter(|_| matches!(routing, "routed" | "unknown")) {
                entry["proxy"] = proxy(data, a, p, crate::now());
            }
            out.insert(a.into(), entry);
        }
    }
    out.into()
}

/// Onboarded agents that should be routed but are not, as `<agent> <routing> (<reason>)`, and agents whose intent is
/// unknown, as `<agent> intended unknown (<reason>)` (heartbeat).
pub fn problems(agents: &Value) -> Vec<String> {
    agents.as_object().into_iter().flatten()
        .filter_map(|(a, r)| match (r["intended"].as_str(), r["routing"].as_str()) {
            (Some("routed"), Some("routed")) if r["proxy"]["state"] == "failed" => Some(format!("{a} proxy failed ({})", r["proxy"]["cause"].as_str().unwrap_or(""))),
            (Some("routed"), Some("routed")) | (Some("direct"), _) | (Some("disabled"), Some("direct")) => None,
            (Some("routed"), routing) => Some(format!("{a} {} ({})", routing.unwrap_or("unknown"), r["reason"].as_str().unwrap_or(""))),
            _ => Some(format!("{a} intended unknown ({})", r["intended_reason"].as_str().unwrap_or(""))),
        })
        .collect()
}

/// Routing edits in the journal that are no longer in their file.
pub fn drift(journal: &Path) -> Vec<String> {
    let j: Vec<Value> = std::fs::read_to_string(journal).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    j.iter()
        .filter(|e| e["routing"].is_string() && e["kind"] == "text")
        .filter(|e| {
            let after = e["after"].as_str().unwrap_or_default();
            !std::fs::read_to_string(e["file"].as_str().unwrap_or_default()).is_ok_and(|t| t.contains(after.trim()))
        })
        .map(|e| format!("{} {}", e["file"].as_str().unwrap_or_default(), e["key"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(".")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listener on a fresh port that sends `reply` to each connection.
    fn http_fixture(reply: &'static [u8]) -> u16 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut buf = [0; 256];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply);
            }
        });
        p
    }

    /// #275 A2: the live defect was a worker that the service counted as running while it never replied. Only a 200
    /// from the proxy proves it answers: an accepted connection (a stalled interpreter) and a 503 (unhealthy loop) do not.
    #[test]
    fn only_a_200_proves_the_proxy_answers() {
        assert!(responds(http_fixture(b"HTTP/1.1 200 OK\r\n\r\n")));
        assert!(!responds(http_fixture(b"HTTP/1.1 503 Service Unavailable\r\n\r\n")));
        assert!(!responds(http_fixture(b"")), "an accept without a reply is a stall");
        let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        assert!(!responds(free));
    }

    /// #275 A2: each silent case has a visible state and a named cause; none reads as healthy or as a silent wait.
    #[test]
    fn silent_proxy_reads_a_named_state() {
        let now = 10_000;
        let at = |silent: u64, answered: Value| json!({"at": now, "silent_since": now - silent, "answered": answered});
        assert_eq!(proxy_read(&at(0, Value::Null), true, now), ("answering", None));
        assert_eq!(proxy_read(&at(PROXY_LIMIT - 1, Value::Null), false, now), ("starting", Some("proxy_starting")));
        assert_eq!(proxy_read(&at(PROXY_LIMIT, Value::Null), false, now), ("failed", Some("proxy_start_timeout")));
        assert_eq!(proxy_read(&at(1, json!(now - 40)), false, now), ("failed", Some("proxy_not_responding")));
        assert_eq!(proxy_read(&Value::Null, false, now), ("failed", Some("proxy_not_started")));
        let stale = json!({"at": now - 3 * crate::PASS - 1, "silent_since": now - 1});
        assert_eq!(proxy_read(&stale, false, now), ("failed", Some("proxy_not_started")), "no service pass proves nothing");
        let listed = problems(&json!({"codex": {"routing": "routed", "intended": "routed", "proxy": {"state": "failed", "cause": "proxy_not_responding"}}}));
        assert_eq!(listed, ["codex proxy failed (proxy_not_responding)"], "the heartbeat names a stalled proxy that TCP calls routed");
        let line = proxy_line("codex", &json!({"proxy": {"state": "failed", "port": 50832, "cause": "proxy_start_timeout", "silent_for": 340}})).unwrap();
        assert!(line.contains("proxy failed (proxy_start_timeout") && line.contains("for 340 s") && line.contains("agentq direct codex"), "{line}");
    }

    /// #275 A2: a worker alive but silent past the limit is replaced and keeps reading failed. Before this, only an
    /// exited worker was replaced, so the live worker stayed for over an hour while the service said running.
    #[test]
    fn silent_worker_is_replaced_and_stays_failed() {
        let install = std::env::temp_dir().join(format!("agentq-silent-{}", crate::mcp::new_id()));
        let data = install.join("data");
        let p = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        crate::control::save(&data, "config.json", &json!({"ports": {"codex": p}})).unwrap();
        let sleeper = || if cfg!(windows) { let mut c = Command::new("ping"); c.args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()); c } else { let mut c = Command::new("sleep"); c.arg("60"); c };
        let now = crate::now();
        std::fs::write(data.join("proxy.json"), json!({"codex": {"port": p, "silent_since": now - 10, "at": now}}).to_string()).unwrap();
        let mut workers = vec![(p, sleeper().spawn().unwrap())];
        supervise(&install, &mut workers);
        assert_eq!(workers.len(), 1, "within the limit the worker keeps starting");
        assert_eq!(proxy(&data, "codex", p, crate::now())["state"], "starting");
        let record = |silent: u64, age: u64| std::fs::write(data.join("proxy.json"), json!({"codex": {"port": p, "started": now - age, "silent_since": now - silent, "at": now}}).to_string()).unwrap();
        record(PROXY_LIMIT, PROXY_LIMIT);
        supervise(&install, &mut workers);
        assert_eq!(workers.len(), 1, "failed is visible while a slow start keeps its progress");
        let read = proxy(&data, "codex", p, crate::now());
        assert_eq!((&read["state"], &read["cause"]), (&json!("failed"), &json!("proxy_start_timeout")), "{read}");
        assert!(read["silent_for"].as_u64().unwrap() >= PROXY_LIMIT);
        // Found live: with the silence clock alone, every new worker died one pass after its start and none could finish.
        record(10 * PROXY_REPLACE, 10);
        supervise(&install, &mut workers);
        assert_eq!(workers.len(), 1, "a replacement gets its own time");
        assert_eq!(proxy(&data, "codex", p, crate::now())["cause"], "proxy_start_timeout", "and the silence still reads failed");
        record(PROXY_REPLACE, PROXY_REPLACE);
        supervise(&install, &mut workers);
        let left = workers.len();
        stop(&mut workers);
        assert_eq!(left, 0, "the silent worker is gone; this fixture has no engine to start another");
        std::fs::remove_dir_all(install).unwrap();
    }

    /// #275 A1: a Task Scheduler start gives the service below-normal CPU, low I/O and low memory priority, and the
    /// proxy inherits them: on the live host its import of `transformers` ran at about 1.5 reads/s and did not end.
    /// The raise must reach a worker started afterwards, so this reads a child, not this process.
    #[cfg(windows)]
    #[test]
    fn worker_inherits_normal_priority_after_task_scheduler_start() {
        use std::ffi::c_void;
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut c_void;
            fn GetPriorityClass(process: *mut c_void) -> u32;
            fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSetInformationProcess(process: *mut c_void, class: u32, info: *const c_void, size: u32) -> i32;
            fn NtQueryInformationProcess(process: *mut c_void, class: u32, info: *mut c_void, size: u32, out: *mut u32) -> i32;
        }
        let read = |h: *mut c_void, class: u32| {
            let (mut v, mut n) = (0u32, 0u32);
            // SAFETY: `h` is a live process handle and `v` is a u32 that outlives the call.
            assert_eq!(unsafe { NtQueryInformationProcess(h, class, (&raw mut v).cast(), 4, &raw mut n) }, 0);
            v
        };
        // SAFETY: the current-process pseudo handle; each value is a u32 that outlives the call.
        unsafe {
            let me = GetCurrentProcess();
            assert_ne!(SetPriorityClass(me, 0x4000), 0);
            let (io, page) = (1u32, 2u32);
            assert_eq!(NtSetInformationProcess(me, 33, (&raw const io).cast(), 4), 0);
            assert_eq!(NtSetInformationProcess(me, 39, (&raw const page).cast(), 4), 0);
        }
        normal_priority().unwrap();
        let mut child = Command::new("ping").args(["-n", "3", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        let h = child.as_raw_handle().cast::<c_void>();
        // SAFETY: `child` owns `h` until the kill below.
        let class = unsafe { GetPriorityClass(h) };
        let seen = (class, read(h, 33), read(h, 39));
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(seen, (0x20, 2, 5), "CPU class, I/O priority and memory priority of a worker started after the raise");
    }

    /// Temporary controls: an enabled sibling must not respawn Codex's safely drained proxy.
    #[cfg(unix)]
    #[test]
    fn drained_proxy_stays_down_while_shared_service_remains_enabled() {
        use std::os::unix::fs::PermissionsExt;
        let install = std::env::temp_dir().join(format!("agentq-no-respawn-{}", crate::mcp::new_id()));
        let data = install.join("data");
        let engine = install.join("engines").join(format!("headroom-{}", crate::engines::HEADROOM));
        std::fs::create_dir_all(engine.join("bin")).unwrap();
        std::fs::write(engine.join("ready"), "fixture").unwrap();
        let python = engine.join("bin/python");
        std::fs::write(&python, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o700)).unwrap();
        crate::control::save(&data, "config.json", &json!({"ports": {"codex": 1}})).unwrap();
        crate::control::save(&data, "integration.json", &json!({"schema": 1, "generation": 1, "host_disabled": false,
            "profiles": {"codex": {"root": install, "phase": "detached", "runtime": "drained", "at": 1, "direct": false, "entries": []}}})).unwrap();
        let mut workers = vec![(1, Command::new("sleep").arg("30").spawn().unwrap())];
        supervise(&install, &mut workers);
        let first = workers.len();
        supervise(&install, &mut workers);
        let second = workers.len();
        stop(&mut workers);
        assert_eq!((first, second), (0, 0), "a drained port is stopped and is not started on subsequent passes");
        assert!(!crate::control::host_disabled(&data).unwrap(), "enabled siblings retain the shared service");
        std::fs::remove_dir_all(install).unwrap();
    }

    #[test]
    fn port_is_fixed_once() {
        let d = std::env::temp_dir().join(format!("agentq-port-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(port(&d, "codex"), None);
        let p = fix_port(&d, "codex").unwrap();
        assert_eq!(fix_port(&d, "codex").unwrap(), p, "kept across installs");
        assert_eq!(port(&d, "codex"), Some(p));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #111 A2: the restart drain counts live proxied connections on macOS, Windows and Linux; listeners and other ports are not.
    #[test]
    fn established_on_proxy_ports() {
        let mac = "tcp4  0  0  127.0.0.1.8787   127.0.0.1.54321  ESTABLISHED\n\
                   tcp4  0  0  127.0.0.1.54321  127.0.0.1.8787   ESTABLISHED\n\
                   tcp4  0  0  127.0.0.1.8787   *.*              LISTEN\n\
                   tcp4  0  0  10.0.0.2.18787   1.2.3.4.443      ESTABLISHED\n";
        assert_eq!(established_in(mac, &[8787]), 2);
        let win = "  TCP    127.0.0.1:8787   127.0.0.1:54321   ESTABLISHED\r\n  TCP    0.0.0.0:8787   0.0.0.0:0   LISTENING\r\n";
        assert_eq!(established_in(win, &[8787]), 1);
        let lsof = "COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n\
                    headroom 7 u 9u IPv4 0x1 0t0 TCP 127.0.0.1:8787->127.0.0.1:54321 (ESTABLISHED)\n\
                    Python 8 u 13u IPv6 0x2 0t0 TCP [2405::1]:58635->[2606::9bd1]:443 (ESTABLISHED)\n";
        assert_eq!(established_in(lsof, &[8787]), 1);
        let proc = "  sl  local_address rem_address   st tx_queue\n\
                    0: 0100007F:2253 00000000:0000 0A 00000000\n\
                    1: 0100007F:2253 0100007F:D431 01 00000000\n\
                    2: 0200000A:D431 04030201:01BB 01 00000000\n";
        assert_eq!(established_in(proc, &[0x2253]), 1);
        assert_eq!(established_in(mac, &[9999]), 0);
        assert_eq!(open_connections(&[]), Ok(0));
        assert_eq!(empty_connections("lsof", Some(1), b"", b""), Ok(true));
        for (cmd, code, out, err) in [("lsof", Some(2), &b""[..], &b""[..]), ("lsof", Some(1), &b""[..], &b"denied"[..]), ("netstat", Some(1), &b"partial"[..], &b""[..]), ("lsof", None, &b""[..], &b""[..])] {
            assert!(empty_connections(cmd, code, out, err).is_err(), "failed socket inventory must not prove a safe drain");
        }
        // this host's own list: a live loopback connection counts
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        let _c = TcpStream::connect((Ipv4Addr::LOCALHOST, p)).unwrap();
        let _s = l.accept().unwrap();
        assert!(open_connections(&[p]).unwrap() >= 1);
    }

    /// #137 A2: onboard waits for a slow proxy start, restarts the service once for a silent one and names a proxy
    /// that still does not answer.
    #[test]
    fn onboard_awaits_proxy() {
        let free = || TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let l = free();
        let p = l.local_addr().unwrap().port();
        let mut restarts = 0;
        assert!(await_proxies(&[p], Duration::from_secs(5), &mut || restarts += 1).is_empty());
        assert_eq!(restarts, 0, "an answering proxy needs no restart");
        drop(l);
        let late = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            let l = TcpListener::bind((Ipv4Addr::LOCALHOST, p)).unwrap();
            std::thread::sleep(Duration::from_secs(3));
            drop(l);
        });
        assert!(await_proxies(&[p], Duration::from_secs(2), &mut || restarts += 1).is_empty());
        assert_eq!(restarts, 0, "the wait covers a slow start");
        late.join().unwrap();
        let q = free().local_addr().unwrap().port();
        let mut up = None;
        assert!(await_proxies(&[q], Duration::from_millis(600), &mut || up = TcpListener::bind((Ipv4Addr::LOCALHOST, q)).ok()).is_empty());
        assert!(up.is_some(), "the restart starts the silent proxy");
        // port 1 refuses
        assert_eq!(await_proxies(&[1], Duration::from_millis(600), &mut || restarts += 1), [1]);
        assert_eq!(restarts, 1, "one restart only");
    }

    /// #76 A1: status.json routing follows config.toml and whether the loopback port answers.
    #[test]
    fn codex_routing_state() {
        let d = std::env::temp_dir().join(format!("agentq-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(state("codex", &d, None).0, "direct");
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let p = l.local_addr().unwrap().port();
        let cfg = format!("model_provider = \"agentq\"\n\n[model_providers.agentq]\nbase_url = \"{}\"\n", base_url(p));
        std::fs::write(d.join("config.toml"), &cfg).unwrap();
        assert_eq!(state("codex", &d, Some(p)).0, "routed");
        assert_eq!(state("codex", &d, Some(p.wrapping_add(1))).0, "bypassed");
        drop(l);
        // a freed ephemeral port may serve a parallel test: port 1 refuses
        std::fs::write(d.join("config.toml"), cfg.replace(&base_url(p), &base_url(1))).unwrap();
        assert_eq!(state("codex", &d, Some(1)).0, "unknown");
        std::fs::write(d.join("config.toml"), format!("[profiles.x]\n{cfg}")).unwrap();
        assert_eq!(state("codex", &d, Some(p)).0, "direct", "a profile key is not the top-level provider");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #132 A1/A2: an onboarded Codex without routing is a problem; `agentq direct`, no onboarding and unshipped
    /// routing are intended direct; `onboard --yes` clears the `direct` record.
    #[test]
    fn intended_routing() {
        let d = std::env::temp_dir().join(format!("agentq-intended-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (data, root) = (d.join("data"), d.join(".codex"));
        std::fs::create_dir_all(&data).unwrap();
        assert_eq!(intended(&data, "codex", &root), ("direct", "not onboarded"));
        std::fs::write(data.join("owned.json"), json!([{"file": root.join("config.toml"), "key": ["mcp_servers", "agentq"]}]).to_string()).unwrap();
        assert_eq!(intended(&data, "codex", &root), ("routed", "onboarded"));
        assert_eq!(intended(&data, "claude", &d.join(".claude")).0, "direct");
        let row = |i: &str| json!({"codex": {"routing": "direct", "reason": "config.toml selects no agentq provider", "intended": i}, "claude": {"routing": "direct", "intended": "direct"}});
        assert_eq!(problems(&row("routed")), ["codex direct (config.toml selects no agentq provider)"]);
        set_direct(&data, &["codex"], true).unwrap();
        assert_eq!(intended(&data, "codex", &root), ("direct", "`agentq direct`"));
        assert!(problems(&row("direct")).is_empty());
        set_direct(&data, &["codex"], false).unwrap();
        assert_eq!(intended(&data, "codex", &root).0, "routed");
        std::fs::write(data.join("owned.json"), "[{").unwrap();
        assert_eq!(intended(&data, "codex", &root), ("unknown", "owned.json unreadable"), "corrupt journal is not intended direct");
        for bad in ["{}", "[1]", "[{\"file\": 1}]"] {
            std::fs::write(data.join("owned.json"), bad).unwrap();
            assert_eq!(intended(&data, "codex", &root), ("unknown", "owned.json unreadable"), "journal shape {bad}");
        }
        std::fs::write(data.join("owned.json"), "[]").unwrap();
        assert_eq!(intended(&data, "codex", &root), ("direct", "not onboarded"));
        for bad in ["[]", "{\"direct\": \"codex\"}", "{\"direct\": [1]}"] {
            std::fs::write(data.join("config.json"), bad).unwrap();
            assert_eq!(intended(&data, "codex", &root), ("unknown", "config.json unreadable"), "config shape {bad}");
            assert!(set_direct(&data, &["codex"], true).is_err(), "config shape {bad} is not replaced");
        }
        std::fs::write(data.join("config.json"), "{").unwrap();
        assert_eq!(intended(&data, "codex", &root), ("unknown", "config.json unreadable"));
        assert!(set_direct(&data, &["codex"], true).is_err(), "a corrupt config.json is not replaced");
        assert_eq!(std::fs::read_to_string(data.join("config.json")).unwrap(), "{");
        assert_eq!(problems(&json!({"codex": {"routing": "direct", "intended": "unknown", "intended_reason": "owned.json unreadable"}})), ["codex intended unknown (owned.json unreadable)"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
