//! M12.9/M12.10: the one runtime trait, its descriptors and the per-runtime adapters (docs/spec/RUNTIME.md § Runtime trait).
//! Callers read only `Descriptor` data and pass an abstract `Policy`; every native flag, file and format stays here.
use super::*;
use crate::optimization::{self, Intent, Optimization};
use super::native::{BackgroundJob, NativeTurn};
use super::app::{AppServer, AppSession};
use super::protected::ProtectedAppServer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::process::Output;
#[path = "preflight.rs"]
mod preflight;
pub use preflight::{Readiness, ReadinessMode, LaunchCheck, Repair};
#[path="ownership.rs"]
mod ownership;
pub use ownership::{SessionStore,SessionOwner,NativeProcess,OwnerObservation,ResumeCommand};
#[path="activity.rs"]
mod activity;
pub use activity::ActivityObservation;
#[path="lifetime.rs"]
mod lifetime;
pub use lifetime::Lifetime;
pub use ownership::Orphan;

/// The only runtime metadata a caller reads. `name` is an opaque ID; `label` and `brand` are for people.
#[derive(Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static str,
    pub label: &'static str,
    pub brand: &'static str,
    pub family: &'static str,
    /// Model aliases a caller may offer without a catalogue read.
    pub models: &'static [&'static str],
    /// Oldest supported CLI version; empty: none.
    pub minimum: &'static str,
    pub capabilities: &'static [&'static str],
}

impl Descriptor {
    pub fn has(&self, capability: &str) -> bool { self.capabilities.contains(&capability) }
    pub fn history_resume(&self, session:&str, launch:&Launch, native:&[NativeProcess])->Res<ResumeCommand> {
        let adapter=Client::parse(self.name).map(registry::builtin).ok_or("history resume unsupported")?;
        if launch.mode!=Mode::Turn || !launch.input.is_empty() || !launch.parts.is_empty() || launch.deltas || launch.mcp.is_some() || launch.instructions.is_some() {return Err("terminal resume requires empty input without relay options".into());}
        if launch.deadline.is_some() || launch.inactivity.is_some() || launch.stop_on_stall {return Err("terminal resume cannot apply worker observation windows".into());}
        let args=super::history::sessions::resume(self.name,session)?;
        adapter.preflight(launch,&Repair::default()).permit(launch.readiness)?;
        adapter.adopt(session,launch,native)?;
        let owner=adapter.session_owner(Some(session),launch)?;
        let (spec,_)=adapter.spec(launch,session.into(),true)?;
        let mut command=preflight::launch_base(adapter,launch)?;
        command.args(&spec.args).args(&args[1..]);
        Ok(ResumeCommand {command:super::environment(command,&spec),owner,arguments:args,process:launch.process.clone(),managed:self.has("process-lifetime")})
    }
}

/// Filesystem scope of a policy part; `Native` keeps the runtime's own default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Scope { #[default] Native, None, Project, Any }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Network { #[default] Native, On, Off }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Approval { #[default] Native, Plan, Edits, Auto, /** Never prompt; a request that needs approval is denied. */ Deny, Bypass }

/// The runtime's own shell and web tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Builtin { #[default] Native, On, Off }

/// Extra writable folders beside the workspace; every path is absolute.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Roots {
    /// The runtime's configured roots.
    #[default]
    Native,
    /// Exactly these folders; configured roots never widen them.
    Exact(Vec<PathBuf>),
    /// These folders added to the configured roots.
    Add(Vec<PathBuf>),
}

/// Abstract permissions. The adapter maps each part to native flags or refuses the launch.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub read: Scope,
    pub write: Scope,
    pub network: Network,
    /// None keeps the native tool set; Some is an allowlist.
    pub tools: Option<Vec<String>>,
    pub approval: Approval,
    pub roots: Roots,
    /// The runtime's own Windows sandbox runs without elevation.
    pub unelevated: bool,
    pub builtin: Builtin,
    /// No user settings, plugins, slash commands or extra MCP servers.
    pub isolated: bool,
    /// Subscription login only, never an API key.
    pub subscription: bool,
}

/// Runs on the child `Command` after the adapter's own setup, directly before spawn.
pub type Configure = std::sync::Arc<dyn Fn(&mut Command) + Send + Sync>;
/// Runs on the spawned child before any input; an error kills the child and fails the launch.
pub type Spawned = std::sync::Arc<dyn Fn(&std::process::Child) -> Result<(), String> + Send + Sync>;

/// Caller process setup of a CLI turn or app-server child (docs/spec/RUNTIME.md § Turn recovery and process setup).
#[derive(Clone, Default)]
pub struct Process { pub configure: Option<Configure>, pub spawned: Option<Spawned> }

impl std::fmt::Debug for Process {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Process").field("configure", &self.configure.is_some()).field("spawned", &self.spawned.is_some()).finish()
    }
}

impl Process {
    pub(crate) fn spawn(&self, mut command: Command, what: &str) -> Res<std::process::Child> {
        if let Some(c) = &self.configure { c(&mut command); }
        let mut child = command.spawn().map_err(|e| format!("{what} spawn: {e}"))?;
        if let Some(s) = &self.spawned {
            if let Err(e) = s(&child) { let _ = child.kill(); let _ = child.wait(); return Err(e); }
        }
        Ok(child)
    }
}

/// One launch or resume request. Every field is runtime-neutral.
#[derive(Clone, Debug)]
pub struct Launch {
    /// Project root and working directory.
    pub root: PathBuf,
    /// Caller log; output of every turn of the session appends here.
    pub log: PathBuf,
    /// Turn text.
    pub input: String,
    pub policy: Policy,
    pub model: Option<String>,
    /// Reasoning effort, a plain word (capability `effort`).
    pub effort: Option<String>,
    /// Command run after native context compaction (capability `compact-hook`).
    pub compact_hook: Option<String>,
    /// Child environment, including `home_env` of a non-default account.
    pub env: Vec<(OsString, OsString)>,
    /// Caller command replacing the native executable (a sandbox wrapper); empty uses the native CLI.
    pub command: Vec<OsString>,
    pub optimization: Intent,
    /// Product code for `AGENTQ_LAUNCHER`.
    pub product: String,
    /// AgentQ installation read for optimization state; None reads no installation.
    pub install: Option<PathBuf>,
    /// Accounts checked by `quota` before launch; None skips the check.
    pub accounts: Option<Accounts>,
    pub mode: Mode,
    /// Session title (capability `title`).
    pub title: Option<String>,
    /// A CLI turn still running at this instant is stopped and reads `cancelled`/`timeout`.
    pub deadline: Option<Instant>,
    /// Caller setup of the child process.
    pub process: Process,
    /// Login home of the run; None keeps the caller's own.
    pub home: Option<PathBuf>,
    /// MCP config file of a CLI turn (capability `mcp-config`); its servers join the tools allowlist.
    pub mcp: Option<PathBuf>,
    /// File appended to the system prompt of a CLI turn (capability `instructions-file`).
    pub instructions: Option<PathBuf>,
    /// Ordered turn input replacing `input` (capability `turn-parts`).
    pub parts: Vec<Part>,
    /// Partial text records in `progress` (capability `turn-deltas`; app-server runs always stream them).
    pub deltas: bool,
    /// Operator-configured native arguments of a turn (capability `host-args`); needs the default `Policy`.
    pub host_args: Vec<OsString>,
    /// Session environment (capability `settings-env`). A Claude launch puts it in argv: never secrets; use `env` for those.
    pub settings_env: Vec<(String, String)>,
    /// Unknown probes continue by default; strict refuses them.
    pub readiness: ReadinessMode,
    /// Caller telemetry store checked before work; None means no store was requested.
    pub store: Option<PathBuf>,
    pub session_store:SessionStore,
    pub inactivity:Option<Duration>,
    pub stop_on_stall:bool,
    pub lifetime:Lifetime,
}

/// How a run reaches the runtime; every mode but `Turn` needs the descriptor capability of the same name.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Mode {
    /// One CLI turn (capability `turn`).
    #[default]
    Turn,
    /// A native background job (capability `background`); `Launch.input` is the caller's protected file reference.
    Background,
    /// Queue `Launch.input` (a reference) into a running desktop thread and observe its native `transcript` (capability `app-queue`).
    Queue { transcript: PathBuf },
    /// A protected app-server session (capability `app-server`), opened only by `ProtectedAppServer::open`: caller `initialize` and `thread` parameters pass through
    /// unchanged, plus the mapped policy and model; `Launch.log` is the private stderr log.
    AppServer { initialize: Value, thread: Value },
}

impl Mode {
    fn capability(&self) -> &'static str {
        match self { Mode::Turn => "turn", Mode::Background => "background", Mode::Queue { .. } => "app-queue", Mode::AppServer { .. } => "app-server" }
    }
}

impl Launch {
    pub fn new(root: impl Into<PathBuf>, log: impl Into<PathBuf>, input: impl Into<String>, product: impl Into<String>) -> Self {
        let log=log.into();let session_store=SessionStore::new(log.parent().unwrap_or(Path::new(".")).join(".agentq-sessions"));
        Launch { root: root.into(), log, input: input.into(), policy: Policy::default(), model: None, effort: None, compact_hook: None,
            env: vec![], command: vec![], optimization: Intent::Inherit, product: product.into(), install: None, accounts: None,
            mode: Mode::Turn, title: None, deadline: None, process: Process::default(), home: None, mcp: None, instructions: None, parts: vec![], deltas: false, host_args: vec![], settings_env: vec![], readiness: ReadinessMode::default(), store: None, session_store, inactivity:None,stop_on_stall:false,lifetime:Lifetime::default() }
    }
}

/// A named reason an operation has no value for this runtime or run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause(pub &'static str);

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}

/// A typed operation error: `Refused` before any native write, `Failed` after a native or transport error (never retried).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure { Refused(Cause), Failed(String) }

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Failure::Refused(c) => write!(f, "{c}"), Failure::Failed(e) => f.write_str(e) }
    }
}

impl From<String> for Failure { fn from(e: String) -> Self { Failure::Failed(e) } }
impl From<&str> for Failure { fn from(e: &str) -> Self { Failure::Failed(e.into()) } }

/// One ordered part of user input. An image is a local file the caller already verified; only its absolute path is forwarded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part { Text(String), Image(PathBuf) }

/// Native IDs are opaque data selected from session metadata and pending requests.
#[derive(Clone, Debug)]
pub enum Control {
    Model(String),
    Mode(String),
    Reply { id: Value, option: Option<String> },
}

/// How a session's native account is signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Login { Subscription, ApiKey, None }

/// Read-only login identity of a session; it carries no credential, token or email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account { pub login: Login, pub plan: Option<String> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Running, Finished, Failed, Unknown }

/// One runtime-neutral observation of a run (RUNTIME.md Session observation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub state: State,
    pub phase: &'static str,
    pub starting_ms: u64,
    pub owner:OwnerObservation,
    pub activity:ActivityObservation,
    /// Why `state` reads `unknown` when the poll failed.
    pub error: Option<String>,
    pub session: Option<String>,
    /// The session URL, else `cause` names its absence.
    pub url: Option<String>,
    pub cause: Option<Cause>,
    /// Native evidence text behind `cause`, such as Claude's logged registration error.
    pub detail: Option<String>,
}

impl State {
    pub fn code(self) -> &'static str {
        match self { State::Running => "running", State::Finished => "finished", State::Failed => "failed", State::Unknown => "unknown" }
    }
}

/// One readiness part: `ok`, `missing` or `unknown`, with the value or the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check { pub state: &'static str, pub detail: String }

/// Numeric parts of the first dotted version token of `s`: `codex-cli 0.162.0` and `2.1.5 (Claude Code)` read alike.
pub(super) fn version_parts(s: &str) -> Option<Vec<u64>> {
    s.split(|c: char| c.is_whitespace() || c == '(').find_map(|t| {
        let v: Option<Vec<u64>> = t.trim_start_matches('v').split('.').map(|p| p.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).collect();
        v.filter(|v| v.len() >= 2)
    })
}

pub(super) fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { s.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' }); }
    }
    s
}

/// Claude stream-json turn input: one user message, images as base64 blocks.
fn stream_message(parts: &[Part]) -> Res<String> {
    let mut content = vec![];
    for p in parts {
        content.push(match p {
            Part::Text(t) => json!({"type": "text", "text": t}),
            Part::Image(path) => {
                if !path.is_absolute() { return Err("image-invalid".into()); }
                let bytes = std::fs::read(path).map_err(|_| "image-invalid".to_string())?;
                let media = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
                    "png" => "image/png", "webp" => "image/webp", "gif" => "image/gif", _ => "image/jpeg" };
                json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": b64(&bytes)}})
            }
        });
    }
    Ok(format!("{}\n", json!({"type": "user", "message": {"role": "user", "content": content}})))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready { pub install: Check, pub version: Check, pub login: Check }

/// Quota before launch: the account a launch takes, whether one is free now, and the hold end when none is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quota { pub account: Option<String>, pub free: bool, pub off: bool, pub hold: Option<(i64, bool)> }

/// Decrements the adapter's live count once: at the first terminal poll or on drop.
struct Live(&'static AtomicUsize);
impl Drop for Live { fn drop(&mut self) { self.0.fetch_sub(1, Ordering::SeqCst); } }

/// One running or finished run. Opaque: operate on it through its `Runtime`.
pub struct Run {
    stop_id: String,
    runtime: &'static str,
    body: Body,
    optimization: Optimization,
    progress: u64,
    events: super::events::Cursor,
    rollout_progress: Option<(PathBuf, u64)>,
    live: Option<Live>,
    recorded: bool,
    /// The records of the finished turn, unchanged across repeated snapshots.
    frozen: Option<super::Frozen>,
    deadline: Option<Instant>,
    /// The launch requested Claude Remote Control (`title`).
    remote: bool,
    readiness: Readiness,
    starting: Instant,
    owner:Option<SessionOwner>,
    activity:activity::Activity,
    inactivity:Option<Duration>,
    stop_on_stall:bool,
}

enum Body { Turn(Turn), Native(Box<Native>), App(Box<App>), Acp(Box<super::acp::Session>) }

/// A background or queue run: an observation cursor over the native transcript, never a process owner.
struct Native {
    queue: bool,
    turn: NativeTurn,
    spec: Spec,
    command: Vec<OsString>,
    /// The launcher's job ID (background).
    job_id: Option<String>,
    job: Option<BackgroundJob>,
    /// The native transcript is bound: always for queue, after job discovery for a new background job.
    attached: bool,
    /// An uncertain launcher or queue response; the run never resubmits.
    fault: Option<String>,
    report: Option<Report>,
    /// The launcher exchange until `launcher_output` takes it; not part of the checkpoint.
    launcher: Option<Result<Output, crate::process::Failure>>,
}

impl Run {
    pub(crate) fn stop_facts(&mut self) -> Res<super::history::stop::Facts> {
        use super::history::stop::Facts;
        Ok(match &mut self.body {
            Body::Acp(s) => s.stop_facts(self.stop_id.clone())?,
            Body::Turn(t) => Facts {run:self.stop_id.clone(),format:protocol(t.client).stream_history,path:t.log.clone(),start:t.start,exited:Some(t.has_exited()),native_end:t.ended,exit:if t.exit_unknown {None}else{t.exit},lost_owner:t.exit_unknown && !(t.ended && t.stopping)},
            Body::Native(n) => Facts {run:self.stop_id.clone(),format:protocol(n.spec.client).native_history,path:n.turn.path().into(),start:0,exited:None,native_end:n.report.is_some(),exit:None,lost_owner:false},
            Body::App(a) => {
                let exit=if a.owned {a.transport.as_mut().map(Transport::exit).transpose()?.flatten()}else{None};
                Facts {run:self.stop_id.clone(),format:"codex-app-server",path:PathBuf::new(),start:0,exited:if a.owned {Some(exit.is_some())}else{None},native_end:a.session.report().is_some(),exit,lost_owner:a.closed.is_some() && exit.is_none()}
            },
        })
    }
    fn restored(mut self, saved: &Value) -> Res<Self> {
        self.stop_id=saved["stop_id"].as_str().filter(|s|native::plain_id(s)).ok_or("run stop identity invalid")?.into();
        if let Some(value)=saved.get("activity") {self.activity=serde_json::from_value(value.clone()).map_err(|_|"activity checkpoint invalid")?;}
        if let Some(value)=saved.get("readiness") {self.readiness=serde_json::from_value(value.clone()).map_err(|_|"run readiness checkpoint invalid")?;}
        if let Some(frozen)=saved.get("frozen") { self.frozen=Some(super::Frozen::restore(frozen)?); }
        if let Some(progress)=saved.get("progress") { self.progress=progress.as_u64().ok_or("run progress checkpoint invalid")?; }
        if let Some(events)=saved.get("events") { self.events=serde_json::from_value(events.clone()).map_err(|_|"run event checkpoint invalid")?; }
        if let Some(value)=saved.get("rollout_progress").filter(|v|!v.is_null()) {
            let (path,offset):(PathBuf,u64)=serde_json::from_value(value.clone()).map_err(|_|"rollout checkpoint invalid")?;
            let Body::Turn(t)=&self.body else {return Err("rollout checkpoint scope differs".into());};
            let session=t.resumed.clone().or_else(||session_of(t.client,&read_from(&t.log,t.start).unwrap_or_default()));
            let expected=session.and_then(|s|(table(t.client).ops.transcript)(&s,t.home.as_deref()));
            if expected.as_ref()!=Some(&path) {return Err("rollout checkpoint scope differs".into());}
            self.rollout_progress=Some((path,offset));
        }
        Ok(self)
    }
    /// The descriptor name of the runtime that owns this run.
    pub fn owner(&self)->Option<&SessionOwner> {self.owner.as_ref()}
    pub fn readiness(&self) -> &Readiness { &self.readiness }
    pub fn runtime(&self) -> &'static str { self.runtime }
    /// The optimization state configured at launch, never proof of compression.
    pub fn optimization(&self) -> &Optimization { &self.optimization }
    /// The direct child process ID of a CLI turn, or the verified background job's native process.
    pub fn id(&self) -> Option<u32> {
        match &self.body { Body::Turn(t) => Some(t.id()), Body::Native(n) => n.job.as_ref().and_then(|j| j.pid), Body::App(a) => a.transport.as_ref().map(Transport::id), Body::Acp(a)=>Some(a.id()) }
    }
    /// The finished run's immutable records for a deferred append, without a store, lock or native call; a repeated call
    /// returns the same value with the first caller context. A run without a terminal observation reads `running`.
    pub fn snapshot(&mut self, ctx: &Context) -> Res<super::Frozen> {
        if let Some(frozen) = &self.frozen { return Ok(frozen.clone()); }
        let frozen = match &self.body {
            Body::Acp(a)=> {let report=a.report()?; let key=crate::sha256_hex(format!("{}\0{}\0{}\0{}",self.runtime,a.session,a.path.display(),a.boundary).as_bytes()); super::capture::freeze("agent.turn",&key,&report.observation_as(self.runtime,ctx),&report,Client(self.runtime),ctx)?},
            Body::Turn(t) => t.snapshot(ctx)?,
            Body::Native(n) => n.turn.snapshot(n.report.as_ref().ok_or("running")?, ctx)?,
            // Each app-server turn records once under its native thread and turn identity.
            Body::App(a) => a.session.snapshot(ctx)?.ok_or("running")?,
        };
        let mut saved = frozen.save();
        saved["records"][0]["observation"]["counts"] = self.readiness.metadata();
        let frozen = super::Frozen::restore(&saved)?;
        self.frozen = Some(frozen.clone());
        Ok(frozen)
    }
    /// Append the finished run's one `agent.turn` observation (M12.4); a second call after a success is refused, never counted twice.
    pub fn record(&mut self, ctx: &Context, store: &crate::telemetry::Store) -> Res<Report> {
        if self.recorded { return Err("run already recorded".into()); }
        let mut r = match &self.body {
            Body::Acp(a)=>a.report().ok(),
            Body::Turn(t) => t.terminal_report(),
            Body::Native(n) => n.report.clone(),
            Body::App(a) => a.session.report(),
        }.ok_or("running")?;
        r.owner=self.owner.as_ref().map(SessionOwner::observation).unwrap_or_default();
        r.readiness = Some(self.readiness.clone());
        r.recorded = self.snapshot(ctx).and_then(|f| f.append(store));
        self.recorded = r.recorded.is_ok();
        Ok(r)
    }
}

fn base(command: &[OsString], client: Client) -> Res<Command> {
    Ok(match command.split_first() {
        Some((exe, rest)) => { let mut c = Command::new(exe); c.args(rest); c }
        None => Command::new(executable(client)?),
    })
}

/// The verified background job: the launcher's job ID, else on resume the job of the same session.
fn discover(n: &Native) -> Res<Option<BackgroundJob>> {
    let find = |rows: &[Value]| rows.iter().find(|r| match &n.job_id {
        Some(id) => r["id"] == id.as_str(),
        None => n.spec.resume && r["kind"] == "background" && r["sessionId"] == n.turn.session(),
    }).cloned();
    let mut row = find(&(table(n.spec.client).ops.background_jobs)(base(&n.command, n.spec.client)?, &n.spec, true, Duration::from_secs(5))?);
    if row.is_none() { row = find(&(table(n.spec.client).ops.background_jobs)(base(&n.command, n.spec.client)?, &n.spec, false, Duration::from_secs(5))?); }
    let Some(row) = row else { return Ok(None) };
    (table(n.spec.client).ops.background_job)(&row, &n.spec, row["id"].as_str().unwrap_or_default()).map(Some)
}

fn state(r: &Report) -> State { match r.outcome { "ok" => State::Finished, "unknown" => State::Unknown, _ => State::Failed } }

/// The stdio transport of an app-server run: a plain child, or a protected host frontend that verifies authority per write.
pub(crate) enum Transport { Plain(AppServer), Protected(Box<ProtectedAppServer>) }

impl Transport {
    fn send(&mut self, m: &Value, until: Instant) -> Res<()> { match self { Transport::Plain(s) => s.send(m, until), Transport::Protected(s) => s.send(m, until) } }
    fn receive(&mut self, until: Instant) -> Res<Option<Value>> { match self { Transport::Plain(s) => s.receive(until), Transport::Protected(s) => s.receive(until) } }
    fn id(&self) -> u32 { match self { Transport::Plain(s) => s.id(), Transport::Protected(s) => s.id() } }
    fn exit(&mut self) -> Res<Option<i32>> { match self { Transport::Plain(s) => s.poll(), Transport::Protected(s) => s.poll() } }
}

/// An app-server session: the caller drives turns with `input`, answers server requests with `send`, reads `progress`.
struct App {
    transport: Option<Transport>,
    session: AppSession,
    /// Native messages received but not yet returned by `progress`.
    pending: Vec<Value>,
    model: Option<String>,
    /// Why the transport ended; the turn outcome stays native evidence only.
    closed: Option<String>,
    owned: bool,
    requests: Vec<Value>,
    effort: Option<String>,
}

impl Drop for App {
    fn drop(&mut self) {
        if self.owned { if let Some(Transport::Plain(s)) = &mut self.transport { let _ = s.close(); } }
    }
}

fn automatic(l: &Launch) -> bool { l.mode == Mode::Turn && (l.mcp.is_some() || l.instructions.is_some() || !l.parts.is_empty() || l.deltas) }

/// Ordinary turns accept only declared needs; native protocol parameters remain protected-host data.
fn turn_thread(l: &Launch) -> Res<Value> {
    if l.parts.is_empty() && l.input.is_empty() { return Err("input-empty".into()); }
    let mut policy = l.clone(); policy.effort = None;
    let mut t = thread_params(&policy, &json!({}))?;
    if let Some(e) = &l.effort {
        if e.is_empty() || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err("effort requires a plain word".into()); }
    }
    t["cwd"] = json!(l.root);
    t["excludeTurns"] = json!(true);
    if let Some(path) = &l.instructions { t["developerInstructions"] = json!(std::fs::read_to_string(path).map_err(|_| "instructions unreadable")?); }
    if let Some(path) = &l.mcp {
        mcp_servers(path)?;
        let v: Value = serde_json::from_str(&std::fs::read_to_string(path).map_err(|_| "mcp config unreadable")?).map_err(|_| "mcp config invalid")?;
        let mut servers = serde_json::Map::new();
        for (name, v) in v["mcpServers"].as_object().ok_or("mcp config names no servers")? {
            let object = v.as_object().ok_or("mcp server must be an object")?;
            if object.keys().any(|k| !matches!(k.as_str(), "command" | "args" | "env")) { return Err("mcp server field unsupported; expected command, args, env".into()); }
            if v["command"].as_str().is_none_or(|s| s.is_empty()) || v.get("args").is_some_and(|a| a.as_array().is_none_or(|a| a.iter().any(|v| !v.is_string())))
                || v.get("env").is_some_and(|e| e.as_object().is_none_or(|e| e.values().any(|v| !v.is_string()))) { return Err("mcp server command, args or env invalid".into()); }
            let mut server = v.clone(); server["default_tools_approval_mode"] = json!("approve");
            servers.insert(name.clone(), server);
        }
        t["config"]["mcp_servers"] = servers.into();
    }
    Ok(t)
}

fn reply_deadline(l: &Launch) -> Instant { l.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30)) }

/// `thread` with the abstract policy and model as app-server thread parameters.
fn thread_params(l: &Launch, thread: &Value) -> Res<Value> {
    let p = &l.policy;
    let refuse = |part: &str| Err(format!("policy {part} unsupported by app-server"));
    if !thread.is_object() { return Err("app-server thread params must be an object".into()); }
    if !matches!(p.read, Scope::Native | Scope::Any) { return refuse("read"); }
    if p.network != Network::Native { return refuse("network"); }
    if p.tools.is_some() { return refuse("tools"); }
    if p.roots != Roots::Native { return refuse("roots"); }
    if p.unelevated { return refuse("unelevated"); }
    if l.effort.is_some() { return Err("effort unsupported by app-server".into()); }
    if l.title.is_some() || l.compact_hook.is_some() { return Err("title and compact-hook unsupported by app-server".into()); }
    let mut t = thread.clone();
    match p.write { Scope::Native => {}, Scope::None => t["sandbox"] = json!("read-only"), Scope::Project => t["sandbox"] = json!("workspace-write"), Scope::Any => t["sandbox"] = json!("danger-full-access") }
    match (p.approval, p.write) { (Approval::Native, _) => {}, (Approval::Bypass, Scope::Any) | (Approval::Deny, _) => t["approvalPolicy"] = json!("never"), _ => return refuse("approval") }
    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        t["model"] = json!(m);
    }
    Ok(t)
}

impl App {
    fn observe(&mut self, m: Value, until: Instant) -> Res<()> {
        self.session.observe(&m)?;
        if m.get("id").is_some() && m["method"].is_string() {
            if self.owned {
                let reply = json!({"id":m["id"],"error":{"code":-32601,"message":"No approval or native tool handler for this turn"}});
                self.transport.as_mut().ok_or("app-server transport ended")?.send(&reply, until)?;
            } else { self.requests.push(m); }
        } else { self.pending.push(m); }
        Ok(())
    }
    /// Exchange one client request and read until its reply; every message read stays pending for `progress`.
    fn exchange(&mut self, request: Value, until: Instant) -> Res<()> {
        self.transport.as_mut().ok_or("app-server transport ended")?.send(&request, until)?;
        loop {
            let m = self.transport.as_mut().ok_or("app-server transport ended")?.receive(until)?.ok_or("app-server reply deadline; session state unknown")?;
            let reply = m["id"] == request["id"] && m.get("method").is_none();
            self.observe(m, until)?;
            if reply { return Ok(()); }
        }
    }

    /// Initialize, open the thread and, with input, start the first turn.
    /// A failed handshake stops a plain child; a protected frontend stays with its host.
    fn open(spec: &Spec, mut transport: Transport, initialize: &Value, thread: &Value, l: &Launch, owner:&SessionOwner) -> Res<App> {
        let mut session = match AppSession::new(spec) {
            Ok(s) => s,
            Err(e) => { if let Transport::Plain(s) = &mut transport { let _ = s.kill(); } return Err(e); }
        };
        if let Transport::Plain(server) = &transport { session.sample(server.id()); }
        let owned = matches!(transport, Transport::Plain(_));
        let mut app = App { transport: Some(transport), session, pending: vec![], model: l.model.clone(), closed: None, owned, requests: vec![], effort: l.effort.clone() };
        let opened = app.handshake(initialize, thread, l, &table(spec.client).descriptor,owner);
        if opened.is_err() { if let Some(Transport::Plain(s)) = &mut app.transport { let _ = s.kill(); } }
        opened.map(|_| app)
    }

    fn handshake(&mut self, initialize: &Value, thread: &Value, l: &Launch, descriptor: &Descriptor,owner:&SessionOwner) -> Res<()> {
        let params = if self.owned { turn_thread(l)? } else { thread_params(l, thread)? };
        let until = reply_deadline(l);
        let request = self.session.initialize(initialize.clone())?;
        self.exchange(request, until)?;
        let initialized = self.session.initialized()?;
        self.transport.as_mut().ok_or("app-server transport ended")?.send(&initialized, until)?;
        let request = self.session.open(params)?;
        self.exchange(request, until)?;
        if !self.session.ready() { return Err("app-server thread not ready".into()); }
        owner.bind(self.session.session().ok_or("session-owner-unknown: app thread missing")?)?;
        if !l.parts.is_empty() { self.start(input_items(descriptor, &l.parts).map_err(|e|e.to_string())?, until)?; }
        else if !l.input.is_empty() { self.input(&l.input, until)?; }
        Ok(())
    }

    fn input(&mut self, text: &str, until: Instant) -> Res<()> { self.start(json!([{"type": "text", "text": text}]), until) }

    /// Start the next turn with exactly `items`; a failed write stays uncertain and is never resent.
    fn start(&mut self, items: Value, until: Instant) -> Res<()> {
        if self.session.report().is_some() { self.session.finish_turn()?; }
        let mut params = json!({"input": items});
        if let Some(m) = &self.model { params["model"] = json!(m); }
        if let Some(e) = &self.effort { params["effort"] = json!(e); }
        let request = self.session.start_turn(params)?;
        self.transport.as_mut().ok_or("app-server transport ended")?.send(&request, until)
    }

    /// One read-only `account/read` without token refresh; its reply stays private to the caller of `account`.
    fn account(&mut self, until: Instant) -> Result<Account, Failure> {
        let request = self.session.request("account/read", json!({"refreshToken": false}))?;
        let id = request["id"].clone();
        self.exchange(request, until)?;
        let at = self.pending.iter().rposition(|m| m["id"] == id && m.get("method").is_none()).ok_or("account/read reply missing")?;
        account_of(&self.pending.remove(at))
    }

    /// Read every message available now.
    fn pump(&mut self) -> Res<()> {
        if self.transport.is_none() { return Ok(()); }
        loop {
            match self.transport.as_mut().expect("transport").receive(Instant::now()) {
                Ok(Some(m)) => {
                    self.observe(m, Instant::now() + Duration::from_secs(5))?;
                    if self.owned && self.session.report().is_some() {
                        if let Some(Transport::Plain(s)) = &mut self.transport { s.close()?; }
                        return Ok(());
                    }
                }
                Ok(None) => return Ok(()),
                Err(e) => { self.closed = Some(e); return Ok(()); }
            }
        }
    }

    fn poll(&mut self) -> Res<State> {
        self.pump()?;
        if let Some(r) = self.session.report() { return Ok(state(&r)); }
        // Reap an exited server; an exit without a native report never proves a turn.
        if let Some(code) = self.transport.as_mut().map(Transport::exit).transpose()?.flatten() { self.closed.get_or_insert(format!("app-server exited {code}")); }
        Ok(if self.closed.is_some() || self.transport.is_none() { State::Unknown } else { State::Running })
    }
}

impl Native {
    fn poll(&mut self) -> Res<State> {
        if let Some(r) = &self.report { return Ok(state(r)); }
        let waiting = if self.fault.is_some() { State::Unknown } else { State::Running };
        if !self.attached {
            let Some(job) = discover(self)? else { return Ok(if self.job_id.is_none() && !self.spec.resume { State::Unknown } else { waiting }) };
            if !self.spec.resume {
                let path = (table(self.spec.client).ops.background_transcript)(&self.spec, &job.session)?;
                self.turn.attach_background(&job, &path)?;
                self.spec.session = job.session.clone();
            }
            self.job = Some(job);
            self.attached = true;
        }
        if !self.turn.path().exists() { return Ok(waiting); }
        match self.turn.poll()? {
            None => Ok(waiting),
            Some(r) => { let s = state(&r); self.report = Some(r); Ok(s) }
        }
    }
}

/// The `Account` of a Codex `account/read` reply: `chatgpt` is a subscription, `apiKey` an API key, no account none.
fn account_of(reply: &Value) -> Result<Account, Failure> {
    if !reply["error"].is_null() { return Err(Failure::Failed("account/read refused; identity unknown".into())); }
    let a = &reply["result"]["account"];
    if a.is_null() && reply["result"].is_object() { return Ok(Account { login: Login::None, plan: None }); }
    let login = match a["type"].as_str() { Some("chatgpt") => Login::Subscription, Some("apiKey") => Login::ApiKey, _ => return Err(Failure::Failed("account login unknown".into())) };
    let plan = a["planType"].as_str().filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')).map(String::from);
    Ok(Account { login, plan })
}

/// The native app-server items of `parts`, or the typed refusal; checked before any native write.
fn input_items(descriptor: &Descriptor, parts: &[Part]) -> Result<Value, Failure> {
    if parts.iter().all(|p| matches!(p, Part::Text(t) if t.is_empty())) { return Err(Failure::Refused(Cause("input-empty"))); }
    let mut items = vec![];
    for p in parts {
        items.push(match p {
            Part::Text(t) => json!({"type": "text", "text": t}),
            Part::Image(_) if !descriptor.has("image-input") => return Err(Failure::Refused(Cause("image-unsupported"))),
            Part::Image(path) if !path.is_absolute() || !path.is_file() || File::open(path).is_err() => return Err(Failure::Refused(Cause("image-invalid"))),
            Part::Image(path) => json!({"type": "localImage", "path": path}),
        });
    }
    Ok(Value::Array(items))
}

/// The one trait every runtime adapter implements (specq agent-runtimes § Runtime trait).
pub trait Runtime: Send + Sync {
    fn descriptor(&self) -> &Descriptor;
    fn stop_event(&self, _run: &mut Run, _bytes: u64) -> Res<Option<super::history::stop::StopEvent>> { Err("stop observation unsupported by this runtime".into()) }
    fn history_sessions(&self, home: Option<&Path>) -> Res<Vec<super::history::sessions::Source>> {
        super::history::sessions::list(self.descriptor().name, home)
    }
    fn history(&self, session: &str, home: Option<&Path>, source: Option<&Path>, checkpoint: Option<&Value>) -> Res<super::history::sessions::Session> {
        super::history::sessions::open(self.descriptor().name, session, home, source, checkpoint)
    }
    fn adopt(&self,_session:&str,_launch:&Launch,_native:&[NativeProcess])->Res<OwnerObservation> {Err("adoption unsupported".into())}
    fn orphans(&self,_launch:&Launch)->Res<Vec<Orphan>> {Err("orphan inventory unsupported".into())}
    fn stop_orphan(&self,_orphan:&Orphan,_launch:&Launch)->Res<()> {Err("orphan cleanup unsupported".into())}
    fn reconcile_stop(&self,_orphan:&Orphan,_launch:&Launch)->Res<()> {Err("stop reconciliation unsupported".into())}
    fn session_owner(&self,_session:Option<&str>,_launch:&Launch)->Res<SessionOwner> {Err("session ownership unsupported".into())}
    fn preflight(&self, launch: &Launch, repair: &Repair) -> Readiness {
        let _ = (launch, repair); Readiness::unsupported()
    }
    fn launch(&self, launch: &Launch) -> Res<Run>;
    fn resume(&self, session: &str, launch: &Launch) -> Res<Run>;
    /// Start the next turn of a session run with exactly `parts`, once; see `Failure` for refusals.
    fn input(&self, run: &mut Run, parts: &[Part]) -> Result<(), Failure>;
    /// Read-only login identity of a session run (capability `account`); never refreshes or writes a credential.
    fn account(&self, run: &mut Run) -> Result<Account, Failure>;
    /// Read the login's model catalogue; no turn or product protocol handler is needed.
    fn catalogue(&self, home: Option<&Path>) -> Res<Vec<Model>> { self.catalogue_with_executable(home, None) }
    /// Read with the caller-configured native executable, using normal lookup only when absent.
    fn catalogue_with_executable(&self, home: Option<&Path>, executable: Option<&Path>) -> Res<Vec<Model>>;
    fn log_limits(&self, log: &str, session: Option<&str>, home: Option<&Path>) -> Vec<Window>;
    fn account_limits(&self, home: Option<&Path>, now: i64) -> Vec<Window>;
    /// Read account quotas using the caller-configured native executable when supported.
    fn account_limits_with_executable(&self, home: Option<&Path>, executable: Option<&Path>, now: i64) -> Vec<Window>;
    /// Native events from complete JSONL log records, without a run or native process.
    fn log_events(&self, log: &str) -> Vec<Event>;
    /// log_events per source record, with native time, assistant failure, lifecycle label and saved permission mode.
    fn log_records(&self, log: &str) -> Vec<super::events::LogRecord>;
    /// Read-only account email/ID or project; unknown and unsupported identities stay absent.
    fn account_identity(&self, home: Option<&Path>) -> Option<String>;
    /// Request the stop; true when the exit is already confirmed.
    fn cancel(&self, run: &mut Run) -> Res<bool>;
    fn poll(&self, run: &mut Run) -> Res<State>;
    fn report(&self, run: &Run) -> Res<Report>;
    /// Ordered neutral events, each returned at most once.
    fn progress(&self, run: &mut Run) -> Res<Vec<Event>>;
    /// Session controls; unsupported runtimes refuse before a native write.
    fn control(&self, _run: &mut Run, _control: Control) -> Result<(), Failure> { Err(Failure::Refused(Cause("control-unsupported"))) }
    /// Capability `background`: the raw launcher exchange of this run, handed over once.
    fn launcher_output(&self, run: &mut Run) -> Result<Result<Output, crate::process::Failure>, Cause>;
    fn session_id(&self, run: &Run) -> Option<String>;
    fn session_url(&self, run: &Run) -> Result<String, Cause>;
    /// Poll, then the native session and session URL or its absence cause, in one record.
    fn observe(&self, run: &mut Run) -> Observation;
    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause>;
    fn ready(&self, executable: Option<&Path>, home: Option<&Path>) -> Ready;
    /// Account-bound probes use the same home, environment and executable as a launch.
    fn ready_for(&self, _launch: &Launch) -> Ready {let unknown=||Check {state:"unknown",detail:"context probe unsupported".into()};Ready {install:unknown(),version:unknown(),login:unknown()}}
    fn catalogue_for(&self, _launch: &Launch) -> Res<Vec<Model>> {Err("catalogue context unsupported".into())}
    fn account_limits_for(&self, _launch: &Launch, _now: i64) -> Vec<Window> {super::limits::unknown("context-probe-unsupported")}

    fn quota(&self, accounts: &Accounts, now: i64) -> Quota;
    fn running(&self) -> usize;
    fn protected_paths(&self, home: Option<&Path>) -> Vec<PathBuf>;
    /// The environment selecting account `label` of consumer `tool`; empty for `default` or an invalid label.
    fn home_env(&self, tool: &str, label: &str) -> Vec<(OsString, OsString)>;
    /// Private recovery value of a background or queue run; the caller protects and stores it before relying on it.
    fn checkpoint(&self, run: &Run) -> Result<Value, Cause>;
    /// The run rebuilt from `checkpoint` and the same launch fields, without a native call.
    fn recover(&self, saved: &Value, launch: &Launch) -> Res<Run>;

    // Native seams for a caller that builds its own native session (docs/spec/RUNTIME.md § Runtime trait).
    /// The `Report` of caller-held log text, without a run.
    fn classify(&self, log: &str) -> Report;
    /// The client executable on `path` (None: the process `PATH`); an unresolved name comes back bare.
    fn executable(&self, path: Option<&OsStr>) -> Res<PathBuf>;
    /// The client's own home.
    fn home(&self) -> Option<PathBuf>;
    /// The native transcript file of `session` in `home` (None: the client's own); none where the runtime keeps none.
    fn session_transcript(&self, session: &str, home: Option<&Path>) -> Option<PathBuf>;
    /// Capability `session-url`: the link of caller-built session `spec.session` (background `job`, if any) in `spec.home`,
    /// or its absence cause with native detail (Session observation).
    fn session_link(&self, spec: &NativeSpec, job: Option<&str>) -> Result<String, (Cause, Option<String>)>;
    /// The optimization intent applied to `spec`, as `launch` applies it.
    fn prepare(&self, intent: Intent, product: &str, spec: &mut NativeSpec, install: Option<&Path>) -> Optimization;
    /// Native turn argument inspection; submission uses `turn` or `launch`.
    fn native_arguments(&self, spec: &NativeSpec) -> Res<Vec<OsString>>;
    /// One owned native turn; `input` goes to the client, output appends to `log`.
    fn turn(&self, base: Command, spec: &NativeSpec, log: &Path, input: &str, process: &Process) -> Res<Turn>;
    /// Capability `background`: the launcher exchange; `reference` is the caller's durably bound input.
    fn background_launch(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Result<Output, crate::process::Failure>>;
    /// Capability `background`: native job rows; `filtered` limits them to `spec.root`.
    fn jobs(&self, base: Command, spec: &NativeSpec, filtered: bool, budget: Duration) -> Res<Vec<Value>>;
    /// Capability `background`: job `id` validated from one `jobs` row against `spec`.
    fn job(&self, row: &Value, spec: &NativeSpec, id: &str) -> Res<BackgroundJob>;
    /// Capability `background`: request the stop of `job`; the output acknowledges only the request.
    fn stop_job(&self, base: Command, spec: &NativeSpec, job: &BackgroundJob, budget: Duration) -> Res<Output>;
    /// The named cause of a launcher refusal in `stderr`, if known.
    fn launch_refusal(&self, text: &str) -> Option<Cause>;
    /// Capability `app-queue`: queue `reference` into the desktop thread `spec.session`.
    fn app_queue(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Output>;
    /// Capability `app-server`: read-only argument inspection; no spawnable command.
    fn app_server_arguments(&self, spec: &NativeSpec) -> Res<Vec<OsString>>;
}

/// The caller's view of one native session for the native seams; the adapter adds its client.
#[derive(Clone, Debug, Default)]
pub struct NativeSpec {
    /// Working directory of the client and Codex `-C` on a new start.
    pub root: PathBuf,
    /// The native session ID to start (Claude) or resume.
    pub session: String,
    pub resume: bool,
    /// Native client arguments in the Arguments position.
    pub args: Vec<OsString>,
    /// The account's login home; None keeps the caller's own.
    pub home: Option<PathBuf>,
    /// Extra child environment; values never enter argv or the log.
    pub env: Vec<(OsString, OsString)>,
    pub owner:Option<SessionOwner>,
}

impl NativeSpec {
    fn validate_session(&self)->Res<()> {
        if self.session=="-" && !self.resume || native::cli_id(&self.session) {Ok(())}
        else {Err("native CLI session invalid".into())}
    }
}

struct Adapter { client: Client, descriptor: Descriptor, live: AtomicUsize, ops: NativeOps }

struct NativeOps {
    acp: Option<&'static super::acp::Config>,
    catalogue:fn(Option<&Path>,Option<&Path>)->Res<Vec<Model>>,
    background_launch:fn(Command,&Spec,&str,Duration)->Res<Result<Output,crate::process::Failure>>,
    background_jobs:fn(Command,&Spec,bool,Duration)->Res<Vec<Value>>,
    background_job:fn(&Value,&Spec,&str)->Res<BackgroundJob>,
    background_stop:fn(Command,&Spec,&BackgroundJob,Duration)->Res<Output>,
    background_id:fn(&str)->Option<String>,
    app_queue:fn(Command,&Spec,&str,Duration)->Res<Output>,
    app_command:fn(Command,&Spec)->Res<Command>,

    protocol: &'static registry::Protocol,
    home_var: &'static str,
    alternate_home_var: Option<&'static str>,
    session_vars: &'static [&'static str],
    auth_vars: &'static [&'static str],
    home: fn() -> Option<PathBuf>,
    executable: fn(&OsStr) -> Res<PathBuf>,
    new_session: fn() -> String,
    args: fn(&Spec) -> Vec<OsString>,
    mapped: fn(&Launch) -> Res<Vec<OsString>>,
    extras: fn(&Policy) -> Vec<OsString>,
    environment: fn(&Launch, &mut Spec),
    settings: fn(&Launch, &mut Spec),
    login: fn(Command, bool) -> Check,
    preparation: fn(&Launch, &Repair) -> (LaunchCheck, LaunchCheck),
    restore_preparation: Option<fn(&Value, bool) -> Res<()>>,
    sandbox:Option<fn(&Launch)->LaunchCheck>,
    hook_status: Option<fn(&Path,Option<&Path>)->LaunchCheck>,
    limits_for: fn(&Launch, i64) -> Vec<Window>,
    validate: fn(&[OsString], &[(String,String)], &Policy) -> Res<()>,
    protected: fn(PathBuf) -> Vec<PathBuf>,
    refusal: fn(&str) -> Option<Cause>,
    transcript: fn(&str, Option<&Path>) -> Option<PathBuf>,
    background_transcript: fn(&Spec, &str) -> Res<PathBuf>,
    link: fn(&Run) -> Result<String, native::Absence>,
    session_link: fn(&NativeSpec, Option<&str>) -> Result<String, native::Absence>,
}

#[path = "registry/mod.rs"]
pub mod registry;

/// Every registered worker, in its existing public order.
pub fn runtimes() -> Vec<&'static dyn Runtime> {
    let mut entries = registry::entries(); entries.sort_by_key(|e| e.worker_order);
    entries.into_iter().filter_map(|e| e.runtime).collect()
}

pub fn runtime(name: &str) -> Option<&'static dyn Runtime> { registry::entry(name)?.runtime }

fn mapped(client: Client, _name: &str, l: &Launch) -> Res<Vec<OsString>> { (table(client).ops.mapped)(l) }

/// Server names of an MCP config file (`mcpServers` keys).
fn mcp_servers(file: &Path) -> Res<Vec<String>> {
    let v: Value = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or("mcp config unreadable")?;
    let names: Vec<String> = v["mcpServers"].as_object().ok_or("mcp config names no servers")?.keys().cloned().collect();
    if names.iter().any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')) { return Err("mcp server name must be plain".into()); }
    Ok(names)
}

/// Complete JSON lines of `log` from byte `from`, and the byte after the last complete line.
fn lines_from(log: &Path, from: u64) -> Res<(Vec<Value>, u64)> {let (rows,next)=activity_lines(log,from)?;Ok((rows.into_iter().filter(Value::is_object).collect(),next))}

fn activity_lines(log: &Path, from: u64) -> Res<(Vec<Value>, u64)> {
    let mut f = File::open(log).map_err(|e| format!("agent log unreadable: {e}"))?;
    if f.metadata().map_err(|e| e.to_string())?.len() < from { return Err("agent log truncated; progress unknown".into()); }
    let mut rest = vec![];
    f.seek(SeekFrom::Start(from)).and_then(|_| f.read_to_end(&mut rest)).map_err(|e| format!("agent log read: {e}"))?;
    let Some(end) = rest.iter().rposition(|b| *b == b'\n') else { return Ok((vec![], from)) };
    let values=rest[..end].split(|b|*b==b'\n').filter(|line|!line.iter().all(u8::is_ascii_whitespace))
        .map(|line|serde_json::from_slice::<Value>(line).unwrap_or(Value::Null)).collect();
    Ok((values, from + end as u64 + 1))
}

impl Adapter {
    fn native(&self, s: &NativeSpec) -> Spec {
        Spec { client: self.client, root: s.root.clone(), session: s.session.clone(), resume: s.resume, args: s.args.clone(), home: s.home.clone(), env: s.env.clone() }
    }

    fn read_progress(&self, run: &mut Run) -> Res<Vec<Event>> {
        let run = self.own_mut(run)?;
        let path = match &mut run.body {
            Body::Acp(a)=>return a.progress(),
            Body::Turn(t) => { t.sync_transcript()?; t.log.clone() }
            Body::Native(n) if n.attached && n.turn.path().exists() => n.turn.path().to_path_buf(),
            Body::Native(_) => return Ok(vec![]),
            Body::App(a) => { a.pump()?;run.activity.pending_seen=0; return Ok(run.events.read_owned(self.client, std::mem::take(&mut a.pending), a.session.session(), a.session.turn_id())); }
        };
        let (mut values, next) = lines_from(&path, run.progress)?;
        if let Body::Turn(t) = &run.body {
            if self.descriptor.has("app-server") {
                if run.rollout_progress.is_none() && t.resumed.is_none() { run.rollout_progress = session_of(self.client, &read_from(&t.log,t.start).unwrap_or_default()).and_then(|s| (self.ops.transcript)(&s, t.home.as_deref())).map(|p|(p,0)); }
                if let Some((path, from)) = &mut run.rollout_progress {
                    let (rows, next) = lines_from(path, *from)?; *from = next;
                    values.extend(rows.into_iter().filter(|v| v["payload"]["type"] == "token_count"));
                }
            }
        }
        run.progress = next;
        Ok(run.events.read(self.client, values))
    }

    fn refresh_activity(&self,run:&mut Run)->Res<()> {
        let run=self.own_mut(run)?;
        let events=match &mut run.body {
            Body::Acp(a)=>{a.pump()?;let (rows,next)=activity_lines(&a.path,run.activity.progress)?;run.activity.progress=next;run.activity.read(self.client,rows)},
            Body::App(a)=>{a.pump()?;let from=run.activity.pending_seen.min(a.pending.len());let rows=a.pending[from..].to_vec();run.activity.pending_seen=a.pending.len();run.activity.cursor.read_owned(self.client,rows,a.session.session(),a.session.turn_id())},
            Body::Turn(t)=>{t.sync_transcript()?;let (rows,next)=activity_lines(&t.log,run.activity.progress)?;run.activity.progress=next;run.activity.read(self.client,rows)},
            Body::Native(n) if n.attached=>{let (rows,next)=activity_lines(n.turn.path(),run.activity.progress)?;run.activity.progress=next;run.activity.read(self.client,rows)},
            _=>vec![],
        };
        run.activity.feed(events,unix_ms());let _=run.activity.observation(run.inactivity,unix_ms(),run.live.is_some());Ok(())
    }

    fn seam_readiness(&self, command: &Command, spec: &NativeSpec, log: Option<&Path>, input: &str) -> Res<Readiness> {
        spec.validate_session()?;
        (self.ops.validate)(&spec.args,&[],&Policy::default())?;
        let mut launch=Launch::new(&spec.root,log.map(Path::to_path_buf).unwrap_or_else(||spec.root.join(".agentq-preflight-log")),input,"agentq");
        launch.home=spec.home.clone();
        launch.command=std::iter::once(command.get_program().to_os_string()).chain(command.get_args().map(OsStr::to_os_string)).collect();
        launch.env=command.get_envs().filter_map(|(k,v)|v.map(|v|(k.to_os_string(),v.to_os_string()))).chain(spec.env.iter().cloned()).collect();
        let mut readiness=self.preflight(&launch,&Repair::default());
        if !spec.args.is_empty() {readiness.checks[4]=LaunchCheck::new("permissions","unknown",Some("native-policy-unchecked"),"Native arguments remain caller-owned","use Runtime::launch for an abstract policy");}
        readiness.permit(ReadinessMode::Continue)?;
        Ok(readiness)
    }

    fn needs(&self, capability: &str) -> Res<()> {
        if self.descriptor.has(capability) { Ok(()) } else { Err(format!("{capability} unsupported by {}", self.descriptor.name)) }
    }

    fn link(&self, run: &Run) -> Result<String, native::Absence> {
        let run = self.own(run).map_err(|_| (Cause("foreign-run"), None))?;
        (self.ops.link)(run)
    }

    fn own<'r>(&self, run: &'r Run) -> Res<&'r Run> {
        if run.runtime != self.descriptor.name { return Err(format!("run belongs to {}, not {}", run.runtime, self.descriptor.name)); }
        Ok(run)
    }

    fn own_mut<'r>(&self, run: &'r mut Run) -> Res<&'r mut Run> {
        self.own(run)?;
        Ok(run)
    }

    /// The child spec for `l`: mapped policy, then the launch's optimization intent.
    fn spec(&self, l: &Launch, session: String, resume: bool) -> Res<(Spec, Optimization)> {
        (self.ops.validate)(&l.command.iter().skip(1).chain(l.host_args.iter()).cloned().collect::<Vec<_>>(),&l.settings_env,&l.policy)?;
        let name = self.descriptor.name;
        let app = matches!(l.mode, Mode::AppServer { .. }) || self.descriptor.has("app-server") && automatic(l);
        for (capability, set) in [("mcp-config", l.mcp.is_some()), ("instructions-file", l.instructions.is_some()), ("turn-parts", !l.parts.is_empty()),
            ("turn-deltas", l.deltas && !app), ("host-args", !l.host_args.is_empty())] {
            if set && (l.mode != Mode::Turn || !self.descriptor.has(capability)) { return Err(format!("{capability} unsupported by {name}")); }
        }
        if !l.host_args.is_empty() && l.policy != Policy::default() { return Err(format!("host-args require the native policy on {name}")); }
        if app && !l.host_args.is_empty() { return Err("host-args unsupported with relay input".into()); }
        if !l.settings_env.is_empty() && (!matches!(l.mode, Mode::Turn | Mode::Background) || !self.descriptor.has("settings-env")) {
            return Err(format!("settings-env unsupported by {name}"));
        }
        // App-server policy travels as thread parameters; only the Codex extras stay flags.
        let mut args = if let Some(acp)=self.ops.acp {super::acp::validate_launch(acp,l)?;vec![]} else if !app { mapped(self.client, name, l)? } else if self.descriptor.has("app-server") { (self.ops.extras)(&l.policy) } else { vec![] };
        args.extend(l.host_args.iter().cloned());
        let env = l.env.clone();
        let mut spec = Spec { client: self.client, root: l.root.clone(), session, resume, args, home: l.home.clone(), env };
        (self.ops.environment)(l, &mut spec);
        // Isolation excludes the user hooks that carry optimization, so it cannot be effective.
        let intent = l.optimization;
        let optimization = optimization::prepare(intent, &l.product, &mut spec, l.install.as_deref());
        (self.ops.settings)(l, &mut spec);
        Ok((spec, optimization))
    }

    fn run(&'static self, body: Body, progress: u64, optimization: Optimization, l: &Launch) -> Run {
        self.live.fetch_add(1, Ordering::SeqCst);
        Run { stop_id: crate::incident::uuid(), runtime: self.descriptor.name, body, progress, events: super::events::Cursor::new(l.deltas), rollout_progress: None, optimization, live: Some(Live(&self.live)), recorded: false, frozen: None, deadline: l.deadline,
            remote: l.title.is_some(), readiness: Readiness::default(), starting: Instant::now(),owner:None,activity:activity::Activity::new(progress),inactivity:l.inactivity,stop_on_stall:l.stop_on_stall }
    }

    fn checked_run(&'static self, readiness: &Readiness, body: Body, progress: u64, optimization: Optimization, l: &Launch) -> Run {
        let mut run = self.run(body, progress, optimization, l); run.readiness = readiness.clone(); run
    }

    fn start(&'static self, l: &Launch, session: Option<&str>) -> Res<Run> {
        let name = self.descriptor.name;
        if !self.descriptor.has(l.mode.capability()) { return Err(format!("mode {} unsupported by {name}", l.mode.capability())); }
        if matches!(l.mode, Mode::AppServer { .. }) { return Err("app-server requires explicit protected host; state needs in Launch".into()); }
        if l.lifetime==Lifetime::StopWithOwner && (!matches!(l.mode,Mode::Turn) || !self.descriptor.has("process-lifetime")) {return Err("lifetime-unsupported: native submission scope".into());}
        let readiness = self.preflight(l, &Repair::default());
        readiness.permit(l.readiness)?;
        let acp=self.ops.acp;
        if session.is_some_and(|s| s.is_empty() || s == "-" || acp.is_none() && !native::plain_id(s)) { return Err("session requires a native session ID".into()); }
        let (spec, optimization) = self.spec(l, session.map_or_else(|| new_session(self.client), String::from), session.is_some())?;
        let mut owner_spec=spec.clone();
        if session.is_none() && matches!(l.mode,Mode::Background) {owner_spec.session="-".into();}
        let owner=l.session_store.acquire(&owner_spec,session.is_none(),None,l.lifetime)?;
        let mut owned_launch=l.clone();owned_launch.process=owner.process_hooks(&l.process,matches!(l.mode,Mode::Turn) && self.descriptor.has("process-lifetime"))?;let l=&owned_launch;
        if let Some(config)=acp {
            if l.mode!=Mode::Turn {return Err("mode unsupported by ACP".into());}
            let command=preflight::command(self,l)?;
            let session=super::acp::Session::open(command,config,l,session,&owner)?;
            let boundary=session.boundary;
            let mut run=self.checked_run(&readiness,Body::Acp(Box::new(session)),boundary,optimization,l);run.owner=Some(owner);return Ok(run);
        }
        let command = preflight::launch_base(self,l)?;
        let native = |turn: NativeTurn, queue, job_id, attached, fault, launcher| Body::Native(Box::new(Native { queue, turn, spec: spec.clone(), command: l.command.clone(),
            job_id, job: None, attached, fault, report: None, launcher }));
        let uncertain = |what: &str, e: &dyn std::fmt::Display| Some(format!("{what} response uncertain: {e}; observe, never resubmit"));
        let result=match &l.mode {
            Mode::Turn => {
                if self.descriptor.has("app-server") && automatic(l) {
                    turn_thread(l)?;
                    if !l.parts.is_empty() { input_items(&self.descriptor, &l.parts).map_err(|e| e.to_string())?; }
                    let server = AppServer::spawn((self.ops.app_command)(command, &spec)?, &l.log, &l.process)?;
                    let app = App::open(&spec, Transport::Plain(server), &json!({"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}), &json!({}), l,&owner)?;
                    let mut run=self.checked_run(&readiness,Body::App(Box::new(app)),0,optimization,l);run.owner=Some(owner);return Ok(run);
                }
                let input = if l.parts.is_empty() { l.input.clone() } else { stream_message(&l.parts)? };
                let baseline = if spec.resume {
                    (self.ops.transcript)(&spec.session, spec_home(&spec).as_deref()).map(|p|std::fs::metadata(&p).map(|m|(p,m.len())).map_err(|_|"rollout baseline unreadable")) .transpose()?
                } else {None};
                let turn = Turn::start(command, &spec, &l.log, &input, &l.process)?.owned(owner.clone()).prepared(readiness.clone());
                let start = turn.start;
                let mut run = self.checked_run(&readiness, Body::Turn(turn), start, optimization, l); run.rollout_progress = baseline; Ok(run)
            }
            Mode::Background => {
                // A new job's transcript is unknown until discovery; the alias path does not exist.
                let turn = NativeTurn::before(&spec, &(self.ops.background_transcript)(&spec, &spec.session)?, &l.input)?;
                owner.submit()?;
                let exchange = (self.ops.background_launch)(command, &spec, &l.input, Duration::from_secs(30))?;
                let (job_id, fault) = match &exchange {
                    Ok(o) if o.status.success() => ((self.ops.background_id)(&String::from_utf8_lossy(&o.stdout)), None),
                    Ok(o) => (None, uncertain("background launcher", &format!("exit {:?}", o.status.code()))),
                    Err(e) => (None, uncertain("background launcher", &e.class())),
                };
                let start = turn.start();
                Ok(self.checked_run(&readiness, native(turn, false, job_id, spec.resume, fault, Some(exchange)), start, optimization, l))
            }
            Mode::Queue { transcript } => {
                if !spec.resume { return Err("queue requires the session of a running desktop thread".into()); }
                let turn = NativeTurn::before(&spec, transcript, &l.input)?;
                owner.submit()?;
                let fault = (self.ops.app_queue)(command, &spec, &l.input, Duration::from_secs(30)).err().and_then(|e| uncertain("queue", &e));
                let start = turn.start();
                Ok(self.checked_run(&readiness, native(turn, true, None, true, fault, None), start, optimization, l))
            }
            Mode::AppServer { initialize, thread } => {
                thread_params(l, thread)?;
                let server = AppServer::spawn((self.ops.app_command)(command, &spec)?, &l.log, &l.process)?;
                let app = App::open(&spec, Transport::Plain(server), initialize, thread, l,&owner)?;
                Ok(self.checked_run(&readiness, Body::App(Box::new(app)), 0, optimization, l))
            }
        };
        result.map(|mut run|{run.owner=Some(owner);run})
    }
}

impl Runtime for Adapter {
    fn stop_event(&self, run: &mut Run, bytes: u64) -> Res<Option<super::history::stop::StopEvent>> {
        super::history::stop::observe(self, run, bytes)
    }
    fn descriptor(&self) -> &Descriptor { &self.descriptor }

    fn adopt(&self,session:&str,launch:&Launch,native:&[NativeProcess])->Res<OwnerObservation> {launch.session_store.adopt(&preflight::spec(self,launch),session,native)}
    fn orphans(&self,launch:&Launch)->Res<Vec<Orphan>> {launch.session_store.orphans(&preflight::spec(self,launch))}
    fn stop_orphan(&self,entry:&Orphan,launch:&Launch)->Res<()> {launch.session_store.cleanup(&preflight::spec(self,launch),entry,false)}
    fn reconcile_stop(&self,entry:&Orphan,launch:&Launch)->Res<()> {launch.session_store.cleanup(&preflight::spec(self,launch),entry,true)}
    fn session_owner(&self,session:Option<&str>,launch:&Launch)->Res<SessionOwner> {
        let mut spec=preflight::spec(self,launch);spec.session=session.unwrap_or("-").into();spec.resume=session.is_some();
        launch.session_store.acquire(&spec,session.is_none(),None,launch.lifetime)
    }
    fn preflight(&self, launch: &Launch, repair: &Repair) -> Readiness { preflight::check(self, launch, repair) }

    fn launch(&self, launch: &Launch) -> Res<Run> { table(self.client).start(launch, None) }

    fn resume(&self, session: &str, launch: &Launch) -> Res<Run> { table(self.client).start(launch, Some(session)) }

    fn input(&self, run: &mut Run, parts: &[Part]) -> Result<(), Failure> {
        let run = self.own_mut(run)?;
        let items = input_items(&self.descriptor, parts)?;
        let until = run.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
        match &mut run.body {
            Body::App(a) if !a.owned => { a.pump()?;run.owner.as_ref().ok_or("session-owner-unknown")?.submit()?; a.start(items, until)?; run.recorded = false; run.frozen = None; if run.live.is_none() { table(self.client).live.fetch_add(1, Ordering::SeqCst); run.live = Some(Live(&table(self.client).live)); } Ok(()) }
            // A CLI run reads one input; the next turn is a resume.
            _ => Err(Failure::Refused(Cause("input-unsupported"))),
        }
    }

    fn control(&self,run:&mut Run,control:Control)->Result<(),Failure> {
        match &mut self.own_mut(run)?.body {
            Body::Acp(a)=>a.control(control).map_err(Failure::Failed),
            _=>Err(Failure::Refused(Cause("control-unsupported"))),
        }
    }

    fn account(&self, run: &mut Run) -> Result<Account, Failure> {
        let run = self.own_mut(run)?;
        let until = run.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
        match &mut run.body {
            Body::App(a) if self.descriptor.has("account") => a.account(until),
            _ => Err(Failure::Refused(Cause("account-unsupported"))),
        }
    }

    fn catalogue_with_executable(&self, home: Option<&Path>, executable: Option<&Path>) -> Res<Vec<Model>> { self.needs("catalogue")?; (self.ops.catalogue)(home, executable) }

    fn log_limits(&self, log: &str, session: Option<&str>, home: Option<&Path>) -> Vec<Window> {
        let events = log.lines().filter_map(|l| serde_json::from_str(l).ok()).collect::<Vec<_>>();
        let path = session.and_then(|s| self.session_transcript(s, home));
        super::limits::read(self.client, &events, path.as_deref())
    }

    fn account_limits(&self, home: Option<&Path>, now: i64) -> Vec<Window> { self.account_limits_with_executable(home,None,now) }
    fn account_limits_with_executable(&self, home: Option<&Path>, executable: Option<&Path>, now: i64) -> Vec<Window> { super::limits::account(self.client,home,executable,now) }

    fn log_events(&self, log: &str) -> Vec<Event> {
        let values=log.lines().filter_map(|line|serde_json::from_str(line).ok()).collect();
        super::events::Cursor::new(true).read(self.client,values)
    }

    fn log_records(&self, log: &str) -> Vec<super::events::LogRecord> { super::events::log_records(self.client,log) }

    fn account_identity(&self, home: Option<&Path>) -> Option<String> { super::limits::identity(self.client,home) }

    fn cancel(&self, run: &mut Run) -> Res<bool> {
        let owner=self.own(run)?.owner.clone().ok_or("session-owner-required")?;
        match &mut self.own_mut(run)?.body {
            Body::Acp(a)=>a.cancel(),
            Body::Turn(t) => {
                if !t.timed_out { t.cancelled = true; }
                t.kill()?;
                Ok(t.exit.is_some())
            }
            Body::Native(n) if n.queue => Err("cancel-unsupported: desktop cancellation is a caller UI operation".into()),
            Body::App(a) => {
                let until = Instant::now() + Duration::from_secs(5);
                if let Ok(request) = a.session.interrupt() {
                    a.transport.as_mut().ok_or("app-server transport ended")?.send(&request, until)?;
                    return Ok(false);
                }
                match a.transport.as_mut().ok_or("app-server transport ended")? {
                    Transport::Plain(s) => {if !owner.stop_scope()? {s.kill()?;} Ok(s.poll()?.is_some()) }
                    Transport::Protected(s) => { s.cancel(until)?; Ok(false) }
                }
            }
            Body::Native(n) => {
                if n.job.is_none() { n.job = discover(n)?; }
                let job = n.job.as_ref().ok_or("background job unknown; nothing to stop")?;
                (self.ops.background_stop)(base(&n.command, n.spec.client)?, &n.spec, job, Duration::from_secs(5))?;
                Ok(false)
            }
        }
    }

    fn poll(&self, run: &mut Run) -> Res<State> {
        let run = self.own_mut(run)?;
        let s = match &mut run.body {
            Body::Acp(a)=>a.poll()?,
            Body::Turn(t) => {
                // An agentapi submitter exits after its receipt while the turn continues, so the deadline applies after exit too.
                if run.deadline.is_some_and(|d| Instant::now() >= d) && !t.timed_out {
                    t.timed_out = true;
                    let _ = t.kill();
                }
                // An agentapi turn has exited once its deadline stop killed it.
                let exited = t.poll()?.is_some() || t.timed_out && t.extension.is_submission() && t.exit.is_some();
                if !exited {State::Running} else {t.freeze()?;state(&t.report())}
            }
            Body::Native(n) => n.poll()?,
            Body::App(a) => {
                if a.owned && run.deadline.is_some_and(|d|Instant::now()>=d) && a.session.report().is_none() {
                    if let Some(Transport::Plain(s))=&mut a.transport {s.close()?;}
                    a.closed=Some("deadline elapsed; native turn outcome unknown".into());
                }
                a.poll()?
            },
        };
        if let Some(owner)=&run.owner {
            if let Some(session)=self.session_id(run) {owner.bind(&session)?;}
            if s==State::Running && matches!(run.body,Body::Native(_)) {
                if let Some(pid)=run.id() {owner.observe_process(pid)?;}
            }
            if matches!(s,State::Finished|State::Failed) {owner.finish(&self.report(run)?)?;}
        }
        if s != State::Running && s != State::Unknown { run.live = None; }
        Ok(s)
    }

    fn report(&self, run: &Run) -> Res<Report> {
        let mut report = match &self.own(run)?.body {
            Body::Acp(a)=>a.report(),
            Body::Turn(t) if t.exit.is_some() => Ok(t.report()),
            Body::Native(n) if n.report.is_some() => Ok(n.report.clone().expect("report")),
            Body::App(a) => a.session.report().ok_or_else(|| String::from("running")),
            _ => Err("running".into()),
        }?;
        report.owner=run.owner.as_ref().map(SessionOwner::observation).unwrap_or_default();
        report.readiness = Some(run.readiness.clone());
        Ok(report)
    }

    fn progress(&self, run: &mut Run) -> Res<Vec<Event>> {
        self.refresh_activity(run)?;
        let mut events=self.read_progress(run)?;
        events.extend(std::mem::take(&mut run.activity.pending_stalls).into_iter().map(|idle_ms|Event::Stalled{idle_ms}));
        Ok(events)
    }

    fn launcher_output(&self, run: &mut Run) -> Result<Result<Output, crate::process::Failure>, Cause> {
        let run = self.own_mut(run).map_err(|_| Cause("foreign-run"))?;
        match &mut run.body {
            Body::Native(n) if !n.queue => n.launcher.take().ok_or(Cause("launcher-output-unavailable")),
            _ => Err(Cause("no-launcher")),
        }
    }

    fn session_id(&self, run: &Run) -> Option<String> {
        match &self.own(run).ok()?.body {
            Body::Acp(a)=>Some(a.session.clone()),
            Body::Turn(t) => session_of(self.client, &read_from(&t.log, t.start).unwrap_or_default())
                .or_else(|| t.extension.session().map(String::from)).or_else(|| t.resumed.clone()),
            Body::Native(n) => n.attached.then(|| n.turn.session().to_string()),
            Body::App(a) => a.session.session().map(String::from),
        }
    }

    fn session_url(&self, run: &Run) -> Result<String, Cause> { self.link(run).map_err(|(cause, _)| cause) }

    fn observe(&self, run: &mut Run) -> Observation {
        let (state, error) = match self.poll(run) { Ok(s) => (s, None), Err(e) => (State::Unknown, Some(e)) };
        let activity_error=self.refresh_activity(run).err();
        let mut activity=if let Some(error)=activity_error {ActivityObservation {state:"unknown".into(),idle_ms:None,reason:Some(error)}} else {run.activity.observation(run.inactivity,unix_ms(),state==State::Running)};
        if activity.state=="stalled" && run.stop_on_stall {
            activity.reason=Some(match run.owner.as_ref().ok_or_else(||"session-owner-unknown".to_string()).and_then(SessionOwner::stop_uncertain) {
                Err(error)=>format!("stall-stop-ownership-unknown: {error}"),
                Ok(())=>match self.cancel(run) {Ok(_)=>"stall-stop-requested: native subtree unverified".into(),Err(error)=>format!("stall-stop-failed: {error}")},
            });
        }
        let (url, cause, detail) = match self.link(run) { Ok(u) => (Some(u), None, None), Err((c, d)) => (None, Some(c), d) };
        let session = self.session_id(run);
        let proven = match &run.body {
            Body::Turn(t) => {
                let text=read_from(&t.log,t.start).unwrap_or_default();
                session_of(self.client,&text).is_some() || self.log_records(&text).iter().any(|r|r.lifecycle.is_some() || r.events.iter().any(|e|!matches!(e,Event::Activity|Event::Unsupported{..})))
            },
            Body::Native(n) => n.attached, Body::App(a) => a.session.session().is_some(), Body::Acp(a)=>!a.session.is_empty(),
        };
        Observation { state, phase: if state != State::Running { state.code() } else if proven { "started" } else { "starting" },
            starting_ms: run.starting.elapsed().as_millis().min(u64::MAX as u128) as u64, owner:run.owner.as_ref().map(SessionOwner::observation).unwrap_or_default(),activity,error, session, url, cause, detail }
    }

    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause> {
        match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Acp(a)=>Ok(a.path.clone()),
            Body::Turn(t) => Ok(t.log.clone()),
            Body::Native(n) if n.attached => Ok(n.turn.path().to_path_buf()),
            Body::Native(_) => Err(Cause("transcript-unknown")),
            // The private log holds stderr only; `progress` is the session stream.
            Body::App(_) => Err(Cause("no-transcript")),
        }
    }

    fn ready(&self, executable: Option<&Path>, home: Option<&Path>) -> Ready {
        let root=std::env::current_dir().unwrap_or_else(|_|std::env::temp_dir());
        let mut launch=Launch::new(root,std::env::temp_dir().join("unused-ready.log"),"","agentq");
        launch.policy.subscription=self.client==Client::Codex;launch.home=home.map(Path::to_path_buf);launch.command=executable.map(|p|vec![p.as_os_str().into()]).unwrap_or_default();
        preflight::ready(self,&launch)
    }
    fn ready_for(&self, launch: &Launch) -> Ready {preflight::ready(self,launch)}
    fn catalogue_for(&self, launch: &Launch) -> Res<Vec<Model>> {preflight::catalogue(self,launch)}
    fn account_limits_for(&self, launch: &Launch, now:i64) -> Vec<Window> {(self.ops.limits_for)(launch,now)}

    fn quota(&self, accounts: &Accounts, now: i64) -> Quota {
        Quota { account: accounts.pick(now), free: accounts.active(now).is_some(), off: accounts.off(), hold: accounts.hold(now) }
    }

    fn running(&self) -> usize { self.live.load(Ordering::SeqCst) }

    fn protected_paths(&self, home: Option<&Path>) -> Vec<PathBuf> {
        let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(self.client)) else { return vec![] };
        (self.ops.protected)(home)
    }

    fn home_env(&self, tool: &str, label: &str) -> Vec<(OsString, OsString)> {
        account_home(self.client, tool, label).map(|h| vec![(OsString::from(self.ops.acp.map_or(self.client.home_var(),|c|c.home_var)), h.into_os_string())]).unwrap_or_default()
    }

    fn checkpoint(&self, run: &Run) -> Result<Value, Cause> {
        let mut saved = match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Acp(_)=>Err(Cause("no-checkpoint")),
            Body::Turn(t) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": "turn", "turn": t.checkpoint()?})),
            Body::App(a) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": "app", "app": a.session.checkpoint(), "model": a.model})),
            Body::Native(n) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": if n.queue { "queue" } else { "background" }, "native": n.turn.checkpoint(),
                "session": n.turn.session(), "resume": n.spec.resume, "job": n.job_id, "attached": n.attached, "fault": n.fault})),
        }?;
        saved["stop_id"]=json!(run.stop_id);
        saved["owner"]=run.owner.as_ref().ok_or(Cause("session-owner-unknown"))?.checkpoint().map_err(|_|Cause("session-owner-unknown"))?;
        saved["activity"]=json!(run.activity);
        saved["readiness"]=json!(run.readiness);
        saved["progress"]=json!(run.progress); saved["events"]=json!(run.events); saved["rollout_progress"]=json!(run.rollout_progress);
        if let Some(frozen)=&run.frozen { saved["frozen"]=frozen.save(); }
        Ok(saved)
    }

    fn recover(&self, saved: &Value, l: &Launch) -> Res<Run> {
        saved["stop_id"].as_str().filter(|s|native::plain_id(s)).ok_or("run stop identity invalid")?;
        let me = table(self.client);
        let mut owner_spec=preflight::spec(self,l);owner_spec.session=saved["owner"]["session"].as_str().ok_or("session-owner-unknown: checkpoint has no owner")?.into();
        let acquire=||l.session_store.acquire(&owner_spec,false,Some(&saved["owner"]),l.lifetime);
        let queue = matches!(l.mode, Mode::Queue { .. });
        let kind = match l.mode { Mode::Turn if self.descriptor.has("app-server") && automatic(l) => "app", Mode::Turn => "turn", Mode::Background => "background", Mode::Queue { .. } => "queue", Mode::AppServer { .. } => "app" };
        if saved["schema"] != "agentq-run/1" || saved["runtime"] != self.descriptor.name || saved["kind"] != kind {
            return Err("run recovery scope differs".into());
        }
        if kind == "turn" {
            // Observation only: the saved child keeps running; a resumed turn's session is the saved one.
            let resumed = saved["turn"]["resumed"].as_str();
            let (spec, optimization) = me.spec(l, resumed.map_or_else(|| new_session(self.client), String::from), resumed.is_some())?;
            let turn = Turn::recover(&spec, &l.log, &saved["turn"])?;
            let start = turn.start;
            let owner=acquire()?;
            let mut run=me.run(Body::Turn(turn.owned(owner.clone())),start,optimization,l);run.owner=Some(owner);return run.restored(saved);
        }
        if kind == "app" {
            // Observation only: the transport ended with its process, so the run reports and records but takes no input.
            let app = &saved["app"];
            let (spec, optimization) = me.spec(l, app["session"].as_str().ok_or("run recovery session invalid")?.into(), app["session"] != "-")?;
            let session = AppSession::recover(&spec, app)?;
            let a = App { transport: None, session, pending: vec![], model: saved["model"].as_str().map(String::from), closed: Some("recovered".into()), owned: automatic(l), requests: vec![], effort: l.effort.clone() };
            let mut run=me.run(Body::App(Box::new(a)),0,optimization,l);run.owner=Some(acquire()?);return run.restored(saved);
        }
        let session = saved["session"].as_str().filter(|s| native::plain_id(s)).ok_or("run recovery session invalid")?;
        let (spec, optimization) = me.spec(l, session.into(), saved["resume"].as_bool().ok_or("run recovery state invalid")?)?;
        let turn = NativeTurn::recover(&spec, &saved["native"])?;
        let text = |k: &str| saved[k].as_str().map(String::from);
        if !saved["job"].is_null() && text("job").is_none_or(|j| !native::plain_id(&j)) { return Err("run recovery job invalid".into()); }
        let start = turn.start();
        let body = Body::Native(Box::new(Native { queue, turn, spec, command: l.command.clone(), job_id: text("job"), job: None,
            attached: saved["attached"].as_bool().ok_or("run recovery state invalid")?, fault: text("fault"), report: None, launcher: None }));
        let mut run=me.run(body,start,optimization,l);run.owner=Some(acquire()?);run.restored(saved)
    }

    fn classify(&self, log: &str) -> Report {
        let events: Vec<Value> = log.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        Report::read(self.client, log, &events, None)
    }

    fn executable(&self, path: Option<&OsStr>) -> Res<PathBuf> {
        match path { Some(p) => executable_in(self.client, p), None => executable(self.client) }
    }

    fn home(&self) -> Option<PathBuf> { self.ops.acp.map_or_else(||base_home(self.client),|c|(c.home)()) }

    fn session_transcript(&self, session: &str, home: Option<&Path>) -> Option<PathBuf> {
        (self.ops.transcript)(session, home)
    }

    fn session_link(&self, spec: &NativeSpec, job: Option<&str>) -> Result<String, (Cause, Option<String>)> {
        (self.ops.session_link)(spec, job)
    }

    fn prepare(&self, intent: Intent, product: &str, spec: &mut NativeSpec, install: Option<&Path>) -> Optimization {
        let mut native = self.native(spec);
        let optimization = optimization::prepare(intent, product, &mut native, install);
        (spec.args, spec.env) = (native.args, native.env);
        optimization
    }

    fn native_arguments(&self, spec: &NativeSpec) -> Res<Vec<OsString>> {spec.validate_session()?;(self.ops.validate)(&spec.args,&[],&Policy::default())?; Ok(args(&self.native(spec))) }

    fn turn(&self, base: Command, spec: &NativeSpec, log: &Path, input: &str, process: &Process) -> Res<Turn> {
        let readiness=self.seam_readiness(&base,spec,Some(log),input)?;
        let owner=spec.owner.as_ref().ok_or("session-owner-required")?;owner.verify(&self.native(spec))?;
        if !spec.resume && self.descriptor.has("caller-session-id") {owner.bind(&spec.session)?;}
        Turn::start(base, &self.native(spec), log, input, &owner.process_hooks(process,false)?).map(|turn|turn.owned(owner.clone()).prepared(readiness))
    }

    fn background_launch(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Result<Output, crate::process::Failure>> {
        self.needs("background")?;
        self.seam_readiness(&base,spec,None,reference)?;
        let owner=spec.owner.as_ref().ok_or("session-owner-required")?;owner.verify(&self.native(spec))?;owner.unscoped_submission()?;owner.submit()?;
        (self.ops.background_launch)(base, &self.native(spec), reference, budget)
    }

    fn jobs(&self, base: Command, spec: &NativeSpec, filtered: bool, budget: Duration) -> Res<Vec<Value>> {
        self.needs("background")?;
        spec.validate_session()?;
        (self.ops.background_jobs)(base, &self.native(spec), filtered, budget)
    }

    fn job(&self, row: &Value, spec: &NativeSpec, id: &str) -> Res<BackgroundJob> {
        self.needs("background")?;
        spec.validate_session()?;
        let job=(self.ops.background_job)(row, &self.native(spec), id)?;
        if let Some(owner)=&spec.owner {owner.bind(&job.session)?;if let Some(pid)=job.pid {owner.observe_process(pid)?;}}
        Ok(job)
    }

    fn stop_job(&self, base: Command, spec: &NativeSpec, job: &BackgroundJob, budget: Duration) -> Res<Output> {
        self.needs("background")?;
        spec.validate_session()?;
        (self.ops.background_stop)(base, &self.native(spec), job, budget)
    }

    fn launch_refusal(&self, text: &str) -> Option<Cause> {
        (self.ops.refusal)(text)
    }

    fn app_queue(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Output> {
        self.needs("app-queue")?;
        self.seam_readiness(&base,spec,None,reference)?;
        let owner=spec.owner.as_ref().ok_or("session-owner-required")?;owner.verify(&self.native(spec))?;owner.unscoped_submission()?;owner.submit()?;
        (self.ops.app_queue)(base, &self.native(spec), reference, budget)
    }

    fn app_server_arguments(&self, spec: &NativeSpec) -> Res<Vec<OsString>> {
        self.needs("app-server")?;
        spec.validate_session()?;
        Ok((self.ops.app_command)(Command::new("native-argument-inspection"), &self.native(spec))?.get_args().map(OsStr::to_os_string).collect())
    }
}

/// A protected host frontend opened as a Codex app-server run (`ProtectedAppServer::open`).
pub(crate) fn open_protected(server: ProtectedAppServer, session: Option<&str>, l: &Launch) -> Res<Run> {
    if l.lifetime==Lifetime::StopWithOwner {return Err("lifetime-unsupported: protected scope".into());}
    let adapter = registry::app_adapter();
    let Mode::AppServer { initialize, thread } = &l.mode else { return Err("protected open requires mode app-server".into()) };
    if session.is_some_and(|s| !native::plain_id(s) || s == "-") { return Err("session requires a native session ID".into()); }
    let (spec, _) = adapter.spec(l, session.unwrap_or("-").into(), session.is_some())?;
    let owner=l.session_store.acquire(&spec,session.is_none(),None,l.lifetime)?;
    owner.spawned(server.id())?;
    if !l.input.is_empty() || !l.parts.is_empty() {owner.submit()?;}
    let optimization = server.optimization();
    let app = App::open(&spec, Transport::Protected(Box::new(server)), initialize, thread, l,&owner)?;
    let mut run=adapter.run(Body::App(Box::new(app)),0,optimization,l);run.owner=Some(owner);Ok(run)
}

fn table(client: Client) -> &'static Adapter { registry::builtin(client) }

pub(super) fn protected_requests(run: &mut Run) -> Res<Vec<Value>> {
    let Body::App(a) = &mut run.body else { return Err("protected-host-required".into()); };
    if a.owned { return Err("protected-host-required".into()); }
    a.pump()?; Ok(std::mem::take(&mut a.requests))
}

pub(super) fn protected_reply(run: &mut Run, reply: &Value, until: Instant) -> Res<()> {
    let Body::App(a) = &mut run.body else { return Err("protected-host-required".into()); };
    if reply.get("method").is_some() || reply.get("id").is_none() || (reply.get("result").is_some() == reply.get("error").is_some()) { return Err("protected server-request reply required".into()); }
    let Some(Transport::Protected(s)) = &mut a.transport else { return Err("protected-host-required".into()); };
    s.send(reply, until)
}

#[cfg(test)]
use registry::project_dir;

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn raw_native_session_validation_precedes_commands() {
        let command=||Command::new("raw-must-not-spawn");let error=Some("native CLI session invalid");let budget=Duration::ZERO;
        for name in ["claude","codex","antigravity"] {
            let runtime=runtime(name).unwrap();
            for session in ["--foreign-flag","-x","opaque:/file","one two","one\nline","","-"] {
                let spec=NativeSpec {session:session.into(),resume:true,..Default::default()};
                assert_eq!(runtime.native_arguments(&spec).err().as_deref(),error);
                assert!(super::super::history::sessions::resume(name,session).is_err(),"terminal argv requires a CLI-safe ID");
                let mut terminal=Launch::new(std::env::temp_dir(),std::env::temp_dir().join("unused.log"),"","fixtureq");terminal.command=vec!["raw-must-not-spawn".into()];
                assert_eq!(runtime.descriptor().history_resume(session,&terminal,&[]).err().as_deref(),Some("history session invalid"),"terminal ID refusal precedes preflight");
                assert_eq!(runtime.turn(command(),&spec,Path::new("unused.log"),"never",&Process::default()).err().as_deref(),error);
                if runtime.descriptor().has("background") {
                    assert_eq!(runtime.background_launch(command(),&spec,"never",budget).err().as_deref(),error);
                    assert_eq!(runtime.jobs(command(),&spec,false,budget).err().as_deref(),error);
                    assert_eq!(runtime.job(&Value::Null,&spec,"job").err().as_deref(),error);
                    let job=BackgroundJob {id:"job".into(),session:"safe".into(),pid:None,root:PathBuf::new(),scope:String::new()};
                    assert_eq!(runtime.stop_job(command(),&spec,&job,budget).err().as_deref(),error);
                }
                if runtime.descriptor().has("app-queue") {assert_eq!(runtime.app_queue(command(),&spec,"never",budget).err().as_deref(),error);}
                if runtime.descriptor().has("app-server") {assert_eq!(runtime.app_server_arguments(&spec).err().as_deref(),error);}
            }
            for (session,resume) in [("session-1",true),("session-1",false),("-",false)] {
                assert!(runtime.native_arguments(&NativeSpec {session:session.into(),resume,..Default::default()}).is_ok());
            }
        }
    }

    fn args(name: &str, l: &Launch) -> Res<Vec<String>> {
        mapped(Client::parse(name).unwrap(), name, l).map(|v| v.iter().map(|a| a.to_string_lossy().into_owned()).collect())
    }

    // Registry declarations must agree with actual operations in both directions, not a second name whitelist.
    #[test]
    fn declared_and_undeclared_capabilities_match_operations() {
        let root=std::env::temp_dir().join(format!("agentq-capabilities-{}",crate::incident::uuid()));
        std::fs::create_dir_all(&root).unwrap();
        let absent=root.join("not-installed");
        let mcp=root.join("mcp.json"); std::fs::write(&mcp,r#"{"mcpServers":{}}"#).unwrap();
        let rules=root.join("rules.md"); std::fs::write(&rules,"rules").unwrap();
        let launch=||Launch {home:Some(root.clone()),command:vec![absent.clone().into_os_string()],..Launch::new(&root,root.join("turn.log"),"input","fixtureq")};
        for runtime in runtimes() {
            let adapter=table(Client::parse(runtime.descriptor().name).unwrap());
            let mut probes=std::collections::BTreeMap::new();
            let supported=|result: Res<()>| result.err().is_none_or(|e|!e.contains("unsupported"));
            // Each option goes through the same spec builder as launch, including native policy mapping.
            for capability in ["compact-hook","title","effort","mcp-config","instructions-file","turn-parts","turn-deltas","host-args","settings-env"] {
                let mut l=launch();
                match capability {
                    "compact-hook"=>l.compact_hook=Some("echo compact".into()),
                    "title"=>l.title=Some("title".into()),
                    "effort"=>l.effort=Some("high".into()),
                    "mcp-config"=>l.mcp=Some(mcp.clone()),
                    "instructions-file"=>l.instructions=Some(rules.clone()),
                    "turn-parts"=>l.parts=vec![Part::Text("part".into())],
                    "turn-deltas"=>l.deltas=true,
                    "host-args"=>l.host_args=vec!["--fixture".into()],
                    "settings-env"=>l.settings_env=vec![("FIXTURE".into(),"yes".into())],
                    _=>unreachable!(),
                }
                probes.insert(capability,supported(adapter.spec(&l,"session-1".into(),false).map(|_|())));
            }
            // A missing caller-owned executable reaches spawn failure without ever starting a native agent.
            probes.insert("turn",supported(runtime.launch(&launch()).map(|_|())));
            let mut scoped=launch();scoped.lifetime=Lifetime::StopWithOwner;
            probes.insert("process-lifetime",supported(runtime.launch(&scoped).map(|_|())));
            probes.insert("resume",supported(runtime.resume("session-1",&launch()).map(|_|())));
            for (capability,mode) in [("background",Mode::Background),("app-queue",Mode::Queue {transcript:root.join("transcript")}),("app-server",Mode::AppServer {initialize:json!({}),thread:json!({})})] {
                let mut l=launch(); l.mode=mode;
                probes.insert(capability,supported(runtime.resume("session-1",&l).map(|_|())));
            }
            let native=NativeSpec {root:root.clone(),session:"session-1".into(),home:Some(root.clone()),..NativeSpec::default()};
            let native_args=runtime.native_arguments(&native).unwrap();
            let args:Vec<_>=native_args.iter().map(|s|s.to_string_lossy()).collect();
            probes.insert("caller-session-id",args.windows(2).any(|p|p==["--session-id","session-1"]));
            probes.insert("session-url",!matches!(runtime.session_link(&native,None),Err((Cause("no-session-url"),_))));
            probes.insert("catalogue",supported(runtime.catalogue_with_executable(Some(&root),Some(&absent)).map(|_|())));
            probes.insert("acp",adapter.ops.acp.is_some());
            probes.insert("image-input",input_items(runtime.descriptor(),&[Part::Image(std::env::current_exe().unwrap())]).is_ok());
            let app_spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],home:Some(root.clone()),env:vec![]};
            let body=Body::App(Box::new(App {transport:None,session:AppSession::new(&app_spec).unwrap(),pending:vec![],model:None,closed:None,owned:false,requests:vec![],effort:None}));
            let mut run=adapter.run(body,0,Optimization {intent:Intent::Off,effective:"off",reason:"optimization_retired"},&launch());
            probes.insert("account",!matches!(runtime.account(&mut run),Err(Failure::Refused(Cause("account-unsupported")))));
            let quotas=concat!(r#"{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"five_hour","utilization":0.5,"resetsAt":9999999999}}"#,"\n",
                r#"{"type":"event_msg","payload":{"rate_limits":{"primary":{"used_percent":50,"window_minutes":300,"resets_at":9999999999}}}}"#);
            probes.insert("quota-snapshot",runtime.log_limits(quotas,None,Some(&root)).iter().any(|w|w.used.is_some()));
            probes.insert("history",runtime.history_sessions(Some(&root.join("empty-history"))).is_ok());
            // Empty PATH and an isolated home avoid live clients and account credentials.
            let windows=registry::with_test_path(&root,||runtime.account_limits(Some(&root),0));
            probes.insert("account-limits",!windows.is_empty() && windows.iter().all(|w|w.unknown.as_deref().is_none_or(|s|!s.contains("unsupported"))));
            for capability in runtime.descriptor().capabilities {assert!(probes.contains_key(capability),"{}: {capability} has no behavioral probe",runtime.descriptor().name);}
            for (capability,actual) in probes {assert_eq!(actual,runtime.descriptor().has(capability),"{}: {capability}",runtime.descriptor().name);}
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    // #216 A2: native Windows layout seen on Claude Code 2.1.317. Risk: a verbatim canonical root (`\\?\C:\…`) names a
    // project folder (`----C--…`) Claude never writes, so a background run never attaches and never reads its URL.
    #[test]
    fn claude_project_dir_uses_the_plain_path() {
        assert_eq!(project_dir(r"\\?\C:\Users\a\agentq\.tmp\x"), "C--Users-a-agentq--tmp-x");
        assert_eq!(project_dir(r"\\?\UNC\srv\share\p"), "--srv-share-p");
        assert_eq!(project_dir("/Users/a/agentq"), "-Users-a-agentq");
    }

    // #210 A1/A4: the abstract policy maps to each runtime's native flags in the caller-argument position, and a part a
    // runtime cannot enforce refuses launch. Risk: a product leaks native flags, or an unenforceable policy runs anyway.
    #[test]
    fn policy_maps_to_native_flags_or_refuses() {
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        assert_eq!(args("claude", &l).unwrap(), Vec::<String>::new(), "native policy adds nothing");
        l.policy.approval = Approval::Auto;
        l.model = Some("claude-opus-5-5".into());
        assert_eq!(args("claude", &l).unwrap(), ["--permission-mode", "auto", "--model", "claude-opus-5-5"]);
        l.policy.write = Scope::Project;
        assert_eq!(args("claude", &l).unwrap_err(), "policy write unsupported by claude");
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy.write = Scope::Project; l.policy.network = Network::On;
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox", "workspace-write", "-c", "sandbox_workspace_write.network_access=true"]);
        l.policy.write = Scope::Any; l.policy.network = Network::Native; l.policy.approval = Approval::Bypass;
        assert_eq!(args("codex", &l).unwrap(), ["--dangerously-bypass-approvals-and-sandbox"]);
        l.policy.write = Scope::Project;
        assert_eq!(args("codex", &l).unwrap_err(), "policy approval unsupported by codex", "bypass needs full write");
        l.policy = Policy { approval: Approval::Bypass, ..Policy::default() };
        l.model = Some("pro".into());
        assert_eq!(args("antigravity", &l).unwrap(), ["--dangerously-skip-permissions", "--model=pro"]);
        l.compact_hook = Some("rules".into());
        assert_eq!(args("antigravity", &l).unwrap_err(), "compact-hook unsupported by antigravity");
        l.policy.tools = Some(vec!["Read".into()]);
        assert_eq!(args("antigravity", &l).unwrap_err(), "policy tools unsupported by antigravity");
    }

    // Built-in tools, isolation, subscription login and CLI-turn relay data map to native flags; a runtime or mode that cannot
    // carry one refuses launch. Risk: a product keeps its own native flag table, or a restricted run keeps built-in tools.
    #[test]
    fn builtin_isolation_subscription_and_relay_map_or_refuse() {
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy = Policy { write: Scope::Any, approval: Approval::Bypass, builtin: Builtin::On, isolated: true, subscription: true, ..Policy::default() };
        assert_eq!(args("claude", &l).unwrap(), ["--strict-mcp-config", "--setting-sources", "project", "--disable-slash-commands", "--system-prompt-snapshot", "off",
            "--permission-mode", "bypassPermissions"], "`on` is Claude's default tool set");
        l.policy = Policy { approval: Approval::Deny, builtin: Builtin::Off, ..Policy::default() };
        assert_eq!(args("claude", &l).unwrap(), ["--tools", "", "--permission-mode", "dontAsk"]);
        // Read-only is not enforceable: MCP servers and project hooks still write without built-in tools.
        l.policy = Policy { write: Scope::None, approval: Approval::Deny, builtin: Builtin::Off, isolated: true, ..Policy::default() };
        assert_eq!(args("claude", &l).unwrap_err(), "policy write unsupported by claude");
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy = Policy { write: Scope::None, builtin: Builtin::Off, isolated: true, subscription: true, ..Policy::default() };
        let codex = args("codex", &l).unwrap();
        assert_eq!(codex[..2], ["--sandbox", "read-only"]);
        assert_eq!(codex[2..].join(" "), "--disable apps --disable plugins --disable browser_use --disable computer_use --disable multi_agent --disable image_generation \
            --disable shell_tool -c web_search=\"disabled\" -c forced_login_method=\"chatgpt\"");
        l.policy.builtin = Builtin::On;
        assert!(args("codex", &l).unwrap().join(" ").contains("--enable shell_tool -c web_search=\"live\""));
        for (part, policy) in [("builtin", Policy { builtin: Builtin::Off, ..Policy::default() }), ("isolated", Policy { isolated: true, ..Policy::default() }),
            ("subscription", Policy { subscription: true, ..Policy::default() })] {
            let l = Launch { policy, ..Launch::new("/p", "/l", "x", "q") };
            assert_eq!(args("antigravity", &l).unwrap_err(), format!("policy {part} unsupported by antigravity"));
        }
        let dir = std::env::temp_dir().join(format!("agentq-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mcp = dir.join("mcp.json");
        std::fs::write(&mcp, r#"{"mcpServers":{"relay":{"command":"x"}}}"#).unwrap();
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy.tools = Some(vec!["Read".into()]);
        l.mcp = Some(mcp.clone()); l.instructions = Some(dir.join("rules.md")); l.parts = vec![Part::Text("hi".into())]; l.deltas = true;
        assert_eq!(args("claude", &l).unwrap(), ["--mcp-config".into(), mcp.to_string_lossy().into_owned(), "--append-system-prompt-file".into(),
            dir.join("rules.md").to_string_lossy().into_owned(), "--input-format".into(), "stream-json".into(), "--include-partial-messages".into(),
            "--allowedTools".into(), "Read".into(), "mcp__relay".into()]);
        std::fs::write(&mcp, r#"{"mcpServers":{"bad name":{}}}"#).unwrap();
        assert_eq!(args("claude", &l).unwrap_err(), "mcp server name must be plain");
        std::fs::remove_dir_all(&dir).unwrap();
        let line = stream_message(&[Part::Text("hi".into())]).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": "hi"}]}}));
        assert_eq!(stream_message(&[Part::Image("rel.png".into())]).unwrap_err(), "image-invalid");
        assert_eq!((version_parts("2.1.3 (Claude Code)"), version_parts("codex-cli 0.162.0")), (Some(vec![2, 1, 3]), Some(vec![0, 162, 0])));
        assert!(version_parts("0.99.9").unwrap() < version_parts("0.162.0").unwrap());
    }

    // #212 A3: every permission request TaskQ passes today maps without loss: Claude `dontAsk`, Codex sandbox modes, network,
    // never-approve, exact writable roots, full-auto, the unelevated Windows sandbox and effort; others refuse by part.
    // Risk: a consumer's owner-configured permission silently widens, narrows or disappears in the abstract policy.
    #[test]
    fn consumer_permission_requests_map_without_loss() {
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy.approval = Approval::Deny;
        assert_eq!(args("claude", &l).unwrap(), ["--permission-mode", "dontAsk"]);
        assert_eq!(args("antigravity", &l).unwrap_err(), "policy approval unsupported by antigravity");
        l.policy = Policy { write: Scope::Any, ..Policy::default() };
        l.model = Some("gpt-5".into()); l.effort = Some("high".into());
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox", "danger-full-access", "--model", "gpt-5", "-c", "model_reasoning_effort=high"]);
        assert_eq!(args("claude", &Launch { effort: Some("high".into()), ..Launch::new("/p", "/l", "x", "q") }).unwrap_err(), "effort unsupported by claude");
        l.effort = Some("high; rm".into());
        assert_eq!(args("codex", &l).unwrap_err(), "effort requires a plain word");
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        let root = std::env::temp_dir();
        l.policy = Policy { write: Scope::Project, network: Network::Off, approval: Approval::Deny, roots: Roots::Exact(vec![]), unelevated: true, ..Policy::default() };
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox", "workspace-write", "-c", "approval_policy=never", "-c", "sandbox_workspace_write.network_access=false",
            "-c", "sandbox_workspace_write.writable_roots=[]", "-c", "windows.sandbox=unelevated"]);
        l.policy = Policy { write: Scope::None, ..Policy::default() };
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox", "read-only"]);
        l.policy = Policy { write: Scope::Project, approval: Approval::Auto, network: Network::On, roots: Roots::Exact(vec![root.clone()]), ..Policy::default() };
        assert_eq!(args("codex", &l).unwrap(), ["--full-auto".into(), "-c".into(), "sandbox_workspace_write.network_access=true".into(),
            "-c".into(), "sandbox_workspace_write.writable_roots=[]".into(), "--add-dir".into(), root.to_string_lossy().into_owned()]);
        // Legacy `-s workspace-write --add-dir <abs>` adds to the configured roots: no reset.
        l.policy = Policy { write: Scope::Project, roots: Roots::Add(vec![root.clone()]), ..Policy::default() };
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox".into(), "workspace-write".into(), "--add-dir".into(), root.to_string_lossy().into_owned()]);
        l.policy.write = Scope::Any;
        assert_eq!(args("codex", &l).unwrap(), ["--sandbox".into(), "danger-full-access".into(), "--add-dir".into(), root.to_string_lossy().into_owned()]);
        for roots in [Roots::Exact(vec!["relative".into()]), Roots::Add(vec!["relative".into()])] {
            l.policy.roots = roots;
            assert_eq!(args("codex", &l).unwrap_err(), "policy roots unsupported by codex");
        }
        for name in ["claude", "antigravity"] {
            for roots in [Roots::Exact(vec![root.clone()]), Roots::Add(vec![root.clone()])] {
                let l = Launch { policy: Policy { roots, ..Policy::default() }, ..Launch::new("/p", "/l", "x", "q") };
                assert_eq!(args(name, &l).unwrap_err(), format!("policy roots unsupported by {name}"));
            }
            let l = Launch { policy: Policy { unelevated: true, ..Policy::default() }, ..Launch::new("/p", "/l", "x", "q") };
            assert_eq!(args(name, &l).unwrap_err(), format!("policy unelevated unsupported by {name}"));
        }
        let mut l = Launch::new("/p", "/l", "x", "fixtureq");
        l.policy = Policy { write: Scope::Project, approval: Approval::Deny, ..Policy::default() };
        let t = thread_params(&l, &json!({})).unwrap();
        assert_eq!((t["sandbox"].as_str(), t["approvalPolicy"].as_str()), (Some("workspace-write"), Some("never")));
        l.policy.unelevated = true;
        assert_eq!(thread_params(&l, &json!({})).unwrap_err(), "policy unelevated unsupported by app-server");
    }

    // #217 A1/A2: parts map to native items or refuse by cause before any write; account identity maps only login kind
    // and a plain plan name. Risk: an image reaches a runtime that cannot carry it, or a credential/email leaves the adapter.
    #[test]
    fn input_parts_and_account_identity_map_or_refuse() {
        let refused = |c: &'static str| Err(Failure::Refused(Cause(c)));
        let codex = runtime("codex").unwrap().descriptor();
        let abs = std::env::current_exe().unwrap();
        assert_eq!(input_items(codex, &[Part::Text("a".into()), Part::Image(abs.clone())]).unwrap(), json!([{"type": "text", "text": "a"}, {"type": "localImage", "path": abs}]));
        assert_eq!(input_items(codex, &[]), refused("input-empty"));
        assert_eq!(input_items(codex, &[Part::Image("rel.png".into())]), refused("image-invalid"));
        for name in ["claude"] {
            assert_eq!(input_items(runtime(name).unwrap().descriptor(), &[Part::Image(abs.clone())]), refused("image-unsupported"), "{name}");
        }
        let read = |v: Value| account_of(&v);
        assert_eq!(read(json!({"id": 1, "result": {"account": {"type": "chatgpt", "email": "e@example.invalid", "planType": "pro"}}})).unwrap(),
            Account { login: Login::Subscription, plan: Some("pro".into()) });
        assert_eq!(read(json!({"id": 1, "result": {"account": {"type": "apiKey"}}})).unwrap(), Account { login: Login::ApiKey, plan: None });
        assert_eq!(read(json!({"id": 1, "result": {"account": null, "requiresOpenaiAuth": true}})).unwrap(), Account { login: Login::None, plan: None });
        assert_eq!(read(json!({"id": 1, "result": {"account": {"type": "chatgpt", "planType": "e@example.invalid"}}})).unwrap().plan, None, "plan is a plain name");
        assert!(matches!(read(json!({"id": 1, "result": {"account": {"type": "other"}}})), Err(Failure::Failed(_))), "unknown login is never guessed");
        assert!(matches!(read(json!({"id": 1, "error": {"code": -1}})), Err(Failure::Failed(_))));
    }

    // #210 A1/A3: descriptors are the only metadata; every runtime answers session_url and home/quota operations or names a
    // cause. Risk: a caller matches on runtime names, or an unsupported operation reads as success.
    #[test]
    fn descriptors_and_neutral_operations() {
        fn send<T:Send>() {}
        send::<Run>();
        let names: Vec<_> = runtimes().iter().map(|r| r.descriptor().name).collect();
        assert_eq!(names, ["claude", "codex", "antigravity"]);
        assert_eq!(runtime("agy").unwrap().descriptor().name, "antigravity");
        assert!(runtime("other").is_none());
        assert!(runtime("claude").unwrap().descriptor().has("compact-hook") && !runtime("codex").unwrap().descriptor().has("compact-hook"));
        assert!(runtime("claude").unwrap().descriptor().has("turn-parts") && runtime("codex").unwrap().descriptor().has("turn-parts") && runtime("antigravity").unwrap().descriptor().has("turn-parts"));
        assert_eq!(runtime("claude").unwrap().descriptor().models, ["opus", "sonnet", "haiku"]);
        assert!(runtimes().iter().all(|r| r.descriptor().minimum.is_empty() || version_parts(r.descriptor().minimum).is_some()));
        for r in runtimes() {
            assert!(r.home_env("fixtureq", DEFAULT).is_empty(), "default keeps the caller's home");
            let q = r.quota(&Accounts::default(), 0);
            assert_eq!((q.account.as_deref(), q.free, q.off), (Some(DEFAULT), true, false));
            assert!(!r.protected_paths(Some(Path::new("/h/.claude"))).is_empty());
        }
        assert_eq!(runtime("claude").unwrap().protected_paths(Some(Path::new("/h/.claude"))), [PathBuf::from("/h/.claude"), PathBuf::from("/h/.claude.json")]);
        let env = runtime("codex").unwrap().home_env("fixtureq", "work");
        assert!(env.iter().all(|(k, v)| k == "CODEX_HOME" && Path::new(v).ends_with("fixtureq-accounts/work")));
    }

    #[test]
    fn session_link_reads_caller_session_records() {
        let base = std::env::temp_dir().join(format!("agentq-session-link-{}", std::process::id()));
        let (root, home) = (base.join("root"), base.join("home"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(home.join("jobs/ab12cd34")).unwrap();
        let spec = |args: &[&str]| NativeSpec { root: root.clone(), session: "s-1".into(), resume: false,
            args: args.iter().map(OsString::from).collect(), home: Some(home.clone()), env: vec![],owner:None };
        let claude = runtime("claude").unwrap();
        assert_eq!(runtime("codex").unwrap().session_link(&spec(&[]), None), Err((Cause("no-session-url"), None)));
        assert_eq!(claude.session_link(&spec(&[]), None), Err((Cause("not-requested"), None)));
        assert_eq!(claude.session_link(&spec(&["--remote-control", "t"]), None), Err((Cause("unlogged"), None)));
        std::fs::write(home.join("jobs/ab12cd34/state.json"), r#"{"bridgeSessionId":"cse_J0b"}"#).unwrap();
        assert_eq!(claude.session_link(&spec(&[]), Some("ab12cd34")).unwrap(), "https://claude.ai/code/session_J0b");
        let dir = home.join("projects").join(project_dir(&root.canonicalize().unwrap().to_string_lossy()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s-1.jsonl"), r#"{"type":"system","subtype":"bridge_status","url":"https://claude.ai/code/session_Tr"}"#).unwrap();
        assert_eq!(claude.session_link(&spec(&[]), Some("ab12cd34")).unwrap(), "https://claude.ai/code/session_Tr");
        let _ = std::fs::remove_dir_all(&base);
    }
}
