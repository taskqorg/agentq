//! agentq: update, service, status, MCP, engines, onboarding, hooks, telemetry and uninstall on coreq (agentq.md § 1-§ 6, § 9, § 10).

mod accounting;
mod check;
mod command_path;
mod configure;
mod compression;
mod connect;
mod control;
mod engines;
mod hooks;
mod executor;
mod legacy;
mod local;
mod mcp;
mod names;
mod onboard;
mod profile;
mod profiles;
mod resource;
mod release;
mod routing;
mod snapshot;
mod telemetry;

use coreq::{Tool, service, status, update, write_atomic};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// One unit per host and runtime (agentq.md § 5): `com.agentq.host`, `AgentQ-host`, `agentq-host.service`.
const PROJECT: &str = "host";
const PASS: u64 = 30;
const UPDATE_EVERY: u64 = 3600;
const DRAIN: u64 = 900;
/// `onboard --yes` waits this long for a started proxy to answer, before and after one restart (#137).
const PROXY_START: Duration = Duration::from_secs(60);
const USAGE: &str = "agentq status [--json] | optimization on|off [agent] [--dry-run] | disable|enable [agent] [--dry-run] | run -- <cmd> [args..] | compress <agent> | stats [--json|--records] | stats --accounting [--from <unix>] [--to <unix>] [--bucket <s>] [--events <path>] | profile [--json] | profile trace on [<minutes>]|off | onboard [--yes] [codex|claude|hermes..] | direct [codex|claude|hermes..] | configure [<profile> <field>=<value>.. [--yes]] | hook <agent> <event> | update [--check] | selftest | setup-path | migrate-names [--dry-run|--rollback] | check compression [codex|claude|both] | mcp codex|claude|hermes|claude-desktop | service install|run|restart|stop|uninstall|status | doctor | langfuse | telemetry status|verify|export | enroll <https-url> [--headless] | connection status [--json] | disconnect | uninstall [--legacy] [--dry-run] | --version";

fn tool() -> Tool {
    Tool::new("agentq", "AgentQ", "taskqorg/agentq", VERSION)
}

fn fail(problem: &str, next: &str) -> ! {
    eprintln!("Error: {problem}\nNext: {next}");
    std::process::exit(1);
}

fn main() {
    if let Some(code) = compression::replay() { std::process::exit(code); }
    let t = tool();
    let args: Vec<String> = std::env::args().skip(1).collect();
    // the launcher copy only picks a release (coreq.md § Launcher); hooks run in place, so a release mid-update or a
    // failed start never fails the client's hook (#132)
    if std::env::current_exe().ok() == Some(t.install.join(t.exe_name())) && t.current().is_some() && args.first().map(String::as_str) != Some("hook") {
        coreq::launcher::launch(&t, &args);
    }
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match a.as_slice() {
        ["--version"] => {
            println!("agentq {VERSION}");
            0
        }
        ["update"] | ["update", "--check"] => match update::update(&t, a.len() == 2) {
            Ok(Some(v)) if a.len() == 2 => println_code(&format!("update: {v} available")),
            Ok(Some(v)) => println_code(&format!("update: {v} installed")),
            Ok(None) => println_code(&format!("update: {} is the latest", t.current().unwrap_or(VERSION.into()))),
            Err(e) => fail(&format!("update: {e}"), "check the network and run `agentq update` again"),
        },
        ["check", "compression", rest @ ..] => match check::run(&t.install,rest) {
            Ok(code)=>code,
            Err(e)=>fail(&e,"install Python or use the managed Headroom runtime; run agentq check compression --help"),
        },
        ["selftest"] => selftest(&t),
        ["migrate-names"] | ["migrate-names", "--rollback"] | ["migrate-names", "--dry-run"] => match names::migrate(&integration_host(&t), a.contains(&"--rollback"), a.contains(&"--dry-run")) {
            Ok(v) => println_code(&v.to_string()),
            Err(e) => fail(&e, "keep native settings and resolve the name migration conflict"),
        },
        ["setup-path"] => match command_path::register(&t.install) {
            Ok(()) => 0,
            Err(e) => fail(&e, "resolve the command registration error and run `agentq setup-path` again"),
        },
        ["run", "--", cmd @ ..] if !cmd.is_empty() => engines::run(&t.install, &args[2..]),
        ["compress", agent] => match std::io::read_to_string(std::io::stdin()).map_err(|e|e.to_string()).and_then(|s|compression::compress(&t.install.join("data"),agent,&s)) {
            Ok(v)=>println_code(&v.to_string()),
            Err(e)=>fail(&e,"send one JSON object with adapter, tool and text; preserve the native result on failure"),
        },
        ["stats", "--records"] => match local::snapshot(&t.install.join("data")) {
            Ok(v) => {
                let mut errors = vec![];
                if let Some(e) = v["error"].as_str() { errors.push(e.to_string()); }
                if !compression::failed(&t.install.join("data")).is_empty() { errors.push("stats write failed".into()); }
                println_code(&serde_json::json!({"schema":1,"records":v["records"],"errors":errors}).to_string())
            }
            Err(e) if e=="stats unknown: no stats file" => println_code(&serde_json::json!({"schema":1,"records":[],"errors":[],"state":"unknown","reason":"no retained observations"}).to_string()),
            Err(e) => fail(&e, "inspect local telemetry storage"),
        },
        ["stats", "--accounting", rest @ ..] => accounting_cmd(&t, rest),
        ["stats"] | ["stats","--json"] =>match compression::stats(&t.install.join("data")) {
            Ok(v) if a.len()==2=>println_code(&v.to_string()),
            Ok(v)=>{
                for g in v["groups"].as_array().unwrap() {
                    let bytes=match (g["bytes_in"].as_u64(),g["bytes_out"].as_u64()) {
                        (Some(a),Some(b))=>format!("{a} bytes in, {b} bytes out"),
                        _=>format!("bytes unknown: {}",g["bytes_unknown"].as_str().unwrap()),
                    };
                    println!("{} · {}: {} calls, {} compressed, {} passed, {} failed, {bytes}",g["agent"].as_str().unwrap(),g["engine"].as_str().unwrap(),g["calls"],g["compressed"],g["passed"],g["failed"]);
                }
                0
            },
            Err(e) if a.len()==2=>{println!("{}",serde_json::json!({"schema":1,"state":"unknown","reason":e,"groups":null}));1},
            Err(e)=>fail(&e,"run a tool through agentq; inspect data/telemetry.sqlite3 and retained legacy files if records are unreadable"),
        },
        ["profile"] => {
            for l in profile::lines(&profile::report(&t.install.join("data"), now())) {
                println!("{l}");
            }
            0
        }
        ["profile", "--json"] => println_code(&profile::report(&t.install.join("data"), now()).to_string()),
        ["profile", "trace", "on", rest @ ..] if rest.len() <= 1 => match profile::trace_minutes(rest.first().copied()).and_then(|m| profile::set_trace(&t.install.join("data"), Some(m), now())) {
            Ok(s) => println_code(&format!("profile: trace on until {}", s["until"])),
            Err(e) => fail(&format!("profile trace: {e}"), "run `agentq profile trace on [1-60]`"),
        },
        ["profile", "trace", "off"] => match profile::set_trace(&t.install.join("data"), None, now()) {
            Ok(_) => println_code("profile: trace off"),
            Err(e) => fail(&format!("profile trace: {e}"), "fix data/config.json, then run `agentq profile trace off` again"),
        },
        ["mcp", agent @ ("codex" | "claude" | "hermes" | "claude-desktop")] => mcp::serve(&t.install.join("data").join("originals"), agent),
        ["service", cmd] => service_cmd(&t, cmd),
        ["onboard", rest @ ..] => onboard(&t, rest),
        ["direct", rest @ ..] => direct(&t, rest),
        ["status"] => control_status(&t, false),
        ["status", "--json"] => control_status(&t, true),
        [action @ ("disable" | "enable"), rest @ ..] => integration_cmd(&t, rest, *action == "enable"),
        ["optimization", mode @ ("on" | "off"), rest @ ..] => {
            let agents: Vec<_> = rest.iter().copied().filter(|a| *a != "--dry-run").collect();
            if agents.len() > 1 { fail("optimization: select one agent or the host", "agentq optimization on|off [agent] [--dry-run]"); }
            match control::optimization(&t.install.join("data"), agents.first().copied(), *mode == "on", rest.contains(&"--dry-run")) {
                Ok(v) => println_code(&v.to_string()),
                Err(e) => fail(&e, "inspect AgentQ control state; no native settings were changed"),
            }
        }
        ["configure", rest @ ..] => configure_cmd(&t, rest),
        ["hook", agent @ ("codex" | "claude" | "hermes"), event] => {
            let data = t.install.join("data");
            // A failed hook answers `{}` and exits 0; closed stdout is recorded without panicking.
            let input = std::io::read_to_string(std::io::stdin());
            let response = input.as_ref().map_err(|e| format!("stdin: {e}")).and_then(|s| {
                match control::disabled(&data, agent) {
                    Ok(false) => {
                        if *agent == "claude" && ["PreToolUse", "PostToolUse"].contains(event) {
                            let c = compression::policy(&data, agent)?;
                            // A wrapper can change native approval decisions even when its compression is paused.
                            if c["enabled"] == false || c["agents"][*agent] == false { return Ok(serde_json::json!({})); }
                        }
                        hooks::response(agent,event,s,&t.install.join(t.exe_name()))
                    },
                    Ok(true) => Ok(serde_json::json!({})),
                    Err(e) => Err(e),
                }
            });
            let payload = response.as_ref().cloned().unwrap_or_else(|_| serde_json::json!({}));
            let wrote = writeln!(std::io::stdout(), "{payload}").err().map(|e| format!("stdout: {e}"));
            let errors: Vec<String> = response.err().into_iter().chain(wrote).collect();
            if errors.is_empty() {
                executor::stamp(&data, agent, event, now());
            } else {
                executor::hook_failed(&data, agent, event, now(), &errors.join("; "));
            }
            0
        }
        ["langfuse"] => match telemetry::read_keys().and_then(|k| telemetry::save(&t.install.join("data"), &k)) {
            Ok(store) => println_code(&format!("langfuse: keys saved in {store}; the service sends on its next pass")),
            Err(e) => fail(&format!("langfuse: {e}"), "run `agentq langfuse` and enter the host, public key and secret key"),
        },
        ["telemetry", "export"] => match local::export_jsonl(&t.install.join("data")) {
            Ok(v)=>println_code(&v),
            Err(e)=>fail(&e,"inspect local telemetry storage; retained files are preserved"),
        },
        ["telemetry", "status", ..] => println_code(&telemetry_line(&t)),
        ["telemetry", "verify", ..] => match telemetry::load(&t.install.join("data")) {
            None => fail("telemetry verify: no Langfuse keys", "run `agentq langfuse`"),
            Some(k) => match telemetry::verify(&t.install.join("data"), &k, 20, Duration::from_secs(3)) {
                Ok(id) => println_code(&format!("telemetry: verified trace {id}")),
                Err(e) => fail(&format!("telemetry verify: {e}"), "no observation: Langfuse has not ingested it yet, retry later; 401: run `agentq langfuse`"),
            },
        },
        ["enroll", url] | ["enroll", url, "--headless"] | ["enroll", "--headless", url] => match connect::enroll(&t.install.join("data"), url, !a.contains(&"--headless")) {
            Ok(id) => println_code(&format!("enroll: Connected to {url}; snapshot {id} projected")),
            Err(e) => fail(&format!("enroll: {e}"), "run `agentq connection status`, then `agentq enroll <https-url>` again"),
        },
        ["connection", "status"] => println_code(&connection_line(&connect::status(&t.install.join("data")))),
        ["connection", "status", "--json"] => println_code(&connect::status(&t.install.join("data")).to_string()),
        ["disconnect"] => match connect::disconnect(&t.install.join("data")) {
            Ok(m) => println_code(&format!("disconnect: {m}")),
            Err(e) => fail(&format!("disconnect: {e}"), "run `agentq disconnect` again"),
        },
        ["doctor"] => doctor(&t),
        ["uninstall"] => uninstall(&t, false),
        ["uninstall", "--dry-run"] => uninstall(&t, true),
        ["uninstall", "--legacy"] => legacy(&legacy_dir(), false),
        ["uninstall", "--legacy", "--dry-run"] | ["uninstall", "--dry-run", "--legacy"] => legacy(&legacy_dir(), true),
        _ => fail(&format!("unknown command `{}`", args.join(" ")), &format!("usage: {USAGE}")),
    };
    std::process::exit(code);
}

fn telemetry_line(t: &Tool) -> String {
    let q = telemetry::status(&t.install.join("data"), now());
    let keys = if telemetry::load(&t.install.join("data")).is_some() { "set" } else { "missing" };
    let oldest = q["oldest_age"].as_u64().map_or("none".into(), |s| format!("{s} s"));
    let last = q["last_delivery"].as_u64().map_or("never".into(), |t| format!("{} s ago", now().saturating_sub(t)));
    format!("telemetry: pending {}, oldest {oldest}, last delivery {last}, dropped {}, keys {keys}, error: {}", q["pending"], q["dropped"], q["error"].as_str().unwrap_or("none"))
}

fn connection_line(s: &Value) -> String {
    let e = &s["export"];
    let last = e["last_delivery"].as_u64().map_or("never".into(), |t| format!("{} s ago", now().saturating_sub(t)));
    let mut l = format!("connection: {}", s["state"].as_str().unwrap_or("unknown"));
    if let Some(d) = s["destination"].as_str() {
        l += &format!(" {d}");
    }
    if !e.is_null() {
        l += &format!(", export pending {}, last delivery {last}, error: {}", e["pending"], e["error"].as_str().unwrap_or("none"));
    }
    if s["revocation_pending"].as_u64().unwrap_or(0) > 0 {
        l += &format!(", revocation pending {}", s["revocation_pending"]);
    }
    match s["next"].as_str() {
        Some(n) => format!("{l}\nNext: {n}"),
        None => l,
    }
}

/// agentq.md § 1: read-only install, service, engines, owned edits, telemetry and legacy state; exit 1 on a problem.
fn doctor(t: &Tool) -> i32 {
    let mut ok = true;
    let mut line = |good: bool, l: String| {
        ok &= good;
        println!("{l}");
    };
    match t.current() {
        Some(v) => line(true, format!("install: {} current {v}", t.install.display())),
        None => line(false, format!("install: none at {}; run `agentq onboard --yes`", t.install.display())),
    }
    let report = control_report(t);
    let service = &report["service"];
    line(matches!(service["state"].as_str(), Some("running" | "stopped")), format!("service: {} ({})", service["state"].as_str().unwrap_or("unknown"), service["reason"].as_str().unwrap_or("fresh process and status readback")));
    line(report["control"] != "unknown", format!("control: {}", report["control"].as_str().unwrap_or("unknown")));
    if service["state"] != "stopped" {
        for l in engines::check(&t.install) { line(!l.contains("unavailable"), l); }
    }
    let agents = routing::agents(&t.install.join("data"));
    for (a, r) in agents.as_object().into_iter().flatten() {
        let want = r["intended"].as_str().unwrap_or("unknown");
        let fix = if want == "routed" && r["routing"] != "routed" { "; run `agentq onboard --yes`" } else { "" };
        line(r["routing"] != "unknown" && want != "unknown" && fix.is_empty(), format!("routing: {a} {} ({}), intended {want} ({}){fix}", r["routing"].as_str().unwrap_or("unknown"), r["reason"].as_str().unwrap_or(""), r["intended_reason"].as_str().unwrap_or("")));
    }
    for f in executor::hook_failures(&t.install.join("data")) {
        if f.split_whitespace().next().is_some_and(|a| control::disabled(&t.install.join("data"), a) == Ok(true)) { continue; }
        line(false, format!("hooks: {f} failed; its next successful run clears it"));
    }
    for p in report["configuration"]["profiles"].as_array().into_iter().flatten().filter(|p| p["integration"]["desired"] == "disabled") {
        line(p["integration"]["configuration"] == "detached", format!("integration: {} disabled, configuration {}, runtime {}", p["kind"].as_str().unwrap_or("unknown"),
            p["integration"]["configuration"].as_str().unwrap_or("unknown"), p["integration"]["runtime"].as_str().unwrap_or("unknown")));
    }
    for d in routing::drift(&journal(t)) {
        line(false, format!("routing: drift: {d} changed since onboard; run `agentq onboard --yes`"));
    }
    let owned = std::fs::read_to_string(journal(t)).ok().and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok()).map_or(0, |j| j.len());
    line(true, format!("owned edits: {owned}"));
    let tel = telemetry_line(t);
    // Remote keys are optional; local collection is qualified independently.
    line(tel.contains("keys set") || !t.install.join("data/langfuse").exists(),format!("{tel} (optional remote export)"));
    match local::check_store(&t.install.join("data")) {
        Ok(n)=>line(true,format!("local telemetry: readable SQLite, {n} retained records; compression unqualified")),
        Err(e)=>line(false,format!("local telemetry: unknown ({e})")),
    }
    let old = legacy_dir();
    let pids = legacy_pids(&old);
    let installed = legacy::installed(&old);
    let locked = legacy::locked(&old);
    line(!installed && pids.is_empty() && locked.is_empty(), format!("legacy: {}{}", if installed { "installed; run `agentq uninstall --legacy`" } else { "removed" }, running(&pids)));
    for f in &locked {
        println!("legacy: locked {}; stop the legacy processes, then delete {}", f.display(), old.display());
    }
    if ok { 0 } else { 1 }
}

fn println_code(line: &str) -> i32 {
    println!("{line}");
    0
}

fn integration_host(t: &Tool) -> executor::Host {
    executor::Host { data: t.install.join("data"), exe: t.install.join(t.exe_name()), clients: profiles::clients(),
        installation: String::new(), generation: 0, grant: Value::Null }
}

fn unit_snapshot(t: &Tool) -> Value {
    let u = service::unit(t, &t.install, PROJECT);
    let read = std::fs::read(&u.path);
    let readable = read.as_ref().map_or_else(|e| e.kind() == std::io::ErrorKind::NotFound, |_| true);
    let bytes = read.ok();
    let text = bytes.as_ref().map(|b| {
        if b.starts_with(&[0xff, 0xfe]) {
            String::from_utf16_lossy(&b[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
        } else { String::from_utf8_lossy(b).into_owned() }
    });
    let launcher = t.install.join(t.exe_name()).display().to_string();
    let launcher = if cfg!(target_os = "macos") || cfg!(windows) { launcher.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") } else { launcher };
    let marker = if cfg!(target_os = "macos") { format!("<key>ProgramArguments</key><array><string>{launcher}</string><string>service</string><string>run</string></array>") }
        else if cfg!(windows) { format!("<Command>{launcher}</Command><Arguments>service run</Arguments>") }
        else { format!("ExecStart=\"{launcher}\" service run") };
    let owned = text.as_ref().is_some_and(|s| s.contains(&marker));
    let loaded = unit_loaded(t);
    let foreign = loaded.as_ref().err().is_some_and(|e| e == "foreign_service");
    let observed = loaded.is_ok() || foreign;
    serde_json::json!({"registered": if readable && observed { Some(owned) } else { None }, "foreign": if readable && observed { Some((text.is_some() && !owned) || foreign) } else { None }, "active": service_owners(t).ok().map(|r| !r.is_empty()),
        "loaded": loaded.ok(),
        "digest": bytes.map(|b| resource::hex(ring::digest::digest(&ring::digest::SHA256, &b).as_ref()))})
}

fn integration_cmd(t: &Tool, args: &[&str], enabled: bool) -> i32 {
    let agents: Vec<_> = args.iter().copied().filter(|a| *a != "--dry-run").collect();
    if agents.len() > 1 { fail("select one agent or the host", "agentq disable|enable [agent] [--dry-run]"); }
    if !args.contains(&"--dry-run") {
        if let Err(e) = names::migrate(&integration_host(t), false, false) { fail(&e, "resolve the name migration conflict"); }
        if let Err(e) = recover_service(t) { fail(&e, "run `agentq status`; service state remains unknown"); }
    }
    let h = integration_host(t);
    let prepare = &mut |ports: &[u16], start: bool| -> Result<(), String> {
        if !start { return Ok(()); }
        let s = control::state(&h.data)?;
        let u = unit_snapshot(t);
        if u["active"].is_null() { return Err("service process inventory unavailable".into()); }
        if u["foreign"] == true { return Err("service unit belongs to another installation".into()); }
        if s["service"]["registered"] != true || u["registered"] != true { return Err("saved service registration is unavailable".into()); }
        if u["loaded"].is_null() { return Err("service registration inventory unavailable".into()); }
        if u["digest"] != s["service"]["digest"] { return Err("service registration changed while disabled".into()); }
        if !ports.is_empty() && engines::headroom_exe(&t.install).is_none() { return Err("existing Headroom engine unavailable".into()); }
        if u["active"] != true && !(cfg!(target_os = "macos") && u["loaded"] == true) { service::run_all(&service::unit(t, &t.install, PROJECT).load, false)?; }
        if !routing::await_proxies(ports, PROXY_START, &mut || {}).is_empty() { return Err("saved proxy not answering".into()); }
        Ok(())
    };
    match control::integrations(&h, agents.first().copied(), enabled, args.contains(&"--dry-run"), unit_snapshot(t), prepare) {
        Ok(mut result) => {
            if !args.contains(&"--dry-run") && !enabled {
                if let Err(e) = finish_disable(t, false) { result["service_error"] = e.into(); }
                result["status"] = control_report(t);
            }
            let good = matches!(result["outcome"].as_str(), Some("planned" | "applied" | "unchanged"));
            println!("{result}");
            if good { 0 } else { 1 }
        }
        Err(e) => fail(&e, "run `agentq status`; preserve the saved state and resolve its conflict"),
    }
}

fn finish_disable(t: &Tool, from_service: bool) -> Result<(), String> {
    recover_service(t)?;
    let data = t.install.join("data");
    let s = control::state(&data)?;
    if s["profiles"].as_object().is_some_and(|p| p.is_empty()) && s["host_disabled"] != true { return Ok(()); }
    if !from_service && s["service"]["state"] == "stopped" && service_owners(t)?.is_empty() { return Ok(()); }
    let ignore = service_owners(t)?;
    let deps = control::dependencies(&ignore);
    let ports: Vec<u16> = routing::ROUTED.iter().filter_map(|a| routing::port(&data, a)).collect();
    if !control::drain(&data, deps, routing::open_connections(&ports))? { return Ok(()); }
    let generation;
    {
        let _lock = executor::lock(&data)?;
        let mut s = control::state(&data)?;
        if s["host_disabled"] != true || s["service"]["state"] != "ready_to_stop" { return Ok(()); }
        generation = s["generation"].clone();
        s["service"]["state"] = "stopping".into();
        s["service"]["owner"] = std::process::id().into();
        control::save(&data, "integration.json", &s)?;
    }
    let current = unit_snapshot(t);
    let saved = control::state(&data)?;
    let result = if current["foreign"] == true {
        Err("service registration belongs to another installation".into())
    } else if current["registered"].is_null() || current["active"].is_null() {
        Err("service registration or process state unavailable".into())
    } else if current["registered"] == true && current["digest"] != saved["service"]["digest"] {
        Err("service registration changed while disabled".into())
    } else if current["registered"] == true {
        unload_disabled(t).and_then(|_| end_tree(t))
    } else if current["active"] != true { Ok(()) } else { Err("service registration unavailable".into()) };
    let _lock = executor::lock(&data)?;
    let mut s = control::state(&data)?;
    if s["generation"] != generation { return Err("integration operation superseded".into()); }
    s["service"]["state"] = if result.is_ok() && (from_service || service_owners(t).is_ok_and(|r| r.is_empty())) { "stopped" } else { "unknown" }.into();
    control::save(&data, "integration.json", &s)?;
    result?;
    if from_service { std::process::exit(0); }
    Ok(())
}

/// An absent OS registration is already unloaded; process absence alone cannot establish that.
fn unit_loaded(t: &Tool) -> Result<bool, String> {
    let u = service::unit(t, &t.install, PROJECT);
    let launcher = t.install.join(t.exe_name()).display().to_string();
    let (mut command, label, platform) = if cfg!(target_os = "macos") {
        let target = &u.unload[0][2];
        let (domain, label) = target.rsplit_once('/').ok_or("invalid service target")?;
        let mut c = Command::new("launchctl");
        c.args(["print", domain]);
        (c, label.to_string(), "macos")
    } else if cfg!(windows) {
        let label = format!("{}-{PROJECT}", t.title);
        let mut c = Command::new("powershell");
        let expected = launcher.replace('\'', "''");
        c.args(["-NoProfile", "-Command", &format!("$tasks=@(Get-ScheduledTask -ErrorAction Stop | Where-Object {{ $_.TaskName -eq '{label}' -and $_.TaskPath -eq '\\' }}); if ($tasks.Count -eq 0) {{ 'absent' }} elseif ($tasks.Count -eq 1 -and @($tasks[0].Actions).Count -eq 1 -and $tasks[0].Actions[0].Execute -eq '{expected}' -and $tasks[0].Actions[0].Arguments -eq 'service run') {{ 'owned' }} else {{ 'foreign' }}")]);
        (c, label, "windows")
    } else {
        let label = format!("{}-{PROJECT}.service", t.name);
        let mut c = Command::new("systemctl");
        c.args(["--user", "show", &label, "--property=LoadState,UnitFileState,ActiveState,ExecStart"]);
        (c, label, "linux")
    };
    let o = coreq::quiet(&mut command).output().map_err(|_| "service registration inventory unavailable")?;
    if !o.status.success() || !o.stderr.is_empty() { return Err("service registration inventory failed".into()); }
    let listing = std::str::from_utf8(&o.stdout).map_err(|_| "service registration inventory unreadable")?;
    let loaded = unit_loaded_in(platform, listing, &label)?;
    if !loaded || platform == "windows" { return Ok(loaded); }
    let registration = if platform == "macos" {
        let o = Command::new("launchctl").args(["print", &u.unload[0][2]]).output().map_err(|_| "service registration inventory unavailable")?;
        if !o.status.success() || !o.stderr.is_empty() { return Err("service registration inventory failed".into()); }
        String::from_utf8(o.stdout).map_err(|_| "service registration inventory unreadable")?
    } else { listing.into() };
    if !unit_owned_in(platform, &registration, &launcher) { return Err("foreign_service".into()); }
    Ok(true)
}

fn unit_loaded_in(platform: &str, listing: &str, label: &str) -> Result<bool, String> {
    match platform {
        "macos" if listing.lines().any(|l| l.trim() == "services = {") => Ok(listing.split_whitespace().any(|t| t == label)),
        "windows" => match listing.trim() { "owned" => Ok(true), "absent" => Ok(false), "foreign" => Err("foreign_service".into()), _ => Err("service registration inventory incomplete".into()) },
        "linux" if listing.lines().any(|l| l == "LoadState=not-found") => Ok(false),
        "linux" if listing.lines().any(|l| l == "LoadState=loaded") && listing.lines().any(|l| l.starts_with("ActiveState=")) => Ok(!listing.lines().any(|l| l == "UnitFileState=disabled")
            || !listing.lines().any(|l| matches!(l, "ActiveState=inactive" | "ActiveState=failed"))),
        _ => Err("service registration inventory incomplete".into()),
    }
}

fn unit_owned_in(platform: &str, listing: &str, launcher: &str) -> bool {
    if platform == "macos" {
        let program = listing.lines().find_map(|l| l.trim().strip_prefix("program = "));
        let args: Vec<_> = listing.lines().skip_while(|l| l.trim() != "arguments = {").skip(1).take_while(|l| l.trim() != "}").map(str::trim).collect();
        program == Some(launcher) && args == [launcher, "service", "run"]
    } else {
        let start = listing.lines().find_map(|l| l.strip_prefix("ExecStart="));
        [launcher.to_string(), launcher.replace(' ', "\\x20")].iter().any(|exe| start.is_some_and(|s| s.contains(&format!("path={exe} ;")) && s.contains(&format!("argv[]={exe} service run ;"))))
    }
}

fn unload_disabled(t: &Tool) -> Result<(), String> {
    if !unit_loaded(t)? && service_owners(t)?.is_empty() { return Ok(()); }
    let u = service::unit(t, &t.install, PROJECT);
    for (i, cmd) in u.unload.iter().enumerate() {
        // Windows /End fails for an idle task; /Delete still has to remove its logon trigger.
        if cfg!(windows) && i == 0 && service_owners(t)?.is_empty() { continue; }
        if let Err(e) = service::run_all(std::slice::from_ref(cmd), false) {
            if !unit_loaded(t)? && service_owners(t)?.is_empty() { return Ok(()); }
            return Err(e);
        }
    }
    if unit_loaded(t)? { return Err("service registration remains active".into()); }
    Ok(())
}

fn recover_service(t: &Tool) -> Result<(), String> {
    let data = t.install.join("data");
    let s = control::state(&data)?;
    if s["service"]["state"] != "stopping" { return Ok(()); }
    let owner = s["service"]["owner"].as_u64().and_then(|p| u32::try_from(p).ok()).ok_or("service operation identity unavailable")?;
    let alive = control::process_alive(owner)?;
    let runs = service_owners(t)?;
    let deps = control::dependencies(&runs)?;
    control::reconcile_stop(&data, owner, alive, !runs.is_empty(), deps.get("controllers").copied().unwrap_or(0) > 0)
}

fn control_report(t: &Tool) -> Value {
    let data = t.install.join("data");
    let clients = profiles::clients();
    let process_result = service_runs(t);
    let runs = process_result.as_deref().unwrap_or(&[]);
    let owners = service_owners(t);
    let inventory = if process_result.is_ok() { owners.as_ref().map_err(|e| e.clone()).and_then(|p| control::dependencies(p)) } else { Err("service inventory unavailable".into()) };
    let ports: Vec<u16> = routing::ROUTED.iter().filter_map(|a| routing::port(&data, a)).collect();
    let connections = routing::open_connections(&ports);
    let mut configuration = profiles::readonly(&clients, &data, now());
    for p in configuration["profiles"].as_array_mut().into_iter().flatten() {
        if let Some(c) = clients.iter().find(|c| p["kind"] == c.kind) {
            p["aliases"] = serde_json::json!(profiles::KINDS.iter().find(|k| k.0 == c.kind).map(|k| k.2).unwrap_or(&[]));
            p["integration"] = control::observation(&data, c, &t.install.join(t.exe_name()));
            if p["integration"]["desired"] == "disabled" {
                p["integration"]["runtime"] = match &inventory {
                    Ok(i) => {
                        let agent = if c.agent == "claude-desktop" { "claude" } else { c.agent };
                        if p["integration"]["configuration"] != "detached" { "unknown" }
                        else if i.get(agent).copied().unwrap_or(0) + i.get("agentq").copied().unwrap_or(0) > 0 {
                            if c.agent == "codex" { "pending_drain" } else { "pending_reload" }
                        } else if c.agent == "codex" && connections != Ok(0) { "pending_drain" }
                        else { "drained" }
                    },
                    Err(_) => "unknown",
                }.into();
                p["integration"]["runtime_at"] = now().into();
            }
        }
    }
    let state = control::state(&data);
    let config = control::config(&data);
    let service = if process_result.is_err() { serde_json::json!({"state": "unknown", "reason": "inventory_unavailable"}) }
        else if let Some(status) = read_status(t, 3 * PASS).filter(|s| runs.iter().any(|p| s["pid"].as_u64() == Some(u64::from(*p)))) {
            serde_json::json!({"state": "running", "version": status["version"], "pid": status["pid"], "at": status["at"]})
        } else if owners.as_ref().is_ok_and(|p| p.is_empty()) && state.as_ref().is_ok_and(|s| s["host_disabled"] == true && s["service"]["state"] == "stopped")
            && unit_snapshot(t)["loaded"] == false {
            serde_json::json!({"state": "stopped", "reason": "integration_disabled"})
        } else { serde_json::json!({"state": "unknown", "reason": "no_fresh_service_evidence"}) };
    let owned: Vec<_> = configuration["profiles"].as_array().into_iter().flatten().filter(|p| p["owned"] == true || p["integration"]["desired"] == "disabled").collect();
    let integration: Vec<_> = owned.iter().map(|p| match p["integration"]["configuration"].as_str() { Some("attached") => "on", Some("detached") => "off", _ => "unknown" }).collect();
    let optimization: Vec<_> = owned.iter().filter(|p| p["effective"]["state"] != "unsupported").map(|p| {
        if p["effective"]["state"] == "unknown" { "unknown" } else if p["effective"]["state"] == "configured" { "on" } else { "off" }
    }).collect();
    serde_json::json!({"schema": 1, "at": now(), "service": service, "configuration": configuration,
        "integration": control::summary(&integration), "optimization": control::summary(&optimization), "engines": engines::status(&t.install),
        "host_disabled": state.as_ref().ok().map(|s| &s["host_disabled"]),
        "compression": config.as_ref().ok().map(|c| serde_json::json!({"enabled": c["compression"]["enabled"].as_bool().unwrap_or(true),
            "min_bytes": c["compression"]["min_bytes"].as_u64().unwrap_or(2048), "stats": c["compression"]["stats"].as_str().unwrap_or("queue")})),
        "control": if state.is_ok() && config.is_ok() { "readable" } else { "unknown" }})
}

fn control_status(t: &Tool, json: bool) -> i32 {
    let report = control_report(t);
    if json { println!("{report}"); } else {
        println!("service: {}", report["service"]["state"].as_str().unwrap_or("unknown"));
        println!("integration: {}, optimization: {} (configured, runtime verification separate)", report["integration"].as_str().unwrap_or("unknown"), report["optimization"].as_str().unwrap_or("unknown"));
        println!("host optimization: {}", report["compression"]["enabled"].as_bool().map_or("unknown", |b| if b { "on" } else { "off" }));
        for p in report["configuration"]["profiles"].as_array().into_iter().flatten() {
            println!("{}: integration {}, configuration {}, runtime {}; optimization {}, adapter {} ({})", p["kind"].as_str().unwrap_or("unknown"),
                p["integration"]["desired"].as_str().unwrap_or("unknown"), p["integration"]["configuration"].as_str().unwrap_or("unknown"),
                p["integration"]["runtime"].as_str().unwrap_or("unknown"), p["optimization"].as_str().unwrap_or("unsupported"),
                p["effective"]["adapter"].as_str().unwrap_or("unsupported"), p["effective"]["state"].as_str().unwrap_or("unknown"));
        }
    }
    i32::from(report["control"] == "unknown")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn host() -> String {
    Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}

/// agentq.md § 5: `<install>/status.json`, not coreq's `<root>/.<name>/`.
fn status_path(t: &Tool) -> PathBuf {
    t.install.join("status.json")
}

fn read_status(t: &Tool, max_age: u64) -> Option<Value> {
    let p = status_path(t);
    status::age(&p).filter(|a| *a <= max_age)?;
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn service_cmd(t: &Tool, cmd: &str) -> i32 {
    if matches!(cmd, "install" | "restart") && control::host_disabled(&t.install.join("data")) != Ok(false) {
        fail("integration_disabled: service activation is disabled", "run `agentq enable` to restore the saved integrations");
    }
    if matches!(cmd, "install" | "restart" | "stop" | "uninstall") && unit_snapshot(t)["foreign"] == true {
        fail("service unit belongs to another installation", "select that installation explicitly; its service was kept");
    }
    if matches!(cmd, "install" | "restart" | "stop" | "uninstall") && unit_snapshot(t)["registered"].is_null() {
        fail("service registration unreadable", "restore access to its existing unit; it was kept");
    }
    let u = service::unit(t, &t.install, PROJECT);
    let res = match cmd {
        "run" => serve(t),
        "install" => install_release(t).and_then(|_| start_unit(&u)),
        // the task first: its launcher would restart a killed service run (agentq.md § 5)
        "restart" => service::run_all(&u.stop, true).and_then(|_| {
            end_tree(t)?;
            service::run_all(&u.load, false)
        }).and_then(|_| if cfg!(windows) { one_service_run(t) } else { Ok(()) }),
        "stop" => end_tree(t).and_then(|_| service::run_all(&u.stop, false)),
        "uninstall" => end_tree(t).and_then(|_| service::run_all(&u.unload, true)).and_then(|_| std::fs::remove_file(&u.path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }).map_err(|e| e.to_string())),
        "status" => {
            let report = control_report(t);
            let s = &report["service"];
            println!("service: {} {} pid {}, reason: {}", s["state"].as_str().unwrap_or("unknown"), s["version"].as_str().unwrap_or(""), s["pid"], s["reason"].as_str().unwrap_or("fresh process and status readback"));
            let code = i32::from(s["state"] == "unknown");
            let runs = service_runs(t).unwrap_or_default();
            if runs.len() > 1 {
                println!("service: error: {} service run processes, pids {}; run `agentq service restart`", runs.len(), pids(&runs));
                return 1;
            }
            return code;
        }
        _ => fail(&format!("unknown service command `{cmd}`"), &format!("usage: {USAGE}")),
    };
    match res {
        Ok(()) => println_code(&format!("service: {cmd} done")),
        Err(e) => fail(&format!("service {cmd}: {e}"), "run `agentq service status`"),
    }
}

/// `service install` before the unit: the release, then its engines.
fn install_release(t: &Tool) -> Result<(), String> {
    names::migrate(&integration_host(t), false, false).and_then(|_| install_self(t)).and_then(|_| command_path::register(&t.install))
        .map(|_| engines::ensure(&t.install).iter().for_each(|l| println!("{l}")))
}

/// Writes the unit and (re)starts it.
fn start_unit(u: &service::Unit) -> Result<(), String> {
    service::write_unit(u).and_then(|_| service::run_all(&u.unload, true)).and_then(|_| service::run_all(&u.load, false))
}

/// `onboard --yes` after the release (#137): fix each routed agent's port, apply `wants` plus routing edits, then
/// `start` the service and wait for each proxy (one `restart` for a silent one). Each routed agent, its port and
/// whether its proxy answers.
#[allow(clippy::too_many_arguments)]
fn route_and_start<'a>(
    data: &Path,
    root: &dyn Fn(&str) -> PathBuf,
    agents: &[&'a str],
    mut wants: Vec<onboard::Want>,
    journal: &Path,
    headroom: Result<(), String>,
    limit: Duration,
    start: &mut dyn FnMut() -> Result<(), String>,
    restart: &mut dyn FnMut(),
) -> Result<Vec<(&'a str, u16, bool)>, String> {
    let mut ports = vec![];
    {
        let _lock = executor::lock(data).map_err(|e| format!("onboard: {e}"))?;
        if agents.iter().any(|a| control::disabled(data, a) != Ok(false)) { return Err("integration_disabled: run `agentq enable`".into()); }
        for a in agents.iter().filter(|a| routing::ROUTED.contains(a)) {
            match headroom.clone().and_then(|_| routing::fix_port(data, a)) {
                Ok(port) => {
                    wants.extend(onboard::routing_wants(a, &root(a), port));
                    ports.push((*a, port));
                }
                Err(e) => println!("routing: {a} direct: {e}"),
            }
        }
        routing::set_direct(data, agents, false).map_err(|e| format!("onboard: {e}"))?;
        onboard::apply(&wants, journal).map_err(|e| format!("onboard: {e}"))?.iter().for_each(|l| println!("{l}"));
    }
    start()?;
    let silent = routing::await_proxies(&ports.iter().map(|p| p.1).collect::<Vec<_>>(), limit, restart);
    Ok(ports.into_iter().map(|(a, p)| (a, p, !silent.contains(&p))).collect())
}

/// coreq `install_self` that skips copies already in place (agentq.md § 5): Windows refuses to overwrite a running exe.
fn install_self(t: &Tool) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    copy_unless_same(&me, &t.release_exe(VERSION))?;
    let launcher = t.install.join(t.exe_name());
    match copy_unless_same(&me, &launcher) {
        // the launcher only reads `current`: an older running one keeps working
        Err(e) if launcher.exists() => println!("launcher: kept the running one ({e})"),
        r => r.map(|_| ())?,
    }
    match t.current() {
        Some(c) if !update::newer(VERSION, &c) => {}
        old => {
            if let Some(c) = old {
                write_atomic(&t.install.join("previous"), &c)?;
            }
            write_atomic(&t.install.join("current"), VERSION)?;
        }
    }
    Ok(())
}

/// Copies `me` to `dst` unless `dst` already holds the same bytes; true when it copied.
fn copy_unless_same(me: &Path, dst: &Path) -> Result<bool, String> {
    let bytes = std::fs::read(me).map_err(|e| format!("{}: {e}", me.display()))?;
    if std::fs::read(dst).is_ok_and(|d| d == bytes) {
        return Ok(false);
    }
    if let Some(d) = dst.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::copy(me, dst).map(|_| true).map_err(|e| format!("{}: {e}", dst.display()))
}

/// `schtasks /End` ends only the launcher; end every service run tree too, orphans included (agentq.md § 5).
fn end_tree(t: &Tool) -> Result<(), String> {
    if cfg!(windows) {
        for pid in service_runs(t)? {
            let o = coreq::quiet(Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"])).output().map_err(|_| "service process stop unavailable")?;
            if !o.status.success() && control::process_alive(pid)? { return Err(format!("service process {pid} could not be stopped")); }
        }
    }
    Ok(())
}

/// Windows restart: wait until exactly one service run is live and `status.json` names it (agentq.md § 5).
fn one_service_run(t: &Tool) -> Result<(), String> {
    let mut runs = vec![];
    for _ in 0..3 * PASS {
        std::thread::sleep(Duration::from_secs(1));
        runs = service_runs(t)?;
        if let [pid] = runs[..]
            && read_status(t, 3 * PASS).and_then(|s| s["pid"].as_u64()) == Some(pid.into())
        {
            return Ok(());
        }
    }
    Err(format!("expected one service run named by status.json after {} s, found pids [{}]", 3 * PASS, pids(&runs)))
}

/// Live `service run` processes of this install (agentq.md § 5).
fn service_listing(t: &Tool) -> Result<String, String> {
    let out = if cfg!(windows) {
        let ps = format!("[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); Get-CimInstance Win32_Process -Filter \"Name='{}'\" | ForEach-Object {{ \"$($_.ProcessId) $($_.CommandLine)\" }}", t.exe_name());
        coreq::quiet(Command::new("powershell").args(["-NoProfile", "-Command", &ps])).output()
    } else {
        Command::new("ps").args(["-axo", "pid=,args="]).output()
    };
    let out = out.map_err(|_| "service process inventory unavailable")?;
    if !out.status.success() { return Err("service process inventory unavailable".into()); }
    let listing = std::str::from_utf8(&out.stdout).map_err(|_| "service process inventory unreadable")?;
    if !cfg!(windows) && listing.trim().is_empty() { return Err("empty service process inventory".into()); }
    Ok(listing.into())
}

fn service_runs(t: &Tool) -> Result<Vec<u32>, String> {
    service_listing(t).map(|s| service_runs_in(&s, &t.install.join("releases")))
}

fn service_owners(t: &Tool) -> Result<Vec<u32>, String> {
    service_listing(t).map(|s| service_runs_in(&s, &t.install))
}

/// Pids from `<pid> <command line>` lines that run a binary under `releases` with `service run`; the launcher is not one.
fn service_runs_in(list: &str, releases: &Path) -> Vec<u32> {
    let dir = releases.display().to_string().replace('\\', "/");
    let windows = cfg!(windows) || dir.as_bytes().get(1) == Some(&b':');
    let dir = if windows { dir.to_lowercase() } else { dir };
    let prefix = format!("{}/", dir.trim_end_matches('/'));
    list.lines()
        .filter_map(|l| {
            let (pid, cmd) = l.trim().split_once(char::is_whitespace)?;
            let exe = cmd.trim().strip_suffix(" service run")?.trim().trim_matches(['\"', '\'']).replace('\\', "/");
            let exe = if windows { exe.to_lowercase() } else { exe };
            if exe.starts_with(&prefix) && (exe.ends_with("/agentq") || exe.ends_with("/agentq.exe")) { pid.parse().ok() } else { None }
        })
        .collect()
}

fn pids(p: &[u32]) -> String {
    p.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

/// Built-in Windows process list: the launcher and other commands are not service runs (agentq.md § 5).
fn single_instance_selftest() -> bool {
    let list = "4120 \"C:\\Users\\u\\AppData\\Local\\agentq\\agentq.exe\" service run\r\n\
                4188 \"C:\\Users\\u\\AppData\\Local\\agentq\\releases\\0.5.3\\agentq.exe\" service run\r\n\
                5230 C:\\Users\\u\\AppData\\Local\\agentq\\releases\\0.5.3\\agentq.exe service run\r\n\
                6001 \"C:\\Users\\u\\AppData\\Local\\agentq\\releases\\0.5.3\\agentq.exe\" service status\r\n";
    service_runs_in(list, Path::new(r"C:\Users\u\AppData\Local\agentq\releases")) == [4188, 5230]
        && service_runs_in(list, Path::new(r"C:\Users\u\AppData\Local\agentq")) == [4120, 4188, 5230]
        && service_runs_in("77 /tmp/install-other/releases/v1/agentq service run\n", Path::new("/tmp/install")).is_empty()
}

/// One status pass every 30 s, update check hourly; a new release exits 75 so the unit restarts into it.
fn serve(t: &Tool) -> ! {
    names::migrate(&integration_host(t), false, false).unwrap_or_else(|e| fail(&e, "resolve the name migration conflict before restarting the service"));
    // a release with a new engine pin installs it here; status keeps its 30 s pace meanwhile
    let install = t.install.clone();
    if control::host_disabled(&install.join("data")) == Ok(false) {
        std::thread::spawn(move || engines::ensure(&install).iter().for_each(|l| eprintln!("agentq: {l}")));
    }
    let install = t.install.clone();
    std::thread::spawn(move || telemetry::sender(&install, PASS));
    // CONFIGURATION "Host executor": an operation interrupted by the last stop resolves before anything else
    if let Ok(_lock) = executor::lock(&t.install.join("data")) {
        executor::reconcile(&executor::Host::local(&t.install, t.install.join(t.exe_name()), now()));
    }
    let mut checked = 0;
    let mut error = None;
    let mut workers = vec![];
    let launched = std::env::current_exe().ok() == Some(t.release_exe(VERSION));
    let mut seen: Option<(u64, String)> = None;
    loop {
        if let Err(e) = finish_disable(t, true) { eprintln!("agentq: disable: {e}"); }
        routing::supervise(&t.install, &mut workers);
        let maintenance = control::host_disabled(&t.install.join("data")) == Ok(false);
        if maintenance && now() >= checked + UPDATE_EVERY {
            checked = now();
            error = match update::update(t, false) {
                Ok(Some(v)) => {
                    eprintln!("agentq: updated to {v}");
                    None
                }
                Ok(None) => None,
                Err(e) => Some(format!("update: {e}")),
            };
        }
        // agentq.md § 5: the CLI `agentq update` moves `current` too; restart into it once the proxy drains
        match t.current().filter(|c| control::host_disabled(&t.install.join("data")) == Ok(false) && launched && c != VERSION) {
            None => seen = None,
            Some(c) => {
                let since = seen.as_ref().map_or(now(), |s| s.0);
                let ports: Vec<u16> = workers.iter().map(|w| w.0).collect();
                match drain(routing::open_connections(&ports), now().saturating_sub(since)) {
                    Ok(why) => {
                        eprintln!("agentq: restarting into {c} ({why})");
                        routing::stop(&mut workers);
                        std::process::exit(75);
                    }
                    Err(why) => {
                        if seen.as_ref().is_none_or(|s| s.1 != why) {
                            eprintln!("agentq: restart into {c} deferred: {why}");
                        }
                        seen = Some((since, why));
                    }
                }
            }
        }
        let mut s = status::common(t, &host(), PROJECT, now(), error.clone(), None);
        s["release"] = release::observe(t, now());
        s["queue"] = telemetry::status(&t.install.join("data"), now());
        s["agents"] = routing::agents(&t.install.join("data"));
        s["engines"] = engines::status(&t.install);
        s["configuration"] = profiles::status(&t.install.join("data"), now());
        match write_atomic(&status_path(t), &serde_json::to_string_pretty(&s).unwrap_or_default()) {
            // ponytail: healthy after the first written status; add real checks when the service owns ports (§ 5)
            Ok(()) => {
                let healthy = t.install.join("releases").join(VERSION).join("healthy");
                if healthy.parent().is_some_and(Path::exists) && !healthy.exists() {
                    let _ = std::fs::write(healthy, "");
                }
            }
            Err(e) => eprintln!("agentq: status: {e}"),
        }
        std::thread::sleep(Duration::from_secs(PASS));
    }
}

/// Restart into a new `current` (agentq.md § 5): Ok once no proxied connection is open or 15 min passed, else the deferral.
fn drain(open: Result<usize, String>, waited: u64) -> Result<String, String> {
    match open {
        Ok(0) => Ok("no open proxied connection".into()),
        _ if waited >= DRAIN => Ok(format!("drain ended after {} min", DRAIN / 60)),
        Ok(n) => Err(format!("{n} open proxied connections")),
        Err(e) => Err(format!("proxied connections unknown: {e}")),
    }
}

/// Built-in fixtures only; writes nothing outside the OS temp dir (coreq.md § Selftest harness).
fn selftest(t: &Tool) -> i32 {
    let u = service::unit(t, &t.install, PROJECT);
    let checks = [
        ("version", !update::newer("0.0.0-dev", VERSION)),
        ("unit", u.path.file_name().is_some_and(|f| f.to_string_lossy().contains(PROJECT))),
        ("unit commands", !u.load.is_empty() && !u.unload.is_empty() && !u.stop.is_empty()),
        ("status", status::common(t, "h", PROJECT, 1, None, None)["version"] == VERSION),
        ("mcp tools", mcp::tools().as_array().is_some_and(|a| a.len() == 3)),
        ("telemetry event", telemetry::span("agentq.verify", "selftest", Value::Null, 1, "i", "h")["attributes"].as_array().is_some_and(|a| a.len() == 7)),
        ("legacy strip", legacy::selftest()),
        ("legacy locked", legacy::selftest_locked()),
        // the running binary is locked on Windows: the installed copy that equals it is read, never overwritten
        ("self copy skip", std::env::current_exe().is_ok_and(|me| copy_unless_same(&me, &me) == Ok(false))),
        ("json bom", onboard::selftest()),
        ("single service run", single_instance_selftest()),
        ("configuration fixtures", configure::selftest()),
        ("snapshot", snapshot::project("i", "linux", None, None, 0)["resource"]["unknown"] == "no sample yet"),
        ("local telemetry", local::selftest()),
        ("engine pins", engines::rtk_asset().is_some() && engines::HEADROOM_LOCK.contains(&format!("headroom-ai=={}", engines::HEADROOM))),
    ];
    let bad: Vec<&str> = checks.iter().filter(|c| !c.1).map(|c| c.0).collect();
    if bad.is_empty() {
        println_code(&format!("selftest: ok {VERSION} {}", update::target()))
    } else {
        fail(&format!("selftest failed: {}", bad.join(", ")), "report the release as broken")
    }
}

fn journal(t: &Tool) -> PathBuf {
    t.install.join("data").join("owned.json")
}

/// agentq.md § 1: discovery without `--yes`; with it, legacy removal (§ 10), § 4 edits, service install.
fn onboard(t: &Tool, args: &[&str]) -> i32 {
    let yes = args.contains(&"--yes");
    let picked: Vec<&str> = args.iter().copied().filter(|a| *a != "--yes").collect();
    if let Some(a) = picked.iter().find(|a| !onboard::AGENTS.contains(a)) {
        fail(&format!("unknown agent `{a}`"), &format!("usage: {USAGE}"));
    }
    let ready: Vec<&str> = onboard::AGENTS.into_iter().filter(|a| onboard::root(a).exists()).collect();
    for a in onboard::AGENTS {
        println!("{a}: {} {}", if ready.contains(&a) { "ready" } else { "not found" }, onboard::root(a).display());
    }
    let old = legacy_dir();
    // a still-running legacy process recreates the folder with incidents/ and machine-id.json only: not an install
    let installed = legacy::installed(&old);
    let pids = running(&legacy_pids(&old));
    if installed {
        println!("legacy: {} found{pids}", old.display());
    } else if !pids.is_empty() {
        println!("legacy: removed{pids}");
    }
    if !yes {
        return 0;
    }
    let selected = if picked.is_empty() { &ready } else { &picked };
    if control::host_disabled(&t.install.join("data")) != Ok(false)
        || selected.iter().any(|a| control::disabled(&t.install.join("data"), a) != Ok(false))
    {
        fail("integration_disabled: onboarding cannot restore a disabled integration", "run `agentq enable` first");
    }
    let unit = unit_snapshot(t);
    if unit["foreign"] == true || unit["registered"].is_null() {
        fail("service registration belongs to another installation or is unreadable", "keep it and select the intended installation explicitly");
    }
    // `--yes` is the consent (§ 10)
    if installed {
        legacy(&old, false);
    }
    let agents: Vec<&str> = if picked.is_empty() { ready } else { picked.into_iter().filter(|a| ready.contains(a)).collect() };
    // engines first: routing points an agent at the loopback port only when headroom can serve it
    if let Err(e) = install_release(t) {
        fail(&format!("service install: {e}"), "run `agentq service status`");
    }
    let exe = t.install.join(t.exe_name());
    let wants: Vec<_> = agents.iter().flat_map(|a| onboard::wants(a, &onboard::root(a), &onboard::claude_json(), &exe)).collect();
    let headroom = engines::headroom_exe(&t.install).map(|_| ()).ok_or("headroom unavailable".to_string());
    let unit = service::unit(t, &t.install, PROJECT);
    let routed = route_and_start(&t.install.join("data"), &onboard::root, &agents, wants, &journal(t), headroom, PROXY_START, &mut || {
        // the unit after the ports and edits (#137): its first pass starts the proxy on each fixed port
        start_unit(&unit).map(|_| println!("service: install done")).map_err(|e| format!("service install: {e}"))
    }, &mut || {
        println!("routing: proxy not answering after {} s; restarting the service", PROXY_START.as_secs());
        service_cmd(t, "restart");
    });
    match routed {
        Ok(ports) => ports.iter().for_each(|(a, p, up)| {
            let state = if *up { "answers" } else { "not answering; run `agentq service restart`" };
            println!("routing: {a} proxy on 127.0.0.1:{p} {state}");
        }),
        Err(e) => fail(&e, "fix it, then run `agentq onboard --yes` again"),
    }
    doctor(t);
    println_code(&format!("verify: start a new {} session and call agentq search_document", agents.join(", ")))
}

/// agentq.md § 1: remove routing edits only; MCP, hooks and telemetry stay. Files only, so it works without the service.
fn direct(t: &Tool, args: &[&str]) -> i32 {
    if let Some(a) = args.iter().find(|a| !onboard::AGENTS.contains(a)) {
        fail(&format!("unknown agent `{a}`"), &format!("usage: {USAGE}"));
    }
    names::migrate(&integration_host(t), false, false).unwrap_or_else(|e| fail(&e, "resolve the name migration conflict"));
    let agents = if args.is_empty() { &onboard::AGENTS[..] } else { args };
    let _lock = executor::lock(&t.install.join("data")).unwrap_or_else(|e| fail(&format!("direct: {e}"), "wait for it to finish, then run `agentq direct` again"));
    if let Err(e) = routing::set_direct(&t.install.join("data"), agents, true) {
        fail(&format!("direct: {e}"), "fix the file, then run `agentq direct` again");
    }
    match onboard::restore(&journal(t), false, Some(agents)) {
        Ok(lines) if lines.is_empty() => println_code(&format!("direct: {} already direct", agents.join(", "))),
        Ok(lines) => {
            lines.iter().for_each(|l| println!("{l}"));
            println_code("direct: done; `agentq onboard --yes` routes again")
        }
        Err(e) => fail(&format!("direct: {e}"), "fix the file, then run `agentq direct` again"),
    }
}

/// ACCOUNTING O1-O3 over local stats or exported events; default window the last 7 days through this second. Unknown
/// exits 1.
fn accounting_cmd(t: &Tool, args: &[&str]) -> i32 {
    let at = now();
    let data = t.install.join("data");
    let mut input = serde_json::json!({"window": {"start": at.saturating_sub(7 * 86400), "end": at + 1}, "now": at});
    let mut source = None;
    for pair in args.chunks(2) {
        let n = || pair.get(1).and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(|| fail(&format!("stats: {} requires unix seconds", pair[0]), "pass a whole number"));
        match pair {
            ["--from", _] => input["window"]["start"] = n().into(),
            ["--to", _] => input["window"]["end"] = n().into(),
            ["--bucket", _] => input["bucket"] = n().into(),
            ["--events", p] => source = Some(accounting::exported(Path::new(p)).unwrap_or_else(|e| fail(&format!("stats: {e}"), "pass a queue folder or an exported events file"))),
            _ => fail(&format!("stats: unknown option {}", pair.join(" ")), "agentq stats --accounting [--from <unix>] [--to <unix>] [--bucket <s>] [--events <path>]"),
        }
    }
    input["sources"] = serde_json::json!([source.unwrap_or_else(|| accounting::local(&data, at))]);
    match accounting::aggregate(&input) {
        Ok(v) => {
            println!("{v}");
            i32::from(v["state"] == "unknown")
        }
        Err(e) => fail(&format!("stats: {e}"), "pass --from before --to"),
    }
}

/// CONFIGURATION "Host executor": no change prints the profiles; changes print the plan; `--yes` applies that plan.
fn configure_cmd(t: &Tool, args: &[&str]) -> i32 {
    if args.contains(&"--yes") {
        names::migrate(&integration_host(t), false, false).unwrap_or_else(|e| fail(&e, "resolve the name migration conflict"));
    }
    let exe = t.install.join(t.exe_name());
    let at = now();
    let host = executor::Host::local(&t.install, exe, at);
    let (profile, rest) = match args.iter().filter(|a| **a != "--yes").collect::<Vec<_>>().split_first() {
        None => return println_code(&serde_json::to_string_pretty(&profiles::status_with(&host.clients, &host.data, at)).unwrap_or_default()),
        Some((p, rest)) => (p.to_string(), rest.iter().map(|a| a.to_string()).collect::<Vec<_>>()),
    };
    let changes: Vec<Value> = rest.iter().map(|c| c.split_once('=').map_or(serde_json::json!({}), |(f, v)| serde_json::json!({"profile": profile, "field": f, "value": v}))).collect();
    let mut op = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut op).unwrap_or_else(|_| fail("configure: no random source", "run `agentq configure` again"));
    let mut req = serde_json::json!({"schema": configure::SCHEMA, "operation": resource::hex(&op), "kind": "plan", "actor": "local", "workspace": "local", "installation": host.installation,
        "generation": host.generation, "issued_at": at, "expires_at": at + configure::EXPIRY, "changes": changes});
    let plan = executor::execute(&host, &req.to_string(), at);
    println!("{}", serde_json::to_string_pretty(&plan).unwrap_or_default());
    if plan["kind"] != "plan" || plan["writable"] != true {
        fail("configure: no writable plan", "fix the reason above, then run `agentq configure` again");
    }
    if !args.contains(&"--yes") {
        return println_code("Next: run the same command with --yes to apply this plan");
    }
    req["kind"] = "apply".into();
    req["plan"] = serde_json::json!({"revision": plan["revision"], "digest": plan["digest"]});
    let r = executor::execute(&host, &req.to_string(), now());
    println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    match r["outcome"].as_str() {
        Some("applied") => println_code("Next: configured, not verified; start a new client session to verify"),
        _ => fail("configure: not applied", "read the reason above; `agentq configure` shows the current state"),
    }
}

/// agentq.md § 9: restore owned edits, remove the service unit and the install folder.
fn uninstall(t: &Tool, dry: bool) -> i32 {
    if !t.install.exists() {
        return println_code("uninstall: already removed");
    }
    if !dry { names::migrate(&integration_host(t), false, false).unwrap_or_else(|e| fail(&e, "resolve the name migration conflict")); }
    onboard::restore(&journal(t), dry, None).unwrap_or_else(|e| fail(&format!("uninstall: {e}"), "fix the file, then run `agentq uninstall` again")).iter().for_each(|l| println!("{l}"));
    if dry {
        return println_code(&format!("would remove: command registration, service unit, {}", t.install.display()));
    }
    service_cmd(t, "uninstall");
    if let Err(e) = command_path::remove(&t.install) {
        fail(&e, "fix command registration permissions, then run `agentq uninstall` again");
    }
    match std::fs::remove_dir_all(&t.install) {
        Ok(()) => println_code(&format!("removed: {}", t.install.display())),
        Err(e) => fail(&format!("remove {}: {e}", t.install.display()), "close agentq processes and run `agentq uninstall` again"),
    }
}

/// `AGENT_CONTEXT_HOME` (or home)/.agent-context, as the Node package resolves it.
fn legacy_dir() -> PathBuf {
    let home = std::env::var_os("AGENT_CONTEXT_HOME").or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })).unwrap_or_default();
    PathBuf::from(home).join(".agent-context")
}

/// agentq.md § 10: back up and strip legacy config entries, unload the monitor, remove the npm package, move the
/// folder aside.
fn legacy(dir: &Path, dry: bool) -> i32 {
    if !legacy::installed(dir) {
        println!("legacy: already removed{}", running(&legacy_pids(dir)));
        return 0;
    }
    let run = |cmd: &str, args: &[&str]| coreq::quiet(Command::new(cmd).args(args)).output().is_ok_and(|o| o.status.success());
    match legacy::remove(dir, dry, &date(now()), &run) {
        Ok(lines) => {
            lines.iter().for_each(|l| println!("{l}"));
            0
        }
        Err(e) => fail(&format!("legacy uninstall: {e}"), "close programs using the legacy folder, then run `agentq uninstall --legacy` again"),
    }
}

/// `YYYY-MM-DD` in UTC (days-to-civil, H. Hinnant).
fn date(secs: u64) -> String {
    let z = (secs / 86400) as i64 + 719468;
    let (era, doe) = (z.div_euclid(146097), z.rem_euclid(146097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let (d, m) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
    format!("{:04}-{m:02}-{d:02}", yoe + era * 400 + i64::from(m <= 2))
}

/// Pids of processes whose command line holds `<legacy>/`, as the legacy launcher, proxies and MCP servers do.
/// Windows: PowerShell `Win32_Process`, case-insensitive. ponytail: pids per folder, not per file; Restart Manager
/// (`RmGetList`) names the holder of each file if a host needs it.
fn legacy_pids(dir: &Path) -> Vec<String> {
    let needle = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    let o = if cfg!(windows) {
        let ps = format!("Get-CimInstance Win32_Process | ? {{ $_.ProcessId -ne $PID -and $_.CommandLine -and $_.CommandLine.ToLower().Contains('{}') }} | % ProcessId", needle.to_lowercase().replace('\'', "''"));
        coreq::quiet(Command::new("powershell").args(["-NoProfile", "-Command", &ps])).output()
    } else {
        Command::new("pgrep").arg("-f").arg(needle).output()
    };
    o.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().map(String::from).collect()).unwrap_or_default()
}

fn running(pids: &[String]) -> String {
    if pids.is_empty() { String::new() } else { format!("; legacy processes still running: pids {}", pids.join(" ")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #137 A2: `onboard --yes` starts the service only after the port and the Codex edits exist, so the service's
    /// first pass starts the proxy and doctor right after onboard reads codex routed. Fails with the 0.5.9 order
    /// (service started first: its pass finds no port and starts no proxy).
    #[test]
    fn onboard_routes_before_service_start() {
        let d = std::env::temp_dir().join(format!("agentq-onboard-route-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (data, codex) = (d.join("data"), d.join("codex"));
        std::fs::create_dir_all(&codex).unwrap();
        std::fs::write(codex.join("config.toml"), "model = \"gpt-5\"\n").unwrap();
        let mut proxy = None;
        // the service's first pass: one proxy per fixed port, as `routing::supervise` does
        let mut start = || {
            let p = routing::port(&data, "codex").ok_or("service started before the port was fixed")?;
            proxy = Some(std::net::TcpListener::bind(("127.0.0.1", p)).map_err(|e| e.to_string())?);
            Ok(())
        };
        let mut restarts = 0;
        let routed = route_and_start(&data, &|_| codex.clone(), &["codex"], vec![], &d.join("journal.json"), Ok(()), Duration::from_secs(5), &mut start, &mut || restarts += 1).unwrap();
        let p = routing::port(&data, "codex").unwrap();
        assert_eq!(routed, [("codex", p, true)]);
        assert_eq!(restarts, 0);
        assert_eq!(routing::state("codex", &codex, Some(p)).0, "routed", "doctor right after onboard");
        drop(proxy);
        let paused = r#"{"compression":{"enabled":false}}"#;
        std::fs::write(data.join("config.json"), paused).unwrap();
        control::save(&data, "integration.json", &serde_json::json!({"schema":1,"generation":1,"host_disabled":true,"profiles":{}})).unwrap();
        let rejected = route_and_start(&data, &|_| codex.clone(), &["codex"], vec![], &d.join("journal.json"), Ok(()), Duration::ZERO,
            &mut || panic!("a disabled installation must not start"), &mut || panic!("a disabled installation must not restart"));
        assert!(rejected.unwrap_err().contains("integration_disabled"));
        assert_eq!(std::fs::read_to_string(data.join("config.json")).unwrap(), paused, "disabled onboarding must not allocate a port or race with compression preferences");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn date_is_utc_civil() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_791_849_600), "2026-10-13");
        assert_eq!(date(951_782_400), "2000-02-29");
    }

    /// #71 A1: an installed copy equal to the running binary is not rewritten; a differing one is replaced.
    #[test]
    fn self_copy_skips_equal_bytes() {
        let d = std::env::temp_dir().join(format!("agentq-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (me, dst) = (d.join("me"), d.join("releases").join("agentq"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(&me, "v1").unwrap();
        assert_eq!(copy_unless_same(&me, &dst), Ok(true));
        assert_eq!(copy_unless_same(&me, &dst), Ok(false));
        assert_eq!(copy_unless_same(&me, &me), Ok(false));
        std::fs::write(&me, "v2").unwrap();
        assert_eq!(copy_unless_same(&me, &dst), Ok(true));
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "v2");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #111 A2/A3: a new `current` restarts at once when idle, defers on open or unknown connections, at most 15 min.
    #[test]
    fn restart_waits_for_drain() {
        assert!(drain(Ok(0), 0).is_ok());
        assert_eq!(drain(Ok(2), DRAIN - 1), Err("2 open proxied connections".into()));
        assert!(drain(Err("netstat: not found".into()), 0).unwrap_err().contains("unknown"));
        assert!(drain(Ok(2), DRAIN).is_ok());
    }

    /// #74 A2/A3: two release service runs are counted, the Windows launcher is not.
    #[test]
    fn service_runs_skip_launcher() {
        assert!(single_instance_selftest());
        assert_eq!(service_runs_in("  77 /h/.local/share/agentq/releases/0.5.3/agentq service run\n", Path::new("/h/.local/share/agentq/releases")), [77]);
        for (platform, listing, loaded) in [
            ("macos", "gui/501 = {\n services = {\n 7 0 com.agentq.host\n }\n}", true),
            ("macos", "gui/501 = {\n services = {\n 7 0 com.agentq.host-other\n }\n}", false),
            ("windows", "owned\r\n", true), ("windows", "absent\n", false),
            ("linux", "LoadState=loaded\nUnitFileState=enabled\nActiveState=inactive\n", true),
            ("linux", "LoadState=loaded\nUnitFileState=disabled\nActiveState=inactive\n", false),
            ("linux", "LoadState=loaded\nUnitFileState=disabled\nActiveState=activating\n", true),
            ("linux", "LoadState=not-found\nActiveState=inactive\n", false),
        ] {
            let label = if platform == "windows" { "AgentQ-host" } else { "com.agentq.host" };
            assert_eq!(unit_loaded_in(platform, listing, label), Ok(loaded), "repeated disable must distinguish an unloaded unit from an idle registered service");
        }
        assert!(unit_loaded_in("macos", "", "com.agentq.host").is_err());
        assert!(unit_loaded_in("windows", "foreign", "AgentQ-host").is_err());
        for platform in ["macos", "linux"] {
            let owned = if platform == "macos" { "program = /install/agentq\narguments = {\n/install/agentq\nservice\nrun\n}\n" }
                else { "ExecStart={ path=/install/agentq ; argv[]=/install/agentq service run ; ignore_errors=no ; }\n" };
            assert!(unit_owned_in(platform, owned, "/install/agentq"));
            assert!(!unit_owned_in(platform, owned, "/other-install/agentq"), "a matching OS label is not ownership");
            assert!(!unit_owned_in(platform, &owned.replace("service run", "service stop").replace("service\nrun", "service\nstop"), "/install/agentq"));
        }
    }

    /// #64 A3: a running legacy process recreated the folder with leftovers only: removed, and its pid is named.
    #[cfg(unix)]
    #[test]
    fn legacy_leftovers_name_running_pids() {
        let d = std::env::temp_dir().join(format!("agentq-leftover-{}", std::process::id()));
        std::fs::create_dir_all(d.join("incidents")).unwrap();
        std::fs::write(d.join("machine-id.json"), "{}").unwrap();
        let mut p = Command::new("sh").args(["-c", "sleep 30; true"]).arg(d.join("launcher.mjs")).spawn().unwrap();
        assert_eq!(legacy_pids(&d), vec![p.id().to_string()]);
        assert_eq!(legacy(&d, false), 0, "no launcher: nothing to uninstall");
        let _ = p.kill();
        let _ = p.wait();
        assert!(legacy_pids(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
