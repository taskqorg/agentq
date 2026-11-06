//! Local temporary controls (agentq.md §4): compression preferences and durable integration intent.
use crate::{executor, onboard, profiles};
use serde_json::{Value, json};
use std::path::Path;

pub use agentq_runtime::optimization::{AGENTS, config, state};

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

pub fn wants(c: &profiles::Client, data: &Path, exe: &Path) -> Vec<onboard::Want> {
    let claude = c.files.get(1).cloned().unwrap_or_default();
    let mut w = onboard::wants(c.agent, &c.root, &claude, exe);
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
    if entries.iter().filter(|e| !onboard::retired_entry(e)).any(|e| onboard::detached(e) != Ok(true) && !current.iter().any(|n| n["file"] == e["file"] && n["key"] == e["key"])) {
        return Err("owned value changed".into());
    }
    let direct = r["direct"] == true || config(&h.data)?["direct"].as_array().is_some_and(|a| a.iter().any(|a| a == c.agent));
    let fields: Vec<_> = entries.iter().filter(|e| !onboard::retired_entry(e) && !(direct && e["field"] == "routing")).cloned().collect();
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
    let ports = vec![];
    if enabled {
        for c in &selected {
            let r = &mut s["profiles"][c.agent];
            r["phase"] = "enabling".into();
        }
    }
    save(&h.data, "integration.json", &s)?;
    if enabled {
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
            else if ["agy", "agentapi", "antigravity"].contains(&base) || name.contains("/antigravity.app/") { Some("antigravity") }
            else if base == "agentq" { Some("agentq") }
            else if ["launchctl", "systemctl", "schtasks"].contains(&base) { Some("controllers") }
            else { None };
        if let Some(a) = agent { *counts.entry(a.into()).or_insert(0) += 1; }
    }
    Ok(counts)
}

/// Observe dependencies and persist pending/drained state under the same lock as configuration writers.
pub fn settle(data: &Path, dependencies: Result<std::collections::BTreeMap<String, usize>, String>) -> Result<bool, String> {
    let _lock = executor::lock(data)?;
    let mut s = state(data)?;
    if s["service"]["state"] == "stopping" { return Ok(false); }
    let before = s.clone();
    let mut safe = dependencies.as_ref().is_ok_and(|d| d.get("controllers").copied().unwrap_or(0) == 0);
    for (agent, r) in s["profiles"].as_object_mut().ok_or("invalid profiles")? {
        let runtime = match &dependencies {
            Ok(counts) if r["phase"] == "detached" => {
                let a = if agent == "claude-desktop" { "claude" } else { agent.as_str() };
                if counts.get(a).copied().unwrap_or(0) + counts.get("agentq").copied().unwrap_or(0) > 0 {
                    "pending_reload"
                } else { "drained" }
            }
            _ => "unknown",
        };
        safe &= runtime == "drained";
        r["runtime"] = runtime.into();
        r["runtime_at"] = crate::now().into();
    }
    if s["host_disabled"] == true {
        s["service"]["state"] = if safe { "ready_to_stop" } else { "pending_reload" }.into();
    }
    if s != before { save(data, "integration.json", &s)?; }
    Ok(s["host_disabled"] == true && safe)
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
        s["service"]["state"] = if service_alive { "pending_reload" } else { "stopped" }.into();
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
    // M4.1/M4.6/M9.2: record-hook detach/enable never changes native policy, foreign hooks or later edits.
    #[test]
    fn record_hooks_roundtrip_and_preserve_foreign_changes() {
        let root = std::env::temp_dir().join(format!("agentq-record-controls-{}", crate::resource::new_id()));
        let home = root.join("codex");
        std::fs::create_dir_all(&home).unwrap();
        let native = "model = \"native\"\n";
        std::fs::write(home.join("config.toml"), native).unwrap();
        std::fs::write(home.join("hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"foreign"}]}]}}"#).unwrap();
        let h = executor::Host { data: root.join("install/data"), exe: root.join("install/agentq"),
            clients: vec![profiles::Client { kind:"codex", agent:"codex", root:home.clone(), files:vec![home.join("config.toml"),home.join("hooks.json")] }],
            installation:"fixture".into(), generation:0, grant:Value::Null };
        onboard::apply(&wants(&h.clients[0], &h.data, &h.exe), &h.data.join("owned.json")).unwrap();
        let installed = std::fs::read_to_string(home.join("hooks.json")).unwrap();
        let mut ready = |ports: &[u16], _: bool| { assert!(ports.is_empty()); Ok(()) };
        assert_eq!(integrations(&h, Some("codex"), false, true, json!({}), &mut ready).unwrap()["outcome"], "planned");
        assert_eq!(std::fs::read_to_string(home.join("hooks.json")).unwrap(), installed);
        assert_eq!(integrations(&h, Some("codex"), false, false, json!({}), &mut ready).unwrap()["outcome"], "applied");
        assert_eq!(disabled(&h.data,"codex"), Ok(true));
        let detached = std::fs::read_to_string(home.join("hooks.json")).unwrap();
        assert!(detached.contains("foreign") && !detached.contains("hook codex"));
        assert_eq!(integrations(&h, Some("codex"), true, false, json!({}), &mut ready).unwrap()["outcome"], "applied");
        assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), native);
        assert_eq!(std::fs::read_to_string(home.join("hooks.json")).unwrap(), installed);
        std::fs::write(home.join("hooks.json"), installed.replace("hook codex Stop", "later-hook")).unwrap();
        let result = integrations(&h, Some("codex"), false, false, json!({}), &mut ready);
        assert!(result.is_err() || result.unwrap()["outcome"] != "applied");
        assert!(std::fs::read_to_string(home.join("hooks.json")).unwrap().contains("later-hook"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn old_disabled_profile_ignores_retired_scope_and_never_restores_it() {
        let root = std::env::temp_dir().join(format!("agentq-old-disabled-{}", crate::resource::new_id()));
        let home = root.join("codex");
        std::fs::create_dir_all(&home).unwrap();
        let h = executor::Host { data:root.join("install/data"),exe:root.join("install/agentq"),
            clients:vec![profiles::Client {kind:"codex",agent:"codex",root:home.clone(),files:vec![home.join("config.toml"),home.join("hooks.json")]}],
            installation:"fixture".into(),generation:0,grant:Value::Null };
        onboard::apply(&wants(&h.clients[0], &h.data, &h.exe), &h.data.join("owned.json")).unwrap();
        let mut ready = |_: &[u16], _: bool| Ok(());
        integrations(&h, Some("codex"), false, false, json!({}), &mut ready).unwrap();
        let mut s = state(&h.data).unwrap();
        for (field,key,kind,after) in [
            ("routing",json!(["model_provider"]),"text",json!("model_provider = \"agentq\"\n")),
            ("mcp",json!(["mcp_servers","agentq"]),"text",json!("[mcp_servers.agentq]\ncommand = \"agentq\"\n")),
            ("hooks",json!(["hooks","PreToolUse"]),"append",json!({"hooks":[{"command":"agentq hook codex PreToolUse"}]})),
        ] {
            s["profiles"]["codex"]["entries"].as_array_mut().unwrap().push(json!({"file":home.join(if field=="hooks" {"hooks.json"}else{"config.toml"}),"key":key,"field":field,"kind":kind,"after":after,"agent":"codex","source":"0".repeat(64)}));
        }
        save(&h.data, "integration.json", &s).unwrap();
        assert_eq!(integrations(&h,Some("codex"),false,false,json!({}),&mut ready).unwrap()["outcome"], "applied");
        assert_eq!(integrations(&h,Some("codex"),true,false,json!({}),&mut ready).unwrap()["outcome"], "applied");
        assert!(!home.join("config.toml").exists());
        let hooks = std::fs::read_to_string(home.join("hooks.json")).unwrap();
        assert!(hooks.contains("SessionStart") && hooks.contains("Stop") && !hooks.contains("PreToolUse"));
        std::fs::remove_dir_all(root).unwrap();
    }

}
