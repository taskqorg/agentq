//! `agent.turn` and `agent.call` capture (agentq.md M12.13, M12.14; docs/spec/RUNTIME.md § Observation).
use super::{Client, Context, Report, Res, model_code, read_calls, sum_usage};
use crate::telemetry::{Event, Store, capture::TraceContext, shared};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

const SAMPLE: Duration = Duration::from_secs(1);
const TRACE_CONTEXT: &str = "COREQ_TRACE_CONTEXT";

/// One model request read from native records; a value the client did not report is None.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Native identity of the request, when the client names one.
    pub id: Option<String>,
    pub at_ms: Option<u64>,
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
    pub model: Option<String>,
    pub outcome: &'static str,
    pub error: &'static str,
    /// The client reported only turn totals, so this record may cover several requests.
    pub aggregate: bool,
}

/// CPU and memory of the turn's process tree; `unknown` names why the numbers are None.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessUse {
    pub user_cpu_us: Option<u64>,
    pub system_cpu_us: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub unknown: Option<&'static str>,
    /// Why `Report::exit` is None.
    pub exit_unknown: &'static str,
}
impl ProcessUse {
    pub(crate) fn none(why: &'static str) -> Self { Self { user_cpu_us: None, system_cpu_us: None, peak_rss_bytes: None, unknown: Some(why), exit_unknown: why } }
}
impl Default for ProcessUse { fn default() -> Self { Self::none("not-owned") } }

/// One process of a sample.
#[derive(Clone, Copy, Debug)]
struct Live { pid: u32, parent: u32, user_us: u64, system_us: u64, rss: u64 }

/// The tree of `root` over samples. Peak RSS is the largest sum of one sample, so per-process peaks that occur at
/// different times are never added (specq Telemetry contract).
#[derive(Debug, Default)]
struct Tree {
    root: u32,
    peak: Option<u64>,
    /// Last sampled CPU per process, and the CPU of processes whose PID was reused.
    last: std::collections::HashMap<u32, (u64, u64)>,
    banked: (u64, u64),
    /// CPU at the start of the current turn of a session process.
    base: (u64, u64),
    why: Option<&'static str>,
}
impl Tree {
    /// Returns whether the root is still alive.
    fn sample(&mut self, live: &[Live]) -> bool {
        let mut members: Vec<Live> = live.iter().filter(|p| p.pid == self.root).copied().collect();
        if members.is_empty() { return false; }
        let mut next = 0;
        while let Some(parent) = members.get(next).map(|p| p.pid) {
            next += 1;
            for p in live.iter().filter(|p| p.parent == parent && p.pid != parent) {
                if !members.iter().any(|m| m.pid == p.pid) { members.push(*p); }
            }
        }
        self.peak = Some(self.peak.unwrap_or(0).max(members.iter().map(|p| p.rss).sum()));
        for p in members {
            let old = self.last.insert(p.pid, (p.user_us, p.system_us)).unwrap_or_default();
            if p.user_us < old.0 || p.system_us < old.1 { self.banked = (self.banked.0 + old.0, self.banked.1 + old.1); }
        }
        true
    }
    fn cpu(&self) -> (u64, u64) { self.last.values().fold(self.banked, |a, c| (a.0 + c.0, a.1 + c.1)) }
    fn mark(&mut self) { self.base = self.cpu(); self.peak = None; }
    fn usage(&self) -> ProcessUse {
        let Some(peak) = self.peak else { return ProcessUse { exit_unknown: "exit-unknown", ..ProcessUse::none(self.why.unwrap_or("not-sampled")) } };
        let cpu = self.cpu();
        ProcessUse { user_cpu_us: Some(cpu.0.saturating_sub(self.base.0)), system_cpu_us: Some(cpu.1.saturating_sub(self.base.1)), peak_rss_bytes: Some(peak), unknown: None, exit_unknown: "exit-unknown" }
    }
}

/// Samples one owned process tree until the root ends or this value drops.
pub(crate) struct Sampler { tree: Arc<Mutex<Tree>>, _stop: mpsc::Sender<()> }
impl Sampler {
    pub(crate) fn start(root: u32) -> Self {
        let tree = Arc::new(Mutex::new(Tree { root, ..Tree::default() }));
        let (stop, stopped) = mpsc::channel::<()>();
        let shared = tree.clone();
        std::thread::spawn(move || loop {
            match processes() {
                Ok(live) => if !shared.lock().is_ok_and(|mut t| t.sample(&live)) { return; },
                Err(why) => { if let Ok(mut t) = shared.lock() { t.why = Some(why); } return; }
            }
            if stopped.recv_timeout(SAMPLE) != Err(mpsc::RecvTimeoutError::Timeout) { return; }
        });
        Self { tree, _stop: stop }
    }
    pub(crate) fn usage(&self) -> ProcessUse { self.tree.lock().map_or_else(|_| ProcessUse::none("not-sampled"), |t| t.usage()) }
    /// A new turn of a session process: its peak and CPU start here.
    pub(crate) fn mark(&self) {
        if let Ok(mut t) = self.tree.lock() { t.mark(); }
    }
}

#[cfg(target_os = "linux")]
fn processes() -> Result<Vec<Live>, &'static str> {
    // SAFETY: sysconf reads a constant.
    let (tick, page) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK), libc::sysconf(libc::_SC_PAGESIZE)) };
    if tick <= 0 || page <= 0 { return Err("not-sampled"); }
    let dir = std::fs::read_dir("/proc").map_err(|_| "not-sampled")?;
    Ok(dir.flatten().filter_map(|e| {
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
        let f: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        let n = |i: usize| f.get(i)?.parse::<u64>().ok();
        Some(Live { pid, parent: n(1)? as u32, user_us: n(11)? * 1_000_000 / tick as u64, system_us: n(12)? * 1_000_000 / tick as u64, rss: n(21)? * page as u64 })
    }).collect())
}

#[cfg(target_os = "macos")]
fn processes() -> Result<Vec<Live>, &'static str> {
    let mut pids = vec![0i32; 8192];
    // SAFETY: the buffer holds the byte count passed; the call returns the pids written.
    let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    if count <= 0 { return Err("not-sampled"); }
    #[repr(C)] struct Timebase { numer: u32, denom: u32 }
    unsafe extern "C" { fn mach_timebase_info(info: *mut Timebase) -> i32; }
    let mut time = Timebase { numer: 0, denom: 0 };
    // SAFETY: the call fills the struct it is given.
    if unsafe { mach_timebase_info(&mut time) } != 0 || time.denom == 0 { return Err("not-sampled"); }
    let us = |t: u64| ((t as u128 * time.numer as u128 / time.denom as u128) / 1000) as u64;
    Ok(pids[..count as usize].iter().filter_map(|pid| {
        let mut info = std::mem::MaybeUninit::<libc::proc_taskallinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_taskallinfo>() as i32;
        // SAFETY: native API writes at most size bytes into the correctly sized buffer.
        if unsafe { libc::proc_pidinfo(*pid, libc::PROC_PIDTASKALLINFO, 0, info.as_mut_ptr().cast(), size) } != size { return None; }
        // SAFETY: proc_pidinfo returned the entire initialized structure.
        let info = unsafe { info.assume_init() };
        Some(Live { pid: *pid as u32, parent: info.pbsd.pbi_ppid, user_us: us(info.ptinfo.pti_total_user), system_us: us(info.ptinfo.pti_total_system), rss: info.ptinfo.pti_resident_size })
    }).collect())
}

#[cfg(windows)]
fn processes() -> Result<Vec<Live>, &'static str> {
    use std::ffi::c_void;
    #[repr(C)] struct Entry { size: u32, usage: u32, pid: u32, heap: usize, module: u32, threads: u32, parent: u32, priority: i32, flags: u32, exe: [u16; 260] }
    #[repr(C)] #[derive(Default)] struct Time { low: u32, high: u32 }
    #[repr(C)] #[derive(Default)] struct Memory { cb: u32, faults: u32, peak_set: usize, set: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize }
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut Entry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut Entry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn GetProcessTimes(process: *mut c_void, create: *mut Time, exit: *mut Time, kernel: *mut Time, user: *mut Time) -> i32;
        fn K32GetProcessMemoryInfo(process: *mut c_void, memory: *mut Memory, size: u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const SNAP_PROCESS: u32 = 2;
    const QUERY_LIMITED: u32 = 0x1000;
    let us = |t: &Time| (((t.high as u64) << 32) | t.low as u64) / 10;
    // SAFETY: each struct matches its Win32 layout and carries its size; every opened handle is closed here.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(SNAP_PROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 { return Err("not-sampled"); }
        let mut entry: Entry = std::mem::zeroed();
        entry.size = std::mem::size_of::<Entry>() as u32;
        let mut out = vec![];
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let process = OpenProcess(QUERY_LIMITED, 0, entry.pid);
            if !process.is_null() {
                let (mut a, mut b, mut kernel, mut user, mut memory) = (Time::default(), Time::default(), Time::default(), Time::default(), Memory::default());
                memory.cb = std::mem::size_of::<Memory>() as u32;
                if GetProcessTimes(process, &mut a, &mut b, &mut kernel, &mut user) != 0 && K32GetProcessMemoryInfo(process, &mut memory, memory.cb) != 0 {
                    out.push(Live { pid: entry.pid, parent: entry.parent, user_us: us(&user), system_us: us(&kernel), rss: memory.set as u64 });
                }
                CloseHandle(process);
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        Ok(out)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn processes() -> Result<Vec<Live>, &'static str> { Err("unsupported-platform") }

fn code(s: &str) -> bool { !s.is_empty() && s.len() <= 128 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)) }

/// The host name as a code; another shape is unknown.
pub(crate) fn host() -> Option<String> {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(unix)]
    let name = {
        let mut b = [0u8; 256];
        // SAFETY: the buffer holds the length passed.
        (unsafe { libc::gethostname(b.as_mut_ptr().cast(), b.len()) } == 0)
            .then(|| String::from_utf8_lossy(&b[..b.iter().position(|c| *c == 0).unwrap_or(b.len())]).into_owned())
    };
    name.filter(|n| code(n))
}

/// A native session ID as a code; another shape is unknown.
pub(crate) fn session_code(session: Option<&str>) -> Option<&str> { session.filter(|s| code(s)) }

/// `2026-10-19T08:32:09.030Z` as unix milliseconds; another shape is unknown.
pub(crate) fn time_ms(s: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    if s.len() < 20 || !s.ends_with('Z') { return None; }
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    // Days from civil (Howard Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146097 + (y - era * 400) * 365 + (y - era * 400) / 4 - (y - era * 400) / 100 + doy - 719468;
    let seconds = days * 86400 + n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?;
    let fraction = format!("{:0<3}", s.get(19..s.len() - 1)?.strip_prefix('.').unwrap_or(""));
    u64::try_from(seconds).ok()?.checked_mul(1000)?.checked_add(fraction.get(..3)?.parse().ok()?)
}

/// The model requests of a Codex rollout, each `token_count` once by its cumulative total. `last_turn` keeps only the
/// requests after the last `task_started`.
pub(crate) fn rollout_calls(text: &str, last_turn: bool) -> Vec<Call> {
    let (mut events, mut model) = (Vec::<Value>::new(), None::<String>);
    for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v["type"] == "turn_context" { model = v["payload"]["model"].as_str().map(String::from); }
        let p = &v["payload"];
        if v["type"] != "event_msg" { continue; }
        if last_turn && p["type"] == "task_started" { events.clear(); }
        if p["type"] != "token_count" || !p["info"]["total_token_usage"].is_object() { continue; }
        let before = events.len();
        super::native::append_codex_usage(&mut events, &p["info"]["last_token_usage"], &p["info"]["total_token_usage"], false);
        if events.len() > before {
            let last = events.last_mut().expect("appended");
            last["timestamp"] = v["timestamp"].clone();
            if let Some(m) = &model { last["model"] = json!(m); }
        }
    }
    read_calls(&events).0
}

thread_local! {
    /// The caller of `agentq agent turn`, which runs no collector (`with_environment_caller`).
    static COMMAND_CALLER: RefCell<Option<TraceContext>> = const { RefCell::new(None) };
}

/// Run one `agentq agent turn` with a valid `COREQ_TRACE_CONTEXT` as its caller. An invalid value stays unknown.
pub(crate) fn with_environment_caller<T>(action: impl FnOnce() -> T) -> T {
    let caller = std::env::var(TRACE_CONTEXT).ok().and_then(|text| TraceContext::decode(&text).ok());
    COMMAND_CALLER.set(caller);
    let out = action();
    COMMAND_CALLER.set(None);
    out
}

/// Give a turn's child the caller's context. CoreQ removes the value without a current span, so a command's caller is restored.
pub(crate) fn inherit_caller(command: &mut std::process::Command) {
    crate::telemetry::capture::inherit_context(command);
    if TraceContext::current().is_none() && let Some(caller) = COMMAND_CALLER.with_borrow(Clone::clone) {
        command.env(TRACE_CONTEXT, caller.encode());
    }
}

/// The `project`, `task`, `attempt`, `actor` and `entry` of a record (agentq.md M12.13, CoreQ R55): explicit `Context`
/// values first, then the caller's. `actor` and `entry` stay absent without a caller, so `shared` names their reason.
pub(crate) struct Attribution { pub project: String, pub task: Option<u64>, pub attempt: Option<u64>, actor: Option<String>, entry: Option<u64> }

impl Attribution {
    pub(crate) fn of(ctx: &Context) -> Self {
        // ponytail: `TraceContext` has no field accessor, so this reads its documented v2 encoding; use an accessor if CoreQ adds one.
        let fields = TraceContext::current().or_else(|| COMMAND_CALLER.with_borrow(Clone::clone)).map(|c| c.encode())
            .and_then(|text| serde_json::from_str::<(String, String, Map<String, Value>)>(text.strip_prefix("2 ")?).ok())
            .map_or_else(Map::new, |(_, _, fields)| fields);
        let number = |key: &str| fields.get(key).and_then(Value::as_u64);
        let (task, attempt) = ctx.task.map_or_else(|| (number("task"), number("attempt")), |(t, a)| (Some(t), Some(a)));
        let project = fields.get("project").and_then(Value::as_str).filter(|_| ctx.project == "other").unwrap_or(ctx.project).to_string();
        Self { project, task, attempt, actor: fields.get("actor").and_then(Value::as_str).map(String::from), entry: number("entry") }
    }
    /// Why `attempt` is null.
    pub(crate) fn no_attempt(&self) -> &'static str { if self.task.is_some() { "no-attempt" } else { "no-task" } }
    /// Set a known `actor` and `entry` on a record.
    pub(crate) fn caller_fields(&self, v: &mut Value) {
        if let Some(actor) = &self.actor { v["actor"] = json!(actor); }
        if let Some(entry) = self.entry { v["entry"] = json!(entry); }
    }
}

fn call_event(c: &Call, index: usize, session: Option<&str>, runtime: &str, ctx: &Context, turn_key: &str) -> Res<Event> {
    let mut unknown = Map::new();
    let mut num = |name: &str, v: Option<u64>, why: &str| -> Value {
        if v.is_none() { unknown.insert(name.into(), json!(why)); }
        json!(v)
    };
    let who = Attribution::of(ctx);
    let (task, attempt) = (who.task, who.attempt);
    let mut v = json!({
        "product": ctx.product, "version": ctx.version, "operation": "agent.call", "kind": "agent.call",
        "trace": null, "span": null, "started_ms": num("started_ms", c.at_ms, "not-reported"), "duration_us": num("duration_us", None, "not-reported"),
        "outcome": c.outcome, "error": c.error, "project": who.project, "runtime": runtime, "account": ctx.account,
        "task": num("task", task, "no-task"), "attempt": num("attempt", attempt, who.no_attempt()),
        "input_tokens": num("input_tokens", c.input, "not-reported"), "cache_read_tokens": num("cache_read_tokens", c.cache_read, "not-reported"),
        "cache_write_tokens": num("cache_write_tokens", c.cache_write, "not-reported"), "output_tokens": num("output_tokens", c.output, "not-reported"),
        "reasoning_tokens": num("reasoning_tokens", c.reasoning, "not-reported"),
    });
    unknown.insert("trace".into(), json!("not-traced"));
    unknown.insert("span".into(), json!("not-traced"));
    let mut text = |k: &str, s: Option<&str>, why: &str| {
        v[k] = json!(s);
        if s.is_none() { unknown.insert(k.into(), json!(why)); }
    };
    text("model", c.model.as_deref(), "not-reported");
    text("route", ctx.route, "task-turn");
    text("session_id", session_code(session), "session-unknown");
    text("host", host().as_deref(), "not-reported");
    if c.aggregate { v["source"] = json!("turn-aggregate"); }
    who.caller_fields(&mut v);
    v["unknown"] = Value::Object(unknown);
    let key = crate::sha256_hex(format!("{turn_key}\0{}", c.id.clone().unwrap_or_else(|| index.to_string())).as_bytes());
    Event::identified("agent.call", &key, shared(&v))
}

/// The `agent.call` records of a finished turn, with immutable IDs under `turn_key`.
pub(crate) fn call_events(r: &Report, client: Client, ctx: &Context, turn_key: &str) -> Res<Vec<Event>> {
    r.calls.iter().enumerate().map(|(i, c)| call_event(c, i, r.session.as_deref(), client.code(), ctx, turn_key)).collect()
}

/// Append `turn` with its calls. An immutable ID already stored keeps its first payload, so a repeated record never
/// counts twice or fails on a value only the first process could measure.
pub(crate) fn record(store: &Store, turn: Event, r: &Report, client: Client, ctx: &Context, turn_key: &str) -> Res<()> {
    let mut events = vec![turn];
    events.extend(call_events(r, client, ctx, turn_key)?);
    append_records(store, &events)
}

/// The most events CoreQ takes in one append.
pub const APPEND_CHUNK: usize = 1024;

/// Append records in order, each chunk atomically; on an immutable ID conflict append that chunk one by one and keep each
/// stored payload. A failed later chunk leaves the earlier ones, so a retry of the same records stays idempotent.
pub fn append_records(store: &Store, events: &[Event]) -> Res<()> {
    let conflict = |e: &str| matches!(e, "telemetry immutable id payload mismatch" | "telemetry immutable retention time mismatch");
    for chunk in events.chunks(APPEND_CHUNK) {
        match store.append(chunk) {
            Err(e) if conflict(&e) => {
                for event in chunk {
                    match store.append(std::slice::from_ref(event)) { Err(e) if !conflict(&e) => return Err(e), _ => {} }
                }
            }
            other => { other?; }
        }
    }
    Ok(())
}

/// One turn of a session no launcher records (agentq.md M6.11).
#[derive(Clone, Copy, Debug)]
pub struct SessionTurn<'a> {
    /// The client's name; a hook session may name one outside the runtime's clients.
    pub runtime: &'a str,
    /// A missing or invalid session reads null `session-unknown`.
    pub session: Option<&'a str>,
    /// Calls keep their IDs under this scope, so a reread never counts twice.
    pub scope: &'a str,
    /// The turn's immutable ID.
    pub key: &'a str,
    /// The start when no call names one.
    pub now_ms: u64,
    pub outcome: &'static str,
    pub error: &'static str,
    /// The hook's model, for a turn or call whose transcript names none.
    pub model: Option<&'a str>,
}

fn call_scope(turn: &SessionTurn) -> String { crate::sha256_hex(format!("hook\0{}\0{}", turn.runtime, turn.scope).as_bytes()) }

fn session_events(turn: &SessionTurn, ctx: &Context, mut r: Report) -> Res<Vec<Event>> {
    (r.outcome, r.error) = (turn.outcome, turn.error);
    r.session = turn.session.map(String::from);
    let model = turn.model.filter(|m| model_code(m)).map(String::from);
    if r.usage.model.is_none() { r.usage.model.clone_from(&model); }
    for c in r.calls.iter_mut().filter(|c| c.model.is_none()) { c.model.clone_from(&model); }
    r.started_ms = r.calls.iter().find_map(|c| c.at_ms).unwrap_or(turn.now_ms) as i64;
    r.process = ProcessUse::none("hook-session");
    let mut record = r.observation_as(turn.runtime, ctx);
    record["duration_us"] = Value::Null;
    record["unknown"]["duration_us"] = json!("not-reported");
    let mut out = vec![Event::identified("agent.turn", turn.key, shared(&record))?];
    let scope = call_scope(turn);
    for (i, c) in r.calls.iter().enumerate() { out.push(call_event(c, i, r.session.as_deref(), turn.runtime, ctx, &scope)?); }
    Ok(out)
}

/// The records of one turn from its new native transcript bytes in `text`, turn first.
pub fn session_records(turn: &SessionTurn, ctx: &Context, text: &str) -> Res<Vec<Event>> {
    let client = Client::parse(turn.runtime).ok_or("session records need a known runtime")?;
    let events: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["message"]["model"] != "<synthetic>").collect();
    let mut r = Report::read(client, text, &events, None);
    if client == Client::Codex {
        r.calls = rollout_calls(text, false);
        let known = !r.calls.is_empty();
        r.usage = sum_usage(&r.calls, None, known);
    }
    if text.is_empty() { r.usage.unknown = Some("no-transcript"); }
    session_events(turn, ctx, r)
}

/// The records of one turn whose calls the hook already holds, turn first.
pub fn session_call_records(turn: &SessionTurn, ctx: &Context, mut calls: Vec<Call>) -> Res<Vec<Event>> {
    // The store refuses a model that is not a code, which would lose the whole turn.
    for c in &mut calls { c.model = c.model.take().filter(|m| model_code(m)); }
    let mut r = Report::read(Client::Claude, "", &[], None);
    r.usage = sum_usage(&calls, None, !calls.is_empty());
    r.calls = calls;
    session_events(turn, ctx, r)
}

/// The `agent.call` record of one model request a hook saw; `turn` names its session and scope.
pub fn session_call(turn: &SessionTurn, ctx: &Context, call: &Call) -> Res<Event> {
    let mut call = call.clone();
    call.model = call.model.filter(|m| model_code(m));
    call_event(&call, 0, turn.session, turn.runtime, ctx, &call_scope(turn))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, rss: u64) -> Live { Live { pid, parent, user_us: 10, system_us: 5, rss } }

    // Chain: specq Telemetry contract § Capture, A3. Risk: per-process peaks at different times inflate peak RSS, or a
    // foreign process enters the tree.
    #[test]
    fn peak_rss_is_the_largest_sampled_sum_never_the_sum_of_process_peaks() {
        let mut t = Tree { root: 1, ..Tree::default() };
        assert!(t.sample(&[p(1, 0, 100), p(2, 1, 50), p(3, 2, 25), p(9, 0, 9000)]));
        assert!(t.sample(&[p(1, 0, 40), p(2, 1, 120), p(3, 2, 5)]));
        let u = t.usage();
        assert_eq!(u.peak_rss_bytes, Some(175), "largest sum of one sample; per-process peaks would read 100+120+25=245");
        assert_eq!((u.user_cpu_us, u.system_cpu_us), (Some(30), Some(15)), "each process counts once, a foreign one never");
        assert!(!t.sample(&[p(9, 0, 9000)]), "the sampler ends with its root");
        assert_eq!(t.usage().peak_rss_bytes, Some(175), "the peak survives the exit");
        let mut reused = Tree { root: 1, ..Tree::default() };
        reused.sample(&[p(1, 0, 1), Live { user_us: 100, ..p(2, 1, 1) }]);
        reused.sample(&[p(1, 0, 1), p(2, 1, 1)]);
        assert_eq!(reused.usage().user_cpu_us, Some(120), "a reused PID keeps the CPU of the earlier process");
        reused.mark();
        assert_eq!((reused.usage().peak_rss_bytes, reused.usage().unknown), (None, Some("not-sampled")), "a new session turn starts its own peak");
        reused.sample(&[p(1, 0, 3), p(2, 1, 4)]);
        assert_eq!((reused.usage().peak_rss_bytes, reused.usage().user_cpu_us), (Some(7), Some(0)), "the turn counts only its own CPU");
    }

    // Risk: the platform probe reads nothing, so every turn reports unknown on a host that can measure it.
    #[test]
    fn sampler_measures_a_live_child_on_this_platform() {
        let mut child = if cfg!(windows) { std::process::Command::new("ping").args(["-n", "4", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn() } else { std::process::Command::new("sleep").arg("3").spawn() }.unwrap();
        let sampler = Sampler::start(child.id());
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while sampler.usage().peak_rss_bytes.is_none() && std::time::Instant::now() < until { std::thread::sleep(Duration::from_millis(20)); }
        let use_ = sampler.usage();
        child.kill().unwrap(); child.wait().unwrap();
        assert!(use_.peak_rss_bytes.is_some_and(|n| n > 0) && use_.user_cpu_us.is_some() && use_.system_cpu_us.is_some(), "{use_:?}");
    }

    fn exported(store: &Store) -> Vec<Value> { store.export(0, 0, 100).unwrap()["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()).collect() }
    fn read(client: Client, lines: &[Value]) -> Report {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        Report::read(client, &text, lines, None)
    }

    // Chain: agentq.md M12.13, specq Telemetry contract § Capture (A2). Risk: a runtime writes no `agent.call`, a turn and
    // its calls disagree, a repeated record counts twice, or an unreported bucket reads as zero.
    #[test]
    fn every_runtime_writes_one_call_per_model_request_and_null_for_unreported_tokens() {
        let d = crate::scratch("capture-calls");
        std::fs::create_dir_all(&d).unwrap();
        let store = Store::open(&d.join("telemetry.db")).unwrap();
        let ctx = Context { product: "fixtureq", version: "1.0.0", project: "owner/repo", task: Some((7, 3)), route: None, account: "default" };
        let claude = |id: &str, usage: Value| json!({"type": "assistant", "message": {"id": id, "model": "claude-opus-5-5", "usage": usage}});
        let cases = [
            (Client::Claude, vec![json!({"type": "system", "session_id": "s-claude"}),
                claude("a", json!({"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4})),
                claude("a", json!({"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4})),
                claude("b", json!({"input_tokens": 5, "output_tokens": 6, "cache_read_input_tokens": 7, "cache_creation_input_tokens": 8})),
                json!({"type": "result"})], 2, Some(6)),
            (Client::Codex, vec![json!({"type": "thread.started", "thread_id": "t-codex"}),
                json!({"type": "turn.completed", "usage": {"input_tokens": 9, "cached_input_tokens": 4, "output_tokens": 1, "reasoning_output_tokens": 1}})], 1, Some(5)),
            (Client::Antigravity, vec![json!({"type": "PLANNER_RESPONSE", "status": "DONE", "step_index": 1, "conversationId": "c-agy", "input_tokens": 2, "output_tokens": 2, "cache_read_tokens": 0, "cache_write_tokens": 0}),
                json!({"type": "PLANNER_RESPONSE", "status": "DONE", "step_index": 2, "input_tokens": 3, "output_tokens": 2, "cache_read_tokens": 0, "cache_write_tokens": 0})], 2, Some(5)),
        ];
        for (client, lines, calls, input) in cases {
            let r = read(client, &lines);
            let before = exported(&store).len();
            let turn = Event::observation(r.observation(client, &ctx)).unwrap();
            record(&store, turn.clone(), &r, client, &ctx, client.code()).unwrap();
            record(&store, turn, &r, client, &ctx, client.code()).unwrap();
            let rows = exported(&store);
            let new = &rows[before..];
            assert_eq!(new.iter().filter(|o| o["kind"] == "agent.turn").count(), 1, "{client:?}");
            let found: Vec<_> = new.iter().filter(|o| o["kind"] == "agent.call").collect();
            assert_eq!(found.len(), calls, "{client:?}: one per model request, repeated record included");
            assert_eq!(found.iter().map(|c| c["input_tokens"].as_u64().unwrap()).sum::<u64>(), input.unwrap(), "{client:?}: calls and turn describe the same tokens");
            assert_eq!(new.iter().find(|o| o["kind"] == "agent.turn").unwrap()["input_tokens"].as_u64(), input, "{client:?}");
            assert!(found.iter().all(|c| c["runtime"] == client.code() && c["task"] == 7 && c["attempt"] == 3 && c["schema"] == 1 && c["session_id"].is_string()), "{client:?}");
        }
        let rows = exported(&store);
        let codex = rows.iter().find(|o| o["kind"] == "agent.call" && o["runtime"] == "codex").unwrap();
        assert_eq!((codex["source"].as_str(), codex["reasoning_tokens"].as_u64()), (Some("turn-aggregate"), Some(1)), "turn totals are never presented as one request");

        let silent = read(Client::Claude, &[claude("c", json!({})), json!({"type": "result"})]);
        let event = Event::observation(silent.observation(Client::Claude, &ctx)).unwrap();
        record(&store, event, &silent, Client::Claude, &ctx, "silent").unwrap();
        let rows = exported(&store);
        let (turn, call) = (&rows[rows.len() - 2], &rows[rows.len() - 1]);
        for row in [turn, call] {
            assert!(row["input_tokens"].is_null() && row["unknown"]["input_tokens"] == "not-reported", "{row}");
            assert!(row["output_tokens"].is_null(), "a client reporting no tokens reads null, never zero");
        }
        let none = read(Client::Codex, &[json!({"type": "thread.started", "thread_id": "t"})]);
        assert!(none.calls.is_empty());
        let turn = none.observation(Client::Codex, &ctx);
        assert!(turn["input_tokens"].is_null() && turn["unknown"]["input_tokens"] == "no-usage", "{turn}");
        drop(store); // Windows keeps an open database undeletable.
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M12.13, specq Telemetry contract § Capture (A2). Risk: CoreQ refuses more than 1024 events in one append, so a
    // long turn loses its calls and its `agent.turn`.
    #[test]
    fn a_turn_over_the_store_batch_keeps_the_turn_and_every_call_once() {
        let d = crate::scratch("capture-chunks");
        std::fs::create_dir_all(&d).unwrap();
        let store = Store::open(&d.join("telemetry.db")).unwrap();
        let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: None, route: Some("manager"), account: "default" };
        let lines: Vec<Value> = (0..1100).map(|i| json!({"type": "assistant", "message": {"id": format!("m{i}"), "model": "claude-opus-5-5", "usage": {"input_tokens": 1, "output_tokens": 1}}})).collect();
        let r = read(Client::Claude, &lines);
        let turn = Event::observation(r.observation(Client::Claude, &ctx)).unwrap();
        record(&store, turn.clone(), &r, Client::Claude, &ctx, "long").unwrap();
        record(&store, turn, &r, Client::Claude, &ctx, "long").unwrap();
        let (mut rows, mut after) = (vec![], 0);
        loop {
            let page = store.export(0, after, 1024).unwrap();
            rows.extend(page["events"].as_array().unwrap().iter().map(|e| e["observation"]["kind"].as_str().unwrap().to_string()));
            after = page["next"].as_i64().unwrap();
            if page["more"] != true { break; }
        }
        assert_eq!(rows.len(), 1101, "a retry stays idempotent");
        assert_eq!(rows[0], "agent.turn", "the turn goes first, so a failed later chunk never loses it");
        assert_eq!(rows.iter().filter(|k| *k == "agent.call").count(), 1100);
        drop(store);
        std::fs::remove_dir_all(d).unwrap();
    }

    // Chain: M12.14 (A3). Risk: the exit code, session or tree numbers are lost, a signal reads as a code, or a process the
    // run does not own reads as measured.
    #[test]
    fn turn_carries_exit_code_session_cpu_and_peak_rss_or_a_reason() {
        let ctx = Context { product: "fixtureq", version: "1.0.0", project: "other", task: None, route: Some("manager"), account: "default" };
        let mut r = read(Client::Claude, &[json!({"type": "system", "session_id": "s-1"})]);
        r.exit = Some(3);
        r.process = ProcessUse { user_cpu_us: Some(11), system_cpu_us: Some(12), peak_rss_bytes: Some(13), unknown: None, exit_unknown: "exit-unknown" };
        let v = r.observation(Client::Claude, &ctx);
        assert_eq!((v["exit_code"].as_u64(), v["session_id"].as_str(), v["user_cpu_us"].as_u64(), v["system_cpu_us"].as_u64(), v["peak_rss_bytes"].as_u64()), (Some(3), Some("s-1"), Some(11), Some(12), Some(13)));
        Event::observation(v).expect("the store accepts every turn field");
        r.process = ProcessUse::none("not-owned");
        r.exit = None;
        r.session = Some("not a code".into());
        let v = r.observation(Client::Claude, &ctx);
        for field in ["exit_code", "user_cpu_us", "system_cpu_us", "peak_rss_bytes"] { assert!(v[field].is_null() && v["unknown"][field] == "not-owned", "{field}"); }
        assert!(v["session_id"].is_null() && v["unknown"]["session_id"] == "session-unknown", "an invalid session must not refuse the record");
        Event::observation(v).expect("null fields keep their reasons");
        #[cfg(unix)]
        {
            r.exit = Some(-1);
            let v = r.observation(Client::Claude, &ctx);
            assert!(v["exit_code"].is_null() && v["unknown"]["exit_code"] == "signalled", "{v}");
        }
    }

    // Risk: a Codex turn loses per-request records, or an earlier turn's requests count again.
    #[test]
    fn rollout_calls_keep_each_request_of_the_last_turn_once() {
        let token = |i: u64, t: u64| json!({"type": "event_msg", "timestamp": "2026-10-19T08:32:09.030Z", "payload": {"type": "token_count", "info": {
            "last_token_usage": {"input_tokens": i, "cached_input_tokens": 0, "output_tokens": i, "reasoning_output_tokens": 1},
            "total_token_usage": {"input_tokens": t, "cached_input_tokens": 0, "output_tokens": t, "reasoning_output_tokens": t}}}});
        let started = json!({"type": "event_msg", "payload": {"type": "task_started", "turn_id": "x"}});
        let context = json!({"type": "turn_context", "payload": {"model": "gpt-5.5-codex"}});
        let no_info = json!({"type": "event_msg", "payload": {"type": "token_count", "info": null}});
        let text: String = [token(1, 1), started, context, no_info, token(2, 3), token(2, 3), token(4, 7)].iter().map(|l| format!("{l}\n")).collect();
        let last = rollout_calls(&text, true);
        assert_eq!(last.iter().map(|c| c.input).collect::<Vec<_>>(), [Some(2), Some(4)]);
        assert!(last.iter().all(|c| c.model.as_deref() == Some("gpt-5.5-codex") && c.reasoning == Some(1) && !c.aggregate && c.at_ms.is_some()));
        assert_eq!(rollout_calls(&text, false).len(), 3);
    }

    #[test]
    fn time_ms_reads_source_time_and_refuses_other_shapes() {
        assert_eq!(time_ms("2026-10-19T08:32:09.030Z"), Some(1_792_398_729_030));
        assert_eq!(time_ms("2026-10-19T08:32:09Z"), Some(1_792_398_729_000));
        assert_eq!(time_ms("yesterday"), None);
    }
}
