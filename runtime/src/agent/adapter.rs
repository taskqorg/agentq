//! M12.9/M12.10: the one runtime trait, its descriptors and the per-runtime adapters (docs/spec/RUNTIME.md § Runtime trait).
//! Callers read only `Descriptor` data and pass an abstract `Policy`; every native flag, file and format stays here.
use super::*;
use crate::optimization::{self, Intent, Optimization};
use super::native::{BackgroundJob, NativeTurn};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The only runtime metadata a caller reads. `name` is an opaque ID; `label` and `brand` are for people.
#[derive(Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static str,
    pub label: &'static str,
    pub brand: &'static str,
    pub family: &'static str,
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
pub enum Approval { #[default] Native, Plan, Edits, Auto, Bypass }

/// Abstract permissions. The adapter maps each part to native flags or refuses the launch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    pub read: Scope,
    pub write: Scope,
    pub network: Network,
    /// None keeps the native tool set; Some is an allowlist.
    pub tools: Option<Vec<String>>,
    pub approval: Approval,
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
}

/// How a run reaches the runtime; every mode but `Turn` needs the descriptor capability of the same name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// One CLI turn (capability `turn`).
    #[default]
    Turn,
    /// A native background job (capability `background`); `Launch.input` is the caller's protected file reference.
    Background,
    /// Queue `Launch.input` (a reference) into a running desktop thread and observe its native `transcript` (capability `app-queue`).
    Queue { transcript: PathBuf },
}

impl Mode {
    fn capability(&self) -> &'static str {
        match self { Mode::Turn => "turn", Mode::Background => "background", Mode::Queue { .. } => "app-queue" }
    }
}

impl Launch {
    pub fn new(root: impl Into<PathBuf>, log: impl Into<PathBuf>, input: impl Into<String>, product: impl Into<String>) -> Self {
        Launch { root: root.into(), log: log.into(), input: input.into(), policy: Policy::default(), model: None, compact_hook: None,
            env: vec![], command: vec![], optimization: Intent::Inherit, product: product.into(), install: None, accounts: None,
            mode: Mode::Turn, title: None, deadline: None }
    }
}

/// A named reason an operation has no value for this runtime or run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause(pub &'static str);

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Running, Finished, Failed, Unknown }

impl State {
    pub fn code(self) -> &'static str {
        match self { State::Running => "running", State::Finished => "finished", State::Failed => "failed", State::Unknown => "unknown" }
    }
}

/// One readiness part: `ok`, `missing` or `unknown`, with the value or the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check { pub state: &'static str, pub detail: String }

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
    live: Option<Live>,
    recorded: bool,
    deadline: Option<Instant>,
}

enum Body { Turn(Turn), Native(Box<Native>) }

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
}

impl Run {
    /// The descriptor name of the runtime that owns this run.
    pub fn runtime(&self) -> &'static str { self.runtime }
    /// The optimization state configured at launch, never proof of compression.
    pub fn optimization(&self) -> &Optimization { &self.optimization }
    /// The direct child process ID of a CLI turn, or the verified background job's native process.
    pub fn id(&self) -> Option<u32> {
        match &self.body { Body::Turn(t) => Some(t.id()), Body::Native(n) => n.job.as_ref().and_then(|j| j.pid) }
    }
    /// Append the finished run's one `agent.turn` observation (M12.4); a second call is refused, never counted twice.
    pub fn record(&mut self, ctx: &Context, store: &crate::telemetry::Store) -> Res<Report> {
        if self.recorded { return Err("run already recorded".into()); }
        let r = match &mut self.body {
            Body::Turn(t) => {
                if t.exit.is_none() { return Err("running".into()); }
                let mut r = t.report();
                r.recorded = crate::telemetry::Event::new(r.observation(t.client, ctx)).and_then(|e| store.append(&[e])).map(|_| ());
                r
            }
            Body::Native(n) => {
                if n.report.is_none() { return Err("running".into()); }
                n.turn.record(ctx, store)?.ok_or("running")?
            }
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

/// Claude's native transcript of `session`: `<home>/projects/<root, each non-alphanumeric character as ->/<session>.jsonl`.
fn claude_transcript(spec: &Spec, session: &str) -> Res<PathBuf> {
    let root = spec.root.canonicalize().map_err(|e| format!("project root: {e}"))?;
    let dir: String = root.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    Ok(spec_home(spec).ok_or("no client home")?.join("projects").join(dir).join(format!("{session}.jsonl")))
}

/// The verified background job: the launcher's job ID, else on resume the job of the same session.
fn discover(n: &Native) -> Res<Option<BackgroundJob>> {
    let rows = native::background_jobs(base(&n.command, n.spec.client)?, &n.spec, Duration::from_secs(5))?;
    let row = rows.iter().find(|r| match &n.job_id {
        Some(id) => r["id"] == id.as_str(),
        None => n.spec.resume && r["kind"] == "background" && r["sessionId"] == n.turn.session(),
    });
    let Some(row) = row else { return Ok(None) };
    BackgroundJob::read(row, &n.spec, row["id"].as_str().unwrap_or_default()).map(Some)
}

fn state(r: &Report) -> State { match r.outcome { "ok" => State::Finished, "unknown" => State::Unknown, _ => State::Failed } }

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

/// The one trait every runtime adapter implements (specq agent-runtimes § Runtime trait).
pub trait Runtime: Send + Sync {
    fn descriptor(&self) -> &Descriptor;
    fn launch(&self, launch: &Launch) -> Res<Run>;
    fn resume(&self, session: &str, launch: &Launch) -> Res<Run>;
    fn input(&self, run: &mut Run, text: &str) -> Res<()>;
    /// Request the stop; true when the exit is already confirmed.
    fn cancel(&self, run: &mut Run) -> Res<bool>;
    fn poll(&self, run: &mut Run) -> Res<State>;
    fn report(&self, run: &Run) -> Res<Report>;
    /// Complete native JSON records appended since the previous call.
    fn progress(&self, run: &mut Run) -> Res<Vec<Value>>;
    fn session_id(&self, run: &Run) -> Option<String>;
    fn session_url(&self, run: &Run) -> Result<String, Cause>;
    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause>;
    fn ready(&self, home: Option<&Path>) -> Ready;
    fn quota(&self, accounts: &Accounts, now: i64) -> Quota;
    fn running(&self) -> usize;
    fn protected_paths(&self, home: Option<&Path>) -> Vec<PathBuf>;
    /// The environment selecting account `label` of consumer `tool`; empty for `default` or an invalid label.
    fn home_env(&self, tool: &str, label: &str) -> Vec<(OsString, OsString)>;
    /// Private recovery value of a background or queue run; the caller protects and stores it before relying on it.
    fn checkpoint(&self, run: &Run) -> Result<Value, Cause>;
    /// The run rebuilt from `checkpoint` and the same launch fields, without a native call.
    fn recover(&self, saved: &Value, launch: &Launch) -> Res<Run>;
}

struct Adapter { client: Client, descriptor: Descriptor, live: AtomicUsize }

static ADAPTERS: [Adapter; 3] = [
    Adapter { client: Client::Claude, live: AtomicUsize::new(0), descriptor: Descriptor { name: "claude", label: "Claude Code", brand: "Anthropic", family: "claude",
        capabilities: &["turn", "resume", "caller-session-id", "compact-hook", "session-url", "quota-snapshot", "background", "title"] } },
    Adapter { client: Client::Codex, live: AtomicUsize::new(0), descriptor: Descriptor { name: "codex", label: "Codex CLI", brand: "OpenAI", family: "gpt",
        capabilities: &["turn", "resume", "quota-snapshot", "app-queue"] } },
    Adapter { client: Client::Antigravity, live: AtomicUsize::new(0), descriptor: Descriptor { name: "antigravity", label: "Antigravity", brand: "Google", family: "gemini",
        capabilities: &["turn", "resume", "title"] } },
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
            if p.write != Scope::Native { return refuse("write"); }
            if p.network != Network::Native { return refuse("network"); }
            if let Some(tools) = &p.tools {
                if tools.is_empty() || tools.iter().any(|t| t.is_empty() || t.starts_with('-')) { return refuse("tools"); }
                a.push("--allowedTools".into());
                a.extend(tools.iter().map(OsString::from));
            }
            let mode = match p.approval { Approval::Native => None, Approval::Plan => Some("plan"), Approval::Edits => Some("acceptEdits"),
                Approval::Auto => Some("auto"), Approval::Bypass => Some("bypassPermissions") };
            if let Some(m) = mode { a.extend(["--permission-mode".into(), m.into()]); }
        }
        Client::Codex => {
            if !matches!(p.read, Scope::Native | Scope::Any) { return refuse("read"); }
            if p.tools.is_some() { return refuse("tools"); }
            match (p.network, p.write) {
                (Network::Native, _) | (Network::On, Scope::Project) | (Network::Off, Scope::None | Scope::Project) => {}
                _ => return refuse("network"),
            }
            match (p.approval, p.write) {
                (Approval::Bypass, Scope::Any) => a.push("--dangerously-bypass-approvals-and-sandbox".into()),
                (Approval::Native, w) => {
                    let sandbox = match w { Scope::Native => None, Scope::None => Some("read-only"), Scope::Project => Some("workspace-write"), Scope::Any => Some("danger-full-access") };
                    if let Some(s) = sandbox { a.extend(["--sandbox".into(), s.into()]); }
                    if p.network == Network::On { a.extend(["-c".into(), "sandbox_workspace_write.network_access=true".into()]); }
                }
                _ => return refuse("approval"),
            }
        }
        Client::Antigravity => {
            for (part, native) in [("read", p.read == Scope::Native), ("write", p.write == Scope::Native), ("network", p.network == Network::Native), ("tools", p.tools.is_none())] {
                if !native { return refuse(part); }
            }
            match p.approval { Approval::Native => {}, Approval::Bypass => a.push("--dangerously-skip-permissions".into()), _ => return refuse("approval") }
        }
    }
    if let Some(m) = &l.model {
        if !model_code(m) { return Err("model requires a model code".into()); }
        if client == Client::Antigravity { a.push(format!("--model={m}").into()); } else { a.extend(["--model".into(), m.into()]); }
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
        a.extend(["--settings".into(), claude_settings(Some(hook), &[]).into()]);
    }
    Ok(a)
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
        let mut spec = Spec { client: self.client, root: l.root.clone(), session, resume, args: mapped(self.client, self.descriptor.name, l)?, home: None, env: l.env.clone() };
        let optimization = optimization::prepare(l.optimization, &l.product, &mut spec, l.install.as_deref());
        Ok((spec, optimization))
    }

    fn run(&'static self, body: Body, progress: u64, optimization: Optimization, l: &Launch) -> Run {
        self.live.fetch_add(1, Ordering::SeqCst);
        Run { runtime: self.descriptor.name, body, progress, optimization, live: Some(Live(&self.live)), recorded: false, deadline: l.deadline }
    }

    fn start(&'static self, l: &Launch, session: Option<&str>) -> Res<Run> {
        let name = self.descriptor.name;
        if !self.descriptor.has(l.mode.capability()) { return Err(format!("mode {} unsupported by {name}", l.mode.capability())); }
        if let Some(accounts) = &l.accounts {
            let now = (unix_ms() / 1000).max(0);
            let q = self.quota(accounts, now);
            if q.off || !q.free { return Err("quota-exhausted".into()); }
        }
        if session.is_some_and(|s| !native::plain_id(s) || s == "-") { return Err("session requires a native session ID".into()); }
        let (spec, optimization) = self.spec(l, session.map_or_else(|| new_session(self.client), String::from), session.is_some())?;
        let command = base(&l.command, self.client)?;
        let native = |turn: NativeTurn, queue, job_id, attached, fault| Body::Native(Box::new(Native { queue, turn, spec: spec.clone(), command: l.command.clone(),
            job_id, job: None, attached, fault, report: None }));
        let uncertain = |what: &str, e: &dyn std::fmt::Display| Some(format!("{what} response uncertain: {e}; observe, never resubmit"));
        match &l.mode {
            Mode::Turn => {
                let turn = Turn::start(command, &spec, &l.log, &l.input)?;
                let start = turn.start;
                Ok(self.run(Body::Turn(turn), start, optimization, l))
            }
            Mode::Background => {
                // A new job's transcript is unknown until discovery; the alias path does not exist.
                let turn = NativeTurn::before(&spec, &claude_transcript(&spec, &spec.session)?, &l.input)?;
                let (job_id, fault) = match native::background_launch(command, &spec, &l.input, Duration::from_secs(30))? {
                    Ok(o) if o.status.success() => (String::from_utf8_lossy(&o.stdout).split_whitespace().next_back().filter(|t| native::plain_id(t)).map(String::from), None),
                    Ok(o) => (None, uncertain("background launcher", &format!("exit {:?}", o.status.code()))),
                    Err(e) => (None, uncertain("background launcher", &e.class())),
                };
                let start = turn.start();
                Ok(self.run(native(turn, false, job_id, spec.resume, fault), start, optimization, l))
            }
            Mode::Queue { transcript } => {
                if !spec.resume { return Err("queue requires the session of a running desktop thread".into()); }
                let turn = NativeTurn::before(&spec, transcript, &l.input)?;
                let fault = native::app_queue(command, &spec, &l.input, Duration::from_secs(30)).err().and_then(|e| uncertain("queue", &e));
                let start = turn.start();
                Ok(self.run(native(turn, true, None, true, fault), start, optimization, l))
            }
        }
    }
}

impl Runtime for Adapter {
    fn descriptor(&self) -> &Descriptor { &self.descriptor }

    fn launch(&self, launch: &Launch) -> Res<Run> { table(self.client).start(launch, None) }

    fn resume(&self, session: &str, launch: &Launch) -> Res<Run> { table(self.client).start(launch, Some(session)) }

    fn input(&self, run: &mut Run, _text: &str) -> Res<()> {
        self.own(run)?;
        Err("input-unsupported: a run reads one input; resume the session for the next turn".into())
    }

    fn cancel(&self, run: &mut Run) -> Res<bool> {
        match &mut self.own_mut(run)?.body {
            Body::Turn(t) => {
                if !t.timed_out { t.cancelled = true; }
                t.kill()?;
                Ok(t.exit.is_some())
            }
            Body::Native(n) if n.queue => Err("cancel-unsupported: desktop cancellation is a caller UI operation".into()),
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
                if run.deadline.is_some_and(|d| Instant::now() >= d) && !t.timed_out && t.exit.is_none() {
                    t.timed_out = true;
                    let _ = t.kill();
                }
                // An agentapi turn has exited once its deadline stop killed it.
                let exited = t.poll()?.is_some() || t.timed_out && t.ag.native_agentapi && t.exit.is_some();
                if !exited { return Ok(State::Running); }
                state(&t.report())
            }
            Body::Native(n) => n.poll()?,
        };
        if s != State::Running && s != State::Unknown { run.live = None; }
        Ok(s)
    }

    fn report(&self, run: &Run) -> Res<Report> {
        match &self.own(run)?.body {
            Body::Turn(t) if t.exit.is_some() => Ok(t.report()),
            Body::Native(n) if n.report.is_some() => Ok(n.report.clone().expect("report")),
            _ => Err("running".into()),
        }
    }

    fn progress(&self, run: &mut Run) -> Res<Vec<Value>> {
        let run = self.own_mut(run)?;
        let path = match &mut run.body {
            Body::Turn(t) => { t.sync_antigravity_transcript()?; t.log.clone() }
            Body::Native(n) if n.attached && n.turn.path().exists() => n.turn.path().to_path_buf(),
            Body::Native(_) => return Ok(vec![]),
        };
        let (values, next) = lines_from(&path, run.progress)?;
        run.progress = next;
        Ok(values)
    }

    fn session_id(&self, run: &Run) -> Option<String> {
        match &self.own(run).ok()?.body {
            Body::Turn(t) => session_of(self.client, &read_from(&t.log, t.start).unwrap_or_default())
                .or_else(|| t.ag.session.clone()).or_else(|| t.resumed.clone()),
            Body::Native(n) => n.attached.then(|| n.turn.session().to_string()),
        }
    }

    fn session_url(&self, run: &Run) -> Result<String, Cause> {
        self.own(run).map_err(|_| Cause("foreign-run"))?;
        if !self.descriptor.has("session-url") { return Err(Cause("no-session-url")); }
        if self.session_id(run).is_none() { return Err(Cause("session-unknown")); }
        match &run.body {
            Body::Turn(t) => {
                let text = read_from(&t.log, t.start).unwrap_or_default();
                text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["subtype"] == "bridge_status")
                    .filter_map(|v| v["url"].as_str().filter(|u| native::remote_url(u)).map(String::from)).next_back().ok_or(Cause("not-observed"))
            }
            Body::Native(n) => n.turn.remote_url().map(String::from).ok_or(Cause("not-observed")),
        }
    }

    fn transcript(&self, run: &Run) -> Result<PathBuf, Cause> {
        match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Turn(t) => Ok(t.log.clone()),
            Body::Native(n) if n.attached => Ok(n.turn.path().to_path_buf()),
            Body::Native(_) => Err(Cause("transcript-unknown")),
        }
    }

    fn ready(&self, home: Option<&Path>) -> Ready {
        let check = |state, detail: &str| Check { state, detail: detail.into() };
        let (install, version) = match executable(self.client) {
            Ok(exe) if exe.is_absolute() => {
                let mut c = Command::new(&exe);
                c.arg("--version");
                crate::quiet(&mut c);
                let version = match crate::process::output("agent.version", &mut c, Duration::from_secs(5)) {
                    Ok(o) => String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim).filter(|s| !s.is_empty())
                        .map_or_else(|| check("unknown", "version not reported"), |v| check("ok", v)),
                    Err(e) => check("unknown", &e.to_string()),
                };
                (check("ok", &exe.to_string_lossy()), version)
            }
            Ok(_) => (check("missing", &format!("{} CLI not found on PATH", self.descriptor.name)), check("unknown", "not installed")),
            Err(e) => (check("unknown", &e), check("unknown", "executable unknown")),
        };
        let home = home.map(Path::to_path_buf).or_else(|| base_home(self.client));
        let file = match self.client { Client::Claude => Some(".credentials.json"), Client::Codex => Some("auth.json"), Client::Antigravity => None };
        let login = match (home, file) {
            (None, _) => check("unknown", "no client home"),
            (_, None) => check("unknown", "no credential file"),
            (Some(h), Some(f)) if h.join(f).is_file() => check("ok", f),
            // A keychain or OS credential store may hold the login.
            (Some(_), Some(_)) => check("unknown", "credential store unknown"),
        };
        Ready { install, version, login }
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
        match &self.own(run).map_err(|_| Cause("foreign-run"))?.body {
            Body::Turn(_) => Err(Cause("no-checkpoint")),
            Body::Native(n) => Ok(json!({"schema": "agentq-run/1", "runtime": self.descriptor.name, "queue": n.queue, "native": n.turn.checkpoint(),
                "session": n.turn.session(), "resume": n.spec.resume, "job": n.job_id, "attached": n.attached, "fault": n.fault})),
        }
    }

    fn recover(&self, saved: &Value, l: &Launch) -> Res<Run> {
        let me = table(self.client);
        let queue = matches!(l.mode, Mode::Queue { .. });
        if saved["schema"] != "agentq-run/1" || saved["runtime"] != self.descriptor.name || saved["queue"] != queue || l.mode == Mode::Turn {
            return Err("run recovery scope differs".into());
        }
        let session = saved["session"].as_str().filter(|s| native::plain_id(s)).ok_or("run recovery session invalid")?;
        let (spec, optimization) = me.spec(l, session.into(), saved["resume"].as_bool().ok_or("run recovery state invalid")?)?;
        let turn = NativeTurn::recover(&spec, &saved["native"])?;
        let text = |k: &str| saved[k].as_str().map(String::from);
        if !saved["job"].is_null() && text("job").is_none_or(|j| !native::plain_id(&j)) { return Err("run recovery job invalid".into()); }
        let start = turn.start();
        let body = Body::Native(Box::new(Native { queue, turn, spec, command: l.command.clone(), job_id: text("job"), job: None,
            attached: saved["attached"].as_bool().ok_or("run recovery state invalid")?, fault: text("fault"), report: None }));
        Ok(me.run(body, start, optimization, l))
    }
}

fn table(client: Client) -> &'static Adapter { ADAPTERS.iter().find(|a| a.client == client).expect("adapter in table") }

#[cfg(test)]
mod tests {
    use super::*;

    fn args(name: &str, l: &Launch) -> Res<Vec<String>> {
        mapped(Client::parse(name).unwrap(), name, l).map(|v| v.iter().map(|a| a.to_string_lossy().into_owned()).collect())
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

    // #210 A1/A3: descriptors are the only metadata; every runtime answers session_url and home/quota operations or names a
    // cause. Risk: a caller matches on runtime names, or an unsupported operation reads as success.
    #[test]
    fn descriptors_and_neutral_operations() {
        let names: Vec<_> = runtimes().iter().map(|r| r.descriptor().name).collect();
        assert_eq!(names, ["claude", "codex", "antigravity"]);
        assert_eq!(runtime("agy").unwrap().descriptor().name, "antigravity");
        assert!(runtime("other").is_none());
        assert!(runtime("claude").unwrap().descriptor().has("compact-hook") && !runtime("codex").unwrap().descriptor().has("compact-hook"));
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
}
