//! Managed engines: pinned RTK and the headroom venv; `agentq run` (agentq.md § 3).

use coreq::update::{download, target, verify};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const RTK: &str = "0.51.0";
pub const HEADROOM: &str = "0.40.0";
/// `rtk-ai/rtk` v0.51.0 release assets per `coreq::update::target()`.
const RTK_ASSETS: [(&str, &str, &str); 5] = [
    ("macos-aarch64", "rtk-aarch64-apple-darwin.tar.gz", "8817d8b71afc02ac8bf06eb24bcc41c306592ab735b68e8fee9db1ba0de7cb59"),
    ("macos-x86_64", "rtk-x86_64-apple-darwin.tar.gz", "bd39c8153f4147358360c7dc51665a8131cc9ee16f81f69a9402ffa500be3cc2"),
    ("linux-x86_64", "rtk-x86_64-unknown-linux-musl.tar.gz", "5028d3b19a8f0990d30fec9fbb07e32782bc5698e618fb1861aad8a9ccba4eb5"),
    ("linux-aarch64", "rtk-aarch64-unknown-linux-gnu.tar.gz", "8d6d1aad9e69b42481eda7039507d1f7ee93698f87713cecd873d287c1931632"),
    ("windows-x86_64", "rtk-x86_64-pc-windows-msvc.zip", "1623e9b45d28b15122d69e7314776e1123a804224885ce07e7182fb40080f05c"),
];
pub const HEADROOM_LOCK: &str = include_str!("headroom.lock");
const KEEP: usize = 500;
const KEEP_AGE: Duration = Duration::from_secs(7 * 86400);
const KEEP_BYTES: u64 = 512 * 1024 * 1024;

pub fn rtk_asset() -> Option<(&'static str, &'static str)> {
    RTK_ASSETS.iter().find(|a| a.0 == target()).map(|a| (a.1, a.2))
}

fn engine_dir(install: &Path, name: &str, ver: &str) -> PathBuf {
    install.join("engines").join(format!("{name}-{ver}"))
}

pub fn rtk_path(install: &Path) -> PathBuf {
    engine_dir(install, "rtk", RTK).join(format!("rtk{}", std::env::consts::EXE_SUFFIX))
}

fn venv_bin(dir: &Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) { "Scripts" } else { "bin" }).join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// stdout of a command that must succeed; a failure is logged with its command (agentq.md § 5 Host requirements).
fn output(c: &mut Command) -> Result<String, String> {
    crate::host::run(c).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn check_version(exe: &Path, want: &str) -> Result<(), String> {
    let got = output(Command::new(exe).env("OPENBLAS_NUM_THREADS", "1").env("OMP_NUM_THREADS", "1").arg("--version"))?.trim().to_string();
    if got == want { Ok(()) } else { Err(format!("{} --version: {got:?}, want {want:?}", exe.display())) }
}

/// One line per engine; a failure names its reason and never fails the caller (agentq.md § 3).
pub fn ensure(install: &Path) -> Vec<String> {
    let line = |name: &str, ver: &str, r: Result<PathBuf, String>| match r {
        Ok(_) => format!("engine {name}: {ver} ready"),
        Err(e) => format!("engine {name}: unavailable: {e}"),
    };
    vec![line("rtk", RTK, install_rtk(install)), line("headroom", HEADROOM, install_headroom(install))]
}

/// The checked headroom, when its install finished.
pub fn headroom_exe(install: &Path) -> Option<PathBuf> {
    let dir = engine_dir(install, "headroom", HEADROOM);
    dir.join("ready").exists().then(|| venv_bin(&dir, "headroom"))
}

/// `status.json` `engines`: each pin and whether its checked install is present; no process runs (30 s pace).
pub fn status(install: &Path) -> serde_json::Value {
    let state = |ok: bool| if ok { "ready" } else { "unavailable" };
    serde_json::json!({"rtk": {"version": RTK, "state": state(rtk_path(install).exists())}, "headroom": {"version": HEADROOM, "state": state(headroom_exe(install).is_some())}})
}

/// Read-only `doctor` lines: each engine's checked version, else why not.
pub fn check(install: &Path) -> Vec<String> {
    let hr = match headroom_exe(install) {
        Some(exe) => check_version(&exe, &format!("headroom, version {HEADROOM}")),
        None => Err("not installed".into()),
    };
    [("rtk", RTK, check_version(&rtk_path(install), &format!("rtk {RTK}"))), ("headroom", HEADROOM, hr)]
        .into_iter()
        .map(|(name, ver, r)| match r {
            Ok(()) => format!("engine {name}: {ver} ok"),
            Err(e) => format!("engine {name}: unavailable: {e}"),
        })
        .collect()
}

/// Download, sha256, unpack into a temporary folder, `--version`, then rename into place.
pub fn install_rtk(install: &Path) -> Result<PathBuf, String> {
    let exe = rtk_path(install);
    let want = format!("rtk {RTK}");
    if check_version(&exe, &want).is_ok() {
        return Ok(exe);
    }
    let (name, sha) = rtk_asset().ok_or(format!("no RTK build for {}", target()))?;
    let dir = exe.parent().unwrap();
    let tmp = dir.with_extension(format!("tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let res = (|| {
        let archive = tmp.join(name);
        download(&format!("https://github.com/rtk-ai/rtk/releases/download/v{RTK}/{name}"), &archive)?;
        verify(format!("{sha}  {name}").as_bytes(), name, &std::fs::read(&archive).map_err(|e| e.to_string())?)?;
        // bsdtar on Windows 10+ also reads zip
        output(Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&tmp))?;
        std::fs::remove_file(&archive).map_err(|e| e.to_string())?;
        check_version(&tmp.join(exe.file_name().unwrap()), &want)?;
        let _ = std::fs::remove_dir_all(dir);
        std::fs::rename(&tmp, dir).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    res?;
    check_version(&exe, &want).map(|_| exe)
}

/// Python candidates in finder order: `AGENTQ_PYTHON`, else the usual names on PATH.
pub fn python_candidates() -> Vec<Vec<String>> {
    match std::env::var("AGENTQ_PYTHON") {
        Ok(p) => vec![vec![p]],
        Err(_) => {
            let py = if cfg!(windows) { vec!["py -3.13", "py -3.12", "py -3.11"] } else { vec![] };
            py.into_iter().chain(["python3.13", "python3.12", "python3.11", "python3", "python"]).map(|c| c.split(' ').map(String::from).collect()).collect()
        }
    }
}

/// Python 3.11-3.13 with venv: the first candidate that passes the probe. Probes are discovery, not logged failures.
pub fn python() -> Result<Vec<String>, String> {
    let probe = "import sys, venv, ensurepip; assert (3, 11) <= sys.version_info[:2] < (3, 14)";
    let found = python_candidates();
    found
        .iter()
        .find(|c| coreq::quiet(py(c).args(["-c", probe])).output().is_ok_and(|o| o.status.success()))
        .cloned()
        .ok_or_else(|| format!("no Python 3.11-3.13 with venv among `{}`; install one or set AGENTQ_PYTHON", found.iter().map(|c| c.join(" ")).collect::<Vec<_>>().join("`, `")))
}

/// Isolated mode (`-I`) from a fixed folder: no PYTHON* env, user site or planted module from the caller's folder.
pub fn py<S: AsRef<std::ffi::OsStr>>(c: &[S]) -> Command {
    let mut cmd = Command::new(&c[0]);
    cmd.args(&c[1..]).arg("-I").env("OPENBLAS_NUM_THREADS", "1").env("OMP_NUM_THREADS", "1").current_dir(std::env::temp_dir());
    cmd
}

/// Private venv from the hashed lock, wheels only; `ready` marks a checked install.
pub fn install_headroom(install: &Path) -> Result<PathBuf, String> {
    let dir = engine_dir(install, "headroom", HEADROOM);
    let exe = venv_bin(&dir, "headroom");
    let want = format!("headroom, version {HEADROOM}");
    if dir.join("ready").exists() {
        check_version(&exe, &want).map_err(|e| format!("checked headroom preserved after failed probe: {e}"))?;
        return Ok(exe);
    }
    if target() == "macos-x86_64" {
        return Err(format!("headroom {HEADROOM} has no macOS x64 wheels"));
    }
    let python = python()?;
    // a venv is not relocatable: build in place, `ready` last
    let _ = std::fs::remove_dir_all(&dir);
    output(py(&python).args(["-m", "venv"]).arg(&dir))?;
    let lock = dir.join("headroom.lock");
    std::fs::write(&lock, HEADROOM_LOCK).map_err(|e| e.to_string())?;
    let pip = ["-m", "pip", "install", "--disable-pip-version-check", "--no-input", "--require-hashes", "--only-binary", ":all:", "--no-deps", "-r"];
    output(py(&[venv_bin(&dir, "python")]).args(pip).arg(&lock))?;
    check_version(&exe, &want)?;
    std::fs::write(dir.join("ready"), HEADROOM).map_err(|e| e.to_string())?;
    Ok(exe)
}

/// Test and build runners RTK filters without changing their semantics.
pub fn qualified(cmd: &[String]) -> bool {
    let c: Vec<&str> = cmd.iter().take(2).map(String::as_str).collect();
    matches!(c.as_slice(), ["pytest", ..] | ["cargo", "test" | "build" | "check"] | ["go", "test"])
}

/// Qualified runner adapter; all decisions and records belong to the shared core.
pub fn run(install: &Path, cmd: &[String]) -> i32 { crate::compression::run(install, cmd) }

/// `get_original` for `rtk:<id>`: RTK's own recall from that run's database.
pub fn recall(install: &Path, r: &str, hash: &str) -> Result<String, String> {
    let id = r.strip_prefix("rtk:").filter(|id| id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).ok_or(format!("Invalid reference {r}"))?;
    let dir = install.join("data/runs").join(id);
    if dir.join("stdout").is_file() {
        if age(&dir) > KEEP_AGE { return Err(format!("expired: {r} is not retained; rerun the command")); }
        return std::fs::read_to_string(dir.join("stdout")).map_err(|e| e.to_string());
    }
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("RTK refs require the hash shown in its recovery hint".into());
    }
    let db = install.join("data").join("runs").join(id).join("recall.db");
    if !db.exists() || age(db.parent().unwrap()) > KEEP_AGE {
        return Err(format!("expired: {r} is not retained; rerun the command"));
    }
    output(Command::new(rtk_path(install)).args(["recall", "--full", hash]).env("RTK_RECALL_DB", db))
}

fn age(p: &Path) -> Duration {
    p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).unwrap_or(Duration::MAX)
}

fn size(p: &Path) -> u64 {
    std::fs::read_dir(p).into_iter().flatten().flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
}

/// Keep at most 500 runs, 7 days and 512 MiB; oldest go first.
pub(crate) fn prune(runs: &Path) {
    let mut dirs: Vec<(Duration, PathBuf)> = std::fs::read_dir(runs).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).map(|p| (age(&p), p)).collect();
    dirs.sort();
    let mut bytes = 0;
    for (i, (a, p)) in dirs.iter().enumerate() {
        bytes += size(p);
        if i >= KEEP || *a > KEEP_AGE || bytes > KEEP_BYTES {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn qualified_list() {
        for c in [&["pytest"][..], &["pytest", "-q"], &["cargo", "test"], &["cargo", "build", "--release"], &["go", "test", "./..."]] {
            assert!(qualified(&s(c)), "{c:?}");
        }
        for c in [&["cargo"][..], &["cargo", "run"], &["go", "build"], &["npm", "test"], &["ls"], &[]] {
            assert!(!qualified(&s(c)), "{c:?}");
        }
    }

    #[test]
    fn pins() {
        assert!(rtk_asset().is_some(), "an RTK asset for {}", target());
        assert!(HEADROOM_LOCK.contains(&format!("\nheadroom-ai=={HEADROOM} \\")));
        assert!(HEADROOM_LOCK.lines().filter(|l| !l.starts_with('#') && l.contains("==")).all(|l| l.ends_with('\\')), "every pin carries hashes");
    }

    #[test]
    fn native_without_rtk_keeps_exit_code() {
        let d = std::env::temp_dir().join(format!("agentq-run-{}", std::process::id()));
        let sh = if cfg!(windows) { s(&["cmd", "/c", "exit 3"]) } else { s(&["sh", "-c", "exit 3"]) };
        assert_eq!(run(&d, &sh), 3);
        assert_eq!(run(&d, &s(&["agentq-no-such-command"])), 127);
        assert_eq!(crate::compression::stats(&d.join("data")).unwrap()["groups"][0]["calls"], 2);
        let _ = std::fs::remove_dir_all(d);
    }

    /// agentq.md § 6: a qualified command without RTK still runs and queues one event without its arguments.
    #[test]
    fn qualified_run_queues_event() {
        let d = std::env::temp_dir().join(format!("agentq-runev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        // missing pytest exits 127, an installed one rejects the flag; either way nonzero and recorded
        let c = run(&d, &s(&["pytest", "--agentq-secret-arg"]));
        assert_ne!(c, 0);
        let q: Vec<_> = std::fs::read_dir(d.join("data").join("queue")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".json")).collect();
        assert_eq!(q.len(), 1);
        let ev = std::fs::read_to_string(q[0].path()).unwrap();
        assert!(ev.contains("\"agentq.compress\"") && ev.contains("\"pytest\""));
        assert!(ev.contains(&format!("{{\"intValue\":{c}}}")), "exit code recorded");
        assert!(!ev.contains("secret-arg"), "no arguments");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn recall_checks_refs() {
        let d = std::env::temp_dir().join(format!("agentq-recall-{}", std::process::id()));
        assert!(recall(&d, "rtk:bad", "abc").unwrap_err().starts_with("Invalid"));
        assert!(recall(&d, "rtk:00000000-0000-0000-0000-000000000000", "").unwrap_err().contains("hash"));
        assert!(recall(&d, "rtk:00000000-0000-0000-0000-000000000000", "abc").unwrap_err().starts_with("expired"));
    }

    #[test]
    fn failed_probe_preserves_checked_headroom() {
        let root = std::env::temp_dir().join(format!("agentq-preserve-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = engine_dir(&root, "headroom", HEADROOM);
        let exe = venv_bin(&dir, "headroom");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(dir.join("ready"), HEADROOM).unwrap();
        std::fs::write(&exe, "deliberately invalid executable").unwrap();
        std::fs::write(dir.join("keep"), "existing engine bytes").unwrap();
        assert!(install_headroom(&root).unwrap_err().contains("checked headroom preserved"));
        assert_eq!(std::fs::read_to_string(dir.join("keep")).unwrap(), "existing engine bytes");
        assert_eq!(std::fs::read_to_string(dir.join("ready")).unwrap(), HEADROOM);
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "deliberately invalid executable");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Network: AGENTQ_ENGINE_TEST=1 installs both engines into a temporary install and runs cargo through RTK.
    #[test]
    fn engines_install_and_run() {
        if std::env::var("AGENTQ_ENGINE_TEST").is_err() {
            return;
        }
        let d = std::env::temp_dir().join(format!("agentq-engines-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        install_rtk(&d).unwrap();
        assert_eq!(install_rtk(&d).unwrap(), rtk_path(&d), "second install reuses the checked engine");
        assert_eq!(run(&d, &s(&["cargo", "build", "--help"])), 0);
        assert_eq!(crate::compression::stats(&d.join("data")).unwrap()["groups"][0]["calls"], 1, "the call is recorded, even when help passes through");
        if target() != "macos-x86_64" {
            install_headroom(&d).unwrap();
            assert!(d.join("engines").join(format!("headroom-{HEADROOM}")).join("ready").exists());
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
