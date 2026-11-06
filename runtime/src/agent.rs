//! Claude Code, Codex CLI and Antigravity turns for every consumer (agentq.md § 12, docs/spec/RUNTIME.md; moved from CoreQ v0.18.0).
//! One native session per conversation or attempt; each turn is one client process resumed by session ID.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod history;
mod native;
mod app;
mod protected;
pub mod host;
mod adapter;
mod capture;
mod events;
mod limits;
pub use events::{Event, LogRecord, Model, RecordTime, TimeFault, ToolKind};
pub use limits::Window;
pub use capture::{APPEND_CHUNK, Call, ProcessUse, SessionTurn, append_records, session_call, session_call_records, session_records};
pub(crate) use capture::with_environment_caller;
pub use adapter::{Account, Approval, Builtin, Cause, Check, Failure, Login, Part, Configure, Descriptor, Launch, Mode, NativeSpec, Network, Observation, Policy, Process, Quota, Ready, Roots, Run, Runtime, Scope, Spawned, State, runtime, runtimes};
pub use native::BackgroundJob;
pub use protected::{ProtectedHost, ProtectedIdentity, ProtectedPreparation, ProtectedAppServer};

type Res<T> = Result<T, String>;

/// A supported client: Claude Code (`claude -p`), Codex CLI (`codex exec`), or Antigravity (`agentapi` / `agy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Client { Claude, Codex, Antigravity }

impl Client {
    pub fn parse(code: &str) -> Option<Client> {
        match code {
            "claude" => Some(Client::Claude),
            "codex" => Some(Client::Codex),
            "antigravity" | "agy" => Some(Client::Antigravity),
            _ => None,
        }
    }
    /// The runtime code in observations.
    pub fn code(self) -> &'static str {
        match self {
            Client::Claude => "claude",
            Client::Codex => "codex",
            Client::Antigravity => "antigravity",
        }
    }
    /// The variable that names a login home to the client.
    pub fn home_var(self) -> &'static str {
        match self {
            Client::Claude => "CLAUDE_CONFIG_DIR",
            Client::Codex => "CODEX_HOME",
            Client::Antigravity => "ANTIGRAVITY_HOME",
        }
    }
    fn home_dir(self) -> &'static str {
        match self {
            Client::Claude => ".claude",
            Client::Codex => ".codex",
            Client::Antigravity => ".gemini/antigravity",
        }
    }
}

// ---------- executable ----------

/// `name` on `path`; on Windows also `name.exe` and `name.cmd` (npm installs `.cmd` shims). Unresolved: the bare name.
pub fn which_in(name: &str, path: &OsStr) -> PathBuf {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ""] } else { &[""] };
    for dir in std::env::split_paths(path) {
        for e in exts {
            let p = dir.join(format!("{name}{e}"));
            if p.is_file() { return p; }
        }
    }
    PathBuf::from(name)
}

/// `PATH` with `<home>/.local/bin` appended when it is a directory `PATH` lacks; `None` when nothing changes.
/// Linux user installs (codex, npm prefixes, pipx) land there, but a non-login shell or a service unit may lack it.
pub fn user_bin_path(path: &OsStr, home: Option<&OsStr>) -> Option<OsString> {
    let bin = Path::new(home.filter(|h| !h.is_empty())?).join(".local/bin");
    let mut dirs: Vec<PathBuf> = std::env::split_paths(path).collect();
    if !bin.is_dir() || dirs.contains(&bin) { return None; }
    dirs.push(bin);
    std::env::join_paths(dirs).ok()
}

fn antigravity_executable(path: &OsStr) -> PathBuf {
    if let Some(path) = std::env::var_os("ANTIGRAVITY_AGENTAPI_EXE").filter(|s| !s.is_empty()) {
        return PathBuf::from(path);
    }
    let path = path.to_os_string();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let path = user_bin_path(&path, home.as_deref()).unwrap_or(path);
    let cli = which_in("agy", &path);
    if cli.is_absolute() { return cli; }
    let api = which_in("agentapi", &path);
    if api.is_absolute() { return api; }
    if let Some(home) = base_home(Client::Antigravity) {
        let p = home.join("bin/agentapi");
        if p.is_file() {
            return p;
        }
    }
    which_in("agy", &path)
}

/// The native Codex executor behind an npm `codex.js` launcher: a kill cannot reach the native child through Node.
pub(crate) fn codex_native(bin: PathBuf) -> Res<PathBuf> {
    if cfg!(windows) || !bin.exists() { return Ok(bin); }
    let bin = bin.canonicalize().map_err(|e| format!("Codex executable unavailable: {e}"))?;
    let native = |path: &Path| -> Res<bool> {
        let mut magic = [0; 4];
        File::open(path).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| format!("Codex executable unreadable: {e}"))?;
        Ok(matches!(magic, [0x7f, b'E', b'L', b'F'] | [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf] | [0xce, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xce] | [0xca, 0xfe, 0xba, 0xbe] | [0xbe, 0xba, 0xfe, 0xca]))
    };
    if native(&bin)? { return Ok(bin); }
    let package = bin.parent().filter(|p| p.file_name().is_some_and(|n| n == "bin")).and_then(Path::parent)
        .filter(|_| bin.file_name().is_some_and(|n| n == "codex.js"))
        .ok_or("Codex launcher ownership unsupported; select a native Codex executable")?;
    let metadata: Value = serde_json::from_reader(File::open(package.join("package.json")).map_err(|e| format!("Codex package unavailable: {e}"))?.take(64 * 1024))
        .map_err(|e| format!("Codex package invalid: {e}"))?;
    if metadata["name"] != "@openai/codex" { return Err("Codex launcher package identity differs".into()); }
    let (platform, target) = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("darwin-arm64", "aarch64-apple-darwin"),
        ("macos", "x86_64") => ("darwin-x64", "x86_64-apple-darwin"),
        ("linux", "aarch64") => ("linux-arm64", "aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => ("linux-x64", "x86_64-unknown-linux-musl"),
        _ => return Err("Codex native package unsupported on this platform".into()),
    };
    // Match Node's package lookup along the entrypoint's ancestors; never scan other installations.
    let installed = package.ancestors().map(|p| p.join(format!("node_modules/@openai/codex-{platform}")))
        .find(|p| p.join("package.json").is_file()).unwrap_or_else(|| package.to_path_buf());
    let executable = installed.join(format!("vendor/{target}/bin/codex"));
    if !native(&executable)? { return Err("Codex package executor is not a native binary".into()); }
    executable.canonicalize().map_err(|e| format!("Codex native executable unavailable: {e}"))
}

/// The client's CLI executable: `claude` on `PATH`, Codex's native executor, or Antigravity's `agy` / `agentapi` CLI. A caller
/// that wraps the client (a sandbox) builds its own base command around this path.
pub(crate) fn executable(client: Client) -> Res<PathBuf> { executable_in(client, &std::env::var_os("PATH").unwrap_or_default()) }

/// `executable` resolved on `path` instead of the process `PATH`.
pub(crate) fn executable_in(client: Client, path: &OsStr) -> Res<PathBuf> {
    match client {
        Client::Claude => Ok(which_in("claude", path)),
        Client::Codex => codex_native(which_in("codex", path)),
        Client::Antigravity => Ok(antigravity_executable(path)),
    }
}

// ---------- accounts ----------

/// The client's own login home account.
pub const DEFAULT: &str = "default";
/// A limit without a reported reset ends this many seconds after it was seen.
pub const PROBE: i64 = 1800;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry { pub label: String, pub enabled: bool }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    /// When the limit was seen (unix seconds).
    pub at: i64,
    /// The reported reset, when the client named one.
    pub until: Option<i64>,
}

impl Limit {
    /// When the limit ends: the reported reset, else the probe after it.
    pub fn end(&self) -> i64 { self.until.unwrap_or(self.at + PROBE) }
}

/// An account label: 1-32 of `[a-z0-9-]`, not starting with `-`.
pub fn valid_label(l: &str) -> bool {
    (1..=32).contains(&l.len()) && !l.starts_with('-') && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One client's ordered accounts and their recorded limits; the caller owns where both are stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Accounts { pub list: Vec<Entry>, pub limits: BTreeMap<String, Limit> }

impl Accounts {
    /// The list in priority order; an empty list is one enabled `default` account.
    pub fn entries(&self) -> Vec<Entry> {
        if self.list.is_empty() { vec![Entry { label: DEFAULT.into(), enabled: true }] } else { self.list.clone() }
    }
    fn free(&self, label: &str, now: i64) -> bool { self.limits.get(label).is_none_or(|l| l.end() <= now) }
    /// The first enabled account whose limit has ended.
    pub fn active(&self, now: i64) -> Option<String> {
        self.entries().into_iter().find(|e| e.enabled && self.free(&e.label, now)).map(|e| e.label)
    }
    /// The account a new start takes: the active one, else the first enabled one; None when all are disabled.
    pub fn pick(&self, now: i64) -> Option<String> {
        self.active(now).or_else(|| self.entries().into_iter().find(|e| e.enabled).map(|e| e.label))
    }
    /// More than one account: failover applies.
    pub fn multi(&self) -> bool { self.entries().len() > 1 }
    /// The account a usage limit moves to: only with more than one account.
    pub fn spare(&self, now: i64) -> Option<String> { self.multi().then(|| self.active(now)).flatten() }
    /// With none left: the earliest live limit end of an enabled account and whether it was reported.
    pub fn hold(&self, now: i64) -> Option<(i64, bool)> {
        self.entries().iter().filter(|e| e.enabled).filter_map(|e| self.limits.get(&e.label)).filter(|l| l.end() > now)
            .map(|l| (l.end(), l.until.is_some())).min()
    }
    /// Every account disabled: the client admits nothing.
    pub fn off(&self) -> bool { self.entries().iter().all(|e| !e.enabled) }
}

/// The client's own home: `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `ANTIGRAVITY_HOME` (or `ANTIGRAVITY_APP_DATA_DIR`)
/// of this process, else `~/.claude` / `~/.codex` / `~/.gemini/antigravity-cli` when present, else the legacy
/// `~/.gemini/antigravity` (a Windows service has no `HOME`, so `USERPROFILE`).
pub(crate) fn base_home(client: Client) -> Option<PathBuf> {
    std::env::var_os(client.home_var()).filter(|v| !v.is_empty()).map(PathBuf::from)
        .or_else(|| (client == Client::Antigravity).then(|| std::env::var_os("ANTIGRAVITY_APP_DATA_DIR")).flatten().filter(|v| !v.is_empty()).map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| {
            let cli = Path::new(&h).join(".gemini/antigravity-cli");
            if client == Client::Antigravity && cli.is_dir() { cli } else { Path::new(&h).join(client.home_dir()) }
        }))
}

/// `<base home>/<tool>-accounts/<label>` for a non-default account; None for `default` (the child keeps the
/// caller's own home). `tool` is the consumer's `Tool.name`.
pub(crate) fn account_home(client: Client, tool: &str, label: &str) -> Option<PathBuf> {
    (label != DEFAULT && valid_label(label)).then(|| base_home(client).map(|b| b.join(format!("{tool}-accounts")).join(label))).flatten()
}

// ---------- start and resume ----------

/// A new session ID: Claude takes a caller-chosen UUID; Codex and Antigravity name their session in output (`-` until then).
pub(crate) fn new_session(client: Client) -> String {
    match client { Client::Claude => crate::incident::uuid(), Client::Codex | Client::Antigravity => "-".into() }
}

/// One turn of one session.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    pub client: Client,
    /// The project root: working directory of the client and Codex `-C` on a new start.
    pub root: PathBuf,
    /// The native session ID: `--session-id` / `--resume` (Claude), `resume <thread>` (Codex), `send-message <conversationId>` (Antigravity).
    pub session: String,
    /// Continue `session` instead of starting it.
    pub resume: bool,
    /// Caller-owned client arguments in TaskQ's position: Claude permission mode and `--settings`, Codex sandbox flags, Antigravity model/title flags.
    pub args: Vec<OsString>,
    /// The account's login home (`account_home`); None keeps the caller's own.
    pub home: Option<PathBuf>,
    /// Extra child environment; values never enter argv or the log.
    pub env: Vec<(OsString, OsString)>,
}

/// The last child override wins, matching `Command::envs`.
pub(crate) fn child_env(spec: &Spec, key: &str) -> Option<OsString> {
    spec.env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.clone()).or_else(|| std::env::var_os(key))
}

pub(crate) fn spec_home(spec: &Spec) -> Option<PathBuf> {
    spec.home.clone().or_else(|| child_env(spec, spec.client.home_var()).filter(|v| !v.is_empty()).map(PathBuf::from))
        .or_else(|| (spec.client == Client::Antigravity).then(|| child_env(spec, "ANTIGRAVITY_APP_DATA_DIR")).flatten().filter(|v| !v.is_empty()).map(PathBuf::from))
        .or_else(|| base_home(spec.client))
}

/// The client argument list after the executable, in TaskQ's order.
pub(crate) fn args(spec: &Spec) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    let s = |v: &str| OsString::from(v);
    match spec.client {
        Client::Claude => {
            a.extend(["-p", "--output-format", "stream-json", "--verbose"].map(s));
            a.extend(spec.args.iter().cloned());
            a.push(s(if spec.resume { "--resume" } else { "--session-id" }));
            a.push(s(&spec.session));
        }
        Client::Codex => {
            a.extend(["exec", "--json"].map(s));
            a.extend(spec.args.iter().cloned());
            if spec.resume { a.extend([s("resume"), s(&spec.session), s("-")]); }
            else { a.extend([s("-C"), spec.root.clone().into_os_string(), s("-")]); }
        }
        Client::Antigravity => {
            a.extend(["--input-format", "stream-json", "--output-format", "stream-json"].map(s));
            a.extend(spec.args.iter().cloned());
            if spec.resume { a.extend([s("--conversation"), s(&spec.session)]); }
        }
    }
    a
}

/// `base` (the client executable, or a sandbox wrapper around it) with the turn's arguments and environment. The
/// inherited session variables of an enclosing client are removed so a child never joins its parent's session.
pub(crate) fn command(mut base: Command, spec: &Spec) -> Command {
    base.args(args(spec));
    environment(base, spec)
}

fn environment(mut base: Command, spec: &Spec) -> Command {
    base.current_dir(&spec.root).env_remove("CLAUDE_CODE_SESSION_ID").env_remove("CODEX_THREAD_ID")
        .env_remove("ANTIGRAVITY_CONVERSATION_ID")
        .envs(spec.env.iter().map(|(k, v)| (k, v)));
    if let Some(home) = &spec.home { base.env(spec.client.home_var(), home); }
    crate::quiet(&mut base);
    base
}

/// Claude `--settings` JSON: a `SessionStart` hook on `compact` that runs `compact_hook` (rule re-delivery after
/// compaction) and `env` for the session, which survives resume. None omits the hook.
pub(crate) fn claude_settings(compact_hook: Option<&str>, env: &[(&str, &str)]) -> String {
    let env: serde_json::Map<String, Value> = env.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    let mut v = json!({"env": env});
    if let Some(cmd) = compact_hook {
        v = json!({"hooks": {"SessionStart": [{"matcher": "compact", "hooks": [{"type": "command", "command": cmd}]}]}, "env": v["env"]});
    }
    v.to_string()
}

// ---------- running turn ----------

#[derive(Clone, Debug, Default)]
struct AntigravityTranscript {
    native_agentapi: bool,
    session: Option<String>,
    cursor: u64,
    identity: Option<String>,
    prefix: Option<String>,
    partial: Vec<u8>,
    pending: std::collections::VecDeque<Value>,
    full_step: Option<u64>,
    full_cursor: u64,
    full_partial: Vec<u8>,
    full_identity: Option<String>,
    full_prefix: Option<String>,
}

fn is_native_agentapi(cmd: &Command, spec: &Spec) -> bool {
    if spec.client != Client::Antigravity {
        return false;
    }
    std::iter::once(cmd.get_program()).chain(cmd.get_args()).any(|arg| {
        matches!(Path::new(arg).file_name().and_then(|n| n.to_str()),
            Some("agentapi" | "agentapi.exe" | "agentapi.cmd" | "language_server" | "language_server.exe"))
    })
}

fn transcript_paths(home: &Path, session: &str) -> (PathBuf, PathBuf) {
    let logs = home.join("brain").join(session).join(".system_generated").join("logs");
    (logs.join("transcript.jsonl"), logs.join("transcript_full.jsonl"))
}

impl AntigravityTranscript {
    fn baseline(home: Option<&Path>, session: Option<String>) -> Res<Self> {
        let mut state = Self { native_agentapi: true, session, ..Self::default() };
        if let Some(session) = state.session.as_deref() {
            if !native::plain_id(session) || session == "-" { return Err("antigravity session invalid".into()); }
            let home = home.ok_or("antigravity home unavailable")?;
            let mut file = File::open(transcript_paths(home, session).0).map_err(|e| format!("antigravity baseline: {e}"))?;
            state.cursor = file.metadata().map_err(|e| e.to_string())?.len();
            if state.cursor > 0 {
                file.seek(SeekFrom::Start(state.cursor-1)).map_err(|e| e.to_string())?;
                let mut last = [0]; file.read_exact(&mut last).map_err(|e| e.to_string())?;
                if last != [b'\n'] { return Err("antigravity baseline incomplete".into()); }
            }
            state.identity = Some(native::file_id(&file)?);
            state.prefix = Some(native::digest_prefix(&mut file, state.cursor)?);
        }
        Ok(state)
    }

    fn full_record(&mut self, path: &Path, step: u64) -> Res<Option<Value>> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.full_identity.is_none() => return Ok(None),
            Err(e) => return Err(format!("antigravity full transcript: {e}")),
        };
        if self.full_step != Some(step) {
            self.full_step = Some(step); self.full_cursor = 0; self.full_partial.clear();
            self.full_identity = None; self.full_prefix = None;
        }
        let identity = native::file_id(&file)?;
        if self.full_identity.as_ref().is_some_and(|id| *id != identity) { return Err("antigravity full transcript replaced".into()); }
        let prefix = native::digest_prefix(&mut file, self.full_cursor)?;
        if self.full_prefix.as_ref().is_some_and(|p| *p != prefix) { return Err("antigravity full transcript rewritten".into()); }
        file.seek(SeekFrom::Start(self.full_cursor)).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        (&mut file).take(8*1024*1024).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        self.full_cursor += bytes.len() as u64;
        self.full_partial.extend(bytes);
        self.full_identity = Some(identity);
        self.full_prefix = Some(native::digest_prefix(&mut file, self.full_cursor)?);
        let mut consumed = 0;
        let mut found = None;
        for row in self.full_partial.split_inclusive(|b| *b == b'\n') {
            if !row.ends_with(b"\n") { break; }
            consumed += row.len();
            let v: Value = serde_json::from_slice(row).map_err(|_| "antigravity full transcript invalid JSON")?;
            if !v.is_object() { return Err("antigravity full transcript invalid record".into()); }
            if v["step_index"].as_u64() == Some(step) && v.get("truncated_fields").is_none() { found = Some(v); }
        }
        self.full_partial.drain(..consumed);
        Ok(found)
    }

    fn append(&mut self, home: &Path, log: &Path) -> Res<()> {
        let Some(session) = self.session.as_deref() else { return Ok(()); };
        if !native::plain_id(session) || session == "-" { return Err("antigravity session invalid".into()); }
        let (compact, full) = transcript_paths(home, session);
        let mut file = match File::open(compact) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && self.identity.is_none() => return Ok(()),
            Err(e) => return Err(format!("antigravity transcript: {e}")),
        };
        let identity = native::file_id(&file)?;
        if self.identity.as_ref().is_some_and(|id| *id != identity) { return Err("antigravity transcript replaced".into()); }
        let prefix = native::digest_prefix(&mut file, self.cursor)?;
        if self.prefix.as_ref().is_some_and(|p| *p != prefix) { return Err("antigravity transcript rewritten".into()); }
        file.seek(SeekFrom::Start(self.cursor)).map_err(|e| e.to_string())?;
        let mut bytes = vec![];
        (&mut file).take(8*1024*1024).read_to_end(&mut bytes).map_err(|e| format!("antigravity transcript: {e}"))?;
        self.cursor += bytes.len() as u64;
        self.partial.extend(bytes);
        let mut consumed = 0;
        for row in self.partial.split_inclusive(|b| *b == b'\n') {
            if !row.ends_with(b"\n") { break; }
            consumed += row.len();
            let value: Value = serde_json::from_slice(row).map_err(|_| "antigravity transcript invalid JSON")?;
            if !value.is_object() { return Err("antigravity transcript invalid record".into()); }
            self.pending.push_back(value);
        }
        self.partial.drain(..consumed);
        self.identity = Some(identity);
        self.prefix = Some(native::digest_prefix(&mut file, self.cursor)?);
        let mut output = File::options().append(true).open(log).map_err(|e| e.to_string())?;
        while let Some(mut value) = self.pending.front().cloned() {
            if value.get("truncated_fields").is_some() {
                let step = value["step_index"].as_u64().ok_or("antigravity truncated step missing index")?;
                let Some(v) = self.full_record(&full, step)? else { break; };
                if v["type"] != value["type"] || v["status"] != value["status"] { break; }
                value = v;
            }
            writeln!(output, "{value}").map_err(|e| format!("agent log: {e}"))?;
            self.pending.pop_front();
        }
        Ok(())
    }
}

/// The turn's client process: the owned child, or after recovery the child observed by its native identity.
enum Proc { Own(Child), Seen(crate::process::Identity) }

impl Proc {
    fn id(&self) -> u32 { match self { Proc::Own(c) => c.id(), Proc::Seen(i) => i.pid } }

    /// The exit once observed: Some(code) for an owned child, Some(None) when a recovered child's incarnation is gone.
    fn try_wait(&mut self) -> Res<Option<Option<i32>>> {
        match self {
            Proc::Own(c) => Ok(c.try_wait().map_err(|e| format!("agent wait: {e}"))?.map(|s| Some(s.code().unwrap_or(-1)))),
            Proc::Seen(i) => {
                let now = crate::process::identity(i.pid);
                if now.as_ref().is_ok_and(|n| n.as_ref() != Some(&*i)) { return Ok(Some(None)); }
                // A re-executed parent keeps its children: reap our own exited child and read its code.
                // Our own child still running (an exiting process may have no identity yet) reads running.
                #[cfg(unix)]
                match reap(i.pid) { Some(Some(code)) => return Ok(Some(Some(code))), Some(None) => return Ok(None), None => {} }
                now.map(|_| None).map_err(|e| format!("agent process identity unknown: {e}"))
            }
        }
    }

    /// Request the stop; a recovered child is signalled only through its identity, never by a reused PID.
    fn kill(&mut self) -> Res<()> {
        match self {
            Proc::Own(c) => c.kill().map_err(|e| format!("agent kill: {e}")),
            Proc::Seen(i) => match crate::process::terminate(i) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("agent kill: {e}")),
                _ => Ok(()),
            },
        }
    }
}

/// Our own child `pid`: Some(Some(code)) once reaped, Some(None) while it runs; None when it is not our child.
#[cfg(unix)]
fn reap(pid: u32) -> Option<Option<i32>> {
    let mut status = 0;
    // SAFETY: WNOHANG never blocks; the call only reaps a child of this process.
    match unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) } {
        0 => Some(None),
        r if r == pid as libc::pid_t => Some(Some(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 })),
        _ => None,
    }
}

/// One running turn: the client process with stdout and stderr appended to the caller's log.
pub struct Turn {
    client: Client,
    /// The caller cancelled the run.
    cancelled: bool,
    child: Proc,
    /// The exit was not observed (a recovered child), so `exit` holds no code.
    exit_unknown: bool,
    /// `native::scope` of the launch, bound into a checkpoint.
    scope: String,
    log: PathBuf,
    start: u64,
    pos: u64,
    ended: bool,
    stopping: bool,
    timed_out: bool,
    started: Instant,
    started_ms: i64,
    exit: Option<i32>,
    home: Option<PathBuf>,
    /// The resumed session; a resume's output need not name it again.
    resumed: Option<String>,
    ag: AntigravityTranscript,
    receipt: Option<std::sync::mpsc::Receiver<Res<String>>>,
    /// Samples the owned process tree; None for a recovered child or a sidecar submission.
    sampler: Option<capture::Sampler>,
}

fn unix_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl Turn {
    /// Spawn `command(base, spec)` with `text` on stdin; output appends to `log`, so a resume continues earlier turns.
    pub(crate) fn start(mut base: Command, spec: &Spec, log: &Path, text: &str, process: &Process) -> Res<Turn> {
        if let Some(dir) = log.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("agent log directory: {e}"))?; }
        let out = File::options().create(true).append(true).open(log).map_err(|e| format!("agent log: {e}"))?;
        let err = out.try_clone().map_err(|e| format!("agent log: {e}"))?;
        let start = out.metadata().map_err(|e| format!("agent log: {e}"))?.len();
        let home = spec_home(spec);
        let native_agentapi = is_native_agentapi(&base, spec);
        let ag = if native_agentapi {
            if std::iter::once(base.get_program()).chain(base.get_args()).any(|arg| Path::new(arg).file_stem().is_some_and(|n| n == "language_server")) {
                base.arg("agentapi");
            }
            base.arg(if spec.resume {"send-message"} else {"new-conversation"}).args(&spec.args);
            if spec.resume { base.arg(&spec.session); }
            base.arg(text);
            base = environment(base, spec);
            let off = base.get_envs().find(|(k,_)| *k == crate::optimization::ENV)
                .map_or_else(|| std::env::var_os(crate::optimization::ENV), |(_,v)| v.map(OsString::from));
            if off.is_some_and(|v| v == "off") {
                return Err("antigravity agentapi cannot propagate optimization off to an existing server; use agy".into());
            }
            let session = spec.resume.then(|| spec.session.clone());
            AntigravityTranscript::baseline(home.as_deref(), session)?
        } else {
            base = command(base, spec);
            AntigravityTranscript::default()
        };
        capture::inherit_caller(&mut base);
        let started = Instant::now();
        let started_ms = unix_ms();
        base.stdin(Stdio::piped()).stdout(if native_agentapi {Stdio::piped()} else {Stdio::from(out)}).stderr(err);
        let mut child = process.spawn(base, spec.client.code())?;
        let sampler = (!native_agentapi).then(|| capture::Sampler::start(child.id()));
        let receipt = if native_agentapi {
            let mut stdout = child.stdout.take().ok_or("agentapi stdout unavailable")?;
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut text = String::new();
                let result = stdout.read_to_string(&mut text).map(|_| text).map_err(|e| format!("agentapi receipt: {e}"));
                let _ = send.send(result);
            });
            Some(receive)
        } else { None };
        let mut stdin = child.stdin.take().ok_or("agent stdin unavailable")?;
        if !native_agentapi {
            let input = if spec.client == Client::Antigravity {format!("{}\n", json!({"event":"user","message":{"content":text}})).into_bytes()} else {text.as_bytes().to_vec()};
            // A client that never reads stdin must not block the caller; the writer ends with the pipe.
            std::thread::spawn(move || { let _ = stdin.write_all(&input); });
        } else {
            drop(stdin);
        }
        Ok(Turn { client: spec.client, cancelled: false, child: Proc::Own(child), exit_unknown: false, scope: native::scope(spec), log: log.to_path_buf(), start, pos: start,
            ended: false, stopping: false, timed_out: false, started, started_ms, exit: None, home, resumed: spec.resume.then(|| spec.session.clone()), ag, receipt, sampler })
    }

    pub fn id(&self) -> u32 { self.child.id() }

    /// Private recovery value of a running turn (docs/spec/RUNTIME.md § Turn recovery and process setup).
    pub(crate) fn checkpoint(&self) -> Result<Value, adapter::Cause> {
        if self.ag.native_agentapi { return Err(adapter::Cause("no-checkpoint")); }
        if self.exit.is_some() { return Err(adapter::Cause("turn-ended")); }
        let identity = match &self.child {
            Proc::Seen(i) => i.clone(),
            Proc::Own(c) => crate::process::identity(c.id()).ok().flatten().ok_or(adapter::Cause("identity-unknown"))?,
        };
        Ok(json!({"pid": identity.pid, "started": identity.started, "client": self.client.code(), "log": self.log, "start": self.start,
            "started_ms": self.started_ms, "resumed": self.resumed, "cancelled": self.cancelled, "timed_out": self.timed_out, "scope": self.scope}))
    }

    /// An observation-only turn rebuilt from `checkpoint` for `spec` and `log`: no process starts and no input is sent.
    pub(crate) fn recover(spec: &Spec, log: &Path, saved: &Value) -> Res<Turn> {
        if saved["client"] != spec.client.code() || saved["log"] != json!(log) || saved["scope"] != native::scope(spec) {
            return Err("turn recovery scope differs".into());
        }
        let invalid = || "turn recovery state invalid".to_string();
        let pid = saved["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).ok_or_else(invalid)?;
        let identity = crate::process::Identity { pid, started: saved["started"].as_str().ok_or_else(invalid)?.into() };
        let start = saved["start"].as_u64().ok_or_else(invalid)?;
        let started_ms = saved["started_ms"].as_i64().filter(|s| *s >= 0).ok_or_else(invalid)?;
        let flag = |k: &str| saved[k].as_bool().ok_or_else(invalid);
        let resumed = match &saved["resumed"] { Value::Null => None, v => Some(v.as_str().filter(|s| native::plain_id(s)).ok_or_else(invalid)?.to_string()) };
        let age = Duration::from_millis(unix_ms().saturating_sub(started_ms).max(0) as u64);
        Ok(Turn { client: spec.client, cancelled: flag("cancelled")?, child: Proc::Seen(identity), exit_unknown: false, scope: native::scope(spec), log: log.to_path_buf(),
            start, pos: start, ended: false, stopping: false, timed_out: flag("timed_out")?, started: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
            started_ms, exit: None, home: spec_home(spec), resumed, ag: AntigravityTranscript::default(), receipt: None, sampler: None })
    }

    fn sync_antigravity_transcript(&mut self) -> Res<()> {
        if !self.ag.native_agentapi {
            return Ok(());
        }
        if self.exit.is_none() { return Ok(()); }
        if let Some(receive) = &self.receipt {
            let text = match receive.try_recv() {
                Ok(result) => result?,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Ok(()),
                Err(_) => return Err("agentapi receipt reader disconnected".into()),
            };
            let session = session_of(Client::Antigravity, &text).or_else(|| self.resumed.clone());
            if self.resumed.is_some() && session != self.resumed { return Err("antigravity receipt session differs".into()); }
            self.ag.session = session;
            let mut log = File::options().append(true).open(&self.log).map_err(|e| e.to_string())?;
            let receipt = serde_json::from_str::<Value>(&text).map(|v| v.to_string()).unwrap_or(text);
            writeln!(log, "\n{receipt}").map_err(|e| e.to_string())?;
            self.receipt = None;
        }
        if self.exit == Some(0) {
            if let Some(home) = self.home.as_deref() { self.ag.append(home, &self.log)?; }
        }
        Ok(())
    }

    /// Read complete new log lines; a turn-end event ends the turn even when the process lingers.
    fn read(&mut self) -> Res<()> {
        self.sync_antigravity_transcript()?;
        let mut f = File::open(&self.log).map_err(|e| format!("agent log unreadable: {e}"))?;
        if f.metadata().map_err(|e| e.to_string())?.len() < self.pos { return Err("agent log truncated; turn boundary unknown".into()); }
        let mut rest = vec![];
        f.seek(SeekFrom::Start(self.pos)).and_then(|_| f.read_to_end(&mut rest)).map_err(|e| format!("agent log read: {e}"))?;
        let Some(end) = rest.iter().rposition(|b| *b == b'\n') else { return Ok(()) };
        self.pos += end as u64 + 1;
        for v in String::from_utf8_lossy(&rest[..end]).lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            if (v["type"] == "PLANNER_RESPONSE" && !ends(&v)) || v["type"] == "TOOL_RESPONSE" {
                self.ended = false;
            } else if ends(&v) {
                self.ended = true;
            }
        }
        Ok(())
    }

    /// The exit code once the process exited, else None. After a turn-end event a lingering process is stopped
    /// (Codex on Windows kept running after its turn ended).
    pub fn poll(&mut self) -> Res<Option<i32>> {
        if self.ag.native_agentapi {
            if self.exit.is_none() {
                self.exit = self.child.try_wait()?.map(|c| c.unwrap_or(-1));
            }
            self.read()?;
            return Ok(self.exit.filter(|code| self.stopping || (self.receipt.is_none() && (*code != 0 || self.ag.session.is_none() || self.ended))));
        }
        if self.exit.is_some() { return Ok(self.exit); }
        self.read()?;
        if self.exit.is_none() {
            if let Some(code) = self.child.try_wait()? {
                self.exit_unknown = code.is_none();
                let code = code.unwrap_or(-1);
                self.read()?;
                self.exit = Some(code);
                return Ok(self.exit);
            }
        }
        if self.ended && !self.stopping { self.stopping = true; let _ = self.child.kill(); }
        Ok(None)
    }

    /// Request termination; `poll` confirms the exit.
    pub fn kill(&mut self) -> Res<()> {
        self.stopping = true;
        if self.exit.is_some() {
            return Ok(());
        }
        self.child.kill()
    }

    /// Poll until exit; at `deadline` stop the process, and the turn reads as timed out.
    pub fn wait(&mut self, deadline: Option<Instant>) -> Res<i32> {
        loop {
            if let Some(code) = self.poll()? { return Ok(code); }
            if deadline.is_some_and(|d| Instant::now() >= d) && !self.timed_out {
                self.timed_out = true;
                let _ = self.kill();
                if self.ag.native_agentapi {
                    if let Some(code) = self.exit {
                        return Ok(code);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Immutable identity of this turn's `agent.call` records: recovery rebuilds the same one.
    pub(crate) fn key(&self) -> String {
        let session = session_of(self.client, &read_from(&self.log, self.start).unwrap_or_default()).or_else(|| self.ag.session.clone()).or_else(|| self.resumed.clone());
        crate::sha256_hex(format!("{}\0{}\0{}\0{}\0{}", self.client.code(), session.as_deref().unwrap_or(""), self.log.display(), self.start, self.started_ms).as_bytes())
    }

    /// The turn's report from its log bytes; call after `poll` returned an exit.
    pub fn report(&self) -> Report {
        let text = read_from(&self.log, self.start).unwrap_or_default();
        let events: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let session = session_of(self.client, &text).or_else(|| self.ag.session.clone()).or_else(|| self.resumed.clone());
        let rollout = match self.client {
            Client::Codex => session.as_deref().and_then(|t| rollout(t, self.home.as_deref())),
            Client::Claude | Client::Antigravity => None,
        };
        let mut r = Report::read(self.client, &text, &events, rollout.as_deref());
        r.session = session;
        r.duration_us = self.started.elapsed().as_micros() as u64;
        r.started_ms = self.started_ms;
        r.exit = if self.exit_unknown { None } else { self.exit };
        r.process = match &self.sampler {
            Some(s) => s.usage(),
            None if self.ag.native_agentapi => capture::ProcessUse { exit_unknown: "exit-unknown", ..capture::ProcessUse::none("sidecar-server") },
            None => capture::ProcessUse { exit_unknown: "exit-unknown", ..capture::ProcessUse::none("recovered") },
        };
        // A Codex `exec` turn names each request only in its rollout; `turn.completed` holds the totals.
        if let Some(text) = rollout.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
            let calls = capture::rollout_calls(&text, true);
            if !calls.is_empty() {
                let per_request = sum_usage(&calls, None, true);
                r.usage.reasoning = per_request.reasoning;
                r.usage.model = r.usage.model.take().or(per_request.model);
                r.calls = calls;
            }
        }
        if self.timed_out { (r.outcome, r.error) = ("cancelled", "timeout"); }
        else if self.cancelled { (r.outcome, r.error) = ("cancelled", "interrupted"); }
        else if r.outcome == "unknown" && r.exit.is_some_and(|c| c != 0) && r.error == "no-end" { (r.outcome, r.error) = ("failed", "exit"); }
        r
    }
}

fn read_from(log: &Path, pos: u64) -> Option<String> { read_log(log, pos).ok() }

fn read_log(log: &Path, pos: u64) -> std::io::Result<String> {
    let mut f = File::open(log)?;
    let mut b = vec![];
    f.seek(SeekFrom::Start(pos)).and_then(|_| f.read_to_end(&mut b))?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

/// A turn-end event: Claude `result`, Codex `turn.completed` or `turn.failed`, Antigravity terminal `PLANNER_RESPONSE` or `turn.cancelled`.
fn ends(v: &Value) -> bool {
    if v["event"] == "result" { return matches!(v["result"]["status"].as_str(), Some("SUCCESS" | "ERROR" | "CANCELED" | "INTERRUPTED" | "INVALID")); }
    match v["type"].as_str() {
        Some("result" | "turn.completed" | "turn.failed" | "turn.cancelled") => true,
        Some("PLANNER_RESPONSE") => {
            matches!(v["status"].as_str(), Some("ERROR" | "CANCELLED"))
                || (v["status"] == "DONE" && v["tool_calls"].as_array().is_none_or(Vec::is_empty))
        }
        _ => false,
    }
}

/// The observation context the caller owns; every value is a code (R27).
#[derive(Clone, Debug)]
pub struct Context<'a> {
    pub product: &'a str,
    pub version: &'a str,
    /// `owner/name` of the project, or `other`.
    pub project: &'a str,
    /// Task and attempt of a task turn.
    pub task: Option<(u64, u64)>,
    /// Route code of a turn outside a task (a manager conversation).
    pub route: Option<&'a str>,
    pub account: &'a str,
}

/// Start, wait and report one turn, then append its `agent.turn` observation to `store`. A failed append is kept in
/// `Report::recorded` and never changes the turn's result (R30).
pub(crate) fn run(base: Command, spec: &Spec, log: &Path, text: &str, deadline: Option<Instant>, ctx: &Context, store: &crate::telemetry::Store) -> Res<Report> {
    let mut turn = Turn::start(base, spec, log, text, &Process::default())?;
    turn.wait(deadline)?;
    let mut r = turn.report();
    let key = turn.key();
    r.recorded = crate::telemetry::Event::observation(r.observation(spec.client, ctx)).and_then(|e| capture::record(store, e, &r, spec.client, ctx, &key));
    Ok(r)
}

// ---------- reading client output ----------

/// Token use of one turn; a count the client did not report is None.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    /// Claude assistant messages; Codex reports no call count.
    pub calls: Option<u64>,
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    /// Reasoning output the client reports apart; part of `output`.
    pub reasoning: Option<u64>,
    pub model: Option<String>,
    /// Why no usage was read; None when some was.
    pub unknown: Option<&'static str>,
}

/// Model use of the turn's events: Claude assistant messages (each message id once: input, cache read, cache write,
/// output, the model of most calls), Codex `turn.completed` (`input_tokens` includes `cached_input_tokens`, no cache
/// writes), Antigravity `PLANNER_RESPONSE` steps (each `step_index` once: `input_tokens`, `cache_read_tokens`,
/// `output_tokens`). A counted call without a value makes that sum unknown, never smaller.
#[cfg(test)]
pub(crate) fn usage(events: &[Value]) -> Usage {
    let (calls, count, known) = read_calls(events);
    sum_usage(&calls, count, known)
}

/// The turn's token sums of its calls; a call without a value makes that sum unknown, never smaller.
pub(crate) fn sum_usage(calls: &[Call], count: Option<u64>, known: bool) -> Usage {
    if !known { return Usage { unknown: Some("no-usage"), ..Usage::default() }; }
    let sum = |f: fn(&Call) -> Option<u64>| calls.iter().try_fold(0u64, |a, c| a.checked_add(f(c)?));
    let mut models = BTreeMap::<&str, u64>::new();
    for m in calls.iter().filter_map(|c| c.model.as_deref()) { *models.entry(m).or_default() += 1; }
    let model = models.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|m| m.0.to_string());
    Usage { calls: count, input: sum(|c| c.input), cache_read: sum(|c| c.cache_read), cache_write: sum(|c| c.cache_write), output: sum(|c| c.output),
        reasoning: if calls.is_empty() { None } else { sum(|c| c.reasoning) }, model, unknown: None }
}

/// The model requests of the turn's events, the Claude message count, and whether any usage was read.
pub(crate) fn read_calls(events: &[Value]) -> (Vec<Call>, Option<u64>, bool) {
    let (mut ids, mut turns, mut calls) = (std::collections::HashSet::new(), false, vec![]);
    let mut cli_usage = false;
    for (idx, v) in events.iter().enumerate() {
        let kind = if v["event"] == "step_update" {"agy-step"} else {v["type"].as_str().unwrap_or_default()};
        let cli = kind == "agy-step";
        let codex = kind == "turn.completed";
        let ag = kind == "PLANNER_RESPONSE";
        let mut id = None;
        let counted = match kind {
            "assistant" => { id = v["message"]["id"].as_str().map(String::from); v["message"]["id"].as_str().is_some_and(|id| ids.insert(id.to_string())) }
            "turn.completed" => { turns = true; id = v.get("total_usage").map(Value::to_string); true }
            "agy-step" => {
                let step = &v["step_update"];
                if step["state"] != "DONE" || (step["step_type"] != "agent_response" && !step["usage"].is_object()) { continue; }
                let Some(n) = step["step_index"].as_u64() else {continue;};
                if events[idx+1..].iter().any(|later| later["event"] == "step_update" && later["step_update"]["step_index"].as_u64() == Some(n) && later["step_update"]["state"] == "DONE" && (later["step_update"]["step_type"] == "agent_response" || later["step_update"]["usage"].is_object())) {continue;}
                cli_usage = true; turns = true;
                id = Some(format!("agy-step-{n}"));
                ids.insert(format!("agy-step-{n}"))
            }
            "PLANNER_RESPONSE" => {
                if !matches!(v["status"].as_str(), Some("DONE" | "ERROR" | "CANCELLED")) { continue; }
                if v["step_index"].as_u64().is_some_and(|step| events[idx+1..].iter().any(|later| later["type"] == "PLANNER_RESPONSE" && later["step_index"].as_u64() == Some(step))) { continue; }
                let Some(step) = v["step_index"].as_u64() else { continue; };
                id = Some(format!("ag-step-{step}"));
                ids.insert(format!("ag-step-{step}"))
            }
            _ => false,
        };
        if !counted { continue }
        let u = if cli { &v["step_update"]["usage"] } else if codex { &v["usage"] } else if ag { v } else { &v["message"]["usage"] };
        let input = u["input_tokens"].as_u64();
        let read = if codex {
            u["cached_input_tokens"].as_u64()
        } else if ag || cli {
            u["cache_read_tokens"].as_u64().or(u["cache_read_input_tokens"].as_u64())
        } else {
            u["cache_read_input_tokens"].as_u64()
        };
        let (outcome, error) = if v["isApiErrorMessage"] == true || v["error"].is_string() { ("failed", "client-error") }
            else if ag && v["status"] == "ERROR" { ("failed", "turn-failed") }
            else if ag && v["status"] == "CANCELLED" { ("cancelled", "turn-cancelled") }
            else { ("ok", "none") };
        calls.push(Call {
            id, at_ms: v["timestamp"].as_str().or(v["created_at"].as_str()).and_then(capture::time_ms),
            input: if codex { input.zip(read).and_then(|(i, r)| i.checked_sub(r)) } else { input },
            cache_read: read,
            cache_write: if codex { Some(0) } else if ag || cli { u["cache_write_tokens"].as_u64().or(u["cache_creation_input_tokens"].as_u64()) } else { u["cache_creation_input_tokens"].as_u64() },
            output: u["output_tokens"].as_u64(),
            reasoning: if codex { u["reasoning_output_tokens"].as_u64() } else { None },
            model: v["message"]["model"].as_str().or_else(|| v["model"].as_str()).filter(|m| model_code(m)).map(String::from),
            outcome, error, aggregate: codex && v["call"] != true,
        });
    }
    let count = (!cli_usage && !ids.is_empty()).then_some(ids.len() as u64);
    let known = count.is_some() || turns;
    (calls, count, known)
}

/// A model id is a short code; anything else stays unreported.
fn model_code(m: &str) -> bool { !m.is_empty() && m.len() <= 64 && m.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)) }

/// A usage or quota limit reported before any work started: `Some(reset)` with the reset (unix seconds) when known;
/// None after work started or without a limit.
pub(crate) fn limit_reached(events: &[Value]) -> Option<Option<i64>> {
    events.iter().find_map(StartLimit::read).and_then(|s| match s {StartLimit::Reached(reset)=>Some(reset),_=>None})
}

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
enum StartLimit { #[default] Pending, Working, Reached(Option<i64>) }

impl StartLimit {
    // The first decisive record fixes whether a limit refused admission or interrupted work.
    fn read(v: &Value) -> Option<Self> {
        if v["type"]=="rate_limit_event" && v["rate_limit_info"]["status"]=="rejected" {return Some(Self::Reached(v["rate_limit_info"]["resetsAt"].as_i64()));}
        if v["event"] == "step_update" && (v["step_update"]["step_type"] == "tool" || v["step_update"]["usage"].is_object()) { return Some(Self::Working); }
        if v["event"] == "result" {
            let msg = v["result"]["error"].as_str().unwrap_or_default().to_lowercase();
            return Some(if v["result"]["status"] != "SUCCESS" && ["usage limit", "quota exceeded", "rate limit"].iter().any(|s| msg.contains(s)) {Self::Reached(None)} else {Self::Working});
        }
        let kind = v["type"].as_str().unwrap_or_default();
        if kind == "assistant" {
            if v["apiError"] == "usage_limit_reached" || v["quotaLimits"]["status"] == "rejected" {
                return Some(Self::Reached(limits::claude_info(v).and_then(|info|info["resetsAt"].as_i64())));
            }
            return Some(Self::Working);
        }
        if kind == "PLANNER_RESPONSE" {
            if v["status"] == "ERROR" {
                let msg = v["content"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
                if msg.contains("usage limit") || msg.contains("quota exceeded") || msg.contains("rate limit") {
                    return Some(Self::Reached(v["quotaLimits"]["resetsAt"].as_i64()));
                }
            }
            return Some(Self::Working);
        }
        if kind.starts_with("item.") || kind == "TOOL_RESPONSE" { return Some(Self::Working); }
        // ponytail: Codex limit text matched by words; it reports no reset. A structured field replaces this when one exists.
        let message = v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase();
        if matches!(kind, "error" | "turn.failed") && (message.contains("usage limit") || message.contains("quota exceeded")) { return Some(Self::Reached(None)); }
        None
    }
}

/// A Codex login the auth server rejected: Codex's own stderr `ERROR codex_` line or a top-level `error`/`turn.failed`
/// event naming a rejected refresh token or a 401; worker output (`item.*`) never matches.
pub(crate) fn auth_rejected(text: &str) -> Option<&'static str> {
    text.lines().find_map(|l| {
        let m = match serde_json::from_str::<Value>(l) {
            Ok(v) if matches!(v["type"].as_str(), Some("error" | "turn.failed")) => v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or_default().to_lowercase(),
            Err(_) if l.contains("ERROR codex_") => l.to_lowercase(),
            _ => return None,
        };
        if m.contains("refresh token") && (m.contains("already used") || m.contains("expired") || m.contains("revoked") || m.contains("invalid")) || m.contains("refresh_token_") {
            Some("refresh token rejected")
        } else if m.contains("401 unauthorized") || m.contains("auth error: 401") { Some("401 Unauthorized") } else { None }
    })
}

/// The last turn end is a Codex `turn.failed` naming model capacity: its message.
pub(crate) fn transient_failure(events: &[Value]) -> Option<String> {
    let end = events.iter().filter(|v| matches!(v["type"].as_str(), Some("turn.completed" | "turn.failed"))).last()?;
    // ponytail: only the "Selected model is at capacity" text is verified; other overload texts read as failed.
    let m = end["error"]["message"].as_str().filter(|_| end["type"] == "turn.failed")?;
    m.to_lowercase().contains("model is at capacity").then(|| m.to_string())
}

/// The final text of a turn: Claude's last `result`, Codex's last `agent_message`, or Antigravity's last `PLANNER_RESPONSE` content.
pub(crate) fn final_text(events: &[Value]) -> Option<String> {
    events.iter().rev().find_map(|v| {
        if v["event"] == "result" { v["result"]["response"].as_str().map(String::from) }
        else if v["type"] == "result" { v["result"].as_str().map(String::from) }
        else if v["type"] == "item.completed" && v["item"]["type"] == "agent_message" { v["item"]["text"].as_str().map(String::from) }
        else if v["type"] == "PLANNER_RESPONSE" && v["status"] == "DONE" && ends(v) { v["content"].as_str().filter(|s| !s.is_empty()).map(String::from) }
        else { None }
    })
}

/// Codex's thread ID: the first `thread.started` line.
pub(crate) fn codex_thread(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        (v["type"] == "thread.started").then(|| v["thread_id"].as_str().map(String::from)).flatten()
    })
}

/// Antigravity's conversation ID from `agentapi` JSON output or a stream event.
pub(crate) fn antigravity_session(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text).into_iter().chain(text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())).find_map(|v| {
        v.pointer("/response/newConversation/conversationId").and_then(Value::as_str)
            .or_else(|| v.pointer("/response/sendMessage/recipientId").and_then(Value::as_str))
            .or_else(|| v["conversationId"].as_str())
            .or_else(|| v["conversation_id"].as_str())
            .or_else(|| v["result"]["conversation_id"].as_str())
            .or_else(|| (matches!(v["type"].as_str(), Some("conversation.started" | "system" | "result"))).then(|| v["session_id"].as_str()).flatten())
            .filter(|s| native::plain_id(s) && *s != "-")
            .map(String::from)
    })
}

/// The native session of a turn: Claude's `session_id` (init or result), Codex's thread, or Antigravity's `conversationId`.
pub(crate) fn session_of(client: Client, text: &str) -> Option<String> {
    match client {
        Client::Codex => codex_thread(text),
        Client::Claude => text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find_map(|v| (matches!(v["type"].as_str(), Some("system" | "result"))).then(|| v["session_id"].as_str().map(String::from)).flatten()),
        Client::Antigravity => antigravity_session(text),
    }
}

/// The thread's rollout `<home>/sessions/**/rollout-*-<thread>.jsonl`, in `home` or the client's own home.
pub(crate) fn rollout(thread: &str, home: Option<&Path>) -> Option<PathBuf> {
    fn find(dir: &Path, end: &str, depth: u8) -> Option<PathBuf> {
        if depth == 0 { return None; }
        std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
            let p = e.path();
            if p.is_dir() { find(&p, end, depth - 1) } else { p.file_name()?.to_str()?.ends_with(end).then_some(p) }
        })
    }
    if thread.is_empty() || !thread.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return None; }
    let home = home.map(Path::to_path_buf).or_else(|| base_home(Client::Codex))?;
    // sessions/<yyyy>/<mm>/<dd>/<file>
    find(&home.join("sessions"), &format!("{thread}.jsonl"), 4)
}

/// A subscription limit snapshot; each None has `unknown`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Used share of the binding window, whole percent.
    pub used_percent: Option<u64>,
    /// When the window resets (unix seconds).
    pub resets_at: Option<u64>,
    /// The window code: Claude `rateLimitType` (`five_hour`, `seven_day`), Codex `<window_minutes>m`.
    pub window: Option<String>,
    pub unknown: Option<&'static str>,
}

fn percent(v: f64) -> Option<u64> { (v.is_finite() && v >= 0.0).then(|| v.round() as u64) }

/// Claude: the last `rate_limit_event` (`rate_limit_info.utilization` 0-1, `resetsAt`, `rateLimitType`). Codex: the
/// last `rate_limits` (`primary`/`secondary`: `used_percent` 0-100, `resets_at`, `window_minutes`) in the turn's events,
/// else the last `token_count` of the thread rollout; the window with the higher use binds.
pub(crate) fn snapshot(client: Client, events: &[Value], rollout: Option<&Path>) -> Snapshot {
    let found = match client {
        Client::Claude | Client::Antigravity => events.iter().rev().find(|v| v["type"] == "rate_limit_event").map(|v| {
            let i = &v["rate_limit_info"];
            Snapshot { used_percent: i["utilization"].as_f64().and_then(|u| percent(u * 100.0)), resets_at: i["resetsAt"].as_u64(),
                window: i["rateLimitType"].as_str().filter(|w| model_code(w)).map(String::from), unknown: None }
        }),
        Client::Codex => {
            let own = events.iter().rev().find_map(|v| codex_limits(v));
            own.or_else(|| {
                let f = File::open(rollout?).ok()?;
                BufReader::new(f).lines().map_while(Result::ok).filter(|l| l.contains("\"rate_limits\""))
                    .filter_map(|l| serde_json::from_str::<Value>(&l).ok()).filter_map(|v| codex_limits(&v)).last()
            })
        }
    };
    match found {
        Some(mut s) => {
            if s.used_percent.is_none() || s.resets_at.is_none() { s.unknown = Some("partial-snapshot"); }
            s
        }
        None => Snapshot { unknown: Some(if client == Client::Codex && rollout.is_none() { "no-rollout" } else { "not-reported" }), ..Snapshot::default() },
    }
}

fn codex_limits(v: &Value) -> Option<Snapshot> {
    let r = [&v["rate_limits"], &v["payload"]["rate_limits"], &v["msg"]["rate_limits"]].into_iter().find(|r| r.is_object())?;
    let (used, w) = [&r["primary"], &r["secondary"]].into_iter().filter_map(|w| w["used_percent"].as_f64().map(|u| (u, w)))
        .fold(None::<(f64, &Value)>, |best, c| if best.is_none_or(|b| c.0 > b.0) { Some(c) } else { best })?;
    let resets = w["resets_at"].as_u64().or_else(|| w["resets_in_seconds"].as_u64().map(|s| (unix_ms() / 1000) as u64 + s));
    Some(Snapshot { used_percent: percent(used), resets_at: resets, window: w["window_minutes"].as_u64().map(|m| format!("{m}m")), unknown: None })
}

/// What one turn did, read from its log.
#[derive(Clone, Debug)]
pub struct Report {
    pub session: Option<String>,
    pub usage: Usage,
    pub limit: Snapshot,
    /// Every reported quota window; an unavailable snapshot names its reason.
    pub limits: Vec<Window>,
    /// A limit reached before work started, with its reset when known: the caller records it for the account switch.
    pub reached: Option<Option<i64>>,
    /// Claude background jobs still live according to this log, never process liveness proof.
    pub background_job: bool,
    /// One fixed tool label per such job, oldest first; never a description, command or task ID.
    pub live_background: Vec<String>,
    /// Telemetry outcome code and error code.
    pub outcome: &'static str,
    pub error: &'static str,
    /// The native reason of error `auth-rejected` or `model-capacity`.
    pub detail: Option<String>,
    pub text: Option<String>,
    pub exit: Option<i32>,
    pub started_ms: i64,
    pub duration_us: u64,
    /// The observation append; Ok(()) before `run` appends.
    pub recorded: Res<()>,
    /// The turn's model requests; their tokens are the same tokens as `usage`.
    pub calls: Vec<Call>,
    pub process: ProcessUse,
}

impl Report {
    /// Classify the turn's log text.
    pub(crate) fn read(client: Client, text: &str, events: &[Value], rollout: Option<&Path>) -> Report {
        let reached = limit_reached(events);
        let end = if client == Client::Antigravity {
            events.iter().rev().find(|v| v["event"] == "result" || matches!(v["type"].as_str(), Some("PLANNER_RESPONSE" | "TOOL_RESPONSE" | "turn.failed" | "turn.cancelled"))).filter(|v| ends(v))
        } else { events.iter().rev().find(|v| ends(v)) };
        let (mut detail, auth, capacity) = (None, auth_rejected(text), transient_failure(events));
        let (outcome, error) = if reached.is_some() { ("refused", "usage-limit") }
            else if let Some(a) = auth { detail = Some(a.to_string()); ("failed", "auth-rejected") }
            else if capacity.is_some() { detail = capacity; ("failed", "model-capacity") }
            else {
                match end {
                    Some(v) if v["event"] == "result" && matches!(v["result"]["status"].as_str(), Some("CANCELED" | "INTERRUPTED")) => ("cancelled", "turn-cancelled"),
                    Some(v) if v["event"] == "result" && v["result"]["status"] != "SUCCESS" => ("failed", "turn-failed"),
                    Some(v) if v["type"] == "turn.failed" || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "ERROR") => ("failed", "turn-failed"),
                    Some(v) if v["type"] == "turn.cancelled" || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "CANCELLED") => ("cancelled", "turn-cancelled"),
                    Some(v) if (v["type"] == "result" && v["is_error"] == true) || (v["type"] == "PLANNER_RESPONSE" && v["status"] == "ERROR") => ("failed", "client-error"),
                    Some(_) => ("ok", "none"),
                    None => ("unknown", "no-end"),
                }
            };
        let (calls, count, known) = read_calls(events);
        let live_background = if client==Client::Claude {events::live_background(events)} else {vec![]};
        Report { session: session_of(client, text), usage: sum_usage(&calls, count, known), calls, process: ProcessUse::default(), limit: snapshot(client, events, rollout), limits: limits::read(client, events, rollout), reached, background_job: !live_background.is_empty(), live_background, outcome, error, detail,
            text: final_text(events), exit: None, started_ms: unix_ms(), duration_us: 0, recorded: Ok(()) }
    }

    /// The `agent.turn` observation: codes and numbers only; every null names its reason.
    pub(crate) fn observation(&self, client: Client, ctx: &Context) -> Value { self.observation_as(client.code(), ctx) }

    /// The record under a runtime name; a hook session may name a client outside `Client` (agentq.md M6.11).
    pub(crate) fn observation_as(&self, runtime: &str, ctx: &Context) -> Value {
        let mut unknown = serde_json::Map::new();
        let mut num = |name: &str, v: Option<u64>, why: &str| -> Value {
            if v.is_none() { unknown.insert(name.into(), json!(why)); }
            json!(v)
        };
        let why = self.usage.unknown.unwrap_or("not-reported");
        let who = capture::Attribution::of(ctx);
        let (task, attempt) = (who.task, who.attempt);
        let limit_why = self.limit.unknown.unwrap_or("not-reported");
        // A signalled child has no code; a Windows code reads as its unsigned DWORD.
        let exit = self.exit.and_then(|c| if cfg!(windows) { Some(u64::from(c as u32)) } else { u64::try_from(c).ok() });
        let exit_why = if self.exit.is_some() { "signalled" } else { self.process.exit_unknown };
        let process_why = self.process.unknown.unwrap_or("not-sampled");
        let mut v = json!({
            "product": ctx.product, "version": ctx.version, "operation": "agent.turn", "kind": "agent.turn",
            "trace": null, "span": null, "started_ms": self.started_ms.max(0), "duration_us": self.duration_us,
            "outcome": self.outcome, "error": self.error, "project": who.project, "runtime": runtime, "account": ctx.account,
            "task": num("task", task, "no-task"), "attempt": num("attempt", attempt, who.no_attempt()),
            "input_tokens": num("input_tokens", self.usage.input, why), "cache_read_tokens": num("cache_read_tokens", self.usage.cache_read, why),
            "cache_write_tokens": num("cache_write_tokens", self.usage.cache_write, why), "output_tokens": num("output_tokens", self.usage.output, why),
            "reasoning_tokens": num("reasoning_tokens", self.usage.reasoning, why),
            "exit_code": num("exit_code", exit, exit_why),
            "user_cpu_us": num("user_cpu_us", self.process.user_cpu_us, process_why),
            "system_cpu_us": num("system_cpu_us", self.process.system_cpu_us, process_why),
            "peak_rss_bytes": num("peak_rss_bytes", self.process.peak_rss_bytes, process_why),
            "limit_used_percent": num("limit_used_percent", self.limit.used_percent, limit_why),
            "limit_resets_at": num("limit_resets_at", self.limit.resets_at, limit_why),
        });
        unknown.insert("trace".into(), json!("not-traced"));
        unknown.insert("span".into(), json!("not-traced"));
        let text = |v: &mut Value, unknown: &mut serde_json::Map<String, Value>, k: &str, s: Option<&str>, why: &str| {
            v[k] = json!(s);
            if s.is_none() { unknown.insert(k.into(), json!(why)); }
        };
        text(&mut v, &mut unknown, "model", self.usage.model.as_deref(), if self.usage.unknown.is_some() { why } else { "not-reported" });
        text(&mut v, &mut unknown, "route", ctx.route, "task-turn");
        text(&mut v, &mut unknown, "limit_window", self.limit.window.as_deref(), limit_why);
        text(&mut v, &mut unknown, "session_id", capture::session_code(self.session.as_deref()), "session-unknown");
        text(&mut v, &mut unknown, "host", capture::host().as_deref(), "not-reported");
        who.caller_fields(&mut v);
        v["unknown"] = Value::Object(unknown);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // #192 A1: service PATH misses user installs; native CLI and legacy homes must both remain usable.
    #[test]
    fn native_agy_user_bin_and_homes() {
        if std::env::var_os("AGENTQ_TEST_AGY_HOME").is_some() {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap());
            assert_eq!(antigravity_executable(&std::env::var_os("PATH").unwrap_or_default()), home.join(".local/bin/agy"));
            assert!(executable(Client::Antigravity).unwrap().is_absolute());
            let legacy = home.join(".gemini/antigravity");
            let cli = home.join(".gemini/antigravity-cli");
            std::fs::create_dir_all(&legacy).unwrap();
            assert_eq!(base_home(Client::Antigravity), Some(legacy.clone()));
            std::fs::create_dir_all(&cli).unwrap();
            assert_eq!(base_home(Client::Antigravity), Some(cli));
            return;
        }
        let home = crate::scratch("agy-lookup");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        std::fs::write(home.join(".local/bin/agy"), "invented CLI fixture").unwrap();
        // A subprocess owns the environment, so parallel tests never see a fake login home or PATH.
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "agent::tests::native_agy_user_bin_and_homes", "--nocapture"])
            .env("AGENTQ_TEST_AGY_HOME", "1").env("HOME", &home).env("USERPROFILE", &home)
            .env("PATH", &home).env_remove("ANTIGRAVITY_HOME").env_remove("ANTIGRAVITY_APP_DATA_DIR")
            .env_remove("ANTIGRAVITY_AGENTAPI_EXE").status().unwrap();
        std::fs::remove_dir_all(home).unwrap();
        assert!(status.success());
    }

    fn events(text: &str) -> Vec<Value> { text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect() }

    // Chain: coreq.md R51 (TaskQ § 3 Runtime accounts, unchanged): order and disable pick the start account; a limit
    // moves to the next enabled account; with none left the hold ends at the earliest limit end.
    // Risk: a disabled or limited account takes new turns, or a spare account is ignored.
    #[test]
    fn account_order_disable_failover_and_hold() {
        let none = Accounts::default();
        assert_eq!((none.pick(0).as_deref(), none.spare(0)), (Some(DEFAULT), None), "no list: one default account, no failover");
        let list = |v: &[(&str, bool)]| v.iter().map(|(l, e)| Entry { label: l.to_string(), enabled: *e }).collect::<Vec<_>>();
        let a = Accounts { list: list(&[("default", false), ("work", true)]), limits: BTreeMap::new() };
        assert_eq!(a.pick(0).as_deref(), Some("work"), "a disabled account takes no start");
        let mut a = Accounts { list: list(&[("default", true), ("work", true), ("spare", false)]), limits: BTreeMap::new() };
        a.limits.insert("default".into(), Limit { at: 100, until: None });
        assert_eq!(a.spare(200).as_deref(), Some("work"), "a limit moves to the next enabled account");
        assert_eq!(a.hold(200), Some((100 + PROBE, false)));
        assert_eq!(a.spare(100 + PROBE).as_deref(), Some("default"), "the probe ends the limit and order returns");
        a.limits.insert("work".into(), Limit { at: 150, until: Some(9000) });
        assert_eq!((a.spare(200), a.pick(200).as_deref()), (None, Some("default")), "all limited: no spare, a start still names one");
        assert_eq!(a.hold(200), Some((100 + PROBE, false)), "the hold ends at the earliest enabled limit end");
        let off = Accounts { list: list(&[("default", false)]), limits: BTreeMap::new() };
        assert!(off.off() && off.pick(0).is_none(), "all disabled admits nothing");
        assert!(valid_label("work-2") && !valid_label("-x") && !valid_label("Work") && !valid_label(""));
        assert_eq!(account_home(Client::Codex, "taskq", DEFAULT), None, "default keeps the caller's home");
        assert!(account_home(Client::Claude, "taskq", "work").is_none_or(|h| h.ends_with(Path::new("taskq-accounts").join("work"))));
    }

    // Chain: coreq.md R48 and docs/design/agent.md § Arguments: TaskQ's argument order for start and resume.
    // Risk: a client format fix in one consumer diverges, or a resume starts a second session.
    #[test]
    fn start_and_resume_arguments_keep_taskq_order() {
        let s = |v: &[OsString]| v.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        let extra = vec![OsString::from("--permission-mode"), OsString::from("auto")];
        let mut spec = Spec { client: Client::Claude, root: PathBuf::from("proj"), session: "id1".into(), resume: false, args: extra, home: None, env: vec![] };
        assert_eq!(s(&args(&spec)), "-p --output-format stream-json --verbose --permission-mode auto --session-id id1");
        spec.resume = true;
        assert_eq!(s(&args(&spec)), "-p --output-format stream-json --verbose --permission-mode auto --resume id1");
        spec.client = Client::Codex;
        spec.args = vec!["--sandbox".into(), "workspace-write".into()];
        assert_eq!(s(&args(&spec)), "exec --json --sandbox workspace-write resume id1 -");
        spec.resume = false;
        assert_eq!(s(&args(&spec)), "exec --json --sandbox workspace-write -C proj -");
        spec.client = Client::Antigravity;
        spec.args = vec!["--model=pro".into()];
        assert_eq!(s(&args(&spec)), "--input-format stream-json --output-format stream-json --model=pro");
        spec.resume = true;
        assert_eq!(s(&args(&spec)), "--input-format stream-json --output-format stream-json --model=pro --conversation id1");
        let settings: Value = serde_json::from_str(&claude_settings(Some("rules compact"), &[("TASKQ_TASK", "7")])).unwrap();
        assert_eq!(settings["hooks"]["SessionStart"][0]["matcher"], "compact");
        assert_eq!(settings["hooks"]["SessionStart"][0]["hooks"][0]["command"], "rules compact");
        assert_eq!(settings["env"]["TASKQ_TASK"], "7");
        assert!(serde_json::from_str::<Value>(&claude_settings(None, &[])).unwrap().get("hooks").is_none());
        assert_eq!(new_session(Client::Claude).len(), 36);
        assert_eq!(new_session(Client::Codex), "-");
        assert_eq!(new_session(Client::Antigravity), "-");
        assert_eq!(Client::parse("antigravity"), Some(Client::Antigravity));
        assert_eq!(Client::parse("agy"), Some(Client::Antigravity));
    }

    // Chain: coreq.md R49 and TaskQ § 4 Profiling usage (unchanged): Claude counts each message id once; Codex input
    // includes cached input; Antigravity counts each PLANNER_RESPONSE step_index once; a missing count makes the sum unknown.
    #[test]
    fn usage_counts_each_message_once_and_unknown_stays_null() {
        let claude = events(concat!(
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_read_input_tokens":30,"cache_creation_input_tokens":5,"output_tokens":7}}}"#, "\n",
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":2,"cache_read_input_tokens":30,"cache_creation_input_tokens":5,"output_tokens":7}}}"#, "\n",
            r#"{"type":"assistant","message":{"id":"m2","model":"claude-opus-5-5","usage":{"input_tokens":1,"cache_read_input_tokens":40,"cache_creation_input_tokens":0,"output_tokens":3}}}"#, "\n"));
        let u = usage(&claude);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model.as_deref()), (Some(2), Some(3), Some(70), Some(5), Some(10), Some("claude-opus-5-5")));
        let codex = events(r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":9}}"#);
        let u = usage(&codex);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model), (None, Some(20), Some(80), Some(0), Some(9), None));
        let partial = events(r#"{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":9}}"#);
        assert_eq!((usage(&partial).input, usage(&partial).cache_read), (None, None), "a missing count is unknown, never zero");
        let ag = events(concat!(
            r#"{"response":{"newConversation":{"conversationId":"conv-1"}}}"#, "\n",
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":120,"cache_read_tokens":500,"cache_write_tokens":0,"output_tokens":15,"tool_calls":[{"id":"c1"}]}"#, "\n",
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":120,"cache_read_tokens":500,"cache_write_tokens":0,"output_tokens":15,"tool_calls":[{"id":"c1"}]}"#, "\n",
            r#"{"step_index":3,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":80,"cache_read_tokens":600,"cache_write_tokens":0,"output_tokens":25,"content":"done"}"#, "\n"));
        let u = usage(&ag);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model.as_deref()), (Some(2), Some(200), Some(1100), Some(0), Some(40), Some("gemini-3.1-pro")));
        let r_ag = Report::read(Client::Antigravity, &ag.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"), &ag, None);
        assert_eq!((r_ag.outcome, r_ag.session.as_deref(), r_ag.text.as_deref()), ("ok", Some("conv-1"), Some("done")));
        let missing = usage(&events(r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","input_tokens":3,"output_tokens":2}"#));
        assert_eq!((missing.cache_read, missing.cache_write), (None, None), "P2: unreported counts stay unknown");
        assert!(ends(&json!({"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]})), "empty tool calls end the turn");
        assert_eq!(usage(&[]).unknown, Some("no-usage"));
    }

    // M12.1/P2: documented agy events must yield per-turn evidence, never cumulative result usage.
    #[test]
    fn antigravity_headless_protocol() {
        let rows = events(r#"{"event":"init","conversation_id":"agy-session","init":{}}
{"event":"step_update","step_update":{"step_index":3,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":12,"cache_read_tokens":5,"output_tokens":3}}}
{"event":"result","result":{"conversation_id":"agy-session","status":"SUCCESS","response":"answer","usage":{"input_tokens":999,"output_tokens":999}}}"#);
        let r = Report::read(Client::Antigravity, &rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"), &rows, None);
        assert_eq!((r.outcome, r.text.as_deref(), r.session.as_deref()), ("ok", Some("answer"), Some("agy-session")));
        assert_eq!((r.usage.input, r.usage.cache_read, r.usage.cache_write), (Some(12), Some(5), None));
        let error = events(r#"{"event":"result","result":{"status":"ERROR","error":"quota exceeded"}}"#);
        assert_eq!(Report::read(Client::Antigravity, "", &error, None).outcome, "refused");
        for (status, outcome) in [("CANCELED","cancelled"),("INTERRUPTED","cancelled"),("INVALID","failed"),("WAITING","unknown"),("RUNNING","unknown")] {
            let rows = vec![json!({"event":"result","result":{"status":status}})];
            assert_eq!(Report::read(Client::Antigravity,"",&rows,None).outcome,outcome,"{status}");
        }
        let mut missing = rows.clone();
        let mut later = missing[1].clone(); later["step_update"]["usage"] = json!({"output_tokens":3});
        missing.insert(2,later);
        let u = usage(&missing);
        assert_eq!((u.input,u.output,u.calls),(None,Some(3),None),"latest partial step stays unknown and cumulative result is ignored");
    }

    // M12.1: a bounded poll must progress through a full transcript larger than its read budget.
    #[test]
    fn antigravity_full_history_progresses_past_budget() {
        let root = crate::scratch("large-full-tail");
        let (path, full) = transcript_paths(&root, "conv-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let log = root.join("turn.log"); std::fs::write(&log, "").unwrap();
        let compact = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","truncated_fields":["content"]});
        std::fs::write(path, format!("{compact}\n")).unwrap();
        let old = json!({"step_index":0,"content":"x".repeat(9*1024*1024)});
        let complete = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","content":"full"});
        std::fs::write(full, format!("{old}\n{complete}\n")).unwrap();
        let mut state = AntigravityTranscript {native_agentapi:true,session:Some("conv-1".into()),..Default::default()};
        for _ in 0..4 {state.append(&root, &log).unwrap();}
        assert_eq!(final_text(&events(&std::fs::read_to_string(log).unwrap())).as_deref(), Some("full"));
        std::fs::remove_dir_all(root).unwrap();
    }

    // M12.1/P2: partial writes, resume history and replacement must not fabricate completion or usage.
    #[test]
    fn antigravity_tail_preserves_boundaries_and_unknowns() {
        let root = crate::scratch("antigravity-tail");
        let (path, full) = transcript_paths(&root, "conv-1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let log = root.join("turn.log"); std::fs::write(&log, "").unwrap();
        let old = json!({"type":"PLANNER_RESPONSE","step_index":0,"status":"DONE","content":"old"});
        std::fs::write(&path, format!("{old}\n")).unwrap();
        let mut state = AntigravityTranscript::baseline(Some(&root), Some("conv-1".into())).unwrap();
        let event = json!({"type":"PLANNER_RESPONSE","step_index":1,"status":"DONE","content":"new","tool_calls":[]}).to_string();
        let half = event.len()/2;
        let mut writer = File::options().append(true).open(&path).unwrap();
        writer.write_all(event[..half].as_bytes()).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
        writeln!(writer, "{}", &event[half..]).unwrap();
        state.append(&root, &log).unwrap(); state.append(&root, &log).unwrap();
        assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1, "new complete record only, once");
        let compact = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","truncated_fields":["content"]});
        writeln!(writer, "{compact}").unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1);
        let complete = json!({"type":"PLANNER_RESPONSE","step_index":2,"status":"DONE","content":"full"});
        std::fs::write(&full, format!("{old}\n{complete}")).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 1, "partial full record waits");
        std::fs::write(&full, format!("{old}\n{complete}\n")).unwrap();
        state.append(&root, &log).unwrap(); assert_eq!(events(&std::fs::read_to_string(&log).unwrap()).len(), 2);
        std::fs::write(&path, "").unwrap(); assert!(state.append(&root, &log).is_err(), "truncation retains unknown ownership");
        let failed = events(r#"{"type":"PLANNER_RESPONSE","step_index":1,"status":"ERROR","content":"quota exceeded"}"#);
        assert_eq!(Report::read(Client::Antigravity, "", &failed, None).outcome, "refused");
        let cancelled = events(r#"{"type":"PLANNER_RESPONSE","step_index":1,"status":"CANCELLED"}"#);
        assert_eq!(Report::read(Client::Antigravity, "", &cancelled, None).outcome, "cancelled");
        assert_eq!(snapshot(Client::Antigravity, &[], None).unknown, Some("not-reported"));
        assert_eq!(antigravity_session(r#"{"conversationId":"../foreign"}"#), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    // Chain: coreq.md R50, docs/design/agent.md § Limit snapshot: shapes copied from a Claude 2026-10 `rate_limit_event`
    // and a Codex 2026-11 rollout `token_count`. Risk: a limit reads as 0 % or a reset is invented.
    #[test]
    fn limit_snapshot_fields_per_client_or_reason() {
        let claude = events(r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1792386000,"rateLimitType":"seven_day","utilization":0.56,"unifiedWindows":{"five_hour":{"utilization":0.11,"resetsAt":1791964200}}}}"#);
        assert_eq!(snapshot(Client::Claude, &claude, None), Snapshot { used_percent: Some(56), resets_at: Some(1792386000), window: Some("seven_day".into()), unknown: None });
        assert_eq!(snapshot(Client::Claude, &[], None).unknown, Some("not-reported"));
        let dir = crate::scratch("agent-rollout");
        let path = dir.join("rollout-2026-11-01T01-00-14-t1.jsonl");
        std::fs::write(&path, concat!(
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":40.0,"window_minutes":10080,"resets_at":1793774000},"secondary":null}}}"#, "\n",
            r#"{"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":51.0,"window_minutes":10080,"resets_at":1793774205},"secondary":{"used_percent":12.0,"window_minutes":300,"resets_at":1793700000}}}}"#, "\n")).unwrap();
        assert_eq!(snapshot(Client::Codex, &[], Some(&path)), Snapshot { used_percent: Some(51), resets_at: Some(1793774205), window: Some("10080m".into()), unknown: None });
        assert_eq!(snapshot(Client::Codex, &[], None).unknown, Some("no-rollout"));
        let partial = events(r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour"}}"#);
        assert_eq!(snapshot(Client::Claude, &partial, None).unknown, Some("partial-snapshot"));
    }

    // Chain: TaskQ usage limit, rejected login and transient failure classification (unchanged), now in Report::read.
    // Risk: a limit or a rejected login reads as a passing turn, so the account switch never happens.
    #[test]
    fn limits_logins_and_capacity_classify_the_turn() {
        let limited = r#"{"type":"assistant","apiError":"usage_limit_reached","quotaLimits":{"status":"rejected","resetsAt":1700000000}}"#;
        let r = Report::read(Client::Claude, limited, &events(limited), None);
        assert_eq!((r.reached, r.outcome, r.error), (Some(Some(1700000000)), "refused", "usage-limit"));
        let worked = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"usage limit\"}}\n{\"type\":\"turn.failed\",\"error\":{\"message\":\"You've hit your usage limit\"}}";
        assert_eq!(limit_reached(&events(worked)), None, "after work started a limit is not a start refusal");
        let codex = r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#;
        assert_eq!(limit_reached(&events(codex)), Some(None));
        assert_eq!(auth_rejected("2026 ERROR codex_core::auth: refresh token was already used"), Some("refresh token rejected"));
        assert_eq!(auth_rejected(r#"{"type":"item.completed","item":{"text":"401 Unauthorized"}}"#), None, "worker output never matches");
        let busy = r#"{"type":"turn.failed","error":{"message":"Selected model is at capacity. Please try a different model."}}"#;
        let r = Report::read(Client::Codex, busy, &events(busy), None);
        assert_eq!((r.outcome, r.error), ("failed", "model-capacity"));
        let done = "{\"type\":\"thread.started\",\"thread_id\":\"t-1\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi\"}}\n{\"type\":\"turn.completed\",\"usage\":{}}";
        let r = Report::read(Client::Codex, done, &events(done), None);
        assert_eq!((r.outcome, r.session.as_deref(), r.text.as_deref()), ("ok", Some("t-1"), Some("hi")));
    }

    // Chain: coreq.md R49 and R27: the observation holds codes and numbers, every null names its reason, and the
    // store accepts it. Risk: free text or an unexplained null enters telemetry, or the store rejects every turn.
    #[test]
    fn observation_is_valid_metadata() {
        let text = r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":4,"output_tokens":2}}"#;
        let r = Report::read(Client::Codex, text, &events(text), None);
        let ctx = Context { product: "taskq", version: "4.6.57", project: "taskqorg/coreq", task: Some((107, 1)), route: None, account: "work" };
        let v = r.observation(Client::Codex, &ctx);
        assert_eq!((v["input_tokens"].as_u64(), v["cache_read_tokens"].as_u64(), v["output_tokens"].as_u64()), (Some(6), Some(4), Some(2)));
        assert_eq!((v["unknown"]["model"].as_str(), v["unknown"]["route"].as_str(), v["unknown"]["limit_used_percent"].as_str()), (Some("not-reported"), Some("task-turn"), Some("no-rollout")));
        crate::telemetry::Event::new(v).expect("valid task observation");
        let route = Context { route: Some("pilotq.manager"), task: None, ..ctx };
        let v = r.observation(Client::Codex, &route);
        assert_eq!((v["route"].as_str(), v["unknown"]["task"].as_str()), (Some("pilotq.manager"), Some("no-task")));
        crate::telemetry::Event::new(v).expect("valid route observation");
    }
}
