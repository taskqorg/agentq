//! Loopback routing (agentq.md § 4, § 5): fixed ports in `data/config.json`, headroom workers owned by the service,
//! per-agent routing state and drift.
//!
//! ponytail: Codex only. Claude Code and Hermes stay direct until their routing lands; Hermes telemetry stays on its own Langfuse exporter.
//! ponytail: one worker per port, respawned each pass; the § 5 engine-pin worker swap joins with the next pin change.

use coreq::write_atomic;
use serde_json::{Value, json};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const ROUTED: [&str; 1] = ["codex"];

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

/// Service side: keep one passthrough headroom worker per fixed port; a dead or missing one is started again.
pub fn supervise(install: &Path, workers: &mut Vec<(u16, Child)>) {
    let data = install.join("data");
    workers.retain_mut(|(p, c)| {
        if ROUTED.iter().any(|a| port(&data, a) == Some(*p) && !crate::control::transport(&data, a)) {
            let _ = c.kill();
            let _ = c.wait();
            false
        } else { true }
    });
    workers.retain_mut(|(_, c)| matches!(c.try_wait(), Ok(None)));
    let Some(exe) = crate::engines::headroom_exe(install) else { return };
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
            Ok(c) => workers.push((p, c)),
            Err(e) => eprintln!("agentq: headroom on port {p}: {e}"),
        }
    }
}

pub fn stop(workers: &mut Vec<(u16, Child)>) {
    for (_, c) in workers.iter_mut() {
        let _ = c.kill();
        let _ = c.wait();
    }
    workers.clear();
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
        let o = coreq::quiet(Command::new(cmd).args(args)).output().map_err(|e| format!("{cmd}: {e}"))?;
        // lsof exits 1 with no output when nothing matches.
        if empty_connections(cmd, o.status.code(), &o.stdout, &o.stderr)? {
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
            out.insert(a.into(), json!({"routing": routing, "reason": reason, "intended": intended, "intended_reason": why}));
        }
    }
    out.into()
}

/// Onboarded agents that should be routed but are not, as `<agent> <routing> (<reason>)`, and agents whose intent is
/// unknown, as `<agent> intended unknown (<reason>)` (heartbeat).
pub fn problems(agents: &Value) -> Vec<String> {
    agents.as_object().into_iter().flatten()
        .filter_map(|(a, r)| match (r["intended"].as_str(), r["routing"].as_str()) {
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
