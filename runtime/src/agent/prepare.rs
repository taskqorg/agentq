//! M12.15-M12.17: what a launch needs before its first native byte: folder trust, the account's view and a clean child
//! environment (docs/spec/RUNTIME.md § Launch preparation).
use super::adapter::{Cause, Failure};
use super::*;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// One account's view of a client: its login home, its environment and its binary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountScope {
    /// Login home; None keeps the home variable in `env`, else the caller's own.
    pub home: Option<PathBuf>,
    /// Applied to the client after the scrub; it may name `PATH` and the home variable.
    pub env: Vec<(OsString, OsString)>,
    /// The client binary; None looks it up on the scope's `PATH`.
    pub executable: Option<PathBuf>,
}

impl AccountScope {
    pub(crate) fn var(&self, key: &str) -> Option<OsString> {
        self.env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }
    /// The home the client would select in this scope, when the scope names one.
    pub(crate) fn home_of(&self, client: Client) -> Option<PathBuf> {
        self.home.clone().or_else(|| self.var(client.home_var()).filter(|v| !v.is_empty()).map(PathBuf::from))
    }
    /// The lookup path of this scope.
    pub(crate) fn path(&self) -> OsString { self.var("PATH").or_else(|| std::env::var_os("PATH")).unwrap_or_default() }
}

/// Outcome of a folder trust request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust { Granted, Unchanged }

/// One hook as Codex lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub key: String,
    /// Native event name, such as `sessionStart`.
    pub event: String,
    pub command: Option<String>,
    pub source_path: Option<PathBuf>,
    /// Native hash; absent when Codex reports none.
    pub hash: Option<String>,
    /// Native trust status: `trusted`, `untrusted`, `modified` or `managed`.
    pub trust: String,
    pub enabled: bool,
}

/// Inherited session and auth variables no child gets (RUNTIME.md § Child environment).
const SCRUB: &[&str] = &[
    "CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_CODE_BRIDGE_SESSION_ID", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN", "CLAUDE_CODE_SESSION_ATTENDED", "CLAUDE_CODE_SSE_PORT", "CLAUDE_JOB_DIR", "CLAUDE_PID",
    "CODEX_THREAD_ID", "CODEX_SANDBOX", "CODEX_SANDBOX_NETWORK_DISABLED", "ANTIGRAVITY_CONVERSATION_ID",
    "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDE_CODE_OAUTH_REFRESH_TOKEN", "OPENAI_API_KEY", "CODEX_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY",
];

/// The git credential guard; it fails a prompt at once and leaves credential helpers running.
pub const GIT_GUARD: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"), ("GIT_ASKPASS", "agentq-no-askpass"), ("SSH_ASKPASS_REQUIRE", "never"), ("GCM_INTERACTIVE", "never"),
];

const BYPASS: &str = "--dangerously-bypass-hook-trust";

/// Scrub and guard `c`. A value the caller set on its own base command stays.
pub(crate) fn clean(c: &mut Command) {
    let set: Vec<OsString> = c.get_envs().filter(|(_, v)| v.is_some()).map(|(k, _)| k.to_os_string()).collect();
    for k in SCRUB { if !set.iter().any(|s| s == k) { c.env_remove(k); } }
    for (k, v) in GIT_GUARD { if !set.iter().any(|s| s == k) { c.env(k, v); } }
}

/// A worker start: batch ssh, unless the child or the project names its own ssh command, which then owns its prompts.
pub(crate) fn ssh_guard(c: &mut Command, spec: &Spec) {
    let named = |k: &str| child_env(spec, k).is_some() || c.get_envs().any(|(n, v)| n == k && v.is_some());
    if named("GIT_SSH_COMMAND") || named("GIT_SSH") { return; }
    let mut git = Command::new("git");
    git.args(["config", "--get", "core.sshCommand"]).current_dir(&spec.root);
    crate::quiet(&mut git);
    let configured = crate::process::output("agent.git-config", &mut git, Duration::from_secs(2)).is_ok_and(|o| !o.stdout.iter().all(u8::is_ascii_whitespace));
    if !configured { c.env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes"); }
}

/// M4.13: no runtime path carries the flag; a fixture on its own isolated home calls the native CLI directly.
pub(crate) fn refuse_bypass<T: AsRef<OsStr>>(args: &[T]) -> Res<()> {
    if args.iter().any(|a| a.as_ref() == BYPASS) { Err("hook-trust-bypass-refused".into()) } else { Ok(()) }
}

fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    ["API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"].iter().any(|m| name.contains(m))
}

/// M12.17: a non-empty value under a secret-like name would enter argv.
pub(crate) fn refuse_secret_settings(settings: &[(String, String)]) -> Res<()> {
    if settings.iter().any(|(k, v)| !v.is_empty() && secret_name(k)) { Err("secret-in-argv".into()) } else { Ok(()) }
}

/// M12.17: operator arguments are opaque, except an assignment under a secret-like name.
pub(crate) fn refuse_secret_args(args: &[OsString]) -> Res<()> {
    let secret = args.iter().filter_map(|a| a.to_str()).filter_map(|a| a.split_once('=')).any(|(k, v)| !v.is_empty() && secret_name(k));
    if secret { Err("secret-in-argv".into()) } else { Ok(()) }
}

/// A cause `launch` returns as the text of its error.
pub(crate) const CAUSES: &[&str] = &[
    "trust-path-refused", "trust-unsupported", "trust-config-missing", "trust-config-unreadable", "trust-locked", "trust-deadline", "trust-conflict",
    "trust-write-failed", "trust-readback-failed", "client-missing", "client-outdated", "hook-trust-bypass-refused", "secret-in-argv",
];

fn refused<T>(cause: &'static str) -> Result<T, Failure> { Err(Failure::Refused(Cause(cause))) }

/// A native binary, by magic number. A script or npm shim keeps its bytes when the client behind it updates.
fn native_binary(path: &Path) -> bool {
    let mut magic = [0; 4];
    File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok()
        && (magic[..2] == *b"MZ" || matches!(magic, [0x7f, b'E', b'L', b'F'] | [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf] | [0xce, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xce] | [0xca, 0xfe, 0xba, 0xbe]))
}

type Identity = (PathBuf, u64, Option<SystemTime>);

fn identity(exe: &Path) -> Option<Identity> {
    let path = exe.canonicalize().ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    native_binary(&path).then(|| (path, meta.len(), meta.modified().ok()))
}

fn versions() -> &'static Mutex<HashMap<PathBuf, (Identity, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Identity, String)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// M12.16: the version text of `exe`, cached by binary identity.
pub(super) fn version_text<E>(exe: &Path, read: impl FnOnce() -> Result<Option<String>, E>) -> Result<Option<String>, E> {
    let id = identity(exe);
    if let Some(id) = &id {
        if let Some((seen, text)) = versions().lock().unwrap_or_else(|e| e.into_inner()).get(&id.0) {
            if seen == id { return Ok(Some(text.clone())); }
        }
    }
    let text = read()?;
    if let (Some(id), Some(text)) = (id, &text) {
        versions().lock().unwrap_or_else(|e| e.into_inner()).insert(id.0.clone(), (id, text.clone()));
    }
    Ok(text)
}

/// The executable of `scope`: its own binary, else the lookup on its `PATH`.
pub(super) fn scope_executable(client: Client, scope: &AccountScope) -> Res<PathBuf> {
    match &scope.executable {
        Some(p) if p.is_absolute() || p.components().count() > 1 => Ok(p.clone()),
        Some(p) => Ok(which_in(&p.to_string_lossy(), &scope.path())),
        None => executable_in(client, &scope.path()),
    }
}

fn plain(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    // A verbatim drive path is the plain one; Codex keeps a verbatim UNC path as it is.
    Some(text.strip_prefix(r"\\?\").filter(|p| !p.starts_with("UNC\\")).unwrap_or(text).to_string())
}

fn user_home() -> Option<PathBuf> { std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|v| !v.is_empty()).map(PathBuf::from) }

/// The canonical folder a request may trust: never a filesystem root, a home or an ancestor of a home.
fn exact_folder(client: Client, folder: &Path, scope: &AccountScope) -> Result<PathBuf, Failure> {
    if !folder.is_absolute() { return refused("trust-path-refused"); }
    let Ok(canonical) = folder.canonicalize() else { return refused("trust-path-refused") };
    if !canonical.is_dir() || canonical.parent().is_none() { return refused("trust-path-refused"); }
    let homes = [user_home(), scope.home_of(client), base_home(client)];
    if homes.into_iter().flatten().filter_map(|h| h.canonicalize().ok()).any(|h| h.starts_with(&canonical)) { return refused("trust-path-refused"); }
    Ok(canonical)
}

/// Keep to the configuration file Claude reads: the legacy name when present, else `.claude.json`.
fn claude_file(scope: &AccountScope) -> Option<PathBuf> {
    let dir = scope.home_of(Client::Claude).or_else(|| std::env::var_os(Client::Claude.home_var()).filter(|v| !v.is_empty()).map(PathBuf::from));
    let legacy = dir.clone().or_else(|| user_home().map(|h| h.join(".claude")))?.join(".config.json");
    if legacy.is_file() { return Some(legacy); }
    Some(dir.or_else(user_home)?.join(".claude.json"))
}

/// Claude's own lock: a directory beside its configuration. A held lock is never broken.
struct DirLock(PathBuf);
impl Drop for DirLock { fn drop(&mut self) { let _ = std::fs::remove_dir(&self.0); } }

fn claude_lock(file: &Path, until: Instant) -> Result<DirLock, Failure> {
    let mut name = file.as_os_str().to_os_string();
    name.push(".lock");
    let path = PathBuf::from(name);
    loop {
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(DirLock(path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if Instant::now() >= until { return refused("trust-locked"); }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return refused("trust-write-failed"),
        }
    }
}

fn claude_config(path: &Path) -> Result<Value, Failure> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return refused("trust-config-missing"),
        Err(_) => return refused("trust-config-unreadable"),
    };
    let doc: Value = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)).or_else(|_| refused("trust-config-unreadable"))?;
    if !doc.is_object() || !(doc["projects"].is_null() || doc["projects"].is_object()) { return refused("trust-config-unreadable"); }
    Ok(doc)
}

/// Replace `target` with `text`, keeping its permissions. A sharing violation on Windows retries until `until`.
fn replace(target: &Path, text: &str, until: Instant) -> Result<(), Failure> {
    let mut name = target.as_os_str().to_os_string();
    name.push(format!(".agentq-trust-{}.tmp", std::process::id()));
    let tmp = PathBuf::from(name);
    let written = std::fs::write(&tmp, text).and_then(|_| std::fs::metadata(target)).and_then(|m| std::fs::set_permissions(&tmp, m.permissions()));
    if written.is_err() { let _ = std::fs::remove_file(&tmp); return refused("trust-write-failed"); }
    loop {
        match std::fs::rename(&tmp, target) {
            Ok(()) => return Ok(()),
            Err(_) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => { let _ = std::fs::remove_file(&tmp); return refused("trust-write-failed"); }
        }
    }
}

fn claude_trust(folder: &Path, scope: &AccountScope, until: Instant) -> Result<Trust, Failure> {
    let Some(text) = plain(folder) else { return refused("trust-path-refused") };
    let key = if cfg!(windows) { text.replace('\\', "/") } else { text };
    // ponytail: combining marks only; a crate for full NFC if a real path needs it. Claude looks up NFC, so such a path refuses.
    if key.chars().any(|c| matches!(c as u32, 0x300..=0x36f | 0x1100..=0x11ff)) { return refused("trust-path-refused"); }
    let Some(file) = claude_file(scope) else { return refused("trust-config-missing") };
    let accepted = |doc: &Value| doc["projects"][&key]["hasTrustDialogAccepted"] == true;
    if !file.exists() { return refused("trust-config-missing"); }
    let _lock = claude_lock(&file, until)?;
    let mut doc = claude_config(&file)?;
    if accepted(&doc) { return Ok(Trust::Unchanged); }
    let projects = doc.as_object_mut().expect("checked object").entry("projects").or_insert_with(|| json!({}));
    let entry = projects.as_object_mut().expect("checked object").entry(key.clone()).or_insert_with(|| json!({}));
    if !entry.is_object() { return refused("trust-config-unreadable"); }
    entry["hasTrustDialogAccepted"] = true.into();
    let target = file.canonicalize().or_else(|_| refused("trust-write-failed"))?;
    replace(&target, &format!("{}\n", serde_json::to_string_pretty(&doc).unwrap_or_default()), until)?;
    if accepted(&claude_config(&file)?) { Ok(Trust::Granted) } else { refused("trust-readback-failed") }
}

/// Codex stores the plain path, lowercased on Windows (`project_trust_key`).
fn codex_key(folder: &Path) -> Option<String> {
    let text = plain(folder)?;
    Some(if cfg!(windows) { text.to_ascii_lowercase() } else { text })
}

fn codex_level(reply: &Value, key: &str) -> Option<String> {
    let projects = reply["result"]["config"]["projects"].as_object()?;
    projects.iter().find(|(k, _)| k.as_str() == key || cfg!(windows) && k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v["trust_level"].as_str()).map(String::from)
}

fn codex_trust(folder: &Path, scope: &AccountScope, until: Instant) -> Result<Trust, Failure> {
    let Some(key) = codex_key(folder) else { return refused("trust-path-refused") };
    // A conflict held until the deadline is the client's writer staying busy, not a slow answer.
    let conflicted = std::cell::Cell::new(false);
    let outcome: Res<Result<Trust, &'static str>> = limits::codex_read(scope, &["--disable", "plugins"], until, |server, session, until| {
        let mut read = |server: &mut app::AppServer, session: &mut app::AppSession| -> Res<Value> {
            let request = session.request("config/read", json!({"includeLayers": true}))?;
            limits::exchange(server, session, request, until)
        };
        let mut granted = false;
        loop {
            let config = read(server, session)?;
            match codex_level(&config, &key).as_deref() {
                Some("trusted") => return Ok(Ok(if granted { Trust::Granted } else { Trust::Unchanged })),
                Some(_) if !granted => return Ok(Err("trust-conflict")),
                _ if granted => return Ok(Err("trust-readback-failed")),
                _ => {}
            }
            let version = config["result"]["layers"].as_array().into_iter().flatten().find(|l| l["name"]["type"] == "user").map(|l| l["version"].clone()).ok_or("config user layer missing")?;
            let request = session.request("config/batchWrite", json!({"edits": [{"keyPath": "projects", "mergeStrategy": "upsert", "value": {&key: {"trust_level": "trusted"}}}], "expectedVersion": version}))?;
            let reply = limits::exchange_raw(server, session, request, until)?;
            if reply["error"]["data"]["config_write_error_code"] == "configVersionConflict" {
                conflicted.set(true);
                if Instant::now() >= until { return Ok(Err("trust-locked")); }
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            if reply.get("error").is_some() { return Ok(Err("trust-write-failed")); }
            granted = true;
        }
    });
    match outcome {
        Ok(Ok(trust)) => Ok(trust),
        Ok(Err(cause)) => refused(cause),
        Err(_) if Instant::now() >= until => refused(if conflicted.get() { "trust-locked" } else { "trust-deadline" }),
        Err(_) => refused("trust-write-failed"),
    }
}

/// M12.15: trust one exact folder, under the client's own lock or writer, by `until`.
pub(super) fn trust_folder(client: Client, folder: &Path, scope: &AccountScope, until: Instant) -> Result<Trust, Failure> {
    if client == Client::Antigravity { return refused("trust-unsupported"); }
    let folder = exact_folder(client, folder, scope)?;
    if client == Client::Claude { claude_trust(&folder, scope, until) } else { codex_trust(&folder, scope, until) }
}

/// M4.11: the hooks Codex lists for `root`, read-only.
pub(super) fn hooks(root: &Path, scope: &AccountScope, until: Instant) -> Res<Vec<Hook>> {
    let reply = limits::codex_read(scope, &["--disable", "plugins"], until, |server, session, until| {
        let request = session.request("hooks/list", json!({"cwds": [root]}))?;
        limits::exchange(server, session, request, until)
    })?;
    let rows = reply["result"]["data"].as_array().ok_or("hooks/list data missing")?;
    Ok(rows.iter().flat_map(|row| row["hooks"].as_array().into_iter().flatten()).filter_map(|h| Some(Hook {
        key: h["key"].as_str()?.into(),
        event: h["eventName"].as_str()?.into(),
        command: h["command"].as_str().map(String::from),
        source_path: h["sourcePath"].as_str().map(PathBuf::from),
        hash: h["currentHash"].as_str().map(String::from),
        trust: h["trustStatus"].as_str().unwrap_or("unknown").into(),
        enabled: h["enabled"].as_bool().unwrap_or(false),
    })).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // M12.15. Risk: a caller trusts the whole home or disk, so every project there runs native hooks unasked.
    #[test]
    fn folder_trust_refuses_home_root_and_ancestors() {
        let home = user_home().unwrap().canonicalize().unwrap();
        let scope = AccountScope::default();
        let until = Instant::now() + Duration::from_secs(1);
        let root = home.ancestors().last().unwrap().to_path_buf();
        let parent = home.parent().unwrap().to_path_buf();
        for folder in [home.clone(), root, parent, PathBuf::from("relative"), home.join("agentq-absent-folder")] {
            for client in [Client::Claude, Client::Codex] {
                assert_eq!(exact_folder(client, &folder, &scope).err(), Some(Failure::Refused(Cause("trust-path-refused"))), "{folder:?}");
            }
        }
        assert_eq!(trust_folder(Client::Antigravity, &home, &scope, until).err(), Some(Failure::Refused(Cause("trust-unsupported"))));
    }

    // M12.15. Risk: the write interleaves with Claude's auth writer or drops its fields, losing the login.
    #[test]
    fn claude_trust_holds_the_lock_keeps_fields_and_refuses_with_named_causes() {
        let d = std::env::temp_dir().join(format!("agentq-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (home, folder) = (d.join("account"), d.join("project"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        let scope = AccountScope { home: Some(home.clone()), ..Default::default() };
        let soon = || Instant::now() + Duration::from_millis(150);
        let cause = |r: Result<Trust, Failure>| r.err();
        assert_eq!(cause(trust_folder(Client::Claude, &folder, &scope, soon())), Some(Failure::Refused(Cause("trust-config-missing"))));
        assert!(!home.join(".claude.json").exists(), "no global file is created");
        let file = home.join(".claude.json");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(cause(trust_folder(Client::Claude, &folder, &scope, soon())), Some(Failure::Refused(Cause("trust-config-unreadable"))));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "not json", "unreadable stays untouched");
        std::fs::write(&file, r#"{"oauthAccount":{"id":"invented"},"projects":{"/other":{"allowedTools":["x"]}}}"#).unwrap();
        let held = home.join(".claude.json.lock");
        std::fs::create_dir(&held).unwrap();
        assert_eq!(cause(trust_folder(Client::Claude, &folder, &scope, soon())), Some(Failure::Refused(Cause("trust-locked"))));
        assert!(held.is_dir(), "a held lock is never broken");
        assert!(!std::fs::read_to_string(&file).unwrap().contains("hasTrustDialogAccepted"), "no write without the lock");
        std::fs::remove_dir(&held).unwrap();
        assert_eq!(trust_folder(Client::Claude, &folder, &scope, soon()), Ok(Trust::Granted));
        assert!(!held.exists(), "the lock is released");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(doc["oauthAccount"]["id"], "invented");
        assert_eq!(doc["projects"]["/other"]["allowedTools"][0], "x");
        let canonical = plain(&folder.canonicalize().unwrap()).unwrap();
        let key = if cfg!(windows) { canonical.replace('\\', "/") } else { canonical };
        assert_eq!(doc["projects"][&key]["hasTrustDialogAccepted"], true);
        assert_eq!(trust_folder(Client::Claude, &folder, &scope, soon()), Ok(Trust::Unchanged));
        std::fs::remove_dir_all(&d).unwrap();
    }

    // M4.13, M12.17. Risk: a launch skips hook trust, or a key reaches a process listing.
    #[test]
    fn bypass_and_argv_secrets_refuse() {
        assert_eq!(refuse_bypass(&["--dangerously-bypass-hook-trust"]), Err("hook-trust-bypass-refused".into()));
        assert_eq!(refuse_bypass(&["--sandbox"]), Ok(()));
        for name in ["ANTHROPIC_API_KEY", "my_token", "X_SECRET", "DB_PASSWORD"] {
            assert_eq!(refuse_secret_settings(&[(name.into(), "invented".into())]), Err("secret-in-argv".into()), "{name}");
            assert_eq!(refuse_secret_settings(&[(name.into(), String::new())]), Ok(()), "clearing is no secret");
        }
        assert_eq!(refuse_secret_settings(&[("AGENTQ_OPTIMIZATION".into(), "off".into())]), Ok(()));
        assert_eq!(refuse_secret_args(&["-c".into(), "shell_environment_policy.set.X_TOKEN=invented".into()]), Err("secret-in-argv".into()));
        assert_eq!(refuse_secret_args(&["-c".into(), "model_reasoning_effort=high".into()]), Ok(()));
    }

    // M12.17. Risk: a parent's key or session reaches the child, or a caller's explicit value is lost.
    #[test]
    fn clean_scrubs_inherited_and_keeps_a_callers_value() {
        let mut c = Command::new("x");
        c.env("OPENAI_API_KEY", "invented");
        clean(&mut c);
        let value = |k: &str| c.get_envs().find(|(n, _)| *n == k).map(|(_, v)| v.map(OsString::from));
        assert_eq!(value("ANTHROPIC_API_KEY"), Some(None), "removed from the child");
        assert_eq!(value("CLAUDE_CODE_SESSION_ID"), Some(None));
        assert_eq!(value("OPENAI_API_KEY"), Some(Some("invented".into())), "the caller's own base value stays");
        assert_eq!(value("GIT_TERMINAL_PROMPT"), Some(Some("0".into())));
        assert_eq!(value("GCM_INTERACTIVE"), Some(Some("never".into())));
    }

    #[test]
    fn codex_key_is_plain_and_lowercased_on_windows() {
        if cfg!(windows) {
            assert_eq!(codex_key(Path::new(r"\\?\C:\Users\A\Proj")).unwrap(), r"c:\users\a\proj");
            assert_eq!(codex_key(Path::new(r"\\?\UNC\Host\Share")).unwrap(), r"\\?\unc\host\share");
        } else {
            assert_eq!(codex_key(Path::new("/Users/A/Proj")).unwrap(), "/Users/A/Proj");
        }
    }
}
