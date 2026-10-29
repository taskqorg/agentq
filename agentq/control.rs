//! Local temporary controls (agentq.md §4): compression preferences and durable integration intent.
use crate::{executor, onboard, profiles, routing};
use serde_json::{Value, json};
use std::path::Path;

pub const AGENTS: [&str; 4] = ["codex", "claude", "hermes", "claude-desktop"];

fn read(path: &Path, missing: Value) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => serde_json::from_slice(&bytes).map_err(|_| "invalid control JSON".into()),
        Ok(_) => Err("control state exceeds 1 MiB".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(missing),
        Err(e) => Err(format!("control state unreadable: {}", e.kind())),
    }
}

/// Missing settings take existing defaults; malformed settings are never replaced by a mutation.
pub fn config(data: &Path) -> Result<Value, String> {
    let c = read(&data.join("config.json"), json!({}))?;
    if !c.is_object() || (!c["direct"].is_null() && !c["direct"].as_array().is_some_and(|a| a.iter().all(Value::is_string))) {
        return Err("invalid config.json".into());
    }
    if let Some(v) = c.get("compression") {
        if !v.is_object()
            || v.get("enabled").is_some_and(|v| !v.is_boolean())
            || v.get("min_bytes").is_some_and(|v| v.as_u64().is_none())
            || v.get("stats").is_some_and(|v| !matches!(v.as_str(), Some("queue" | "off")))
            || v.get("agents").is_some_and(|v| !v.as_object().is_some_and(|a| a.values().all(Value::is_boolean)))
        {
            return Err("invalid compression settings".into());
        }
    }
    Ok(c)
}

pub fn state(data: &Path) -> Result<Value, String> {
    let s = read(&data.join("integration.json"), json!({"schema": 1, "generation": 0, "host_disabled": false, "profiles": {}}))?;
    if s["schema"] != 1 || s["generation"].as_u64().is_none() || !s["host_disabled"].is_boolean()
        || (!s["service"].is_null() && !s["service"].as_object().is_some_and(|o| {
            ["registered", "loaded", "active", "foreign"].iter().all(|k| o.get(*k).is_none_or(|v| v.is_null() || v.is_boolean()))
                && o.get("state").is_none_or(Value::is_string) && o.get("owner").is_none_or(|v| v.as_u64().is_some())
        }))
        || !s["profiles"].as_object().is_some_and(|p| p.len() <= 4 && p.iter().all(|(agent, r)| {
            AGENTS.contains(&agent.as_str()) && r.is_object() && r["root"].as_str().is_some_and(|p| Path::new(p).is_absolute())
                && matches!(r["phase"].as_str(), Some("detaching" | "detached" | "enabling" | "conflict"))
                && r["at"].as_u64().is_some() && r["direct"].is_boolean()
                && r["entries"].as_array().is_some_and(|a| a.len() <= 128 && a.iter().all(|e| {
                    e.is_object() && e["file"].as_str().is_some_and(|p| Path::new(p).is_absolute())
                        && e["key"].as_array().is_some_and(|ks| !ks.is_empty() && ks.len() <= 8 && ks.iter().all(Value::is_string))
                        && matches!(e["kind"].as_str(), Some("set" | "append" | "text" | "replace" | "file"))
                        && matches!(e["field"].as_str(), Some("routing" | "mcp" | "hooks" | "plugin" | "instructions"))
                        && e["source"].as_str().is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                }))
        }))
    {
        return Err("invalid integration.json".into());
    }
    Ok(s)
}

/// Private state may carry managed values. The enclosing directory is private before the atomic write.
pub fn save(data: &Path, name: &str, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 { return Err("control state exceeds 1 MiB".into()); }
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let path = data.join(name);
    coreq::write_atomic(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn disabled(data: &Path, agent: &str) -> Result<bool, String> {
    let s = state(data)?;
    Ok(s["host_disabled"] == true || s["profiles"].get(agent).is_some())
}

pub fn host_disabled(data: &Path) -> Result<bool, String> {
    Ok(state(data)?["host_disabled"] == true)
}

/// Public projection contains no saved paths, owned values or arbitrary error messages.
pub fn intent(data: &Path, agent: &str) -> Value {
    match state(data) {
        Ok(s) => match s["profiles"].get(agent) {
            Some(r) => json!({"desired": "disabled", "configuration": if r["phase"] == "detached" { "detached" } else { "unknown" },
                "phase": r["phase"], "runtime": if r["runtime_at"].as_u64().is_some_and(|at| at <= crate::now() && crate::now() - at <= 3 * crate::PASS) { r["runtime"].as_str().unwrap_or("unknown") } else { "unknown" },
                "at": r["at"], "runtime_at": r["runtime_at"].as_u64(), "generation": s["generation"]}),
            None if s["host_disabled"] == true => json!({"desired": "disabled", "configuration": "unknown", "runtime": "unknown", "generation": s["generation"]}),
            None => json!({"desired": "enabled", "generation": s["generation"]}),
        },
        Err(_) => json!({"desired": "unknown", "configuration": "unknown", "runtime": "unknown", "reason": "unreadable_control"}),
    }
}

pub fn optimization(data: &Path, agent: Option<&str>, enabled: bool, dry: bool) -> Result<Value, String> {
    if !data.is_absolute() { return Err("select an absolute AgentQ installation root".into()); }
    if agent.is_some_and(|a| !onboard::AGENTS.contains(&a)) { return Err("unsupported optimization agent".into()); }
    let _lock = if dry { None } else { Some(executor::lock(data)?) };
    let _ = state(data)?;
    let mut c = config(data)?;
    let old = c["compression"].clone();
    if c["compression"].is_null() { c["compression"] = json!({}); }
    if let Some(a) = agent {
        if c["compression"]["agents"].is_null() { c["compression"]["agents"] = json!({}); }
        let agents = c["compression"]["agents"].as_object_mut().ok_or("invalid agent preferences")?;
        if enabled { agents.shift_remove(a); } else { agents.insert(a.into(), false.into()); }
    } else {
        c["compression"]["enabled"] = enabled.into();
    }
    let blocked = disabled(data, agent.unwrap_or("unknown"))?;
    let effective = enabled && c["compression"]["enabled"] != false && !blocked;
    let result = json!({"scope": agent.unwrap_or("host"), "requested": if enabled { "on" } else { "off" },
        "effective": if effective { "on" } else { "off" }, "reason": if blocked { "integration_disabled" } else if enabled && !effective { "host_pause" } else { "requested" },
        "outcome": if dry { "planned" } else if old == c["compression"] { "unchanged" } else { "applied" }, "reload": false});
    if !dry { save(data, "config.json", &c)?; }
    Ok(result)
}

pub fn wants(c: &profiles::Client, data: &Path, exe: &Path) -> Vec<onboard::Want> {
    let claude = c.files.get(1).cloned().unwrap_or_default();
    let mut w = onboard::wants(c.agent, &c.root, &claude, exe);
    w.extend(onboard::routing_wants(c.agent, &c.root, routing::port(data, c.agent).unwrap_or(0)));
    let mut entries = onboard::load(&data.join("owned.json"));
    if let Ok(s) = state(data) { entries.extend(s["profiles"][c.agent]["entries"].as_array().into_iter().flatten().cloned()); }
    w.extend(onboard::instruction_wants(c.agent, &c.root, &entries));
    w
}

fn record_scope(c: &profiles::Client, r: &Value, data: &Path, exe: &Path) -> Result<(), String> {
    if r["root"] != c.root.display().to_string() { return Err("profile_root_changed: select the recorded native configuration root".into()); }
    if !c.root.is_dir() { return Err("profile_root_missing: select the recorded native configuration root".into()); }
    let entries = r["entries"].as_array().ok_or("invalid saved entries")?;
    let _ = onboard::saved_wants(wants(c, data, exe), entries)?;
    Ok(())
}

fn enable_wants(h: &executor::Host, c: &profiles::Client, r: &Value) -> Result<Vec<onboard::Want>, String> {
    record_scope(c, r, &h.data, &h.exe)?;
    let entries = r["entries"].as_array().ok_or("invalid saved entries")?;
    let current = onboard::capture(&wants(c, &h.data, &h.exe), &h.data.join("owned.json"))?;
    if entries.iter().any(|e| onboard::detached(e) != Ok(true) && !current.iter().any(|n| n["file"] == e["file"] && n["key"] == e["key"])) {
        return Err("owned value changed".into());
    }
    let direct = r["direct"] == true || config(&h.data)?["direct"].as_array().is_some_and(|a| a.iter().any(|a| a == c.agent));
    let fields: Vec<_> = entries.iter().filter(|e| !(direct && e["field"] == "routing")).cloned().collect();
    onboard::saved_wants(wants(c, &h.data, &h.exe), &fields)
}

fn next_generation(s: &mut Value) -> Result<u64, String> {
    let n = s["generation"].as_u64().and_then(|n| n.checked_add(1)).ok_or("control generation exhausted")?;
    s["generation"] = n.into();
    Ok(n)
}

/// One reversible operation. Service preparation is outside the lifecycle lock and before routing is restored.
pub fn integrations(
    h: &executor::Host, target: Option<&str>, enabled: bool, dry: bool, service: Value,
    ready: &mut dyn FnMut(&[u16], bool) -> Result<(), String>,
) -> Result<Value, String> {
    if !h.data.is_absolute() { return Err("select an absolute AgentQ installation root".into()); }
    if target.is_some_and(|a| !AGENTS.contains(&a)) { return Err("unknown integration agent".into()); }
    let lock = if dry { None } else { Some(executor::lock(&h.data)?) };
    let settings = config(&h.data)?;
    let mut s = state(&h.data)?;
    if s["service"]["state"] == "stopping" { return Err("service transition pending; inspect agentq status".into()); }
    if s["service"].is_null() || (!enabled && s["host_disabled"] != true && (target.is_none() || s["profiles"].as_object().is_some_and(|p| p.is_empty()))) {
        s["service"] = service.clone();
    }
    let mut selected = vec![];
    for c in h.clients.iter().filter(|c| target.is_none_or(|a| c.agent == a)) {
        if let Some(r) = s["profiles"].get(c.agent) {
            record_scope(c, r, &h.data, &h.exe)?;
            selected.push(c);
        } else if !enabled {
            let entries = onboard::capture(&wants(c, &h.data, &h.exe), &h.data.join("owned.json"))?;
            let _ = onboard::saved_wants(wants(c, &h.data, &h.exe), &entries)?;
            if onboard::unowned_present(&wants(c, &h.data, &h.exe), &entries)? { return Err("integration has an unowned registration; kept".into()); }
            if entries.is_empty() { continue; }
            if !c.root.is_absolute() || !c.root.is_dir() { return Err("select the existing absolute native configuration root".into()); }
            s["profiles"][c.agent] = json!({"root": c.root, "phase": "detaching", "at": crate::now(), "runtime": "pending_reload",
                "entries": entries, "direct": settings["direct"].as_array().is_some_and(|a| a.iter().any(|a| a == c.agent))});
            selected.push(c);
        }
    }
    if target.is_some() && selected.is_empty() {
        return Ok(json!({"outcome": "unchanged", "profiles": [], "reason": "no_saved_integration"}));
    }
    if target.is_none() && !enabled {
        s["host_disabled"] = true.into();
        if s["service"].is_null() { s["service"] = service; }
    }
    if enabled && selected.is_empty() && s["host_disabled"] != true {
        return Ok(json!({"outcome": "unchanged", "profiles": []}));
    }
    let preview: Vec<Value> = selected.iter().map(|c| {
        let aliases = profiles::KINDS.iter().find(|k| k.0 == c.kind).map(|k| k.2).unwrap_or(&[]);
        let fields: Vec<_> = s["profiles"][c.agent]["entries"].as_array().into_iter().flatten()
            .map(|e| json!({"field": e["field"], "file": e["file"], "key": e["key"]})).collect();
        let conflict = if !enabled { onboard::detach(&h.data.join("owned.json"), s["profiles"][c.agent]["entries"].as_array().expect("validated entries"), true).is_err() }
            else { enable_wants(h, c, &s["profiles"][c.agent]).and_then(|ws| onboard::preflight(&ws)).is_err() };
        json!({"agent": c.agent, "aliases": aliases, "changes": fields, "conflict": conflict, "reload": true})
    }).collect();
    if dry { return Ok(json!({"outcome": "planned", "action": if enabled { "enable" } else { "disable" }, "blocked": preview.iter().any(|p| p["conflict"] == true), "host": target.is_none(), "profiles": preview})); }
    let generation = next_generation(&mut s)?;
    let mut ports = vec![];
    if enabled {
        for c in &selected {
            let r = &mut s["profiles"][c.agent];
            r["phase"] = "enabling".into();
        }
    }
    save(&h.data, "integration.json", &s)?;
    if enabled {
        for c in &selected {
            let r = &s["profiles"][c.agent];
            if r["direct"] != true && r["entries"].as_array().is_some_and(|es| es.iter().any(|e| e["field"] == "routing")) {
                ports.push(routing::fix_port(&h.data, c.agent)?);
            }
        }
        if s["service"]["registered"] == true && s["service"]["loaded"].is_null() && s["service"]["active"] != true {
            return Err("saved service activation state unavailable".into());
        }
        let start = s["service"]["active"] == true || s["service"]["loaded"] == true || !ports.is_empty();
        drop(lock);
        let readiness = ready(&ports, start);
        let _lock = executor::lock(&h.data)?;
        s = state(&h.data)?;
        if s["generation"] != generation { return Err("integration operation superseded".into()); }
        if let Err(e) = readiness {
            return Ok(json!({"outcome": "unknown", "reason": "not_answering", "detail": e, "profiles": preview}));
        }
        let mut results = vec![];
        for c in selected {
            let r = s["profiles"][c.agent].clone();
            let result = enable_wants(h, c, &r).and_then(|w| {
                onboard::apply(&w, &h.data.join("owned.json")).and_then(|lines| {
                    if lines.iter().any(|l| l.starts_with("conflict:")) { Err("owned value changed".into()) }
                    else if onboard::present(&w)? { Ok(()) } else { Err("enable readback mismatch".into()) }
                })
            });
            if result.is_ok() {
                s["profiles"].as_object_mut().ok_or("invalid profiles")?.shift_remove(c.agent);
                // An explicit per-agent enable resumes that integration, leaving siblings disabled.
                s["host_disabled"] = false.into();
            } else {
                s["profiles"][c.agent]["phase"] = "conflict".into();
            }
            results.push(json!({"agent": c.agent, "outcome": if result.is_ok() { "applied" } else { "conflict" }, "reload": true}));
            save(&h.data, "integration.json", &s)?;
        }
        if target.is_none() && s["profiles"].as_object().is_some_and(|p| p.is_empty()) {
            s["host_disabled"] = false.into();
            save(&h.data, "integration.json", &s)?;
        }
        let all = results.iter().all(|r| r["outcome"] == "applied");
        return Ok(json!({"outcome": if all { "applied" } else { "partial" }, "profiles": results}));
    }
    let mut results = vec![];
    for c in selected {
        let entries = s["profiles"][c.agent]["entries"].as_array().ok_or("invalid saved entries")?;
        let result = onboard::detach(&h.data.join("owned.json"), entries, false);
        s["profiles"][c.agent]["phase"] = if result.is_ok() { "detached" } else { "conflict" }.into();
        s["profiles"][c.agent]["at"] = crate::now().into();
        results.push(json!({"agent": c.agent, "outcome": if result.is_ok() { "configured" } else { "conflict" }, "runtime": "pending_reload"}));
        save(&h.data, "integration.json", &s)?;
    }
    let all = results.iter().all(|r| r["outcome"] == "configured");
    Ok(json!({"outcome": if all { "applied" } else { "partial" }, "profiles": results, "host": target.is_none(), "runtime": "pending_reload"}))
}

/// Native executable inventory, without command arguments or environment values.
pub fn dependencies(ignore: &[u32]) -> Result<std::collections::BTreeMap<String, usize>, String> {
    #[cfg(unix)]
    let output = crate::host::run(std::process::Command::new("ps").args(["-axo", "pid=,ppid=,comm="])).map_err(|_| "process inventory unavailable")?;
    #[cfg(windows)]
    let output = crate::host::run(std::process::Command::new("powershell").args(["-NoProfile", "-Command",
        "[Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); Get-CimInstance Win32_Process | ForEach-Object { '{0} {1} {2}' -f $_.ProcessId,$_.ParentProcessId,$_.Name }"])).map_err(|_| "process inventory unavailable")?;
    let listing = std::str::from_utf8(&output.stdout).map_err(|_| "process inventory unreadable")?;
    inventory(listing, ignore)
}

fn inventory(listing: &str, ignore: &[u32]) -> Result<std::collections::BTreeMap<String, usize>, String> {
    if listing.trim().is_empty() { return Err("empty process inventory".into()); }
    let mut counts = std::collections::BTreeMap::new();
    for line in listing.lines().filter(|l| !l.trim().is_empty()) {
        let (pid, rest) = line.trim().split_once(char::is_whitespace).ok_or("malformed process inventory")?;
        let pid = pid.parse::<u32>().map_err(|_| "malformed process inventory")?;
        let (parent, name) = rest.trim().split_once(char::is_whitespace).ok_or("malformed process inventory")?;
        let parent = parent.parse::<u32>().map_err(|_| "malformed process inventory")?;
        if ignore.contains(&pid) || pid == std::process::id() { continue; }
        let name = name.trim().to_ascii_lowercase();
        let base = name.rsplit(['/', '\\']).next().unwrap_or("").trim_end_matches(".exe");
        // A generic interpreter can hold a Hermes/CLI session; only service children have known ownership.
        if (base == "node" || base == "nodejs" || base == "electron" || base.starts_with("python")) && !ignore.contains(&parent) {
            return Err("client identity unavailable for an interpreter process".into());
        }
        let agent = if base == "codex" || name.contains("/codex.app/") || base.starts_with("codex helper")
            || base == "chatgpt" || name.contains("/chatgpt.app/") || base.starts_with("chatgpt helper") { Some("codex") }
            else if base == "claude" || name.contains("/claude.app/") || base.starts_with("claude helper") { Some("claude") }
            else if base == "hermes" { Some("hermes") }
            else if base == "agentq" { Some("agentq") }
            else if ["launchctl", "systemctl", "schtasks"].contains(&base) { Some("controllers") }
            else { None };
        if let Some(a) = agent { *counts.entry(a.into()).or_insert(0) += 1; }
    }
    Ok(counts)
}

/// Observe dependencies and persist pending/drained state under the same lock as configuration writers.
pub fn drain(data: &Path, dependencies: Result<std::collections::BTreeMap<String, usize>, String>, connections: Result<usize, String>) -> Result<bool, String> {
    let _lock = executor::lock(data)?;
    let mut s = state(data)?;
    if s["service"]["state"] == "stopping" { return Ok(false); }
    let before = s.clone();
    let mut safe = dependencies.as_ref().is_ok_and(|d| d.get("controllers").copied().unwrap_or(0) == 0) && connections == Ok(0);
    for (agent, r) in s["profiles"].as_object_mut().ok_or("invalid profiles")? {
        let routed = r["entries"].as_array().is_some_and(|es| es.iter().any(|e| e["field"] == "routing"));
        let runtime = match &dependencies {
            Ok(counts) if r["phase"] == "detached" => {
                let a = if agent == "claude-desktop" { "claude" } else { agent.as_str() };
                if counts.get(a).copied().unwrap_or(0) + counts.get("agentq").copied().unwrap_or(0) > 0 {
                    if routed { "pending_drain" } else { "pending_reload" }
                } else if routed && connections != Ok(0) { "pending_drain" } else { "drained" }
            }
            _ => "unknown",
        };
        safe &= runtime == "drained";
        r["runtime"] = runtime.into();
        r["runtime_at"] = crate::now().into();
    }
    if s["host_disabled"] == true {
        s["service"]["state"] = if safe { "ready_to_stop" } else { "pending_drain" }.into();
    }
    if s != before { save(data, "integration.json", &s)?; }
    Ok(s["host_disabled"] == true && safe)
}

/// An old port stays available until its native dependencies and connections are known to have drained.
pub fn transport(data: &Path, agent: &str) -> bool {
    match state(data) {
        Ok(s) => match s["profiles"].get(agent) {
            Some(r) => r["phase"] != "detached" || r["runtime"] != "drained",
            None => s["host_disabled"] != true,
        },
        // Unknown state cannot justify taking away an existing client's provider.
        Err(_) => true,
    }
}

pub fn observation(data: &Path, c: &profiles::Client, exe: &Path) -> Value {
    let mut out = intent(data, c.agent);
    if let Ok(s) = state(data) {
        if let Some(r) = s["profiles"].get(c.agent) {
            if record_scope(c, r, data, exe).is_err() {
                out["configuration"] = "unknown".into();
                out["reason"] = "profile_scope_changed".into();
            } else {
                let reads: Vec<_> = r["entries"].as_array().into_iter().flatten().map(onboard::detached).collect();
                out["configuration"] = if reads.iter().all(|r| *r == Ok(true)) { "detached" }
                    else if reads.iter().all(|r| *r == Ok(false)) { "attached" } else { "unknown" }.into();
                if reads.iter().any(Result::is_err) { out["reason"] = "owned_value_changed".into(); }
            }
        } else {
            let ws = wants(c, data, exe);
            match onboard::capture(&ws, &data.join("owned.json")) {
                Ok(entries) if !entries.is_empty() => {
                    out["configuration"] = if entries.iter().all(|e| onboard::detached(e) == Ok(false)) { "attached" } else { "unknown" }.into();
                }
                Ok(entries) => match onboard::unowned_present(&ws, &entries) {
                    Ok(false) => { out["configuration"] = "detached".into(); out["reason"] = "not_onboarded".into(); }
                    _ => { out["configuration"] = "unknown".into(); out["reason"] = "unowned_registration".into(); }
                },
                Err(_) => { out["configuration"] = "unknown".into(); out["reason"] = "unreadable_ownership".into(); }
            }
        }
    }
    out
}

pub fn summary(values: &[&str]) -> &'static str {
    if values.is_empty() || values.contains(&"unknown") { "unknown" }
    else if values.iter().all(|s| *s == "on") { "on" }
    else if values.iter().all(|s| *s == "off") { "off" }
    else { "mixed" }
}

/// The owner can die during an OS stop. Only a fresh inventory, after its controller exits,
/// may reconcile the durable stop receipt; this never clears disable intent.
pub fn reconcile_stop(data: &Path, owner: u32, owner_alive: bool, service_alive: bool, controllers: bool) -> Result<(), String> {
    if owner_alive || controllers { return Ok(()); }
    let _lock = executor::lock(data)?;
    let mut s = state(data)?;
    if s["service"]["state"] == "stopping" && s["service"]["owner"].as_u64() == Some(u64::from(owner)) {
        s["service"]["state"] = if service_alive { "pending_drain" } else { "stopped" }.into();
        save(data, "integration.json", &s)?;
    }
    Ok(())
}

pub fn process_alive(pid: u32) -> Result<bool, String> {
    if pid == 0 || pid > i32::MAX as u32 { return Err("invalid process identity".into()); }
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as i32, 0) } == 0 { return Ok(true); }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ESRCH) => Ok(false), Some(libc::EPERM) => Ok(true), _ => Err("process identity unavailable".into()),
        }
    }
    #[cfg(windows)]
    {
        let query = format!("if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ 'alive' }} else {{ 'gone' }}");
        let output = crate::host::run(std::process::Command::new("powershell").args(["-NoProfile", "-Command", &query])).map_err(|_| "process identity unavailable")?;
        match String::from_utf8_lossy(&output.stdout).trim() {
            "alive" => Ok(true), "gone" => Ok(false), _ => Err("process identity unavailable".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_controls_preserve_scopes_and_reject_corruption() {
        let d = std::env::temp_dir().join(format!("agentq-control-{}", crate::mcp::new_id()));
        assert_eq!(optimization(&d, None, false, true).unwrap()["outcome"], "planned");
        assert!(!d.exists(), "dry-run creates no key, lock, directory or settings");
        optimization(&d, Some("claude"), false, false).unwrap();
        optimization(&d, None, false, false).unwrap();
        assert_eq!(optimization(&d, Some("codex"), true, false).unwrap()["reason"], "host_pause");
        optimization(&d, None, true, false).unwrap();
        let c = config(&d).unwrap();
        assert_eq!(c["compression"]["agents"]["claude"], false);
        assert!(c["compression"]["agents"]["codex"].is_null());
        std::fs::write(d.join("config.json"), "broken").unwrap();
        assert!(optimization(&d, None, true, false).is_err());
        assert_eq!(std::fs::read_to_string(d.join("config.json")).unwrap(), "broken");
        std::fs::write(d.join("integration.json"), "{}").unwrap();
        assert!(disabled(&d, "codex").is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    fn host(name: &str, routed: bool) -> executor::Host {
        let root = std::env::temp_dir().join(format!("agentq-control-{name}-{}", crate::mcp::new_id()));
        let data = root.join("install/data");
        std::fs::create_dir_all(&data).unwrap();
        let c = |kind, agent, folder: &str, files: &[&str]| {
            let r = root.join(folder);
            std::fs::create_dir_all(&r).unwrap();
            profiles::Client { kind, agent, root: r.clone(), files: files.iter().map(|f| r.join(f)).collect() }
        };
        let clients = vec![c("codex", "codex", "codex", &["config.toml", "hooks.json"]),
            c("claude-code", "claude", "claude", &["settings.json", ".claude.json"]),
            c("claude-desktop-chat", "claude-desktop", "desktop", &["claude_desktop_config.json"]),
            c("hermes", "hermes", "hermes", &["config.yaml"])];
        std::fs::write(&clients[0].files[0], "model = \"native-model\"\n").unwrap();
        std::fs::write(&clients[1].files[0], r#"{"model":"native-model","permissions":{"defaultMode":"default"}}"#).unwrap();
        std::fs::write(&clients[3].files[0], "model: native-model\nplugins: {}\n").unwrap();
        save(&data, "config.json", &json!({"ports": {"codex": 41001}, "compression": {"agents": {"claude": false}},
            "direct": if routed { vec![] } else { vec!["codex"] }})).unwrap();
        let h = executor::Host { data, exe: root.join("install/agentq"), clients, installation: "fixture".into(), generation: 0, grant: Value::Null };
        let mut all = vec![];
        for c in &h.clients {
            let mut ws = wants(c, &h.data, &h.exe);
            if !routed && c.agent == "codex" { ws.retain(|w| w.field() != "routing"); }
            all.extend(ws);
        }
        let lines = onboard::apply(&all, &h.data.join("owned.json")).unwrap();
        assert!(!lines.iter().any(|l| l.starts_with("conflict:")), "{lines:?}");
        let _ = profiles::status_with(&h.clients, &h.data, 1000);
        h
    }

    fn change(h: &executor::Host, agent: Option<&str>, on: bool) -> Value {
        integrations(h, agent, on, false, json!({"registered": false, "active": false}), &mut |_, _| Ok(())).unwrap()
    }

    #[test]
    fn reversible_controls_preserve_native_settings_foreign_siblings_and_baseline() {
        for routed in [false, true] {
            let h = host("roundtrip", routed);
            // Add foreign siblings after the old parent-shaped ownership receipts were written.
            let path = &h.clients[1].files[1];
            let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            doc["mcpServers"]["foreign"] = json!({"command": "foreign-tool"});
            std::fs::write(path, doc.to_string()).unwrap();
            let hermes = &h.clients[3].files[0];
            let text = std::fs::read_to_string(hermes).unwrap() + "  foreign:\n    command: foreign-tool\n";
            std::fs::write(hermes, text).unwrap();
            assert_eq!(change(&h, None, false)["outcome"], "applied");
            assert!(disabled(&h.data, "unknown").unwrap());
            let entries = state(&h.data).unwrap()["profiles"]["codex"]["entries"].clone();
            assert_eq!(change(&h, None, false)["outcome"], "applied");
            assert_eq!(state(&h.data).unwrap()["profiles"]["codex"]["entries"], entries);
            assert!(onboard::load(&h.data.join("owned.json")).is_empty());
            let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
            assert_eq!(doc["mcpServers"]["foreign"]["command"], "foreign-tool");
            let text = std::fs::read_to_string(hermes).unwrap();
            assert!(text.contains("mcp_servers:\n  foreign:\n"), "{text}");
            assert!(!text.contains("agentq:"));
            let mut prepared = false;
            let r = integrations(&h, None, true, false, json!({}), &mut |ports, _| {
                prepared = true;
                assert_eq!(ports.is_empty(), !routed);
                assert!(!std::fs::read_to_string(&h.clients[0].files[0]).unwrap().lines().any(|l| l.starts_with("model_provider =")));
                Ok(())
            }).unwrap();
            assert!(prepared);
            assert_eq!(r["outcome"], "applied", "{r}");
            assert!(!disabled(&h.data, "unknown").unwrap());
            assert_eq!(config(&h.data).unwrap()["compression"]["agents"]["claude"], false);
            let text = std::fs::read_to_string(&h.clients[0].files[0]).unwrap();
            assert_eq!(text.lines().any(|l| l.starts_with("model_provider =")), routed);
            assert!(text.contains("native-model"));
            let settings: Value = serde_json::from_str(&std::fs::read_to_string(&h.clients[1].files[0]).unwrap()).unwrap();
            assert_eq!(settings["permissions"]["defaultMode"], "default");
            assert_eq!(change(&h, None, true)["outcome"], "unchanged");
            std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn repeated_disable_repairs_codex_provider_removed_by_older_releases() {
        let h = host("provider-repair", true);
        assert_eq!(change(&h, Some("codex"), false)["outcome"], "applied");
        let path = &h.clients[0].files[0];
        let snapshot = state(&h.data).unwrap()["profiles"]["codex"]["entries"].clone();
        for text in [Some("model = \"native-model\"\n"), None] {
            if let Some(text) = text { std::fs::write(path, text).unwrap(); }
            else { std::fs::remove_file(path).unwrap(); }
            assert_eq!(change(&h, Some("codex"), false)["outcome"], "applied");
            let repaired = std::fs::read_to_string(path).unwrap();
            assert!(repaired.contains(onboard::CODEX_DIRECT_PROVIDER));
            assert!(!repaired.contains("base_url"));
            assert_eq!(state(&h.data).unwrap()["profiles"]["codex"]["entries"], snapshot);
        }
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn duplicate_legacy_receipts_detach_once_and_restore_canonical_fields() {
        let h = host("duplicate-receipts", false);
        let journal = h.data.join("owned.json");
        let mut entries = onboard::load(&journal);
        let duplicates: Vec<_> = entries.iter().filter(|e| e["file"] == h.clients[0].files[0].display().to_string()
            || e["file"] == h.clients[1].files[1].display().to_string()).cloned().map(|mut e| {
                e["orig"] = "{\"historical_context\":true}".into();
                e
            }).collect();
        assert_eq!(duplicates.len(), 2);
        entries.extend(duplicates);
        std::fs::write(&journal, serde_json::to_string(&entries).unwrap()).unwrap();
        let preview = integrations(&h, None, false, true, json!({}), &mut |_, _| panic!("dry-run starts no service")).unwrap();
        assert_eq!(preview["blocked"], false, "duplicate owned receipts are not foreign edits");
        assert_eq!(change(&h, None, false)["outcome"], "applied");
        assert!(onboard::load(&journal).is_empty(), "all duplicate source receipts must be suspended");
        assert_eq!(change(&h, None, true)["outcome"], "applied");
        assert_eq!(change(&h, None, false)["outcome"], "applied");
        let c = &h.clients[1];
        std::fs::write(&c.files[1], r#"{"mcpServers":{"agentq":{"command":"foreign-tool"}}}"#).unwrap();
        let foreign = std::fs::read(&c.files[1]).unwrap();
        assert_eq!(change(&h, Some("claude"), true)["outcome"], "partial");
        assert_eq!(std::fs::read(&c.files[1]).unwrap(), foreign);
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn interrupted_detach_and_enable_reconcile_without_replacing_foreign_values() {
        let h = host("crash", false);
        let codex = std::fs::read(&h.clients[0].files[0]).unwrap();
        let journal = std::fs::read(h.data.join("owned.json")).unwrap();
        assert_eq!(change(&h, Some("claude"), false)["outcome"], "applied");
        assert_eq!(std::fs::read(&h.clients[0].files[0]).unwrap(), codex);
        // Crash after native detach and before forgetting active receipts.
        std::fs::write(h.data.join("owned.json"), journal).unwrap();
        let mut s = state(&h.data).unwrap();
        s["profiles"]["claude"]["phase"] = "detaching".into();
        save(&h.data, "integration.json", &s).unwrap();
        assert_eq!(change(&h, Some("claude"), false)["outcome"], "applied");
        let r = state(&h.data).unwrap()["profiles"]["claude"].clone();
        let ws = onboard::saved_wants(wants(&h.clients[1], &h.data, &h.exe), r["entries"].as_array().unwrap()).unwrap();
        // Crash after canonical reapply and before clearing disable intent.
        s = state(&h.data).unwrap();
        s["profiles"]["claude"]["phase"] = "enabling".into();
        save(&h.data, "integration.json", &s).unwrap();
        onboard::apply(&ws, &h.data.join("owned.json")).unwrap();
        let n = onboard::load(&h.data.join("owned.json")).len();
        assert_eq!(change(&h, Some("claude"), true)["outcome"], "applied");
        assert_eq!(onboard::load(&h.data.join("owned.json")).len(), n);
        change(&h, Some("claude"), false);
        let path = &h.clients[1].files[1];
        let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        doc["mcpServers"]["agentq"] = json!({"command": "foreign-tool"});
        std::fs::write(path, doc.to_string()).unwrap();
        let before = std::fs::read(path).unwrap();
        assert_eq!(change(&h, Some("claude"), true)["outcome"], "partial");
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert!(disabled(&h.data, "claude").unwrap());
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn drain_needs_native_inventory_and_empty_connections_not_elapsed_time() {
        let h = host("drain", true);
        change(&h, None, false);
        let old = inventory("123 1 /Applications/Codex.app/Contents/MacOS/Codex\n456 1 /bin/claude\n", &[]).unwrap();
        assert!(!drain(&h.data, Ok(old), Ok(0)).unwrap());
        assert_eq!(state(&h.data).unwrap()["profiles"]["codex"]["runtime"], "pending_drain");
        assert!(transport(&h.data, "codex"));
        let desktop = inventory("321 1 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT\n", &[]).unwrap();
        assert!(!drain(&h.data, Ok(desktop), Ok(0)).unwrap(), "the current Codex Desktop executable is named ChatGPT and may retain its provider");
        assert!(!drain(&h.data, Err("unsupported".into()), Ok(0)).unwrap());
        assert!(inventory("789 1 /usr/bin/python3\n", &[]).is_err(), "Hermes can run as a generic Python process");
        assert!(inventory("789 456 /private/engine/python\n", &[456]).is_ok(), "the service owns its proxy child");
        let empty = inventory("1 0 /sbin/init\n", &[]).unwrap();
        assert!(!drain(&h.data, Ok(empty.clone()), Err("unreadable".into())).unwrap());
        assert!(drain(&h.data, Ok(empty), Ok(0)).unwrap());
        assert!(!transport(&h.data, "codex"));
        let mut s = state(&h.data).unwrap();
        for phase in ["detaching", "conflict", "enabling"] {
            s["profiles"]["codex"]["phase"] = phase.into();
            save(&h.data, "integration.json", &s).unwrap();
            assert!(transport(&h.data, "codex"), "an unfinished {phase} must retain the old transport");
        }
        s["service"]["state"] = "stopped".into();
        save(&h.data, "integration.json", &s).unwrap();
        assert!(host_disabled(&h.data).unwrap(), "a stopped service does not clear intent");
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn dry_run_revisions_and_scope_checks_protect_native_configuration() {
        let h = host("revision", false);
        let native = std::fs::read(&h.clients[0].files[0]).unwrap();
        let journal = std::fs::read(h.data.join("owned.json")).unwrap();
        let r = integrations(&h, None, false, true, json!({}), &mut |_, _| panic!("dry-run starts no service")).unwrap();
        assert_eq!(r["outcome"], "planned");
        assert!(!h.data.join("integration.json").exists());
        assert_eq!(std::fs::read(h.data.join("owned.json")).unwrap(), journal);
        let first = profiles::status_with(&h.clients, &h.data, 1000)["revision"].clone();
        optimization(&h.data, None, false, false).unwrap();
        let second = profiles::status_with(&h.clients, &h.data, 1000)["revision"].clone();
        assert_ne!(first, second, "host policy invalidates a confirmed plan");
        routing::set_direct(&h.data, &["codex"], false).unwrap();
        assert_ne!(second, profiles::status_with(&h.clients, &h.data, 1000)["revision"]);
        assert_eq!(std::fs::read(&h.clients[0].files[0]).unwrap(), native);
        change(&h, Some("claude"), false);
        let projection = profiles::status_with(&h.clients, &h.data, 1000);
        let id = projection["profiles"].as_array().unwrap().iter().find(|p| p["kind"] == "claude-code").unwrap()["id"].clone();
        let plan = crate::configure::plan(&json!({"changes": [{"profile": id, "field": "hooks", "value": "registered"}]}), &projection);
        assert!(plan["blocked"].as_array().unwrap().contains(&json!("integration_disabled")));
        let outside = h.data.parent().unwrap().parent().unwrap().join("outside.json");
        std::fs::write(&outside, "leave intact").unwrap();
        let mut s = state(&h.data).unwrap();
        s["profiles"]["claude"]["entries"][0]["file"] = json!(outside);
        save(&h.data, "integration.json", &s).unwrap();
        assert!(integrations(&h, Some("claude"), true, false, json!({}), &mut |_, _| panic!("invalid scope starts no service")).is_err());
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "leave intact");
        assert_eq!(summary(&["on", "off"]), "mixed");
        assert_eq!(summary(&["off", "unknown"]), "unknown");
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn owned_instructions_originals_and_interrupted_service_stop_survive_roundtrip() {
        let h = host("instructions", false);
        let path = h.clients[0].root.join("AGENTS.md");
        let after = format!("\n\n{}\n", crate::mcp::INSTRUCTIONS);
        std::fs::write(&path, format!("Foreign instructions.{after}")).unwrap();
        let mut journal = onboard::load(&h.data.join("owned.json"));
        journal.push(json!({"file": path, "key": ["instruction_block"], "kind": "text", "after": after, "before": null}));
        save(&h.data, "owned.json", &json!(journal)).unwrap();
        let reference_file = h.data.join("reference.txt");
        std::fs::write(&reference_file, "original fixture bytes\n").unwrap();
        let dir = h.data.join("originals");
        let original = crate::mcp::call(&dir, "index_document", &json!({"path": reference_file})).unwrap();
        let grant = h.data.join("connection");
        std::fs::create_dir_all(&grant).unwrap();
        std::fs::write(grant.join("fixture-grant"), "invented observation grant").unwrap();
        assert_eq!(change(&h, Some("codex"), false)["outcome"], "applied");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Foreign instructions.");
        assert_eq!(change(&h, Some("codex"), true)["outcome"], "applied");
        assert!(std::fs::read_to_string(path).unwrap().contains(crate::mcp::INSTRUCTIONS));
        change(&h, None, false);
        let mut s = state(&h.data).unwrap();
        s["service"]["state"] = "stopping".into();
        s["service"]["owner"] = 12345.into();
        save(&h.data, "integration.json", &s).unwrap();
        reconcile_stop(&h.data, 12345, true, false, false).unwrap();
        assert_eq!(state(&h.data).unwrap()["service"]["state"], "stopping");
        assert!(integrations(&h, None, true, false, json!({}), &mut |_, _| Ok(())).is_err());
        reconcile_stop(&h.data, 12345, false, false, true).unwrap();
        assert_eq!(state(&h.data).unwrap()["service"]["state"], "stopping");
        reconcile_stop(&h.data, 12345, false, false, false).unwrap();
        assert_eq!(state(&h.data).unwrap()["service"]["state"], "stopped");
        assert!(host_disabled(&h.data).unwrap());
        let recovered = crate::mcp::call(&dir, "get_original", &json!({"ref": original["ref"]})).unwrap();
        assert_eq!(recovered["content"], "original fixture bytes\n");
        assert_eq!(std::fs::read_to_string(grant.join("fixture-grant")).unwrap(), "invented observation grant");
        assert_eq!(change(&h, None, true)["outcome"], "applied");
        std::fs::remove_dir_all(h.data.parent().unwrap().parent().unwrap()).unwrap();
    }
}
