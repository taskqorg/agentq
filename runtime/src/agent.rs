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
mod stdio;
mod acp;
mod control;
mod jsonl_rpc;
mod native;
use registry::app;
mod protected;
pub mod host;
mod adapter;
pub(crate) use adapter::SelectedSource;
#[cfg(test)]
pub(crate) use adapter::test_receipts;
pub(crate) mod capture;
mod events;
mod schema;
pub use schema::RecordShape;
mod limits;
pub use events::{Answer,Question,AssistantFailure, Choice, Image, Event, LogRecord, Model, RecordTime, RequestKind, TimeFault, ToolKind};
pub use limits::Window;
pub use capture::{APPEND_CHUNK, Call, Frozen, ProcessUse, SessionTurn, append_records, session_call, session_call_records, session_records, session_frozen, session_calls_frozen, session_call_frozen};
pub(crate) use capture::with_environment_caller;
pub use adapter::registry;
pub use adapter::{InputStage,LaunchError,BackgroundExecution,BackgroundLocator,LoginFlow,LoginOptions,LoginPrompt,LoginPhase,LoginStatus,LoginSession,Lifetime,Orphan,SessionStore,SessionOwner,ResumeCommand,NativeProcess,OwnerObservation,ActivityObservation,Binding,Account, Approval, Builtin, Cause, Check, Control, Failure, Login, Part, Configure, Descriptor, Launch, Mode, NativeSpec, Network, Observation, Policy, Process, Quota, Ready, Roots, Run, Runtime, Readiness, ReadinessMode, LaunchCheck, Repair, Scope, Spawned, State, runtime, runtimes};
pub use native::BackgroundJob;
pub use registry::{BackgroundCandidate,BackgroundKind,BackgroundState,BackgroundActivity,BackgroundPid};
pub use protected::{ProtectedHost, ProtectedIdentity, ProtectedPreparation, ProtectedAppServer};

type Res<T> = Result<T, String>;

/// Opaque native identity supplied by its registration module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Client(&'static str);

impl Client {
    pub fn parse(code: &str) -> Option<Client> {registry::native_client(code)}
    pub fn code(self) -> &'static str {self.0}
    pub fn home_var(self) -> &'static str {registry::native_home_var(self)}
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

/// The client's CLI executable: `claude` on `PATH`, Codex's native executor, or Antigravity's `agy` / `agentapi` CLI. A caller
/// that wraps the client (a sandbox) builds its own base command around this path.
pub(crate) fn executable(client: Client) -> Res<PathBuf> { executable_in(client, &std::env::var_os("PATH").unwrap_or_default()) }

/// `executable` resolved on `path` instead of the process `PATH`.
pub(crate) fn executable_in(client: Client, path: &OsStr) -> Res<PathBuf> {
    registry::native_executable(client,path)
}

#[cfg(test)]
fn antigravity_executable(path: &OsStr) -> PathBuf {registry::native_executable(Client::Antigravity,path).expect("native lookup is infallible")}

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

/// The client's own home: `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / `GEMINI_HOME`
/// of this process, else `~/.claude` / `~/.codex` / `~/.gemini` (a Windows service has no `HOME`, so `USERPROFILE`).
pub(crate) fn base_home(client: Client) -> Option<PathBuf> {
    registry::native_home(client)
}

/// `<base home>/<tool>-accounts/<label>` for a non-default account; None for `default` (the child keeps the
/// caller's own home). `tool` is the consumer's `Tool.name`.
pub(crate) fn account_home(client: Client, tool: &str, label: &str) -> Option<PathBuf> {
    (label != DEFAULT && valid_label(label)).then(|| base_home(client).map(|b| b.join(format!("{tool}-accounts")).join(label))).flatten()
}

// ---------- start and resume ----------

/// A new session ID: Claude takes a caller-chosen UUID; Codex and Antigravity name their session in output (`-` until then).
pub(crate) fn new_session(client: Client) -> String {
    registry::native_session(client)
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
        .or_else(|| registry::alternate_home_var(spec.client).and_then(|var|child_env(spec,var)).filter(|v| !v.is_empty()).map(PathBuf::from))
        .or_else(|| base_home(spec.client))
}

/// The client argument list after the executable, in TaskQ's order.
pub(crate) fn args(spec: &Spec) -> Vec<OsString> {
    registry::native_args(spec)
}

/// `base` (the client executable, or a sandbox wrapper around it) with the turn's arguments and environment. The
/// inherited session variables of an enclosing client are removed so a child never joins its parent's session.
pub(crate) fn command(mut base: Command, spec: &Spec) -> Command {
    base.args(args(spec));
    environment(base, spec)
}

/// A worker must not inherit another account's authentication or an interactive credential prompt.
fn clean_environment(command: &mut Command, account_bound: bool) {
    for var in registry::session_vars() {command.env_remove(var);}
    if account_bound {
        let explicit:Vec<_>=command.get_envs().filter_map(|(k,v)|v.map(|_|k.to_os_string())).collect();
        for var in registry::auth_vars() {if !explicit.iter().any(|k|k==var) {command.env_remove(var);}}
    }
    for (key,value) in [("GIT_ASKPASS",""),("SSH_ASKPASS",""),("GIT_TERMINAL_PROMPT","0"),("GCM_INTERACTIVE","never"),("SSH_ASKPASS_REQUIRE","never")] {
        if !command.get_envs().any(|(k,_)|k==key) {command.env(key,value);}
    }
    if !command.get_envs().any(|(k,_)|k=="GIT_SSH_COMMAND") && std::env::var_os("GIT_SSH_COMMAND").is_none() {command.env("GIT_SSH_COMMAND","ssh -oBatchMode=yes");}
}

fn environment(mut base: Command, spec: &Spec) -> Command {
    base.current_dir(&spec.root);
    clean_environment(&mut base, spec.home.is_some() || spec.env.iter().any(|(k,_)|k==spec.client.home_var()));
    base.envs(spec.env.iter().map(|(k, v)| (k, v)));
    if let Some(home) = &spec.home { base.env(spec.client.home_var(), home); }
    crate::quiet(&mut base);
    base
}

// ---------- running turn ----------

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
    readiness: Option<Readiness>,
    owner:Option<SessionOwner>,
    client: Client,
    /// The caller cancelled the run.
    cancelled: bool,
    child: Proc,
    /// The native exit code was not observed, or belongs to post-terminal cleanup.
    exit_unknown: bool,
    /// A child wait observed termination; a missing recovered identity is not a wait.
    reaped: bool,
    /// `native::scope` of the launch, bound into a checkpoint.
    scope: String,
    log: PathBuf,
    source: Option<String>,
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
    extension: Box<dyn TurnExtension>,
    receipt: Option<std::sync::mpsc::Receiver<Res<String>>>,
    /// Samples the owned process tree; None for a recovered child or a sidecar submission.
    sampler: Option<capture::Sampler>,
    /// The report and record identity, frozen at the terminal observation.
    terminal: Option<(Report, String)>,
}

fn unix_ms() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64) }

impl Turn {
    /// Spawn `command(base, spec)` with `text` on stdin; output appends to `log`, so a resume continues earlier turns.
    pub(crate) fn start(base: Command, spec: &Spec, log: &Path, text: &str, process: &Process) -> Res<Turn> {
        if let Some(dir) = log.parent() { std::fs::create_dir_all(dir).map_err(|e| format!("agent log directory: {e}"))?; }
        let out = File::options().create(true).append(true).open(log).map_err(|e| format!("agent log: {e}"))?;
        let err = out.try_clone().map_err(|e| format!("agent log: {e}"))?;
        let start = out.metadata().map_err(|e| format!("agent log: {e}"))?.len();
        let source=if spec.client==Client::Claude && spec.resume {Some(native::file_id(&out)?)} else {None};
        let home = spec_home(spec);
        let (mut base,extension)=(protocol(spec.client).prepare_command)(base,spec,text)?;
        let native_agentapi=extension.is_submission();
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
            let input=(protocol(spec.client).input_bytes)(text);
            // A client that never reads stdin must not block the caller; the writer ends with the pipe.
            std::thread::spawn(move || { let _ = stdin.write_all(&input); });
        } else {
            drop(stdin);
        }
        Ok(Turn { owner:None,readiness: None, client: spec.client, cancelled: false, child: Proc::Own(child), exit_unknown: false, reaped: false, scope: native::scope(spec), log: log.to_path_buf(), source, start, pos: start,
            ended: false, stopping: false, timed_out: false, started, started_ms, exit: None, home, resumed: spec.resume.then(|| spec.session.clone()), extension, receipt, sampler, terminal: None })
    }

    pub fn id(&self) -> u32 { self.child.id() }

    pub(crate) fn owned(mut self,owner:SessionOwner)->Self {self.owner=Some(owner);self}
    pub(crate) fn prepared(mut self, readiness: Readiness) -> Self {self.readiness=Some(readiness);self}

    /// Private recovery value of a running turn (docs/spec/RUNTIME.md § Turn recovery and process setup).
    pub(crate) fn checkpoint(&self) -> Result<Value, adapter::Cause> {
        if self.extension.is_submission() { return Err(adapter::Cause("no-checkpoint")); }
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
        Ok(Turn { owner:None,readiness: None, client: spec.client, cancelled: flag("cancelled")?, child: Proc::Seen(identity), exit_unknown: false, reaped: false, scope: native::scope(spec), log: log.to_path_buf(),source:None,
            start, pos: start, ended: false, stopping: false, timed_out: flag("timed_out")?, started: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
            started_ms, exit: None, home: spec_home(spec), resumed, extension: Box::new(StdioTurn), receipt: None, sampler: None, terminal: None })
    }

    fn sync_transcript(&mut self)->Res<()> {self.extension.synchronize(self.exit,&mut self.receipt,&self.resumed,self.home.as_deref(),&self.log)}
    /// Read complete new log lines; a turn-end event ends the turn even when the process lingers.
    fn read(&mut self) -> Res<()> {
        self.sync_transcript()?;
        let mut f = File::open(&self.log).map_err(|e| format!("agent log unreadable: {e}"))?;
        if f.metadata().map_err(|e| e.to_string())?.len() < self.pos { return Err("agent log truncated; turn boundary unknown".into()); }
        let mut rest = vec![];
        f.seek(SeekFrom::Start(self.pos)).and_then(|_| f.read_to_end(&mut rest)).map_err(|e| format!("agent log read: {e}"))?;
        let Some(end) = rest.iter().rposition(|b| *b == b'\n') else { return Ok(()) };
        self.pos += end as u64 + 1;
        for v in String::from_utf8_lossy(&rest[..end]).lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
            self.ended=(protocol(self.client).turn_ended)(self.ended,&v);
        }
        Ok(())
    }

    /// The exit code once the process exited, else None. After a turn-end event a lingering process is stopped
    /// (Codex on Windows kept running after its turn ended).
    pub fn poll(&mut self) -> Res<Option<i32>> {
        let code = self.poll_exit()?;
        if let Some(owner)=&self.owner {
            let report=self.report();
            if let Some(session)=report.session.as_deref() {owner.bind(session)?;}
            if code.is_some() && report.outcome!="unknown" {owner.finish(&report)?;}
        }
        if code.is_some() { self.freeze()?; }
        Ok(code)
    }

    /// A sidecar submission process can exit while its native turn remains live.
    fn has_exited(&self) -> bool {
        self.exit.is_some() && (!self.extension.is_submission() || self.stopping
            || self.receipt.is_none() && (self.exit != Some(0) || self.extension.session().is_none() || self.ended))
    }

    fn poll_exit(&mut self) -> Res<Option<i32>> {
        if self.extension.is_submission() {
            if self.exit.is_none() {
                self.exit = self.child.try_wait()?.map(|c| c.unwrap_or(-1));
            }
            self.read()?;
            return Ok(self.exit.filter(|_| self.has_exited()));
        }
        if self.exit.is_some() { return Ok(self.exit); }
        self.read()?;
        if self.exit.is_none() {
            if let Some(code) = self.child.try_wait()? {
                self.reaped = code.is_some();
                // After our stop of an ended turn, the code is the kill's (Windows 1, Unix -1), not the client's.
                self.exit_unknown = code.is_none() || self.ended && self.stopping;
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
        if self.exit.is_some() {return Ok(());}
        if let Some(owner)=&self.owner {if owner.stop_scope()? {return Ok(());}}
        self.child.kill()
    }

    /// Poll until exit; at `deadline` stop the process, and the turn reads as timed out.
    pub fn wait(&mut self, deadline: Option<Instant>) -> Res<i32> {
        loop {
            if let Some(code) = self.poll()? { return Ok(code); }
            if deadline.is_some_and(|d| Instant::now() >= d) && !self.timed_out {
                self.timed_out = true;
                let _ = self.kill();
                if self.extension.is_submission() {
                    if let Some(code) = self.exit {
                        self.freeze()?;
                        return Ok(code);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Immutable identity of this turn's records: recovery rebuilds the same one.
    fn key(&self, session: Option<&str>) -> String {
        crate::sha256_hex(format!("{}\0{}\0{}\0{}\0{}", self.client.code(), session.unwrap_or(""), self.log.display(), self.start, self.started_ms).as_bytes())
    }

    /// Fix the report at the terminal observation, once: later log bytes, a later cancel and elapsed time never change it.
    /// An unreadable log freezes nothing, so the turn stays unobserved and the next poll retries.
    pub(crate) fn freeze(&mut self) -> Res<()> {
        if self.terminal.is_some() { return Ok(()); }
        let report = self.read_report(&read_log(&self.log, self.start).map_err(|e| format!("agent log read: {e}"))?);
        let key = self.key(report.session.as_deref());
        self.terminal = Some((report, key));
        Ok(())
    }

    /// The report frozen at the terminal observation.
    pub(crate) fn terminal_report(&self) -> Option<Report> { self.terminal.as_ref().map(|(r, _)| r.clone()) }

    /// The turn's records from its frozen report; a turn without a terminal observation reads `running`.
    pub(crate) fn snapshot(&self, ctx: &Context) -> Res<Frozen> {
        let (r, key) = self.terminal.as_ref().ok_or("running")?;
        capture::freeze("agent.turn", key, &r.observation(self.client, ctx), r, self.client, ctx)
    }

    /// The report frozen at the terminal observation, else from the current log bytes; call after `poll` returned an exit.
    pub fn report(&self) -> Report {
        self.terminal_report().unwrap_or_else(|| self.read_report(&read_from(&self.log, self.start).unwrap_or_default()))
    }

    fn read_report(&self, text: &str) -> Report {
        let events: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let session = session_of(self.client, text).or_else(|| self.extension.session().map(String::from)).or_else(|| self.resumed.clone());
        let rollout=session.as_deref().and_then(|session|registry::native_transcript(self.client,session,self.home.as_deref()));
        let mut r = Report::read(self.client, text, &events, rollout.as_deref());
        r.session = session;
        r.owner=self.owner.as_ref().map(SessionOwner::observation).unwrap_or_default();
        r.readiness = self.readiness.clone();
        r.duration_us = self.started.elapsed().as_micros() as u64;
        r.started_ms = self.started_ms;
        r.exit = if self.exit_unknown { None } else { self.exit };
        r.termination = if self.extension.is_submission() { ProcessTermination::Unknown { reason: "sidecar-process-not-observed" } }
            else if self.reaped { ProcessTermination::Observed }
            else { ProcessTermination::Unknown { reason: if self.exit.is_some() { "recovered-process-exit-not-observed" } else { "native-process-exit-not-observed" } } };
        r.process = match &self.sampler {
            Some(s) => s.usage(),
            None if self.extension.is_submission() => capture::ProcessUse { exit_unknown: "exit-unknown", ..capture::ProcessUse::none("sidecar-server") },
            None => capture::ProcessUse { exit_unknown: "exit-unknown", ..capture::ProcessUse::none("recovered") },
        };
        (protocol(self.client).rollout_report)(rollout.as_deref(),&mut r);
        if r.outcome=="unknown" {
            if let Some(cause)=adapter::runtime(self.client.code()).and_then(|a|a.launch_refusal(text)) {(r.outcome,r.error)=("refused",cause.0);}
        }
        if self.client==Client::Claude && self.reaped && !self.exit_unknown && self.exit.is_some_and(|c|c!=0) && !self.cancelled && !self.timed_out {
            if let Some(failure)=registry::claude_start_failure(text,&events,self.resumed.as_deref(),r.session.as_deref()) {
                // A rejection is proof only for this owned source, never recovered/copied logs.
                let source_ok=self.source.as_ref().is_some_and(|id| {
                    File::open(&self.log).ok().is_some_and(|mut file| {
                        let mut observed=String::new();
                        native::file_id(&file).ok().as_ref()==Some(id) && file.seek(SeekFrom::Start(self.start)).is_ok()
                            && file.read_to_string(&mut observed).is_ok() && observed==text
                            && File::open(&self.log).ok().and_then(|f|native::file_id(&f).ok()).as_ref()==Some(id)
                    })
                });
                if source_ok {r.start_failure=Some(failure);}
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
pub(crate) fn run(base: Command, spec: &Spec, log: &Path, text: &str, deadline: Option<Instant>, ctx: &Context, store: &crate::telemetry::Store, readiness: Readiness,owner:SessionOwner,managed:bool,archive:&mut crate::archive::Handle) -> Res<Report> {
    let mut turn = Turn::start(base, spec, log, text, &owner.process_hooks(&Process::default(),managed)?)?.owned(owner).prepared(readiness);
    turn.wait(deadline)?;
    let mut r = turn.report();
    r.recorded = turn.snapshot(ctx).and_then(|f| {archive.record(&f);f.append(store)});
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
    /// Native-reported USD only; no inferred price.
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    /// Why no usage was read; None when some was.
    pub unknown: Option<&'static str>,
}

/// The turn's token sums of its calls; a call without a value makes that sum unknown, never smaller.
pub(crate) fn sum_usage(calls: &[Call], count: Option<u64>, known: bool) -> Usage {
    if !known { return Usage { unknown: Some("no-usage"), ..Usage::default() }; }
    if let Some(reason)=calls.iter().find_map(|c|c.accounting_unknown){return Usage {unknown:Some(reason),..Usage::default()};}
    let sum = |f: fn(&Call) -> Option<u64>| calls.iter().try_fold(0u64, |a, c| a.checked_add(f(c)?));
    let mut models = BTreeMap::<&str, u64>::new();
    for m in calls.iter().filter_map(|c| c.model.as_deref()) { *models.entry(m).or_default() += 1; }
    let model = models.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|m| m.0.to_string());
    Usage { calls: count, input: sum(|c| c.input), cache_read: sum(|c| c.cache_read), cache_write: sum(|c| c.cache_write), output: sum(|c| c.output),
        reasoning: if calls.is_empty() { None } else { sum(|c| c.reasoning) },
        cost_usd: if calls.is_empty(){None}else{calls.iter().try_fold(0.0,|n,c|Some(n+c.cost_usd?)).filter(|n|n.is_finite())}, model, unknown: None }
}

/// A model id is a short code; anything else stays unreported.
fn model_code(m: &str) -> bool { !m.is_empty() && m.len() <= 64 && m.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)) }

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum StartLimit { #[default] Pending, Working, Reached(Option<i64>) }

/// The native session of a turn: Claude's `session_id` (init or result), Codex's thread, or Antigravity's `conversationId`.
pub(crate) fn session_of(client:Client,text:&str)->Option<String> {(protocol(client).session)(text)}

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

/// Native process termination is separate from the turn outcome and numeric exit code.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProcessTermination { Observed, Unknown { reason: &'static str } }

/// What one turn did, read from its log.
#[derive(Clone, Debug)]
pub struct Report {
    /// Run-qualified native rejection; absent for log-only classification. Never telemetry.
    pub start_failure: Option<LaunchError>,
    /// Ordered launch evidence; absent for log-only classification.
    pub readiness: Option<Readiness>,
    pub owner:OwnerObservation,
    pub session: Option<String>,
    pub turn: Option<String>,
    pub images: Vec<Image>,
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
    /// Native terminal detail (Claude result subtype, Codex failed-turn message), else a classified failure reason.
    pub detail: Option<String>,
    /// Short classified native failure reason, independently of the full native detail.
    pub reason: Option<String>,
    pub text: Option<String>,
    pub exit: Option<i32>,
    pub termination: ProcessTermination,
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
    pub(crate) fn read(client:Client,text:&str,events:&[Value],rollout:Option<&Path>)->Report {(protocol(client).report)(text,events,rollout)}
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
        if let Some(readiness)=&self.readiness {v["counts"]=readiness.metadata();}
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
        history::live::turn_metadata(&mut v,self);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_prompt_defaults_preserve_explicit_helpers() {
        let mut command=Command::new("unused");
        clean_environment(&mut command,false);
        assert!(command.get_envs().any(|(k,v)|k=="GIT_ASKPASS" && v==Some(OsStr::new(""))));
        if std::env::var_os("GIT_SSH_COMMAND").is_none() {assert!(command.get_envs().any(|(k,v)|k=="GIT_SSH_COMMAND" && v==Some(OsStr::new("ssh -oBatchMode=yes"))));}
        command.env("GIT_ASKPASS","explicit-helper").env("GIT_SSH_COMMAND","explicit-ssh");
        clean_environment(&mut command,false);
        assert!(command.get_envs().any(|(k,v)|k=="GIT_ASKPASS" && v==Some(OsStr::new("explicit-helper"))));
        assert!(command.get_envs().any(|(k,v)|k=="GIT_SSH_COMMAND" && v==Some(OsStr::new("explicit-ssh"))));
    }


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
            assert_eq!(base_home(Client::Antigravity), Some(home.join(".gemini")));
            std::fs::create_dir_all(&cli).unwrap();
            assert_eq!(base_home(Client::Antigravity), Some(home.join(".gemini")));
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
            .env_remove("ANTIGRAVITY_AGENTAPI_EXE").env_remove("GEMINI_HOME").status().unwrap();
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
        let u = test_usage(Client::Claude,&claude);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model.as_deref()), (Some(2), Some(3), Some(70), Some(5), Some(10), Some("claude-opus-5-5")));
        let codex = events(r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":9}}"#);
        let u = test_usage(Client::Codex,&codex);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model), (None, Some(20), Some(80), Some(0), Some(9), None));
        let partial = events(r#"{"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":9}}"#);
        assert_eq!((test_usage(Client::Codex,&partial).input, test_usage(Client::Codex,&partial).cache_read), (None, None), "a missing count is unknown, never zero");
        let ag = events(concat!(
            r#"{"response":{"newConversation":{"conversationId":"conv-1"}}}"#, "\n",
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":120,"cache_read_tokens":500,"cache_write_tokens":0,"output_tokens":15,"tool_calls":[{"id":"c1"}]}"#, "\n",
            r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":120,"cache_read_tokens":500,"cache_write_tokens":0,"output_tokens":15,"tool_calls":[{"id":"c1"}]}"#, "\n",
            r#"{"step_index":3,"type":"PLANNER_RESPONSE","status":"DONE","model":"gemini-3.1-pro","input_tokens":80,"cache_read_tokens":600,"cache_write_tokens":0,"output_tokens":25,"content":"done"}"#, "\n"));
        let u = test_usage(Client::Antigravity,&ag);
        assert_eq!((u.calls, u.input, u.cache_read, u.cache_write, u.output, u.model.as_deref()), (Some(2), Some(200), Some(1100), Some(0), Some(40), Some("gemini-3.1-pro")));
        let r_ag = Report::read(Client::Antigravity, &ag.iter().map(Value::to_string).collect::<Vec<_>>().join("\n"), &ag, None);
        assert_eq!((r_ag.outcome, r_ag.session.as_deref(), r_ag.text.as_deref()), ("ok", Some("conv-1"), Some("done")));
        let missing = test_usage(Client::Antigravity,&events(r#"{"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","input_tokens":3,"output_tokens":2}"#));
        assert_eq!((missing.cache_read, missing.cache_write), (None, None), "P2: unreported counts stay unknown");
        assert!((protocol(Client::Antigravity).ends)(&json!({"type":"PLANNER_RESPONSE","status":"DONE","tool_calls":[]})), "empty tool calls end the turn");
        assert_eq!(test_usage(Client::Antigravity,&[]).unknown, Some("no-usage"));
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
        let u = test_usage(Client::Antigravity,&missing);
        assert_eq!((u.input,u.output,u.calls),(None,Some(3),None),"latest partial step stays unknown and cumulative result is ignored");
    }

    // M12.1: a bounded poll must progress through a full transcript larger than its read budget.

    // M12.1/P2: partial writes, resume history and replacement must not fabricate completion or usage.

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
        assert_eq!(limit_reached(Client::Codex,&events(worked)), None, "after work started a limit is not a start refusal");
        let codex = r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit."}}"#;
        assert_eq!(limit_reached(Client::Codex,&events(codex)), Some(None));
        assert_eq!(registry::codex_auth_rejected("2026 ERROR codex_core::auth: refresh token was already used"), Some("refresh token rejected"));
        assert_eq!(registry::codex_auth_rejected(r#"{"type":"item.completed","item":{"text":"401 Unauthorized"}}"#), None, "worker output never matches");
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

pub(crate) fn snapshot(client:Client,events:&[Value],rollout:Option<&Path>)->Snapshot {(protocol(client).snapshot)(events,rollout)}

pub(crate) trait TurnExtension: Send {
    fn is_submission(&self)->bool;
    fn session(&self)->Option<&str>;
    fn synchronize(&mut self,exit:Option<i32>,receipt:&mut Option<std::sync::mpsc::Receiver<Res<String>>>,resumed:&Option<String>,home:Option<&Path>,log:&Path)->Res<()>;
}
struct StdioTurn;
impl TurnExtension for StdioTurn {
    fn is_submission(&self)->bool {false}
    fn session(&self)->Option<&str> {None}
    fn synchronize(&mut self,_:Option<i32>,_:&mut Option<std::sync::mpsc::Receiver<Res<String>>>,_:&Option<String>,_:Option<&Path>,_:&Path)->Res<()> {Ok(())}
}
fn stdio_command(base:Command,spec:&Spec,_:&str)->Res<(Command,Box<dyn TurnExtension>)> {Ok((command(base,spec),Box::new(StdioTurn)))}
fn plain_input(text:&str)->Vec<u8> {text.as_bytes().to_vec()}

use registry::parsing::*;
#[cfg(test)]
use registry::claude_settings;

fn protocol(client:Client)->&'static registry::Protocol {registry::protocol(client)}

impl Report {
    fn empty()->Self {Report {start_failure:None,owner:OwnerObservation::default(),readiness:None,session:None,turn:None,images:vec![],usage:sum_usage(&[],None,false),limit:Snapshot {unknown:Some("not-reported"),..Snapshot::default()},limits:limits::unknown("not-reported"),reached:None,background_job:false,live_background:vec![],outcome:"unknown",error:"no-end",detail:None,reason:None,text:None,exit:None,termination:ProcessTermination::Unknown {reason:"native-process-exit-not-observed"},started_ms:unix_ms(),duration_us:0,recorded:Ok(()),calls:vec![],process:ProcessUse::default()}}
}

#[cfg(test)]
fn test_usage(client:Client,events:&[Value])->Usage {let (calls,count,known)=(protocol(client).read_calls)(events);sum_usage(&calls,count,known)}
/// Standard padded base64 for the archive wire.
pub(crate) fn encode_base64(bytes: &[u8]) -> String { adapter::b64(bytes) }

/// Physical file/directory identity for archive and hook replacement detection; not execution authority.
pub fn archive_file_identity(file:&File)->Res<String> {native::file_id(file)}

#[cfg(test)]
mod start_failure_tests {
    use super::*;
    #[test]
    fn start_failure_diagnostics_never_enter_metadata() {
        let mut report=Report::empty();
        report.start_failure=Some(LaunchError {stage:InputStage::BeforeInput,cause:Some(Cause("session-missing")),session:Some("private-session".into()),detail:"private-diagnostic".into(),native:Some(json!({"token":"private-token"}))});
        let context=Context {product:"fixtureq",version:"1",project:"other",task:None,route:None,account:"test"};
        let metadata=report.observation(Client::Claude,&context).to_string();
        for secret in ["private-session","private-diagnostic","private-token","start_failure"] {assert!(!metadata.contains(secret));}
    }
}

/// Attribute runtime launches independently of installation or retired optimization.
pub(crate) fn attribute(product: &str, spec: &mut Spec) {
    let set = |k: &str| child_env(spec, k).is_some_and(|v| !v.is_empty());
    if !set("TASKQ_TASK") && !set("AGENTQ_LAUNCHER") { spec.env.push(("AGENTQ_LAUNCHER".into(), product.into())); }
}
