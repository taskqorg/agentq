//! M12.9/M12.10: the one runtime trait, its descriptors and the per-runtime adapters (docs/spec/RUNTIME.md § Runtime trait).
//! Callers read only `Descriptor` data and pass an abstract `Policy`; every native flag, file and format stays here.
use super::*;
use crate::optimization::{self, Intent, Optimization};
use super::native::{BackgroundJob, NativeTurn};
use super::app::{AppServer, AppSession, app_server_command};
use super::protected::ProtectedAppServer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::process::Output;

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
}

/// Filesystem scope of a policy part; `Native` keeps the runtime's own default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope { #[default] Native, None, Project, Any }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Network { #[default] Native, On, Off }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Approval { #[default] Native, Plan, Edits, Auto, /** Never prompt; a request that needs approval is denied. */ Deny, Bypass }

/// The runtime's own shell and web tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Builtin { #[default] Native, On, Off }

/// Extra writable folders beside the workspace; every path is absolute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
        Launch { root: root.into(), log: log.into(), input: input.into(), policy: Policy::default(), model: None, effort: None, compact_hook: None,
            env: vec![], command: vec![], optimization: Intent::Inherit, product: product.into(), install: None, accounts: None,
            mode: Mode::Turn, title: None, deadline: None, process: Process::default(), home: None, mcp: None, instructions: None, parts: vec![], deltas: false, host_args: vec![], settings_env: vec![] }
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
fn version_parts(s: &str) -> Option<Vec<u64>> {
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
    runtime: &'static str,
    body: Body,
    optimization: Optimization,
    progress: u64,
    events: super::events::Cursor,
    rollout_progress: Option<(PathBuf, u64)>,
    live: Option<Live>,
    recorded: bool,
    deadline: Option<Instant>,
    /// The launch requested Claude Remote Control (`title`).
    remote: bool,
}

enum Body { Turn(Turn), Native(Box<Native>), App(Box<App>) }

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
    fn restored(mut self, saved: &Value) -> Res<Self> {
        if let Some(progress)=saved.get("progress") { self.progress=progress.as_u64().ok_or("run progress checkpoint invalid")?; }
        if let Some(events)=saved.get("events") { self.events=serde_json::from_value(events.clone()).map_err(|_|"run event checkpoint invalid")?; }
        if let Some(value)=saved.get("rollout_progress").filter(|v|!v.is_null()) {
            let (path,offset):(PathBuf,u64)=serde_json::from_value(value.clone()).map_err(|_|"rollout checkpoint invalid")?;
            let Body::Turn(t)=&self.body else {return Err("rollout checkpoint scope differs".into());};
            let session=t.resumed.clone().or_else(||session_of(Client::Codex,&read_from(&t.log,t.start).unwrap_or_default()));
            let expected=session.and_then(|s|rollout(&s,t.home.as_deref()));
            if expected.as_ref()!=Some(&path) {return Err("rollout checkpoint scope differs".into());}
            self.rollout_progress=Some((path,offset));
        }
        Ok(self)
    }
    /// The descriptor name of the runtime that owns this run.
    pub fn runtime(&self) -> &'static str { self.runtime }
    /// The optimization state configured at launch, never proof of compression.
    pub fn optimization(&self) -> &Optimization { &self.optimization }
    /// The direct child process ID of a CLI turn, or the verified background job's native process.
    pub fn id(&self) -> Option<u32> {
        match &self.body { Body::Turn(t) => Some(t.id()), Body::Native(n) => n.job.as_ref().and_then(|j| j.pid), Body::App(a) => a.transport.as_ref().map(Transport::id) }
    }
    /// Append the finished run's one `agent.turn` observation (M12.4); a second call is refused, never counted twice.
    pub fn record(&mut self, ctx: &Context, store: &crate::telemetry::Store) -> Res<Report> {
        if self.recorded { return Err("run already recorded".into()); }
        let r = match &mut self.body {
            Body::Turn(t) => {
                if t.exit.is_none() { return Err("running".into()); }
                let mut r = t.report();
                let key = t.key();
                r.recorded = crate::telemetry::Event::observation(r.observation(t.client, ctx)).and_then(|e| super::capture::record(store, e, &r, t.client, ctx, &key));
                r
            }
            Body::Native(n) => {
                if n.report.is_none() { return Err("running".into()); }
                n.turn.record(ctx, store)?.ok_or("running")?
            }
            // Each app-server turn records once under its native thread and turn identity.
            Body::App(a) => a.session.record(ctx, store)?.ok_or_else(|| "running".to_string())?,
        };
        self.recorded = true;
        Ok(r)
    }
}

fn base(command: &[OsString], client: Client) -> Res<Command> {
    Ok(match command.split_first() {
        Some((exe, rest)) => { let mut c = Command::new(exe); c.args(rest); c }
        None => Command::new(executable(client)?),
    })
}

/// The Remote Control URL of a `system` `bridge_status` record.
fn bridge(v: &Value) -> Option<String> {
    (v["subtype"] == "bridge_status").then(|| v["url"].as_str().filter(|u| native::remote_url(u)).map(String::from)).flatten()
}

/// Claude's native transcript of `session`: `<home>/projects/<root, each non-alphanumeric character as ->/<session>.jsonl`.
fn claude_transcript(spec: &Spec, session: &str) -> Res<PathBuf> {
    let root = spec.root.canonicalize().map_err(|e| format!("project root: {e}"))?;
    Ok(spec_home(spec).ok_or("no client home")?.join("projects").join(project_dir(&root.to_string_lossy())).join(format!("{session}.jsonl")))
}

/// Claude's project folder name for a canonical root. Windows canonical paths are verbatim (`\\?\C:\…`); Claude names the
/// folder from the plain path (`C--…`).
fn project_dir(root: &str) -> String {
    let plain = root.strip_prefix(r"\\?\UNC\").map(|p| format!(r"\\{p}")).or_else(|| root.strip_prefix(r"\\?\").map(String::from));
    plain.as_deref().unwrap_or(root).chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The verified background job: the launcher's job ID, else on resume the job of the same session.
fn discover(n: &Native) -> Res<Option<BackgroundJob>> {
    let find = |rows: &[Value]| rows.iter().find(|r| match &n.job_id {
        Some(id) => r["id"] == id.as_str(),
        None => n.spec.resume && r["kind"] == "background" && r["sessionId"] == n.turn.session(),
    }).cloned();
    let mut row = find(&native::background_jobs(base(&n.command, n.spec.client)?, &n.spec, true, Duration::from_secs(5))?);
    if row.is_none() { row = find(&native::background_jobs(base(&n.command, n.spec.client)?, &n.spec, false, Duration::from_secs(5))?); }
    let Some(row) = row else { return Ok(None) };
    BackgroundJob::read(&row, &n.spec, row["id"].as_str().unwrap_or_default()).map(Some)
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
    fn open(spec: &Spec, mut transport: Transport, initialize: &Value, thread: &Value, l: &Launch) -> Res<App> {
        let mut session = match AppSession::new(spec) {
            Ok(s) => s,
            Err(e) => { if let Transport::Plain(s) = &mut transport { let _ = s.kill(); } return Err(e); }
        };
        if let Transport::Plain(server) = &transport { session.sample(server.id()); }
        let owned = matches!(transport, Transport::Plain(_));
        let mut app = App { transport: Some(transport), session, pending: vec![], model: l.model.clone(), closed: None, owned, requests: vec![], effort: l.effort.clone() };
        let opened = app.handshake(initialize, thread, l);
        if opened.is_err() { if let Some(Transport::Plain(s)) = &mut app.transport { let _ = s.kill(); } }
        opened.map(|_| app)
    }

    fn handshake(&mut self, initialize: &Value, thread: &Value, l: &Launch) -> Res<()> {
        let params = if self.owned { turn_thread(l)? } else { thread_params(l, thread)? };
        let until = reply_deadline(l);
        let request = self.session.initialize(initialize.clone())?;
        self.exchange(request, until)?;
        let initialized = self.session.initialized()?;
        self.transport.as_mut().ok_or("app-server transport ended")?.send(&initialized, until)?;
        let request = self.session.open(params)?;
        self.exchange(request, until)?;
        if !self.session.ready() { return Err("app-server thread not ready".into()); }
        if !l.parts.is_empty() { self.start(input_items(&table(Client::Codex).descriptor, &l.parts).map_err(|e|e.to_string())?, until)?; }
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
                let path = claude_transcript(&self.spec, &job.session)?;
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
    fn launch(&self, launch: &Launch) -> Res<Run>;
    fn resume(&self, session: &str, launch: &Launch) -> Res<Run>;
    /// Start the next turn of a session run with exactly `parts`, once; see `Failure` for refusals.
    fn input(&self, run: &mut Run, parts: &[Part]) -> Result<(), Failure>;
    /// Read-only login identity of a session run (capability `account`); never refreshes or writes a credential.
    fn account(&self, run: &mut Run) -> Result<Account, Failure>;
    /// Read the login's model catalogue; no turn or product protocol handler is needed.
    fn catalogue(&self, home: Option<&Path>) -> Res<Vec<Model>>;
    fn log_limits(&self, log: &str, session: Option<&str>, home: Option<&Path>) -> Vec<Window>;
    fn account_limits(&self, home: Option<&Path>, now: i64) -> Vec<Window>;
    /// Request the stop; true when the exit is already confirmed.
    fn cancel(&self, run: &mut Run) -> Res<bool>;
    fn poll(&self, run: &mut Run) -> Res<State>;
    fn report(&self, run: &Run) -> Res<Report>;
    /// Ordered neutral events, each returned at most once.
    fn progress(&self, run: &mut Run) -> Res<Vec<Event>>;
    /// Capability `background`: the raw launcher exchange of this run, handed over once.
    fn launcher_output(&self, run: &mut Run) -> Result<Result<Output, crate::process::Failure>, Cause>;
    fn session_id(&self, run: &Run) -> Option<String>;
    fn session_url(&self, run: &Run) -> Result<String, Cause>;
    /// Poll, then the native session and session URL or its absence cause, in one record.
    fn observe(&self, run: &mut Run) -> Observation;
    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause>;
    fn ready(&self, executable: Option<&Path>, home: Option<&Path>) -> Ready;
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
    /// `base` with the native turn arguments, working directory and environment of `spec`.
    fn command(&self, base: Command, spec: &NativeSpec) -> Res<Command>;
    /// One turn started with `command(base, spec)`; `input` goes to the client, output appends to `log`.
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
    /// Capability `app-server`: the app-server command for a caller-owned host.
    fn app_server_command(&self, base: Command, spec: &NativeSpec) -> Res<Command>;
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
}

struct Adapter { client: Client, descriptor: Descriptor, live: AtomicUsize }

static ADAPTERS: [Adapter; 3] = [
    Adapter { client: Client::Claude, live: AtomicUsize::new(0), descriptor: Descriptor { name: "claude", label: "Claude Code", brand: "Anthropic", family: "claude",
        models: &["opus", "sonnet", "haiku"], minimum: "2.1.0",
        capabilities: &["turn", "resume", "caller-session-id", "compact-hook", "session-url", "quota-snapshot", "account-limits", "background", "title", "mcp-config",
            "instructions-file", "turn-parts", "turn-deltas", "settings-env"] } },
    Adapter { client: Client::Codex, live: AtomicUsize::new(0), descriptor: Descriptor { name: "codex", label: "Codex CLI", brand: "OpenAI", family: "gpt", models: &[], minimum: "0.162.0",
        capabilities: &["turn", "resume", "quota-snapshot", "account-limits", "catalogue", "app-queue", "app-server", "effort", "image-input", "account", "host-args", "settings-env", "mcp-config", "instructions-file", "turn-parts", "turn-deltas"] } },
    Adapter { client: Client::Antigravity, live: AtomicUsize::new(0), descriptor: Descriptor { name: "antigravity", label: "Antigravity", brand: "Google", family: "gemini", models: &[], minimum: "",
        capabilities: &["turn", "resume", "title", "account-limits"] } },
];

/// Every runtime adapter.
pub fn runtimes() -> Vec<&'static dyn Runtime> { ADAPTERS.iter().map(|a| a as &dyn Runtime).collect() }

/// The adapter whose descriptor name (or alias) is `name`.
pub fn runtime(name: &str) -> Option<&'static dyn Runtime> {
    let client = Client::parse(name)?;
    ADAPTERS.iter().find(|a| a.client == client).map(|a| a as &dyn Runtime)
}

/// The native arguments for `policy`, `model` and `compact_hook`, in the caller-argument position.
fn mapped(client: Client, name: &str, l: &Launch) -> Res<Vec<OsString>> {
    let p = &l.policy;
    let refuse = |part: &str| Err(format!("policy {part} unsupported by {name}"));
    let mut a: Vec<OsString> = vec![];
    match client {
        Client::Claude => {
            if p.read != Scope::Native { return refuse("read"); }
            // Full write only without prompts. No write stays refused: MCP servers and project hooks can write whatever the
            // built-in tools are, so Claude has no flag that enforces it.
            if !matches!((p.write, p.approval), (Scope::Native, _) | (Scope::Any, Approval::Bypass)) { return refuse("write"); }
            if p.network != Network::Native { return refuse("network"); }
            if p.roots != Roots::Native { return refuse("roots"); }
            if p.unelevated { return refuse("unelevated"); }
            // `on` is Claude's own default tool set.
            if p.builtin == Builtin::Off { a.extend(["--tools".into(), "".into()]); }
            if p.isolated {
                a.extend(["--strict-mcp-config", "--setting-sources", "project", "--disable-slash-commands", "--system-prompt-snapshot", "off"].map(OsString::from));
            }
            let mut tools = p.tools.clone();
            if tools.as_ref().is_some_and(|t| t.is_empty() || t.iter().any(|t| t.is_empty() || t.starts_with('-'))) { return refuse("tools"); }
            if let Some(file) = &l.mcp {
                a.extend(["--mcp-config".into(), file.clone().into_os_string()]);
                tools.get_or_insert_with(Vec::new).extend(mcp_servers(file)?.into_iter().map(|s| format!("mcp__{s}")));
            }
            if let Some(file) = &l.instructions { a.extend(["--append-system-prompt-file".into(), file.clone().into_os_string()]); }
            if !l.parts.is_empty() { a.extend(["--input-format", "stream-json"].map(OsString::from)); }
            if l.deltas { a.push("--include-partial-messages".into()); }
            if let Some(tools) = tools.filter(|t| !t.is_empty()) {
                a.push("--allowedTools".into());
                a.extend(tools.iter().map(OsString::from));
            }
            let mode = match p.approval { Approval::Native => None, Approval::Plan => Some("plan"), Approval::Edits => Some("acceptEdits"),
                Approval::Auto => Some("auto"), Approval::Deny => Some("dontAsk"), Approval::Bypass => Some("bypassPermissions") };
            if let Some(m) = mode { a.extend(["--permission-mode".into(), m.into()]); }
        }
        Client::Codex => {
            if !matches!(p.read, Scope::Native | Scope::Any) { return refuse("read"); }
            if p.tools.is_some() { return refuse("tools"); }
            match (p.network, p.write) {
                (Network::Native, _) | (Network::On, Scope::Project) | (Network::Off, Scope::None | Scope::Project) => {}
                _ => return refuse("network"),
            }
            let c = |a: &mut Vec<OsString>, v: &str| a.extend(["-c".into(), v.into()]);
            match (p.approval, p.write) {
                (Approval::Bypass, Scope::Any) => a.push("--dangerously-bypass-approvals-and-sandbox".into()),
                (Approval::Auto, Scope::Project) => a.push("--full-auto".into()),
                (Approval::Native | Approval::Deny, w) => {
                    let sandbox = match w { Scope::Native => None, Scope::None => Some("read-only"), Scope::Project => Some("workspace-write"), Scope::Any => Some("danger-full-access") };
                    if let Some(s) = sandbox { a.extend(["--sandbox".into(), s.into()]); }
                    if p.approval == Approval::Deny { c(&mut a, "approval_policy=never"); }
                }
                _ => return refuse("approval"),
            }
            match (p.network, p.write) {
                (Network::On, _) => c(&mut a, "sandbox_workspace_write.network_access=true"),
                (Network::Off, Scope::Project) => c(&mut a, "sandbox_workspace_write.network_access=false"),
                _ => {}
            }
            let roots = match &p.roots {
                Roots::Native => &[][..],
                Roots::Exact(r) => { c(&mut a, "sandbox_workspace_write.writable_roots=[]"); r }
                Roots::Add(r) => r,
            };
            if roots.iter().any(|r| !r.is_absolute()) { return refuse("roots"); }
            for r in roots { a.extend(["--add-dir".into(), r.clone().into_os_string()]); }
            if p.unelevated { c(&mut a, "windows.sandbox=unelevated"); }
            a.extend(codex_extras(p));
        }
        Client::Antigravity => {
            for (part, native) in [("read", p.read == Scope::Native), ("write", p.write == Scope::Native), ("network", p.network == Network::Native), ("tools", p.tools.is_none()),
                ("roots", p.roots == Roots::Native), ("unelevated", !p.unelevated), ("builtin", p.builtin == Builtin::Native), ("isolated", !p.isolated),
                ("subscription", !p.subscription)] {
                if !native { return refuse(part); }
            }
            match p.approval { Approval::Native => {}, Approval::Bypass => a.push("--dangerously-skip-permissions".into()), _ => return refuse("approval") }
        }
    }
    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        if client == Client::Antigravity { a.push(format!("--model={m}").into()); } else { a.extend(["--model".into(), m.into()]); }
    }
    if let Some(e) = &l.effort {
        if client != Client::Codex { return Err(format!("effort unsupported by {name}")); }
        if e.is_empty() || !e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') { return Err("effort requires a plain word".into()); }
        a.extend(["-c".into(), format!("model_reasoning_effort={e}").into()]);
    }
    if let Some(t) = &l.title {
        match client {
            Client::Claude => a.extend(["--name".into(), t.into(), "--remote-control".into(), t.into()]),
            Client::Antigravity => a.push(format!("--title={t}").into()),
            Client::Codex => return Err(format!("title unsupported by {name}")),
        }
    }
    if let Some(hook) = &l.compact_hook {
        if client != Client::Claude { return Err(format!("compact-hook unsupported by {name}")); }
        // `settings_env` shares one `--settings` with the hook, added once the child environment is final.
        if l.settings_env.is_empty() { a.extend(["--settings".into(), claude_settings(Some(hook), &[]).into()]); }
    }
    Ok(a)
}

/// Codex `builtin`, `isolated` and `subscription` flags; `exec` and `app-server` both take them.
fn codex_extras(p: &Policy) -> Vec<OsString> {
    let mut a: Vec<&str> = vec![];
    if p.isolated { for f in ["apps", "plugins", "browser_use", "computer_use", "multi_agent", "image_generation"] { a.extend(["--disable", f]); } }
    match p.builtin {
        Builtin::Native => {}
        Builtin::On => a.extend(["--enable", "shell_tool", "-c", "web_search=\"live\""]),
        Builtin::Off => a.extend(["--disable", "shell_tool", "-c", "web_search=\"disabled\""]),
    }
    if p.subscription { a.extend(["-c", "forced_login_method=\"chatgpt\""]); }
    a.into_iter().map(OsString::from).collect()
}

/// Server names of an MCP config file (`mcpServers` keys).
fn mcp_servers(file: &Path) -> Res<Vec<String>> {
    let v: Value = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).ok_or("mcp config unreadable")?;
    let names: Vec<String> = v["mcpServers"].as_object().ok_or("mcp config names no servers")?.keys().cloned().collect();
    if names.iter().any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')) { return Err("mcp server name must be plain".into()); }
    Ok(names)
}

/// Complete JSON lines of `log` from byte `from`, and the byte after the last complete line.
fn lines_from(log: &Path, from: u64) -> Res<(Vec<Value>, u64)> {
    let mut f = File::open(log).map_err(|e| format!("agent log unreadable: {e}"))?;
    if f.metadata().map_err(|e| e.to_string())?.len() < from { return Err("agent log truncated; progress unknown".into()); }
    let mut rest = vec![];
    f.seek(SeekFrom::Start(from)).and_then(|_| f.read_to_end(&mut rest)).map_err(|e| format!("agent log read: {e}"))?;
    let Some(end) = rest.iter().rposition(|b| *b == b'\n') else { return Ok((vec![], from)) };
    let values = String::from_utf8_lossy(&rest[..end]).lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(Value::is_object).collect();
    Ok((values, from + end as u64 + 1))
}

impl Adapter {
    fn native(&self, s: &NativeSpec) -> Spec {
        Spec { client: self.client, root: s.root.clone(), session: s.session.clone(), resume: s.resume, args: s.args.clone(), home: s.home.clone(), env: s.env.clone() }
    }

    fn needs(&self, capability: &str) -> Res<()> {
        if self.descriptor.has(capability) { Ok(()) } else { Err(format!("{capability} unsupported by {}", self.descriptor.name)) }
    }

    fn link(&self, run: &Run) -> Result<String, native::Absence> {
        let none = |c| Err((Cause(c), None));
        let Ok(run) = self.own(run) else { return none("foreign-run") };
        if !self.descriptor.has("session-url") { return none("no-session-url"); }
        // An unreadable transcript names its error rather than reading as an unknown session (P2).
        let text = match &run.body {
            Body::Turn(t) => read_log(&t.log, t.start).map_err(|e| (Cause("evidence-unreadable"), Some(format!("transcript: {e}"))))?,
            _ => String::new(),
        };
        let Some(session) = self.session_id(run) else { return none("session-unknown") };
        let (home, job, seen) = match &run.body {
            Body::Turn(t) => (t.home.clone(), None, text.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| bridge(&v)).next_back()),
            Body::Native(n) => (spec_home(&n.spec), n.job.as_ref().map(|j| j.id.clone()).or_else(|| n.job_id.clone()), n.turn.remote_url().map(String::from)),
            Body::App(_) => return none("no-session-url"),
        };
        native::claude_link(home.as_deref(), &session, job.as_deref(), run.remote, seen)
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
        let name = self.descriptor.name;
        let app = matches!(l.mode, Mode::AppServer { .. }) || self.client == Client::Codex && automatic(l);
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
        let mut args = if !app { mapped(self.client, name, l)? } else if self.client == Client::Codex { codex_extras(&l.policy) } else { vec![] };
        args.extend(l.host_args.iter().cloned());
        let mut env = l.env.clone();
        if self.client != Client::Claude { env.extend(l.settings_env.iter().map(|(k, v)| (k.into(), v.into()))); }
        else if l.settings_env.iter().rev().find(|(k, _)| k == optimization::ENV).is_some_and(|(_, v)| v == "off") {
            env.push((optimization::ENV.into(), "off".into()));
        }
        if l.policy.subscription && self.client == Client::Claude { env.extend(["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"].map(|k| (k.into(), OsString::new()))); }
        let mut spec = Spec { client: self.client, root: l.root.clone(), session, resume, args, home: l.home.clone(), env };
        // Isolation excludes the user hooks that carry optimization, so it cannot be effective.
        let intent = if l.policy.isolated && self.client == Client::Claude { Intent::Off } else { l.optimization };
        let optimization = optimization::prepare(intent, &l.product, &mut spec, l.install.as_deref());
        if self.client == Client::Claude && !l.settings_env.is_empty() {
            // A `--bg` session runs under the Claude daemon, whose environment is not this child's: settings `env` reaches it.
            let child: Vec<(String, String)> = [optimization::ENV, "TASKQ_TASK", "AGENTQ_LAUNCHER"].into_iter()
                .filter_map(|k| child_env(&spec, k).map(|v| (k.into(), v.to_string_lossy().into_owned()))).collect();
            let mut vars: Vec<(&str, &str)> = l.settings_env.iter().chain(&child).map(|(k, v)| (k.as_str(), v.as_str())).collect();
            if l.policy.subscription { vars.extend([("ANTHROPIC_API_KEY", ""), ("ANTHROPIC_AUTH_TOKEN", "")]); }
            spec.args.extend(["--settings".into(), claude_settings(l.compact_hook.as_deref(), &vars).into()]);
        }
        Ok((spec, optimization))
    }

    fn run(&'static self, body: Body, progress: u64, optimization: Optimization, l: &Launch) -> Run {
        self.live.fetch_add(1, Ordering::SeqCst);
        Run { runtime: self.descriptor.name, body, progress, events: super::events::Cursor::new(l.deltas), rollout_progress: None, optimization, live: Some(Live(&self.live)), recorded: false, deadline: l.deadline,
            remote: l.title.is_some() }
    }

    fn start(&'static self, l: &Launch, session: Option<&str>) -> Res<Run> {
        let name = self.descriptor.name;
        if !self.descriptor.has(l.mode.capability()) { return Err(format!("mode {} unsupported by {name}", l.mode.capability())); }
        if matches!(l.mode, Mode::AppServer { .. }) { return Err("app-server requires explicit protected host; state needs in Launch".into()); }
        if let Some(accounts) = &l.accounts {
            let now = (unix_ms() / 1000).max(0);
            let q = self.quota(accounts, now);
            if q.off || !q.free { return Err("quota-exhausted".into()); }
        }
        if session.is_some_and(|s| !native::plain_id(s) || s == "-") { return Err("session requires a native session ID".into()); }
        let (spec, optimization) = self.spec(l, session.map_or_else(|| new_session(self.client), String::from), session.is_some())?;
        let command = base(&l.command, self.client)?;
        let native = |turn: NativeTurn, queue, job_id, attached, fault, launcher| Body::Native(Box::new(Native { queue, turn, spec: spec.clone(), command: l.command.clone(),
            job_id, job: None, attached, fault, report: None, launcher }));
        let uncertain = |what: &str, e: &dyn std::fmt::Display| Some(format!("{what} response uncertain: {e}; observe, never resubmit"));
        match &l.mode {
            Mode::Turn => {
                if self.client == Client::Codex && automatic(l) {
                    turn_thread(l)?;
                    if !l.parts.is_empty() { input_items(&self.descriptor, &l.parts).map_err(|e| e.to_string())?; }
                    let server = AppServer::spawn(app_server_command(command, &spec)?, &l.log, &l.process)?;
                    let app = App::open(&spec, Transport::Plain(server), &json!({"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}), &json!({}), l)?;
                    return Ok(self.run(Body::App(Box::new(app)), 0, optimization, l));
                }
                let input = if l.parts.is_empty() { l.input.clone() } else { stream_message(&l.parts)? };
                let baseline = if self.client == Client::Codex && spec.resume {
                    rollout(&spec.session, spec_home(&spec).as_deref()).map(|p|std::fs::metadata(&p).map(|m|(p,m.len())).map_err(|_|"rollout baseline unreadable")) .transpose()?
                } else {None};
                let turn = Turn::start(command, &spec, &l.log, &input, &l.process)?;
                let start = turn.start;
                let mut run = self.run(Body::Turn(turn), start, optimization, l); run.rollout_progress = baseline; Ok(run)
            }
            Mode::Background => {
                // A new job's transcript is unknown until discovery; the alias path does not exist.
                let turn = NativeTurn::before(&spec, &claude_transcript(&spec, &spec.session)?, &l.input)?;
                let exchange = native::background_launch(command, &spec, &l.input, Duration::from_secs(30))?;
                let (job_id, fault) = match &exchange {
                    Ok(o) if o.status.success() => (native::background_job_id(&String::from_utf8_lossy(&o.stdout)), None),
                    Ok(o) => (None, uncertain("background launcher", &format!("exit {:?}", o.status.code()))),
                    Err(e) => (None, uncertain("background launcher", &e.class())),
                };
                let start = turn.start();
                Ok(self.run(native(turn, false, job_id, spec.resume, fault, Some(exchange)), start, optimization, l))
            }
            Mode::Queue { transcript } => {
                if !spec.resume { return Err("queue requires the session of a running desktop thread".into()); }
                let turn = NativeTurn::before(&spec, transcript, &l.input)?;
                let fault = native::app_queue(command, &spec, &l.input, Duration::from_secs(30)).err().and_then(|e| uncertain("queue", &e));
                let start = turn.start();
                Ok(self.run(native(turn, true, None, true, fault, None), start, optimization, l))
            }
            Mode::AppServer { initialize, thread } => {
                thread_params(l, thread)?;
                let server = AppServer::spawn(app_server_command(command, &spec)?, &l.log, &l.process)?;
                let app = App::open(&spec, Transport::Plain(server), initialize, thread, l)?;
                Ok(self.run(Body::App(Box::new(app)), 0, optimization, l))
            }
        }
    }
}

impl Runtime for Adapter {
    fn descriptor(&self) -> &Descriptor { &self.descriptor }

    fn launch(&self, launch: &Launch) -> Res<Run> { table(self.client).start(launch, None) }

    fn resume(&self, session: &str, launch: &Launch) -> Res<Run> { table(self.client).start(launch, Some(session)) }

    fn input(&self, run: &mut Run, parts: &[Part]) -> Result<(), Failure> {
        let run = self.own_mut(run)?;
        let items = input_items(&self.descriptor, parts)?;
        let until = run.deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
        match &mut run.body {
            Body::App(a) if !a.owned => { a.pump()?; a.start(items, until)?; run.recorded = false; if run.live.is_none() { table(self.client).live.fetch_add(1, Ordering::SeqCst); run.live = Some(Live(&table(self.client).live)); } Ok(()) }
            // A CLI run reads one input; the next turn is a resume.
            _ => Err(Failure::Refused(Cause("input-unsupported"))),
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

    fn catalogue(&self, home: Option<&Path>) -> Res<Vec<Model>> { self.needs("catalogue")?; super::limits::catalogue(home) }

    fn log_limits(&self, log: &str, session: Option<&str>, home: Option<&Path>) -> Vec<Window> {
        let events = log.lines().filter_map(|l| serde_json::from_str(l).ok()).collect::<Vec<_>>();
        let path = session.and_then(|s| self.session_transcript(s, home));
        super::limits::read(self.client, &events, path.as_deref())
    }

    fn account_limits(&self, home: Option<&Path>, now: i64) -> Vec<Window> { super::limits::account(self.client, home, now) }

    fn cancel(&self, run: &mut Run) -> Res<bool> {
        match &mut self.own_mut(run)?.body {
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
                    Transport::Plain(s) => { s.kill()?; Ok(s.poll()?.is_some()) }
                    Transport::Protected(s) => { s.cancel(until)?; Ok(false) }
                }
            }
            Body::Native(n) => {
                if n.job.is_none() { n.job = discover(n)?; }
                let job = n.job.as_ref().ok_or("background job unknown; nothing to stop")?;
                native::background_stop(base(&n.command, n.spec.client)?, &n.spec, job, Duration::from_secs(5))?;
                Ok(false)
            }
        }
    }

    fn poll(&self, run: &mut Run) -> Res<State> {
        let run = self.own_mut(run)?;
        let s = match &mut run.body {
            Body::Turn(t) => {
                // An agentapi submitter exits after its receipt while the turn continues, so the deadline applies after exit too.
                if run.deadline.is_some_and(|d| Instant::now() >= d) && !t.timed_out {
                    t.timed_out = true;
                    let _ = t.kill();
                }
                // An agentapi turn has exited once its deadline stop killed it.
                let exited = t.poll()?.is_some() || t.timed_out && t.ag.native_agentapi && t.exit.is_some();
                if !exited { return Ok(State::Running); }
                state(&t.report())
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
        if s != State::Running && s != State::Unknown { run.live = None; }
        Ok(s)
    }

    fn report(&self, run: &Run) -> Res<Report> {
        match &self.own(run)?.body {
            Body::Turn(t) if t.exit.is_some() => Ok(t.report()),
            Body::Native(n) if n.report.is_some() => Ok(n.report.clone().expect("report")),
            Body::App(a) => a.session.report().ok_or_else(|| "running".into()),
            _ => Err("running".into()),
        }
    }

    fn progress(&self, run: &mut Run) -> Res<Vec<Event>> {
        let run = self.own_mut(run)?;
        let path = match &mut run.body {
            Body::Turn(t) => { t.sync_antigravity_transcript()?; t.log.clone() }
            Body::Native(n) if n.attached && n.turn.path().exists() => n.turn.path().to_path_buf(),
            Body::Native(_) => return Ok(vec![]),
            Body::App(a) => { a.pump()?; return Ok(run.events.read(self.client, std::mem::take(&mut a.pending))); }
        };
        let (mut values, next) = lines_from(&path, run.progress)?;
        if let Body::Turn(t) = &run.body {
            if self.client == Client::Codex {
                if run.rollout_progress.is_none() && t.resumed.is_none() { run.rollout_progress = session_of(self.client, &read_from(&t.log,t.start).unwrap_or_default()).and_then(|s| rollout(&s, t.home.as_deref())).map(|p|(p,0)); }
                if let Some((path, from)) = &mut run.rollout_progress {
                    let (rows, next) = lines_from(path, *from)?; *from = next;
                    values.extend(rows.into_iter().filter(|v| v["payload"]["type"] == "token_count"));
                }
            }
        }
        run.progress = next;
        Ok(run.events.read(self.client, values))
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
            Body::Turn(t) => session_of(self.client, &read_from(&t.log, t.start).unwrap_or_default())
                .or_else(|| t.ag.session.clone()).or_else(|| t.resumed.clone()),
            Body::Native(n) => n.attached.then(|| n.turn.session().to_string()),
            Body::App(a) => a.session.session().map(String::from),
        }
    }

    fn session_url(&self, run: &Run) -> Result<String, Cause> { self.link(run).map_err(|(cause, _)| cause) }

    fn observe(&self, run: &mut Run) -> Observation {
        let (state, error) = match self.poll(run) { Ok(s) => (s, None), Err(e) => (State::Unknown, Some(e)) };
        let (url, cause, detail) = match self.link(run) { Ok(u) => (Some(u), None, None), Err((c, d)) => (None, Some(c), d) };
        Observation { state, error, session: self.session_id(run), url, cause, detail }
    }

    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause> {
        match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Turn(t) => Ok(t.log.clone()),
            Body::Native(n) if n.attached => Ok(n.turn.path().to_path_buf()),
            Body::Native(_) => Err(Cause("transcript-unknown")),
            // The private log holds stderr only; `progress` is the session stream.
            Body::App(_) => Err(Cause("no-transcript")),
        }
    }

    fn ready(&self, executable: Option<&Path>, home: Option<&Path>) -> Ready {
        let check = |state, detail: &str| Check { state, detail: detail.into() };
        let exe = match executable {
            Some(p) if p.is_absolute() || p.components().count() > 1 => Ok(p.to_path_buf()),
            Some(p) => Ok(which_in(&p.to_string_lossy(), &std::env::var_os("PATH").unwrap_or_default())),
            None => self::executable(self.client),
        };
        let run = |exe: &Path, args: &[&str], budget| {
            let mut c = Command::new(exe);
            c.args(args);
            if let Some(h) = home { c.env(self.client.home_var(), h); }
            crate::quiet(&mut c);
            crate::process::output("agent.ready", &mut c, Duration::from_secs(budget))
        };
        let exe = match exe {
            Ok(exe) if exe.is_absolute() && exe.is_file() => exe,
            Ok(exe) => return Ready { install: check("missing", &format!("{} not found", exe.display())), version: check("unknown", "not installed"),
                login: check("unknown", "not installed") },
            Err(e) => return Ready { install: check("unknown", &e), version: check("unknown", "executable unknown"), login: check("unknown", "executable unknown") },
        };
        let minimum = self.descriptor.minimum;
        let version = match run(&exe, &["--version"], 5) {
            Ok(o) => match String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim).filter(|s| !s.is_empty()) {
                None => check("unknown", "version not reported"),
                Some(v) if minimum.is_empty() => check("ok", v),
                Some(v) => match (version_parts(v), version_parts(minimum)) {
                    (Some(have), Some(need)) if have < need => check("outdated", &format!("{v} is below {minimum}")),
                    (Some(_), _) => check("ok", v),
                    (None, _) => check("unknown", &format!("{v} is not a version")),
                },
            },
            Err(e) => check("unknown", &e.to_string()),
        };
        let status = |args: &[&str]| run(&exe, args, 10).map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)));
        let login = match self.client {
            Client::Claude => match status(&["auth", "status", "--json"]) {
                Ok(out) => match serde_json::from_str::<Value>(out.trim()).ok().or_else(|| out.lines().find_map(|l| serde_json::from_str(l).ok())) {
                    Some(v) if v["loggedIn"] == false => check("missing", "not logged in"),
                    Some(v) if v["loggedIn"] == true && v["authMethod"] == "claude.ai" && v["apiProvider"] == "firstParty" => check("ok", "subscription"),
                    Some(v) if v["loggedIn"] == true => check("api-key", v["authMethod"].as_str().unwrap_or("other login")),
                    _ => check("unknown", "login status unreadable"),
                },
                Err(e) => check("unknown", &e.to_string()),
            },
            Client::Codex => match status(&["-c", "forced_login_method=\"chatgpt\"", "login", "status"]) {
                Ok(out) => {
                    let plain: String = out.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
                    if plain.contains("loggedinusingchatgpt") { check("ok", "subscription") }
                    else if plain.contains("notloggedin") { check("missing", "not logged in") }
                    else if plain.contains("loggedin") { check("api-key", "not a ChatGPT login") }
                    else { check("unknown", "login status unreadable") }
                }
                Err(e) => check("unknown", &e.to_string()),
            },
            // No status command; a keychain or OS credential store may hold the login.
            Client::Antigravity => check("unknown", "no login status command"),
        };
        Ready { install: check("ok", &exe.to_string_lossy()), version, login }
    }

    fn quota(&self, accounts: &Accounts, now: i64) -> Quota {
        Quota { account: accounts.pick(now), free: accounts.active(now).is_some(), off: accounts.off(), hold: accounts.hold(now) }
    }

    fn running(&self) -> usize { self.live.load(Ordering::SeqCst) }

    fn protected_paths(&self, home: Option<&Path>) -> Vec<PathBuf> {
        let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(self.client)) else { return vec![] };
        let mut v = vec![home.clone()];
        match self.client {
            // The default `~/.claude` home keeps `~/.claude.json` beside it; `CLAUDE_CONFIG_DIR` keeps it inside.
            Client::Claude => v.push(if home.file_name().is_some_and(|n| n == ".claude") { home.with_file_name(".claude.json") } else { home.join(".claude.json") }),
            Client::Antigravity => if let Some(p) = home.parent() { v.push(p.join("config")) },
            Client::Codex => {}
        }
        v
    }

    fn home_env(&self, tool: &str, label: &str) -> Vec<(OsString, OsString)> {
        account_home(self.client, tool, label).map(|h| vec![(OsString::from(self.client.home_var()), h.into_os_string())]).unwrap_or_default()
    }

    fn checkpoint(&self, run: &Run) -> Result<Value, Cause> {
        let mut saved = match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Turn(t) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": "turn", "turn": t.checkpoint()?})),
            Body::App(a) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": "app", "app": a.session.checkpoint(), "model": a.model})),
            Body::Native(n) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "kind": if n.queue { "queue" } else { "background" }, "native": n.turn.checkpoint(),
                "session": n.turn.session(), "resume": n.spec.resume, "job": n.job_id, "attached": n.attached, "fault": n.fault})),
        }?;
        saved["progress"]=json!(run.progress); saved["events"]=json!(run.events); saved["rollout_progress"]=json!(run.rollout_progress);
        Ok(saved)
    }

    fn recover(&self, saved: &Value, l: &Launch) -> Res<Run> {
        let me = table(self.client);
        let queue = matches!(l.mode, Mode::Queue { .. });
        let kind = match l.mode { Mode::Turn if self.client == Client::Codex && automatic(l) => "app", Mode::Turn => "turn", Mode::Background => "background", Mode::Queue { .. } => "queue", Mode::AppServer { .. } => "app" };
        if saved["schema"] != "agentq-run/1" || saved["runtime"] != self.descriptor.name || saved["kind"] != kind {
            return Err("run recovery scope differs".into());
        }
        if kind == "turn" {
            // Observation only: the saved child keeps running; a resumed turn's session is the saved one.
            let resumed = saved["turn"]["resumed"].as_str();
            let (spec, optimization) = me.spec(l, resumed.map_or_else(|| new_session(self.client), String::from), resumed.is_some())?;
            let turn = Turn::recover(&spec, &l.log, &saved["turn"])?;
            let start = turn.start;
            return me.run(Body::Turn(turn), start, optimization, l).restored(saved);
        }
        if kind == "app" {
            // Observation only: the transport ended with its process, so the run reports and records but takes no input.
            let app = &saved["app"];
            let (spec, optimization) = me.spec(l, app["session"].as_str().ok_or("run recovery session invalid")?.into(), app["session"] != "-")?;
            let session = AppSession::recover(&spec, app)?;
            let a = App { transport: None, session, pending: vec![], model: saved["model"].as_str().map(String::from), closed: Some("recovered".into()), owned: automatic(l), requests: vec![], effort: l.effort.clone() };
            return me.run(Body::App(Box::new(a)), 0, optimization, l).restored(saved);
        }
        let session = saved["session"].as_str().filter(|s| native::plain_id(s)).ok_or("run recovery session invalid")?;
        let (spec, optimization) = me.spec(l, session.into(), saved["resume"].as_bool().ok_or("run recovery state invalid")?)?;
        let turn = NativeTurn::recover(&spec, &saved["native"])?;
        let text = |k: &str| saved[k].as_str().map(String::from);
        if !saved["job"].is_null() && text("job").is_none_or(|j| !native::plain_id(&j)) { return Err("run recovery job invalid".into()); }
        let start = turn.start();
        let body = Body::Native(Box::new(Native { queue, turn, spec, command: l.command.clone(), job_id: text("job"), job: None,
            attached: saved["attached"].as_bool().ok_or("run recovery state invalid")?, fault: text("fault"), report: None, launcher: None }));
        me.run(body, start, optimization, l).restored(saved)
    }

    fn classify(&self, log: &str) -> Report {
        let events: Vec<Value> = log.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        Report::read(self.client, log, &events, None)
    }

    fn executable(&self, path: Option<&OsStr>) -> Res<PathBuf> {
        match path { Some(p) => executable_in(self.client, p), None => executable(self.client) }
    }

    fn home(&self) -> Option<PathBuf> { base_home(self.client) }

    fn session_transcript(&self, session: &str, home: Option<&Path>) -> Option<PathBuf> {
        (self.client == Client::Codex).then(|| rollout(session, home)).flatten()
    }

    fn session_link(&self, spec: &NativeSpec, job: Option<&str>) -> Result<String, (Cause, Option<String>)> {
        if !self.descriptor.has("session-url") { return Err((Cause("no-session-url"), None)); }
        let native = self.native(spec);
        let requested = spec.args.iter().any(|a| a == "--remote-control");
        // ponytail: whole transcript read; the last bridge_status URL wins, a missing transcript falls to the job and debug log.
        let seen = claude_transcript(&native, &spec.session).ok().and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| t.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| bridge(&v)).next_back());
        native::claude_link(spec_home(&native).as_deref(), &spec.session, job, requested, seen)
    }

    fn prepare(&self, intent: Intent, product: &str, spec: &mut NativeSpec, install: Option<&Path>) -> Optimization {
        let mut native = self.native(spec);
        let optimization = optimization::prepare(intent, product, &mut native, install);
        (spec.args, spec.env) = (native.args, native.env);
        optimization
    }

    fn command(&self, base: Command, spec: &NativeSpec) -> Res<Command> { Ok(command(base, &self.native(spec))) }

    fn turn(&self, base: Command, spec: &NativeSpec, log: &Path, input: &str, process: &Process) -> Res<Turn> {
        Turn::start(base, &self.native(spec), log, input, process)
    }

    fn background_launch(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Result<Output, crate::process::Failure>> {
        self.needs("background")?;
        native::background_launch(base, &self.native(spec), reference, budget)
    }

    fn jobs(&self, base: Command, spec: &NativeSpec, filtered: bool, budget: Duration) -> Res<Vec<Value>> {
        self.needs("background")?;
        native::background_jobs(base, &self.native(spec), filtered, budget)
    }

    fn job(&self, row: &Value, spec: &NativeSpec, id: &str) -> Res<BackgroundJob> {
        self.needs("background")?;
        BackgroundJob::read(row, &self.native(spec), id)
    }

    fn stop_job(&self, base: Command, spec: &NativeSpec, job: &BackgroundJob, budget: Duration) -> Res<Output> {
        self.needs("background")?;
        native::background_stop(base, &self.native(spec), job, budget)
    }

    fn launch_refusal(&self, text: &str) -> Option<Cause> {
        let lower = text.to_ascii_lowercase();
        let cause = match self.client {
            Client::Claude if text.contains("Workspace not trusted") => "workspace-untrusted",
            Client::Claude if text.contains("No conversation found with session ID") => "session-missing",
            Client::Claude if text.contains("Not logged in") || text.contains("/login") => "login-required",
            Client::Codex if text.contains("app-server thread/resume refused") => "session-missing",
            Client::Codex if ["401", "login", "log in"].iter().any(|m| lower.contains(m)) => "login-required",
            _ => return None,
        };
        Some(Cause(cause))
    }

    fn app_queue(&self, base: Command, spec: &NativeSpec, reference: &str, budget: Duration) -> Res<Output> {
        self.needs("app-queue")?;
        native::app_queue(base, &self.native(spec), reference, budget)
    }

    fn app_server_command(&self, base: Command, spec: &NativeSpec) -> Res<Command> {
        self.needs("app-server")?;
        app_server_command(base, &self.native(spec))
    }
}

/// A protected host frontend opened as a Codex app-server run (`ProtectedAppServer::open`).
pub(crate) fn open_protected(server: ProtectedAppServer, session: Option<&str>, l: &Launch) -> Res<Run> {
    let codex = table(Client::Codex);
    let Mode::AppServer { initialize, thread } = &l.mode else { return Err("protected open requires mode app-server".into()) };
    if session.is_some_and(|s| !native::plain_id(s) || s == "-") { return Err("session requires a native session ID".into()); }
    let (spec, _) = codex.spec(l, session.unwrap_or("-").into(), session.is_some())?;
    let optimization = server.optimization();
    let app = App::open(&spec, Transport::Protected(Box::new(server)), initialize, thread, l)?;
    Ok(codex.run(Body::App(Box::new(app)), 0, optimization, l))
}

fn table(client: Client) -> &'static Adapter { ADAPTERS.iter().find(|a| a.client == client).expect("adapter in table") }

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
mod tests {
    use super::*;

    fn args(name: &str, l: &Launch) -> Res<Vec<String>> {
        mapped(Client::parse(name).unwrap(), name, l).map(|v| v.iter().map(|a| a.to_string_lossy().into_owned()).collect())
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
        for name in ["claude", "antigravity"] {
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
        let names: Vec<_> = runtimes().iter().map(|r| r.descriptor().name).collect();
        assert_eq!(names, ["claude", "codex", "antigravity"]);
        assert_eq!(runtime("agy").unwrap().descriptor().name, "antigravity");
        assert!(runtime("other").is_none());
        assert!(runtime("claude").unwrap().descriptor().has("compact-hook") && !runtime("codex").unwrap().descriptor().has("compact-hook"));
        assert!(runtime("claude").unwrap().descriptor().has("turn-parts") && runtime("codex").unwrap().descriptor().has("turn-parts") && !runtime("antigravity").unwrap().descriptor().has("turn-parts"));
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
            args: args.iter().map(OsString::from).collect(), home: Some(home.clone()), env: vec![] };
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
