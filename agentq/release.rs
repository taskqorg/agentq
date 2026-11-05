//! Transactional install and update (agentq.md § 5) and the DashQ release observation, which activates nothing.
use coreq::process::{Failure, output};
use coreq::{Tool, update};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

fn active_files(t: &Tool) -> Vec<String> {
    let files = vec!["current".into(), "previous".into(), t.exe_name()];
    #[cfg(windows)]
    let files = { let mut files = files; files.push(t.gui_exe_name()); files };
    files
}

fn recovery_dir(t: &Tool) -> std::path::PathBuf { t.install.join("data/install-recovery") }

fn copy_synced(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let file = {
        // FlushFileBuffers needs a write handle, even when retaining a read-only original.
        let permissions = std::fs::metadata(from).map_err(|e| e.to_string())?.permissions();
        let mut writable = permissions.clone();
        writable.set_readonly(false);
        std::fs::set_permissions(to, writable).map_err(|e| e.to_string())?;
        let file = std::fs::File::options().write(true).open(to).map_err(|e| e.to_string())?;
        std::fs::set_permissions(to, permissions).map_err(|e| e.to_string())?;
        file
    };
    #[cfg(unix)]
    let file = std::fs::File::open(to).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

/// Flush the rename, including parent directories on Unix; Windows uses the same write-through move as CoreQ.
fn move_durable(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name="kernel32")]
        unsafe extern "system" { fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32; }
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // Both terminated buffers stay live for this synchronous call (replace existing, write through).
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 { return Err(std::io::Error::last_os_error().to_string()); }
    }
    #[cfg(unix)]
    {
        std::fs::rename(from, to).map_err(|e| e.to_string())?;
        for parent in [from.parent().unwrap(), to.parent().unwrap()] {
            std::fs::File::open(parent).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn originals(t: &Tool) -> Result<Value, String> {
    let dir = recovery_dir(t);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut saved = json!({});
    for name in active_files(t) {
        let path = t.install.join(&name);
        let present = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => true,
            Ok(_) => return Err(format!("{name}: active file is not regular; kept")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("{name}: active file unknown: {e}")),
        };
        if present {
            let backup = dir.join(&name);
            copy_synced(&path, &backup)?;
        }
        saved[&name] = json!(present);
    }
    coreq::write_durable(&dir.join("ready"), saved.to_string())?;
    Ok(saved)
}

fn restore(t: &Tool, saved: &Value) -> Result<(), String> {
    let dir = recovery_dir(t);
    if saved.as_object().is_none_or(|m| m.len() != active_files(t).len()) { return Err("invalid install recovery record".into()); }
    for name in active_files(t) {
        let present = saved[&name].as_bool().ok_or("invalid install recovery record")?;
        let path = t.install.join(&name);
        let exists = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => true,
            Ok(_) => return Err(format!("{name}: recovery target is not regular; kept")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("{name}: recovery target unknown: {e}")),
        };
        // A crashed CoreQ write may leave its reserved temporary; serialization proves this operation owns it.
        match std::fs::remove_file(path.with_file_name(format!("{name}.pending"))) {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("{name}: pending recovery: {e}")),
        }
        let backup = dir.join(&name);
        if present {
            if !std::fs::symlink_metadata(&backup).map_err(|e| e.to_string())?.is_file() { return Err("invalid install backup".into()); }
            let staged = dir.join(format!("{name}.restore"));
            copy_synced(&backup, &staged)?;
            let result = move_durable(&staged, &path);
            #[cfg(windows)]
            let result = if result.is_err() && exists {
                // A running Windows launcher can be renamed, never overwritten. Keep it beside CoreQ's old copies.
                let old = (0..).map(|n| t.install.join(format!("{name}.old.{n}"))).find(|p| !p.exists()).unwrap();
                move_durable(&path, &old).and_then(|_| move_durable(&staged, &path))
            } else { result };
            result?;
            if std::fs::read(&path).map_err(|e| e.to_string())? != std::fs::read(&backup).map_err(|e| e.to_string())? { return Err(format!("{name}: recovery readback differs")); }
        } else if exists {
            let removed = dir.join(format!("{name}.removed"));
            move_durable(&path, &removed)?;
            if path.exists() { return Err(format!("{name}: recovery removal readback differs")); }
        }
    }
    Ok(())
}

fn finish(t: &Tool) -> Result<(), String> {
    let dir = recovery_dir(t);
    move_durable(&dir.join("ready"), &dir.join("committed"))?;
    // A committed record never rolls back, even if cleanup is interrupted.
    let _ = std::fs::remove_dir_all(dir);
    Ok(())
}

fn recover_locked(t: &Tool) -> Result<(), String> {
    let dir = recovery_dir(t);
    match std::fs::read(dir.join("ready")) {
        Ok(bytes) => {
            let saved = serde_json::from_slice(&bytes).map_err(|e| format!("install recovery unknown: {e}"))?;
            restore(t, &saved).map_err(|e| format!("install recovery uncertain: {e}"))?;
            finish(t)?;
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(format!("install recovery unknown: {e}")),
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("install recovery cleanup: {e}")),
    }
}

/// Lifecycle startup recovers before the launcher can select a partially activated release; read-only commands stay read-only.
pub fn recover(t: &Tool) -> Result<(), String> {
    match std::fs::symlink_metadata(recovery_dir(t)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("install recovery unknown: {e}")),
        Ok(_) => {},
    }
    let _lock = crate::executor::lock(&t.install.join("data"))?;
    recover_locked(t)
}

fn install_locked(t: &Tool, verified: &[u8], activate: impl FnOnce(&Tool) -> Result<(), String>) -> Result<(), String> {
    update::prepare_install(t, verified)?;
    let saved = originals(t)?;
    if let Err(e) = activate(t).and_then(|_| finish(t)) {
        // Keep the ready record on restoration failure so another start can retry safely.
        coreq::write_durable(&recovery_dir(t).join("ready"), saved.to_string())
            .and_then(|_| restore(t, &saved)).and_then(|_| finish(t))
            .map_err(|recovery| format!("{e}; install recovery uncertain: {recovery}"))?;
        return Err(e);
    }
    Ok(())
}

/// Prepare, then activate (coreq.md R15). The launcher only reads `current`, so the prior release stays active until it moves.
fn install(t: &Tool, verified: &[u8]) -> Result<(), String> {
    install_with(t, verified, update::activate_install)
}

// Keep the activation boundary injectable: even an error after `current` moves must restore the originals.
fn install_with(t: &Tool, verified: &[u8], activate: impl FnOnce(&Tool) -> Result<(), String>) -> Result<(), String> {
    let _lock = crate::executor::lock(&t.install.join("data"))?;
    recover_locked(t)?;
    install_locked(t, verified, activate)
}

pub fn install_self(t: &Tool) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    install(t, &std::fs::read(&me).map_err(|e| format!("{}: {e}", me.display()))?)
}

/// Downloaded files in `dir` to frozen bytes: sha256, then the candidate's own selftest; nothing active changes.
fn verified_candidate(t: &Tool, name: &str, dir: &Path) -> Result<Vec<u8>, String> {
    let exe = dir.join(name);
    let bytes = std::fs::read(&exe).map_err(|e| e.to_string())?;
    update::verify(&std::fs::read(dir.join("SHA256SUMS")).map_err(|e| e.to_string())?, name, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let mut smoke = Command::new(&exe);
    smoke.arg("selftest").env(t.env("INSTALL"), &t.install).env(t.env("SELFTEST_OFFLINE"), "1");
    match output("release.selftest", &mut smoke, Duration::from_secs(30)) {
        Ok(o) if o.status.success() => Ok(bytes),
        Ok(o) | Err(Failure::Exit(o)) => Err(format!("selftest failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(e) => Err(format!("selftest failed: {e}")),
    }
}

/// Ok(Some(version)) when a new release became current. CoreQ's check selects the version and prunes old releases.
pub fn check(t: &Tool) -> Result<Option<String>, String> {
    let _lock = crate::executor::lock(&t.install.join("data"))?;
    recover_locked(t)?;
    update::update(t, true)
}

pub fn update(t: &Tool) -> Result<Option<String>, String> {
    let _lock = crate::executor::lock(&t.install.join("data"))?;
    recover_locked(t)?;
    let Some(ver) = update::update(t, true)? else { return Ok(None) };
    let name = format!("{}-{ver}-{}{}", t.name, update::target(), std::env::consts::EXE_SUFFIX);
    let base = format!("https://github.com/{}/releases/latest/download", t.repo);
    let tmp = t.install.join("releases").join(format!("{ver}.tmp"));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let bytes = ["SHA256SUMS", name.as_str()].iter().try_for_each(|f| update::download(&format!("{base}/{f}"), &tmp.join(f)))
        .and_then(|_| verified_candidate(t, &name, &tmp));
    let _ = std::fs::remove_dir_all(&tmp);
    // ponytail: leaks one version string per new release; Tool::version is 'static in CoreQ
    let candidate = Tool { version: Box::leak(ver.clone().into_boxed_str()), ..t.clone() };
    install_locked(&candidate, &bytes?, update::activate_install)?;
    // the check prunes before its download, so offline only skips its answer (agentq.md § 5: keep current and previous)
    let _ = update::update(t, true);
    Ok(Some(ver))
}

/// Own bytes into a scratch installation, twice: identical bytes reuse the release and the running exe is never rewritten.
pub fn selftest() -> bool {
    let d = std::env::temp_dir().join(format!("agentq-selftest-install-{}", std::process::id()));
    let t = Tool { install: d.clone(), ..Tool::new("agentq", "AgentQ", "taskqorg/agentq", crate::VERSION) };
    let ok = install_self(&t).and_then(|_| install_self(&t)).is_ok() && t.current().as_deref() == Some(crate::VERSION);
    let _ = std::fs::remove_dir_all(d);
    ok
}

/// The CoreQ version this binary was built with, read from its lockfile; None reads unknown (P2).
fn coreq_version() -> Option<&'static str> {
    let mut lines = include_str!("../Cargo.lock").lines();
    lines.find(|l| *l == "name = \"coreq\"")?;
    lines.next()?.strip_prefix("version = \"")?.strip_suffix('"')
}

fn version(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b)) && *s != "." && *s != "..")
}

pub fn observe(t: &Tool, now: u64) -> Value {
    let current = t.current();
    let selected = version(current.as_deref());
    let marker = t.release_exe(t.version).with_file_name("healthy");
    let receipt = std::fs::symlink_metadata(&marker).ok().filter(|m| m.is_file() && m.len() == 0)
        .and_then(|m| m.modified().ok()).and_then(|at| at.duration_since(UNIX_EPOCH).ok()).map(|at| at.as_secs())
        .filter(|at| *at <= now && std::fs::read(&marker).is_ok_and(|b| b.is_empty()));
    let health = match receipt {
        Some(at) => json!({"state": "verified", "scope": "status_written", "version": t.version, "at": at}),
        None => json!({"state": "unknown", "reason": "no_receipt"}),
    };
    let (lifecycle, reason) = match crate::control::host_disabled(&t.install.join("data")) {
        Err(_) => ("unknown", "control_unknown"),
        Ok(true) => ("pending_disable", "host_disabled"),
        Ok(false) => match selected {
            None => ("unknown", "selection_unknown"),
            Some(v) if v != t.version => ("waiting_for_activation", "selection_changed"),
            Some(_) if receipt.is_some() => ("healthy", "status_written"),
            _ => ("starting", "no_receipt"),
        },
    };
    json!({"at": now, "selected": selected, "running": t.version, "coreq_version": coreq_version(),
        "health": health, "lifecycle": lifecycle, "reason": reason})
}

/// Project only bounded identifiers and fixed codes; stale observations retain history, never current health.
pub fn project(v: &Value, now: u64) -> Value {
    if !v.is_object() { return json!({"lifecycle": "unknown", "health": {"state": "unknown"}, "reason": "not_reported"}); }
    let at = v["at"].as_u64();
    let fresh = at.is_some_and(|at| at <= now && now - at <= 90);
    let running = version(v["running"].as_str());
    let selected = version(v["selected"].as_str());
    let health_at = v["health"]["at"].as_u64();
    let verified = fresh && v["health"]["state"] == "verified" && v["health"]["scope"] == "status_written"
        && running.is_some() && v["health"]["version"].as_str() == running && health_at.is_some_and(|h| Some(h) <= at);
    let health = if verified {
        json!({"state": "verified", "scope": "status_written", "version": running, "at": health_at})
    } else { json!({"state": "unknown", "reason": if fresh { "no_receipt" } else { "stale_observation" }, "at": health_at}) };
    let lifecycle = v["lifecycle"].as_str().filter(|s| ["waiting_for_activation", "healthy", "starting", "pending_disable", "unknown"].contains(s)).unwrap_or("unknown");
    let lifecycle = if !fresh || (lifecycle == "healthy" && (!verified || selected.is_none() || selected != running)) { "unknown" } else { lifecycle };
    let reason = v["reason"].as_str().filter(|s| ["control_unknown", "host_disabled", "selection_unknown", "selection_changed", "status_written", "no_receipt"].contains(s)).unwrap_or("unknown");
    json!({"at": at, "fresh": fresh, "selected": selected, "running": running, "coreq_version": version(v["coreq_version"].as_str()),
        "health": health, "lifecycle": lifecycle, "reason": if fresh { reason } else { "stale_observation" }})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A prior release 1.0.0 with launcher and selection, and the candidate 2.0.0 from a real executable image.
    fn installed(tag: &str) -> (std::path::PathBuf, Tool, Vec<u8>) {
        let d = std::env::temp_dir().join(format!("agentq-238-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let t = Tool { install: d.clone(), ..Tool::new("agentq", "AgentQ", "taskqorg/agentq", "2.0.0") };
        fs::create_dir_all(t.release_exe("1.0.0").parent().unwrap()).unwrap();
        fs::create_dir_all(t.release_exe("0.9.0").parent().unwrap()).unwrap();
        fs::write(t.release_exe("1.0.0"), "prior release").unwrap();
        fs::write(d.join(t.exe_name()), "prior launcher").unwrap();
        fs::write(d.join("current"), "1.0.0").unwrap();
        fs::write(d.join("previous"), "0.9.0").unwrap();
        (d, t, fs::read(std::env::current_exe().unwrap()).unwrap())
    }

    fn prior_active(d: &Path, t: &Tool) {
        assert_eq!(t.current().as_deref(), Some("1.0.0"));
        assert_eq!(fs::read_to_string(t.release_exe("1.0.0")).unwrap(), "prior release");
        assert!(d.join("current").is_file());
    }

    /// #238 A1: install prepares, then activates; a repeat reuses the release and keeps `previous`.
    #[test]
    fn install_prepares_then_activates() {
        let (d, t, bytes) = installed("ok");
        install(&t, &bytes).unwrap();
        install(&t, &bytes).unwrap();
        assert_eq!(t.current().as_deref(), Some("2.0.0"));
        assert_eq!(fs::read_to_string(d.join("previous")).unwrap(), "1.0.0");
        assert_eq!(fs::read(t.release_exe("2.0.0")).unwrap(), bytes);
        assert_eq!(fs::read(d.join(t.exe_name())).unwrap(), bytes);
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2: an interruption after preparation, a held lock and each refused or failed activation keep the prior release active.
    #[test]
    fn failed_install_keeps_prior_release_active() {
        let (d, t, bytes) = installed("interrupted");
        update::prepare_install(&t, &bytes).unwrap();
        prior_active(&d, &t);
        assert_eq!(fs::read_to_string(d.join(t.exe_name())).unwrap(), "prior launcher", "preparation leaves the launcher");
        assert_eq!(fs::read_to_string(d.join("previous")).unwrap(), "0.9.0", "preparation leaves the rollback");
        install(&t, &bytes).unwrap();
        assert_eq!(t.current().as_deref(), Some("2.0.0"), "a repeat finishes the interrupted install");
        fs::remove_dir_all(d).unwrap();

        let (d, t, bytes) = installed("locked");
        let held = crate::executor::lock(&d.join("data")).unwrap();
        assert!(install(&t, &bytes).unwrap_err().contains("lifecycle lock"));
        drop(held);
        prior_active(&d, &t);
        assert!(!t.release_exe("2.0.0").exists());
        fs::remove_dir_all(d).unwrap();

        // the launcher is already replaced at this failure; it only reads `current`
        let (d, t, bytes) = installed("activation");
        fs::remove_file(d.join("previous")).unwrap();
        fs::create_dir(d.join("previous")).unwrap();
        assert!(install(&t, &bytes).is_err());
        prior_active(&d, &t);
        fs::remove_dir_all(d).unwrap();

        let (d, t, bytes) = installed("differs");
        fs::create_dir_all(t.release_exe("2.0.0").parent().unwrap()).unwrap();
        fs::write(t.release_exe("2.0.0"), "other build, same version").unwrap();
        assert!(install(&t, &bytes).unwrap_err().contains("existing file kept"));
        prior_active(&d, &t);
        assert_eq!(fs::read_to_string(d.join(t.exe_name())).unwrap(), "prior launcher");
        fs::remove_dir_all(d).unwrap();

        let (d, t, bytes) = installed("downgrade");
        let older = Tool { version: "0.9.5", ..t.clone() };
        assert!(install(&older, &bytes).unwrap_err().contains("downgrade"));
        prior_active(&d, &t);
        assert_eq!(fs::read_to_string(d.join(t.exe_name())).unwrap(), "prior launcher");
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2: a candidate with a wrong sha256 or a failing selftest never reaches preparation.
    #[test]
    fn rejected_candidate_stops_before_preparation() {
        let (d, t, _) = installed("candidate");
        let dir = d.join("releases/2.0.0.tmp");
        fs::create_dir_all(&dir).unwrap();
        let name = "agentq-2.0.0-test";
        fs::write(dir.join(name), "#!/bin/sh\necho candidate broken >&2\nexit 1\n").unwrap();
        fs::write(dir.join("SHA256SUMS"), format!("{} {name}\n", "0".repeat(64))).unwrap();
        assert!(verified_candidate(&t, name, &dir).unwrap_err().contains("not in SHA256SUMS"));
        #[cfg(unix)]
        {
            let refused = verified_candidate(&t, name, &dir).unwrap_err();
            let sum = refused.split_whitespace().nth(2).unwrap();
            fs::write(dir.join("SHA256SUMS"), format!("{sum}  {name}\n")).unwrap();
            assert!(verified_candidate(&t, name, &dir).unwrap_err().contains("candidate broken"));
        }
        prior_active(&d, &t);
        assert!(!t.release_exe("2.0.0").exists());
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2, CoreQ R15: a late activation error must restore selection, rollback and launcher bytes.
    #[test]
    fn late_activation_failure_restores_originals() {
        let (d, t, bytes) = installed("late-error");
        let error = install_with(&t, &bytes, |t| {
            update::activate_install(t)?;
            assert_eq!(t.current().as_deref(), Some("2.0.0"));
            Err("injected failure after current rename".into())
        }).unwrap_err();
        assert!(error.contains("injected failure"));
        prior_active(&d, &t);
        assert_eq!(fs::read_to_string(d.join("previous")).unwrap(), "0.9.0");
        assert_eq!(fs::read_to_string(d.join(t.exe_name())).unwrap(), "prior launcher");
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2, M8.1: no in-memory state is needed to restore a crash after selection; failed recovery keeps its evidence.
    #[test]
    fn interrupted_activation_recovers_on_next_start() {
        let (d, t, bytes) = installed("recover");
        #[cfg(windows)]
        fs::write(d.join(t.gui_exe_name()), "prior GUI launcher").unwrap();
        {
            let _lock = crate::executor::lock(&d.join("data")).unwrap();
            update::prepare_install(&t, &bytes).unwrap();
            originals(&t).unwrap();
            update::activate_install(&t).unwrap();
        }
        assert_eq!(t.current().as_deref(), Some("2.0.0"));
        // Leave a CoreQ temporary as a crash could, and temporarily lose access to a backup.
        fs::write(d.join("current.pending"), "partial").unwrap();
        fs::rename(recovery_dir(&t).join("previous"), recovery_dir(&t).join("hidden-previous")).unwrap();
        assert!(recover(&t).unwrap_err().contains("uncertain"));
        assert!(recovery_dir(&t).join("ready").is_file());
        fs::rename(recovery_dir(&t).join("hidden-previous"), recovery_dir(&t).join("previous")).unwrap();
        recover(&t.clone()).unwrap();
        prior_active(&d, &t);
        assert_eq!(fs::read_to_string(d.join("previous")).unwrap(), "0.9.0");
        assert_eq!(fs::read_to_string(d.join(t.exe_name())).unwrap(), "prior launcher");
        #[cfg(windows)]
        assert_eq!(fs::read_to_string(d.join(t.gui_exe_name())).unwrap(), "prior GUI launcher");
        assert!(!d.join("current.pending").exists());
        assert!(!recovery_dir(&t).exists());
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2: competing --check and service update cannot prune a prepared candidate or its staging directory.
    #[test]
    fn concurrent_checks_cannot_prune_install() {
        let (d, t, bytes) = installed("concurrent");
        install_with(&t, &bytes, |t| {
            let staging = d.join("releases/2.0.0.tmp");
            fs::create_dir_all(&staging).unwrap();
            fs::write(staging.join("capture"), "keep").unwrap();
            std::thread::scope(|scope| {
                let check = scope.spawn(|| super::check(t));
                let update = scope.spawn(|| super::update(t));
                assert!(check.join().unwrap().unwrap_err().contains("lifecycle lock"));
                assert!(update.join().unwrap().unwrap_err().contains("lifecycle lock"));
            });
            assert!(t.release_exe("2.0.0").is_file());
            assert_eq!(fs::read_to_string(staging.join("capture")).unwrap(), "keep");
            update::activate_install(t)
        }).unwrap();
        assert_eq!(t.current().as_deref(), Some("2.0.0"));
        fs::remove_dir_all(d).unwrap();
    }

    /// #238 A2: a failed first install restores absence, and Unix executable permissions survive recovery.
    #[test]
    fn failed_first_install_restores_absence() {
        let (d, t, bytes) = installed("absent");
        for name in active_files(&t) { let _ = fs::remove_file(d.join(name)); }
        assert!(install_with(&t, &bytes, |t| {
            update::activate_install(t)?;
            Err("injected late failure".into())
        }).is_err());
        for name in active_files(&t) { assert!(!d.join(name).exists()); }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(d.join(t.exe_name()), "original executable").unwrap();
            fs::set_permissions(d.join(t.exe_name()), fs::Permissions::from_mode(0o751)).unwrap();
            assert!(install_with(&t, &bytes, |t| {
                update::activate_install(t)?;
                Err("injected late failure".into())
            }).is_err());
            assert_eq!(fs::metadata(d.join(t.exe_name())).unwrap().permissions().mode() & 0o777, 0o751);
        }
        fs::remove_dir_all(d).unwrap();
    }

    /// #141 A1/A3: real CoreQ selection and marker, never binary presence alone; #235 A3: one CoreQ, the one the lockfile holds.
    #[test]
    fn installation_sources() {
        let lock = include_str!("../Cargo.lock");
        assert_eq!(lock.lines().filter(|l| *l == "name = \"coreq\"").count(), 1, "one CoreQ in Cargo.lock");
        let pin = coreq_version().unwrap();
        assert!(include_str!("../runtime/Cargo.toml").contains(&format!("tag = \"v{pin}\"")), "the runtime names the locked CoreQ");
        let d = std::env::temp_dir().join(format!("agentq-141-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mut t = Tool::new("agentq", "AgentQ", "taskqorg/agentq", "1.0.0");
        t.install = d.clone();
        assert_eq!(observe(&t, crate::now())["reason"], "selection_unknown");
        std::fs::create_dir_all(d.join("releases/1.0.0")).unwrap();
        std::fs::write(d.join("current"), "1.0.0").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "starting");
        std::fs::write(d.join("releases/1.0.0/healthy"), "").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "healthy");
        std::fs::write(d.join("current"), "2.0.0").unwrap();
        assert_eq!(observe(&t, crate::now())["lifecycle"], "waiting_for_activation");
        std::fs::write(d.join("releases/1.0.0/healthy"), "invalid").unwrap();
        assert_eq!(observe(&t, crate::now())["health"]["state"], "unknown");
        std::fs::create_dir_all(d.join("data")).unwrap();
        std::fs::write(d.join("data/integration.json"), "{").unwrap();
        assert_eq!(observe(&t, crate::now())["reason"], "control_unknown");
        std::fs::remove_dir_all(d).unwrap();
    }
}
