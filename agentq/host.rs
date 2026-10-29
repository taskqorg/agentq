//! Host tools agentq runs: minimum versions, `doctor` checks and the logged call wrapper (agentq.md § 5 Host requirements).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A versioned host tool: its version query, displayed minimum, the minimum its version text selects, and the fix.
struct Req {
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    min: &'static str,
    pick: fn(&str) -> Option<&'static str>,
    fix: &'static str,
}

const CURL: Req = Req { name: "curl", program: "curl", args: &["--version"], min: "7.55.1", pick: |_| Some("7.55.1"),
    fix: "install curl 7.55.1 or newer from the OS package manager" };
const TAR: Req = Req { name: "tar", program: "tar", args: &["--version"], min: "bsdtar 3.3.2 or GNU tar 1.29",
    pick: |v| if v.contains("bsdtar") { Some("3.3.2") } else if v.contains("GNU tar") { Some("1.29") } else { None },
    fix: "install bsdtar (libarchive) 3.3.2 or GNU tar 1.29 or newer from the OS package manager" };
const POWERSHELL: Req = Req { name: "powershell", program: "powershell", args: &["-NoProfile", "-NonInteractive", "-Command", "$PSVersionTable.PSVersion.ToString()"],
    min: "5.1", pick: |_| Some("5.1"), fix: "install Windows PowerShell 5.1 (Windows Management Framework 5.1)" };
const PYTHON_FIX: &str = "install Python 3.11-3.13 with venv, or set AGENTQ_PYTHON to one";

const OS: &[&str] = if cfg!(target_os = "macos") {
    &["launchctl", "ps", "pgrep", "lsof", "id", "hostname", "stty"]
} else if cfg!(windows) {
    &["schtasks", "taskkill", "netstat", "hostname", "whoami", "icacls"]
} else {
    &["systemctl", "ps", "pgrep", "id", "hostname", "stty"]
};

/// Every requirement as `doctor` lines: (requirement met, line).
pub fn check() -> Vec<(bool, String)> {
    let mut lines = vec![versioned(&CURL, Path::new(CURL.program)), versioned(&TAR, Path::new(TAR.program))];
    if cfg!(windows) {
        lines.push(versioned(&POWERSHELL, Path::new(POWERSHELL.program)));
    }
    lines.push(python());
    lines.push(present(OS));
    let store = if cfg!(target_os = "macos") { Some("security") } else if cfg!(target_os = "linux") { Some("secret-tool") } else { None };
    if let Some(s) = store {
        let state = if find(s).is_some() { "found" } else { "missing; Langfuse keys use data/langfuse" };
        lines.push((true, format!("host tool {s}: {state} (optional key store)")));
    }
    lines
}

/// The failing requirements, each with its fix step: what install, onboarding and service start report.
pub fn problems() -> Vec<String> {
    check().into_iter().filter(|l| !l.0).map(|l| l.1).collect()
}

fn versioned(r: &Req, program: &Path) -> (bool, String) {
    let o = match run(Command::new(program).args(r.args)) {
        Ok(o) => o,
        Err(e) => return (false, format!("host tool {}: not found or not runnable, minimum {}; {} ({e})", r.name, r.min, r.fix)),
    };
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    match (version_in(first), (r.pick)(first)) {
        (Some(v), Some(m)) if at_least(&v, m) => (true, format!("host tool {}: {v} ok (minimum {m})", r.name)),
        (Some(v), Some(m)) => (false, format!("host tool {}: {v} below minimum {m}; {}", r.name, r.fix)),
        _ => (false, format!("host tool {}: unrecognized version {first:?}, minimum {}; {}", r.name, r.min, r.fix)),
    }
}

/// The finder's Python (§ 3), else the version of the first candidate that answers.
fn python() -> (bool, String) {
    if coreq::update::target() == "macos-x86_64" {
        return (true, "host tool python: not needed (no headroom wheels for macOS x64)".into());
    }
    let version = |c: &[String]| {
        let o = coreq::quiet(crate::engines::py(c).arg("--version")).output().ok().filter(|o| o.status.success())?;
        version_in(&format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    };
    match crate::engines::python() {
        Ok(c) => match version(&c) {
            Some(v) => (true, format!("host tool python: {v} ok ({}; 3.11-3.13 with venv)", c.join(" "))),
            None => (false, format!("host tool python: {} passed the probe but reports no version; {PYTHON_FIX}", c.join(" "))),
        },
        Err(e) => match crate::engines::python_candidates().iter().find_map(|c| version(c).map(|v| (v, c.join(" ")))) {
            Some((v, c)) if at_least(&v, "3.11") && !at_least(&v, "3.14") => (false, format!("host tool python: {v} ({c}) lacks venv or ensurepip, need 3.11-3.13 with venv; {PYTHON_FIX}")),
            Some((v, c)) => (false, format!("host tool python: {v} ({c}) outside 3.11-3.13; {PYTHON_FIX}")),
            None => (false, format!("host tool python: not found, need 3.11-3.13 with venv; {PYTHON_FIX} ({e})")),
        },
    }
}

/// OS components: presence on PATH only, their versions follow the OS release.
fn present(tools: &[&str]) -> (bool, String) {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| find(t).is_none()).collect();
    if missing.is_empty() {
        (true, format!("host tools: OS components {} found", tools.join(", ")))
    } else {
        (false, format!("host tools: OS components missing: {}; restore them from the OS release or add their folder to PATH", missing.join(", ")))
    }
}

fn find(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&name)).find(|p| p.is_file())
}

/// The first dotted number in `text`, e.g. `8.4.0` from `curl 8.4.0 (Windows) libcurl/8.4.0`.
fn version_in(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
        .map(String::from)
}

/// Numeric dotted comparison; a component's trailing letters (`0rc1`) count as its leading digits.
fn at_least(found: &str, min: &str) -> bool {
    let parts = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).collect() };
    let (f, m) = (parts(found), parts(min));
    (0..f.len().max(m.len())).map(|i| (f.get(i).copied().unwrap_or(0), m.get(i).copied().unwrap_or(0))).find(|(a, b)| a != b).is_none_or(|(a, b)| a > b)
}

/// Output of one host tool call; a failed start is logged and returned as `Err`. The caller judges the exit.
pub fn output(c: &mut Command) -> Result<Output, String> {
    coreq::quiet(c).output().map_err(|e| log(c, &e.to_string()))
}

/// Output of one host tool call that must exit 0; a failed start or exit is logged and returned as `Err`.
pub fn run(c: &mut Command) -> Result<Output, String> {
    run_allow(c, &[0])
}

/// `run` for tools whose listed exit codes are answers, e.g. `pgrep` exit 1: nothing matched.
pub fn run_allow(c: &mut Command, codes: &[i32]) -> Result<Output, String> {
    let o = output(c)?;
    if o.status.code().is_some_and(|x| codes.contains(&x)) {
        return Ok(o);
    }
    Err(exit(c, &o))
}

/// Logs a failed exit of `c` and returns the caller's own error text.
pub fn failed(c: &Command, o: &Output, error: &str) -> String {
    exit(c, o);
    error.into()
}

fn exit(c: &Command, o: &Output) -> String {
    let err = String::from_utf8_lossy(&o.stderr);
    log(c, &format!("{}: {}", o.status, err.trim().lines().last().unwrap_or("no error output")))
}

/// Logs `agentq: `<command>`: <error>` to stderr (the service log under `service run`) and returns the message.
pub fn log(c: &Command, error: &str) -> String {
    let short = |s: &str| if s.chars().count() > 80 { format!("{}...", s.chars().take(77).collect::<String>()) } else { s.to_string() };
    let cmd: Vec<String> = std::iter::once(c.get_program()).chain(c.get_args()).map(|a| short(&a.to_string_lossy())).collect();
    let m = format!("`{}`: {error}", cmd.join(" "));
    eprintln!("agentq: {m}");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake tool that prints `line` for any arguments.
    fn fake(dir: &Path, name: &str, line: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        if cfg!(windows) {
            let p = dir.join(format!("{name}.cmd"));
            std::fs::write(&p, format!("@echo {line}\r\n")).unwrap();
            p
        } else {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        }
    }

    /// specq Host requirements: an old or missing tool fails naming the tool, the found version, the minimum and the fix;
    /// the oldest supported version passes.
    #[test]
    fn old_or_missing_tool_fails_with_fix() {
        let d = std::env::temp_dir().join(format!("agentq-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (ok, l) = versioned(&CURL, &fake(&d, "curl-old", "curl 7.50.0 (x86_64-pc-linux-gnu) libcurl/7.50.0"));
        assert!(!ok && l.contains("curl: 7.50.0 below minimum 7.55.1; install curl"), "{l}");
        let (ok, l) = versioned(&CURL, &fake(&d, "curl-min", "curl 7.55.1 (Windows) libcurl/7.55.1 WinSSL"));
        assert!(ok && l.contains("7.55.1 ok"), "{l}");
        let (ok, l) = versioned(&TAR, &fake(&d, "tar-old", "tar (GNU tar) 1.28"));
        assert!(!ok && l.contains("1.28 below minimum 1.29"), "{l}");
        let (ok, l) = versioned(&TAR, &fake(&d, "tar-bsd", "bsdtar 3.3.2 - libarchive 3.3.2 zlib/1.2.5.f-ipp"));
        assert!(ok && l.contains("3.3.2 ok"), "{l}");
        let (ok, l) = versioned(&TAR, &fake(&d, "tar-other", "busybox tar"));
        assert!(!ok && l.contains("unrecognized version") && l.contains("bsdtar 3.3.2 or GNU tar 1.29"), "{l}");
        let (ok, l) = versioned(&CURL, &d.join("agentq-no-such-curl"));
        assert!(!ok && l.contains("curl: not found or not runnable, minimum 7.55.1; install curl") && l.contains("agentq-no-such-curl"), "{l}");
        let (ok, l) = present(&["agentq-no-such-tool"]);
        assert!(!ok && l.contains("missing: agentq-no-such-tool"), "{l}");
        let _ = std::fs::remove_dir_all(d);
    }

    /// A failed call names its command and error, never an empty result; an allowed exit code is an answer.
    #[test]
    fn failed_call_names_command_and_error() {
        let e = run(Command::new("agentq-no-such-tool").arg("--version")).unwrap_err();
        assert!(e.starts_with("`agentq-no-such-tool --version`: "), "{e}");
        let (sh, fail): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/c", "echo boom 1>&2 & exit 3"]) } else { ("sh", &["-c", "echo boom >&2; exit 3"]) };
        let e = run(Command::new(sh).args(fail)).unwrap_err();
        assert!(e.contains("boom") && e.contains('3'), "{e}");
        assert!(run_allow(Command::new(sh).args(fail), &[3]).is_ok());
    }

    #[test]
    fn versions() {
        assert_eq!(version_in("tar (GNU tar) 1.34").as_deref(), Some("1.34"));
        assert_eq!(version_in("Python 3.13.0rc1").as_deref(), Some("3.13.0rc1"));
        assert!(at_least("7.55.1", "7.55.1") && at_least("8.0", "7.55.1") && at_least("3.13.0rc1", "3.11"));
        assert!(!at_least("7.55.0", "7.55.1") && !at_least("3.10.12", "3.11") && !at_least("3.13.5", "3.14"));
    }
}
