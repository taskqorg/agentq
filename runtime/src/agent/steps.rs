//! One ordered list of typed steps for any run, live or finished (agentq.md M12.18; RUNTIME.md § Steps).
//! Native parsing stays here; a client reads only `Turn`, `Step` and their revision rule.
use super::{Client, Res, events, history::History};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// Inline bound of one text field; the full value reads by `Output::source`.
pub const INLINE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status { Running, Completed, Failed, Cancelled, Unknown }

impl Status {
    fn terminal(self) -> bool { self != Self::Running }
    fn native(status: Option<&str>) -> Option<Self> {
        Some(match status? {
            "inProgress" | "in_progress" | "running" | "pending" | "ACTIVE" | "RUNNING" => Self::Running,
            "completed" | "success" | "SUCCESS" | "DONE" | "ok" => Self::Completed,
            "failed" | "error" | "declined" | "ERROR" | "FAILED" | "INVALID" | "killed" | "timeout" | "expired" => Self::Failed,
            "interrupted" | "cancelled" | "canceled" | "stopped" | "CANCELLED" | "CANCELED" | "INTERRUPTED" => Self::Cancelled,
            _ => Self::Unknown,
        })
    }
}

/// Byte range of a source record, and the value inside it when set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub text: String,
    pub truncated: bool,
    pub bytes: Option<u64>,
    pub source: Option<Source>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEdit {
    pub path: String,
    pub action: Option<String>,
    pub diff: Option<Output>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Background {
    pub task: Option<String>,
    pub state: Option<String>,
    pub output_file: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shell {
    pub command: Option<String>,
    pub argv: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
    pub output: Option<Output>,
    pub stdout: Option<Output>,
    pub stderr: Option<Output>,
    pub background: Option<Background>,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Message { role: Option<String>, text: Option<Output> },
    Thinking { text: Option<Output>, encrypted: Option<bool> },
    Shell(Shell),
    FileChange { files: Vec<FileEdit>, output: Option<Output>, stderr: Option<Output> },
    Tool { name: Option<String>, input: Option<Output>, output: Option<Output>, duration_ms: Option<u64> },
    Mcp { server: Option<String>, tool: Option<String>, input: Option<Output>, output: Option<Output>, duration_ms: Option<u64> },
    Web { action: Option<String>, query: Option<String>, url: Option<String>, output: Option<Output>, duration_ms: Option<u64> },
    Subagent { action: Option<String>, agent: Option<String>, sessions: Vec<String>, input: Option<Output>, output: Option<Output> },
    Image { path: Option<String>, media_type: Option<String> },
    Question { prompt: Option<Output>, answer: Option<Output> },
    Approval { request: Option<Output>, decision: Option<String> },
    Compaction { trigger: Option<String>, tokens_before: Option<u64>, tokens_after: Option<u64> },
    Usage { input: Option<u64>, cache_read: Option<u64>, cache_write: Option<u64>, output: Option<u64>, reasoning: Option<u64>, total: Option<u64>, context_window: Option<u64>, model: Option<String> },
    Unknown { record: String, raw: String },
}

impl Body {
    fn tool() -> Self { Self::Tool { name: None, input: None, output: None, duration_ms: None } }
    /// Fields a client expects; a null one gets a reason.
    fn expected(&self) -> &'static [&'static str] {
        match self {
            Self::Message { .. } | Self::Thinking { .. } => &["text"],
            Self::Shell(_) => &["command", "cwd", "exit_code", "duration_ms", "output", "stdout", "stderr"],
            Self::FileChange { .. } => &["output"],
            Self::Tool { .. } => &["name", "input", "output", "duration_ms"],
            Self::Mcp { .. } => &["server", "tool", "input", "output", "duration_ms"],
            Self::Web { .. } => &["output", "duration_ms"],
            Self::Subagent { .. } => &["agent", "input", "output"],
            Self::Image { .. } => &["path"],
            Self::Question { .. } => &["prompt", "answer"],
            Self::Approval { .. } => &["request", "decision"],
            Self::Compaction { .. } => &["trigger", "tokens_before", "tokens_after"],
            Self::Usage { .. } => &["input", "cache_read", "cache_write", "output", "reasoning", "total", "context_window", "model"],
            Self::Unknown { .. } => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    #[serde(flatten)]
    pub body: Body,
    pub order: u64,
    pub revision: u64,
    pub turn: Option<String>,
    pub parent: Option<String>,
    pub status: Status,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub source: Option<Source>,
    pub unknown: BTreeMap<String, String>,
}

fn empty(v: &Value) -> bool { v.is_null() || v.as_array().is_some_and(Vec::is_empty) }

impl Step {
    /// The revision rule of RUNTIME.md § Steps: repeated delivery, in any order, gives the same step.
    pub fn merge(&mut self, other: Step) {
        let (newer, older) = if other.revision >= self.revision { (other, self.clone()) } else { (self.clone(), other) };
        let (mut n, o) = (serde_json::to_value(&newer).expect("step json"), serde_json::to_value(&older).expect("step json"));
        let (n_map, o_map) = (n.as_object_mut().expect("step object"), o.as_object().expect("step object"));
        // A specific kind is never replaced by a generic one.
        let generic = |k: &str| matches!(k, "tool" | "unknown");
        let keep_older = o_map["kind"] != n_map["kind"] && generic(n_map["kind"].as_str().unwrap_or("")) && (!generic(o_map["kind"].as_str().unwrap_or("")) || n_map["kind"] == "unknown");
        if keep_older { n_map.insert("kind".into(), o_map["kind"].clone()); }
        for (key, value) in o_map {
            if n_map.get(key).is_none_or(empty) { n_map.insert(key.clone(), value.clone()); }
        }
        let mut unknown = older.unknown;
        unknown.extend(newer.unknown);
        unknown.retain(|field, _| n_map.get(field).is_none_or(empty));
        n_map.insert("unknown".into(), json!(unknown));
        n_map.insert("order".into(), json!(newer.order.min(older.order)));
        // A partial revision stamps its own record's time; the step began at the earlier one.
        if let Some(start) = [newer.started_ms, older.started_ms].into_iter().flatten().min() { n_map.insert("started_ms".into(), json!(start)); }
        *self = serde_json::from_value(n).expect("merged step");
        self.explain();
    }

    fn explain(&mut self) {
        let value = serde_json::to_value(&*self).expect("step json");
        let reason = if self.status == Status::Running { "running" } else { "not-reported" };
        // A reason of a field the final kind does not have came from a generic revision.
        let expected = self.body.expected();
        self.unknown.retain(|field, why| expected.contains(&field.as_str()) || !matches!(why.as_str(), "running" | "not-reported"));
        for field in expected {
            if empty(&value[*field]) {
                let known = self.unknown.entry((*field).into()).or_insert_with(|| reason.into());
                // A revision that ended the step ends the reason `running` too.
                if known == "running" { *known = reason.into(); }
            }
            else { self.unknown.remove(*field); }
        }
    }
}

/// A Rust client's view: revisions in, one ordered list out.
#[derive(Clone, Debug, Default)]
pub struct Timeline(BTreeMap<String, Step>);

impl Timeline {
    pub fn apply(&mut self, step: Step) {
        match self.0.get_mut(&step.id) { Some(known) => known.merge(step), None => { self.0.insert(step.id.clone(), step); } }
    }
    pub fn steps(&self) -> Vec<&Step> {
        let mut steps: Vec<&Step> = self.0.values().collect();
        steps.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
        steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub status: Status,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    pub revision: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fork { pub session: Option<String>, pub end_byte: Option<u64>, pub end_ordinal: Option<u64> }

/// What the source says about its own session; a null field names its reason in `unknown`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: Option<String>,
    pub root: Option<String>,
    pub parent: Option<String>,
    pub parent_step: Option<String>,
    pub relation: Option<String>,
    pub fork: Option<Fork>,
    pub cwd: Option<String>,
    pub version: Option<String>,
    pub unknown: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Tool { name: String, input: Value, at: Option<i64> }

#[derive(Serialize, Deserialize)]
pub(super) struct Reducer {
    client: Client,
    turn: Option<Turn>,
    open: BTreeMap<String, Step>,
    tools: BTreeMap<String, Tool>,
    cwd: Option<String>,
    model: Option<String>,
    window: Option<u64>,
    last_user: Option<(String, String)>,
    compaction: Option<String>,
    counter: u64,
    session: Session,
}

/// Revisions and turns of a batch of records, one revision per id.
#[derive(Debug, Default)]
pub(super) struct Batch {
    pub steps: Vec<Step>,
    pub turns: Vec<Turn>,
    pub covered: BTreeMap<String, u64>,
}

impl Batch {
    fn step(&mut self, step: Step) {
        match self.steps.iter_mut().find(|s| s.id == step.id) { Some(known) => known.merge(step), None => self.steps.push(step) }
    }
    fn turn(&mut self, turn: Turn) {
        match self.turns.iter_mut().find(|t| t.id == turn.id) { Some(known) => *known = turn, None => self.turns.push(turn) }
    }
}

/// One record being read. `order` of a block adds its index, so blocks of one record keep native order.
#[derive(Clone, Copy)]
struct Record<'a> { v: &'a Value, raw: &'a str, range: Option<(u64, u64)>, order: u64, revision: u64, time: Option<i64> }

impl Record<'_> {
    /// A record with no native ID: stable by content across sources, so identical delivery converges.
    fn id(&self) -> String { format!("record:{}", &crate::sha256_hex(self.v.to_string().as_bytes())[..16]) }
    fn source(&self) -> Option<Source> { self.range.map(|(start, end)| Source { start, end, pointer: None }) }
    fn output(&self, pointer: &str, value: &Value) -> Option<Output> {
        let text = value_text(value)?;
        let bytes = text.len() as u64;
        let (text, truncated) = bound(text);
        Some(Output { text, truncated, bytes: Some(bytes), source: self.range.map(|(start, end)| Source { start, end, pointer: Some(pointer.into()) }) })
    }
}

fn bound(mut text: String) -> (String, bool) {
    if text.len() <= INLINE { return (text, false); }
    let mut cut = INLINE;
    while !text.is_char_boundary(cut) { cut -= 1; }
    text.truncate(cut);
    (text, true)
}

/// A string, an array of text blocks as joined text, or other JSON as JSON text. Null reads none.
pub(super) fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Array(parts) if parts.is_empty() => None,
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) if parts.iter().all(|p| p["text"].is_string() || p.is_string()) =>
            Some(parts.iter().filter_map(|p| p["text"].as_str().or(p.as_str())).collect::<Vec<_>>().join("")),
        other => Some(other.to_string()),
    }
}

fn pointer_key(key: &str) -> String { key.replace('~', "~0").replace('/', "~1") }
fn string(v: &Value) -> Option<String> { v.as_str().filter(|s| !s.is_empty()).map(String::from) }
fn first<'a>(v: &'a Value, keys: &[&'static str]) -> (&'static str, &'a Value) {
    for key in keys { if let Some(found) = v.get(*key).filter(|f| !f.is_null()) { return (key, found); } }
    (keys[0], &Value::Null)
}
fn time_of(v: &Value) -> Option<i64> { v.as_str().and_then(events::native_time).or_else(|| v.as_i64()) }

/// A `file://` URL reads as its path; any other value stays unchanged.
fn local_path(url: &str) -> String {
    let Some(rest) = url.strip_prefix("file://") else { return url.into() };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| rest.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        if let Some(byte) = hex { out.push(byte); i += 3; } else { out.push(bytes[i]); i += 1; }
    }
    let path = String::from_utf8_lossy(&out).into_owned();
    // file:///C:/x is a Windows drive path, file:///x an absolute one.
    let drive = path.as_bytes().get(2) == Some(&b':') && path.starts_with('/');
    if drive { path[1..].into() } else { path }
}

fn duration_ms(item: &Value) -> Option<u64> {
    let (_, d) = first(item, &["duration_ms", "durationMs", "duration", "totalDurationMs"]);
    d.as_u64().or_else(|| Some(d["secs"].as_u64()? * 1000 + d["nanos"].as_u64().unwrap_or(0) / 1_000_000))
}

/// The command as the agent wrote it: the native parse of one command, the script of a shell wrapper, else the words.
fn command(item: &Value) -> (Option<String>, Option<Vec<String>>) {
    let raw = &item["command"];
    let argv = raw.as_array().map(|words| words.iter().filter_map(|w| w.as_str().map(String::from)).collect::<Vec<_>>());
    let (_, parsed) = first(item, &["parsed_cmd", "commandActions"]);
    let single = parsed.as_array().filter(|p| p.len() == 1).and_then(|p| first(&p[0], &["cmd", "command"]).1.as_str()).map(String::from);
    let text = single.or_else(|| raw.as_str().map(String::from)).or_else(|| argv.as_ref().map(|words| match words.as_slice() {
        [_, flag, script] if flag.starts_with('-') => script.clone(),
        _ => words.join(" "),
    }));
    (text, argv)
}

/// Fields of one `<task-notification>`; text that holds no complete one reads none.
fn notification(text: &str) -> Option<BTreeMap<&'static str, String>> {
    let body = text.split_once("<task-notification>")?.1.split_once("</task-notification>")?.0;
    let mut fields = BTreeMap::new();
    for tag in ["task-id", "tool-use-id", "output-file", "status", "summary"] {
        if let Some((_, rest)) = body.split_once(&format!("<{tag}>")) {
            if let Some((value, _)) = rest.split_once(&format!("</{tag}>")) { fields.insert(tag, value.trim().to_string()); }
        }
    }
    Some(fields)
}

fn leading_number(text: &str) -> Option<i64> { text.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>().parse().ok() }

/// A failed call opens its text with its code; other text naming a code is command output.
fn failed_exit(text: &str) -> Option<i64> {
    let text = text.trim_start();
    leading_number(text.strip_prefix("Error: ").unwrap_or(text).strip_prefix("Exit code ")?)
}

const CLAUDE_METADATA: &[&str] = &["api-request", "api-request-blob", "api-request-shape", "last-prompt", "atis-latch", "agent-name", "custom-title", "ai-title", "mode", "permission-mode", "session-owner", "relocated", "worktree-state", "tools-used", "bridge-session", "file-history-snapshot", "file-history-delta", "cost-state", "pr-link", "rate_limit_event", "stream_event"];
const CLAUDE_SYSTEM: &[&str] = &["bridge_status", "stop_hook_summary", "away_summary"];
const CLAUDE_CONTEXT: &[&str] = &["deferred_tools_record", "deferred_tools_delta", "silent_turn_reminder", "task_reminder", "environment", "prompt_snapshot", "remote_session_change", "mcp_instructions_delta", "hook_additional_context", "date", "instructions", "model", "session_context", "credential_org", "skill_listing", "agent_listing_delta", "auto_mode", "read_truncation_notice", "compact_file_reference", "nested_memory", "thinking_drop", "total_tokens_reminder"];
const CODEX_METADATA: &[&str] = &["session_meta", "turn_context", "world_state", "thread_settings_applied", "inter_agent_communication_metadata"];

impl Reducer {
    pub(super) fn new(client: Client) -> Self {
        Self { client, turn: None, open: BTreeMap::new(), tools: BTreeMap::new(), cwd: None, model: None, window: None, last_user: None, compaction: None, counter: 0, session: Session::default() }
    }

    /// A saved reducer of another runtime refuses.
    pub(super) fn restore(saved: &Value, client: Client) -> Res<Self> {
        let reducer: Self = serde_json::from_value(saved.clone()).map_err(|e| format!("history checkpoint steps invalid: {e}"))?;
        if reducer.client != client { return Err("history checkpoint runtime differs".into()); }
        Ok(reducer)
    }

    /// One complete source record at its byte range.
    pub(super) fn read(&mut self, v: &Value, raw: &str, start: u64, end: u64, batch: &mut Batch) {
        let time = v.get("timestamp").or_else(|| v.get("time")).or_else(|| v.get("emittedAtMs")).or_else(|| v.get("created_at")).and_then(time_of);
        self.dispatch(&Record { v, raw, range: Some((start, end)), order: start, revision: end, time }, batch);
    }

    /// One record of a live stream: positions are a counter, and `source` reads null.
    pub(super) fn read_live(&mut self, v: &Value, batch: &mut Batch) {
        self.counter += 1;
        let raw = v.to_string();
        // Room for the blocks of one record between two live positions.
        let order = self.counter * 1024;
        let time = v.get("timestamp").or_else(|| v.get("emittedAtMs")).or_else(|| v.get("created_at")).and_then(time_of);
        self.dispatch(&Record { v, raw: &raw, range: None, order, revision: self.counter, time }, batch);
    }

    fn dispatch(&mut self, r: &Record, batch: &mut Batch) {
        match self.client {
            Client::Claude => self.claude(r, batch),
            Client::Codex => self.codex(r, batch),
            Client::Antigravity => self.antigravity(r, batch),
        }
    }

    fn new_step(&self, id: String, body: Body, status: Status, r: &Record) -> Step {
        let ended = status.terminal().then_some(r.time).flatten();
        let mut unknown = BTreeMap::new();
        if r.range.is_none() { unknown.insert("source".into(), "live-stream".into()); }
        Step { id, body, order: r.order, revision: r.revision, turn: self.turn.as_ref().map(|t| t.id.clone()), parent: None, status, started_ms: r.time, ended_ms: ended, source: r.source(), unknown }
    }

    /// Merge a revision into the open step, publish it, and close it at a terminal status.
    fn emit(&mut self, mut step: Step, batch: &mut Batch) {
        if let Some(mut known) = self.open.remove(&step.id) { known.merge(step); step = known; }
        step.explain();
        if !step.status.terminal() { self.open.insert(step.id.clone(), step.clone()); }
        batch.step(step);
    }

    fn start_turn(&mut self, id: String, r: &Record, batch: &mut Batch) {
        if self.turn.as_ref().is_some_and(|t| t.id == id) { return; }
        self.close_turn(None, None, r, batch);
        let turn = Turn { id, status: Status::Running, started_ms: r.time, ended_ms: None, duration_ms: None, revision: r.revision };
        batch.turn(turn.clone());
        self.turn = Some(turn);
    }

    /// A turn with no terminal record reads unknown when the next one starts; its open foreground steps do too.
    fn close_turn(&mut self, status: Option<Status>, duration: Option<u64>, r: &Record, batch: &mut Batch) {
        let Some(mut turn) = self.turn.take() else { return };
        turn.status = status.unwrap_or(Status::Unknown);
        turn.ended_ms = status.and(r.time);
        turn.duration_ms = duration;
        turn.revision = r.revision;
        batch.turn(turn.clone());
        let pending: Vec<Step> = self.open.values().filter(|s| s.turn.as_ref() == Some(&turn.id) && !matches!(&s.body, Body::Shell(Shell { background: Some(_), .. }))).cloned().collect();
        for mut step in pending {
            self.open.remove(&step.id);
            step.status = Status::Unknown;
            step.revision = r.revision;
            step.unknown.insert("ended_ms".into(), "no-terminal-record".into());
            batch.step(step);
        }
    }

    fn unknown(&mut self, record: String, id: String, r: &Record, batch: &mut Batch) {
        let step = self.new_step(id, Body::Unknown { record, raw: r.raw.into() }, Status::Unknown, r);
        self.emit(step, batch);
    }

    fn usage(&mut self, id: String, usage: [Option<u64>; 6], r: &Record, batch: &mut Batch) {
        let [input, cache_read, cache_write, output, reasoning, total] = usage;
        let body = Body::Usage { input, cache_read, cache_write, output, reasoning, total, context_window: self.window, model: self.model.clone() };
        let step = self.new_step(id, body, Status::Completed, r);
        self.emit(step, batch);
    }

    // Codex: rollout, `exec --json` and app-server shapes reach one item mapper.
    fn codex(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        let method = v["method"].as_str().unwrap_or("");
        let kind = v["type"].as_str().unwrap_or("");
        if !method.is_empty() { return self.codex_app(method, r, batch); }
        if matches!(kind, "item.started" | "item.completed") {
            return self.codex_item(&v["item"], "/item", kind == "item.started", r, batch);
        }
        match kind {
            "thread.started" => {
                self.session.id = string(&v["thread_id"]);
                return;
            }
            "turn.started" => { self.start_turn(format!("turn:{}", r.order), r, batch); return; }
            "turn.completed" | "turn.failed" => {
                let u = &v["usage"];
                if let (Some(total), Some(turn)) = (Self::total(u), self.turn.as_ref().map(|t| t.id.clone())) {
                    self.usage(format!("usage:{turn}:{total}"), Self::counters(u), r, batch);
                }
                self.close_turn(Some(if kind == "turn.completed" { Status::Completed } else { Status::Failed }), None, r, batch);
                return;
            }
            "error" => { self.unknown(kind.into(), r.id(), r, batch); return; }
            _ => {}
        }
        // A JSON-RPC reply is the transport's, not the session's.
        if kind.is_empty() && v.get("id").is_some() { *batch.covered.entry("rpc-response".into()).or_default() += 1; return; }
        let p = &v["payload"];
        if kind == "compacted" {
            let id = self.compaction.take().unwrap_or_else(|| format!("compaction:{}", r.order));
            self.compaction = Some(id.clone());
            let body = Body::Compaction { trigger: None, tokens_before: None, tokens_after: p["latest_token_usage_record"]["thread_token_usage"]["total_tokens"].as_u64() };
            let step = self.new_step(id, body, Status::Completed, r);
            self.emit(step, batch);
            return;
        }
        if kind == "token_usage_record" {
            let turn = string(&p["turn_id"]).or_else(|| self.turn.as_ref().map(|t| t.id.clone())).unwrap_or_default();
            let total = p["thread_token_usage"]["total_tokens"].as_u64().unwrap_or(0);
            self.usage(format!("usage:{turn}:{total}"), Self::counters(&p["usage"]), r, batch);
            return;
        }
        if CODEX_METADATA.contains(&kind) {
            if kind == "session_meta" { self.codex_session(p); }
            if kind == "turn_context" { self.cwd = string(&p["cwd"]).or(self.cwd.take()); self.model = string(&p["model"]).or(self.model.take()); }
            *batch.covered.entry(kind.into()).or_default() += 1;
            return;
        }
        match kind {
            "event_msg" => self.codex_event(p, r, batch),
            "response_item" => self.codex_response(p, r, batch),
            _ => self.unknown(kind.into(), r.id(), r, batch),
        }
    }

    fn codex_session(&mut self, p: &Value) {
        let s = &mut self.session;
        s.id = string(&p["id"]);
        s.root = string(&p["session_id"]);
        s.cwd = string(&p["cwd"]).map(|c| local_path(&c));
        s.version = string(&p["cli_version"]);
        let spawn = &p["source"]["subagent"]["thread_spawn"];
        s.parent = string(&p["parent_thread_id"]).or_else(|| string(&spawn["parent_thread_id"]));
        if s.parent.is_some() { s.relation = Some("subagent".into()); s.unknown.insert("parent_step".into(), "not-reported".into()); }
        if let Some(from) = string(&p["forked_from_id"]) {
            let base = &p["history_base"];
            s.fork = Some(Fork { session: Some(from.clone()), end_byte: base["end_byte_offset"].as_u64(), end_ordinal: base["end_ordinal_exclusive"].as_u64().or(p["forked_from_ordinal_exclusive"].as_u64()) });
            if s.parent.is_none() { s.relation = Some("fork".into()); s.parent = Some(from); }
        }
    }

    fn codex_event(&mut self, p: &Value, r: &Record, batch: &mut Batch) {
        let kind = p["type"].as_str().unwrap_or("");
        match kind {
            "item_completed" | "item_started" => self.codex_item(&p["item"], "/payload/item", kind == "item_started", r, batch),
            "task_started" => {
                self.window = p["model_context_window"].as_u64().or(self.window);
                let id = string(&p["turn_id"]).unwrap_or_else(|| format!("turn:{}", r.order));
                self.start_turn(id, r, batch);
            }
            "task_complete" => self.close_turn(Some(Status::Completed), p["duration_ms"].as_u64(), r, batch),
            "turn_aborted" => self.close_turn(Some(Status::Cancelled), p["duration_ms"].as_u64(), r, batch),
            "token_count" => {
                let info = &p["info"];
                self.window = info["model_context_window"].as_u64().or(self.window);
                match (info["total_token_usage"]["total_tokens"].as_u64(), self.turn.as_ref().map(|t| t.id.clone())) {
                    (Some(total), Some(turn)) => self.usage(format!("usage:{turn}:{total}"), Self::counters(&info["last_token_usage"]), r, batch),
                    // A limit-only reading has no usage to show.
                    _ => *batch.covered.entry("token_count".into()).or_default() += 1,
                }
            }
            _ if CODEX_METADATA.contains(&kind) => *batch.covered.entry(kind.into()).or_default() += 1,
            _ => self.unknown(format!("event_msg:{kind}"), r.id(), r, batch),
        }
    }

    fn total(u: &Value) -> Option<u64> { first(u, &["total_tokens", "totalTokens"]).1.as_u64().or_else(|| Some(first(u, &["input_tokens", "inputTokens"]).1.as_u64()? + first(u, &["output_tokens", "outputTokens"]).1.as_u64()?)) }

    fn counters(u: &Value) -> [Option<u64>; 6] {
        let read = |keys: &[&'static str]| first(u, keys).1.as_u64();
        let cached = read(&["cached_input_tokens", "cachedInputTokens"]);
        // Codex input includes its cached tokens.
        let input = read(&["input_tokens", "inputTokens"]).map(|i| i.saturating_sub(cached.unwrap_or(0)));
        [input, cached, read(&["cache_write_input_tokens", "cacheWriteInputTokens"]), read(&["output_tokens", "outputTokens"]), read(&["reasoning_output_tokens", "reasoningOutputTokens"]), Self::total(u)]
    }

    fn codex_response(&mut self, p: &Value, r: &Record, batch: &mut Batch) {
        let kind = p["type"].as_str().unwrap_or("");
        let id = string(&p["id"]).unwrap_or_else(|| r.id());
        match kind {
            "message" | "agent_message" => {
                let role = if kind == "message" { string(&p["role"]) } else { string(&p["author"]) };
                let text = value_text(&p["content"]).unwrap_or_default();
                if role.as_deref() == Some("user") { self.last_user = Some((id.clone(), text)); }
                let body = Body::Message { role, text: r.output("/payload/content", &p["content"]) };
                let step = self.new_step(id, body, Status::Completed, r);
                self.emit(step, batch);
            }
            "reasoning" => {
                let encrypted = p["encrypted_content"].is_string();
                let mut step = self.new_step(id, Body::Thinking { text: r.output("/payload/summary", &p["summary"]), encrypted: Some(encrypted) }, Status::Completed, r);
                if encrypted { step.unknown.insert("text".into(), "encrypted".into()); }
                self.emit(step, batch);
            }
            "custom_tool_call" | "function_call" => {
                let call = string(&p["call_id"]).unwrap_or(id);
                let (key, input) = first(p, &["input", "arguments"]);
                let body = Body::Tool { name: string(&p["name"]), input: r.output(&format!("/payload/{key}"), input), output: None, duration_ms: None };
                let step = self.new_step(call, body, Status::Running, r);
                self.emit(step, batch);
            }
            "custom_tool_call_output" | "function_call_output" => {
                let call = string(&p["call_id"]).unwrap_or(id);
                let body = Body::Tool { name: None, input: None, output: r.output("/payload/output", &p["output"]), duration_ms: None };
                let step = self.new_step(call, body, Status::Completed, r);
                self.emit_result(step, r, batch);
            }
            _ => self.unknown(format!("response_item:{kind}"), id, r, batch),
        }
    }

    /// A result keeps the kind and start of its open call.
    fn emit_result(&mut self, mut step: Step, r: &Record, batch: &mut Batch) {
        if let Some(open) = self.open.get(&step.id) { step.started_ms = open.started_ms; }
        step.ended_ms = r.time;
        self.emit(step, batch);
    }

    fn codex_app(&mut self, method: &str, r: &Record, batch: &mut Batch) {
        let p = &r.v["params"];
        match method {
            "item/started" => self.codex_item(&p["item"], "/params/item", true, r, batch),
            "item/completed" => self.codex_item(&p["item"], "/params/item", false, r, batch),
            "item/commandExecution/outputDelta" => self.delta(p, r, batch),
            "turn/started" => {
                let id = string(&p["turn"]["id"]).unwrap_or_else(|| format!("turn:{}", r.order));
                self.start_turn(id, r, batch);
            }
            "turn/completed" => {
                let status = Status::native(p["turn"]["status"].as_str()).unwrap_or(Status::Unknown);
                self.close_turn(Some(status), p["turn"]["durationMs"].as_u64(), r, batch);
            }
            "thread/started" => {
                let t = &p["thread"];
                self.session.id = string(&t["id"]);
                self.session.root = string(&t["sessionId"]);
                self.session.parent = string(&t["parentThreadId"]).or_else(|| string(&t["forkedFromId"]));
                self.session.relation = if !t["parentThreadId"].is_null() { Some("subagent".into()) } else if !t["forkedFromId"].is_null() { Some("fork".into()) } else { None };
                *batch.covered.entry(method.into()).or_default() += 1;
            }
            "thread/tokenUsage/updated" => {
                let u = &p["tokenUsage"];
                self.window = u["modelContextWindow"].as_u64().or(self.window);
                let turn = string(&p["turnId"]).unwrap_or_default();
                match Self::total(&u["total"]) {
                    Some(total) => self.usage(format!("usage:{turn}:{total}"), Self::counters(&u["last"]), r, batch),
                    None => *batch.covered.entry(method.into()).or_default() += 1,
                }
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let body = Body::Approval { request: r.output("/params", p), decision: None };
                let step = self.new_step(format!("request:{}", r.v["id"]), body, Status::Running, r);
                self.emit(step, batch);
            }
            "item/tool/requestUserInput" => {
                let body = Body::Question { prompt: r.output("/params", p), answer: None };
                let step = self.new_step(format!("request:{}", r.v["id"]), body, Status::Running, r);
                self.emit(step, batch);
            }
            "serverRequest/resolved" => {
                let id = format!("request:{}", p["requestId"]);
                if let Some(mut step) = self.open.get(&id).cloned() {
                    step.status = Status::Completed; step.ended_ms = r.time; step.revision = r.revision;
                    self.emit(step, batch);
                }
            }
            _ if method.contains("Delta") || method.contains("delta") => *batch.covered.entry(method.into()).or_default() += 1,
            _ if method.starts_with("thread/") || method.starts_with("account/") || method.starts_with("remoteControl/") || method == "warning" || method == "deprecationNotice" || method == "error" => *batch.covered.entry(method.into()).or_default() += 1,
            _ => self.unknown(method.into(), r.id(), r, batch),
        }
    }

    /// Output that arrives before its item ends. The native item replaces it at the end.
    fn delta(&mut self, p: &Value, r: &Record, batch: &mut Batch) {
        let Some(id) = string(&p["itemId"]) else { return };
        let Some(delta) = p["delta"].as_str() else { return };
        let mut step = self.open.get(&id).cloned().unwrap_or_else(|| {
            let mut started = self.new_step(id, Body::Shell(Shell::default()), Status::Running, r);
            started.unknown.insert("started".into(), "delta-before-item".into());
            started
        });
        if let Body::Shell(shell) = &mut step.body {
            let previous = shell.output.as_ref().map_or(0, |o| o.bytes.unwrap_or(0));
            let mut text = shell.output.take().map_or_else(String::new, |o| o.text);
            text.push_str(delta);
            let (text, truncated) = bound(text);
            shell.output = Some(Output { text, truncated, bytes: Some(previous + delta.len() as u64), source: None });
        }
        step.revision = r.revision;
        step.source = r.source();
        // The delta record holds only its own text, so the running output names no source pointer.
        self.emit(step, batch);
    }

    fn codex_item(&mut self, item: &Value, base: &str, started: bool, r: &Record, batch: &mut Batch) {
        let native = item["type"].as_str().unwrap_or("").replace('_', "").to_ascii_lowercase();
        let id = string(&item["id"]).unwrap_or_else(|| r.id());
        let at = |key: &str| format!("{base}/{key}");
        let envelope = |keys: &[&'static str]| if base == "/payload/item" { first(&r.v["payload"], keys).1.as_i64() } else if base == "/params/item" { first(&r.v["params"], keys).1.as_i64() } else { None };
        let (from, to) = (envelope(&["started_at_ms", "startedAtMs"]), envelope(&["completed_at_ms", "completedAtMs"]));
        let turn = string(&r.v["payload"]["turn_id"]).or_else(|| string(&r.v["params"]["turnId"]));
        let done = |native_status: Option<&str>, default: Status| if started { Status::Running } else { Status::native(native_status).unwrap_or(default) };
        let status = item["status"].as_str();
        let (body, status) = match native.as_str() {
            "usermessage" | "agentmessage" => {
                let (key, value) = first(item, &["content", "text"]);
                let role = if native == "usermessage" { "user" } else { "assistant" };
                let text = value_text(value).unwrap_or_default();
                // The user's response item came first with the same text, so this revises it.
                if let Some((known, _)) = self.last_user.as_ref().filter(|(_, t)| native == "usermessage" && *t == text) {
                    let mut step = self.new_step(known.clone(), Body::Message { role: Some(role.into()), text: r.output(&at(key), value) }, done(None, Status::Completed), r);
                    step.turn = turn.or(step.turn);
                    return self.emit(step, batch);
                }
                (Body::Message { role: Some(role.into()), text: r.output(&at(key), value) }, done(None, Status::Completed))
            }
            "reasoning" => {
                let (key, value) = first(item, &["summary_text", "summary", "text"]);
                let content = first(item, &["raw_content", "content"]);
                let text = r.output(&at(key), value).or_else(|| r.output(&at(content.0), content.1));
                (Body::Thinking { text, encrypted: None }, done(None, Status::Completed))
            }
            "commandexecution" => {
                let (command, argv) = command(item);
                let (out_key, out) = first(item, &["aggregated_output", "aggregatedOutput"]);
                let (_, code) = first(item, &["exit_code", "exitCode"]);
                let output = if out.as_str() == Some("") && started { None } else { r.output(&at(out_key), out) };
                let shell = Shell { command, argv, cwd: string(&item["cwd"]).map(|c| local_path(&c)), exit_code: code.as_i64(), duration_ms: duration_ms(item), output, stdout: None, stderr: None, background: None, origin: string(&item["source"]) };
                (Body::Shell(shell), done(status, Status::Unknown))
            }
            "filechange" => {
                let files = match &item["changes"] {
                    Value::Object(map) => map.iter().map(|(path, change)| {
                        let pointer = format!("{base}/changes/{}", pointer_key(path));
                        let (key, diff) = first(change, &["unified_diff", "content"]);
                        let pointer = format!("{pointer}/{key}");
                        FileEdit { path: path.clone(), action: string(&change["type"]), diff: r.output(&pointer, diff) }
                    }).collect(),
                    Value::Array(list) => list.iter().enumerate().map(|(index, change)| {
                        let (key, diff) = first(change, &["diff", "unified_diff"]);
                        FileEdit { path: change["path"].as_str().unwrap_or_default().into(), action: string(&change["kind"]).or_else(|| string(&change["kind"]["type"])), diff: r.output(&format!("{base}/changes/{index}/{key}"), diff) }
                    }).collect(),
                    _ => vec![],
                };
                (Body::FileChange { files, output: r.output(&at("stdout"), &item["stdout"]), stderr: r.output(&at("stderr"), &item["stderr"]).filter(|o| !o.text.is_empty()) }, done(status, Status::Unknown))
            }
            "mcptoolcall" => {
                let (key, result) = first(item, &["result", "error"]);
                (Body::Mcp { server: string(&item["server"]), tool: string(&item["tool"]), input: r.output(&at("arguments"), &item["arguments"]), output: r.output(&at(key), result), duration_ms: duration_ms(item) }, done(status, Status::Unknown))
            }
            "dynamictoolcall" => {
                let (key, result) = first(item, &["contentItems", "result", "output"]);
                (Body::Tool { name: string(&item["tool"]), input: r.output(&at("arguments"), &item["arguments"]), output: r.output(&at(key), result), duration_ms: duration_ms(item) }, done(status, Status::Unknown))
            }
            "websearch" => (Body::Web { action: string(&item["action"]["type"]), query: string(&item["query"]), url: string(&item["action"]["url"]), output: None, duration_ms: duration_ms(item) }, done(status, Status::Completed)),
            "extension" if item["kind"] == "web.search" => (Body::Web { action: string(&item["action"]["type"]), query: string(&item["query"]), url: string(&item["action"]["url"]), output: r.output(&at("results"), &item["results"]), duration_ms: duration_ms(item) }, done(status, Status::Completed)),
            "extension" => (Body::Tool { name: string(&item["kind"]), input: r.output(&at("action"), &item["action"]), output: r.output(&at("results"), &item["results"]), duration_ms: duration_ms(item) }, done(status, Status::Completed)),
            "imageview" | "imagegeneration" => (Body::Image { path: string(&item["path"]).map(|p| local_path(&p)), media_type: None }, done(status, Status::Completed)),
            "collabagenttoolcall" | "collabtoolcall" => {
                let (_, receivers) = first(item, &["receiver_thread_ids", "receiverThreadIds"]);
                let sessions = receivers.as_array().map_or(vec![], |list| list.iter().filter_map(|s| s.as_str().map(String::from)).collect());
                (Body::Subagent { action: string(&item["tool"]), agent: None, sessions, input: r.output(&at("prompt"), &item["prompt"]), output: None }, done(status, Status::Unknown))
            }
            "subagentactivity" => {
                let sessions = string(&item["agent_thread_id"]).into_iter().collect();
                (Body::Subagent { action: string(&item["kind"]), agent: string(&item["agent_path"]), sessions, input: None, output: None }, done(status, Status::Completed))
            }
            "contextcompaction" => {
                let id = self.compaction.take().unwrap_or(id);
                self.compaction = Some(id.clone());
                let body = Body::Compaction { trigger: None, tokens_before: None, tokens_after: None };
                let mut step = self.new_step(id, body, done(status, Status::Completed), r);
                step.turn = turn.or(step.turn);
                return self.emit(step, batch);
            }
            other => return self.unknown(format!("item:{other}"), id, r, batch),
        };
        let mut step = self.new_step(id, body, status, r);
        step.turn = turn.or(step.turn);
        if from.is_some() { step.started_ms = from; }
        if !started && to.is_some() { step.ended_ms = to; }
        if matches!(step.body, Body::Shell(_)) && !started {
            step.unknown.insert("stdout".into(), "merged-output".into());
            step.unknown.insert("stderr".into(), "merged-output".into());
        }
        self.emit(step, batch);
    }

    // Claude: one reader for transcript and stream records.
    fn claude(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        let kind = v["type"].as_str().unwrap_or("");
        self.cwd = string(&v["cwd"]).or(self.cwd.take());
        self.claude_session(v);
        match kind {
            "user" => self.claude_user(r, batch),
            "assistant" => self.claude_assistant(r, batch),
            "attachment" => self.claude_attachment(r, batch),
            "system" => self.claude_system(r, batch),
            "result" => {
                let status = if v["is_error"] == true { Status::Failed } else { Status::Completed };
                self.close_turn(Some(status), v["duration_ms"].as_u64(), r, batch);
            }
            "queue-operation" => {
                let handled = v["content"].as_str().and_then(notification).map(|fields| self.claude_notification(&fields, r, batch)).is_some();
                if !handled { *batch.covered.entry(kind.into()).or_default() += 1; }
            }
            "control_request" => {
                let request = &v["request"];
                let id = format!("request:{}", v["request_id"].as_str().unwrap_or_default());
                if request["subtype"] == "can_use_tool" {
                    let body = Body::Approval { request: r.output("/request", request), decision: None };
                    let step = self.new_step(id, body, Status::Running, r);
                    self.emit(step, batch);
                } else { *batch.covered.entry(kind.into()).or_default() += 1; }
            }
            "control_response" => {
                let response = &v["response"];
                let id = format!("request:{}", response["request_id"].as_str().unwrap_or_default());
                if self.open.contains_key(&id) {
                    let body = Body::Approval { request: None, decision: string(&response["response"]["behavior"]).or_else(|| string(&response["subtype"])) };
                    let step = self.new_step(id, body, Status::Completed, r);
                    self.emit_result(step, r, batch);
                } else { *batch.covered.entry(kind.into()).or_default() += 1; }
            }
            _ if CLAUDE_METADATA.contains(&kind) => *batch.covered.entry(kind.into()).or_default() += 1,
            _ => self.unknown(kind.into(), string(&v["uuid"]).unwrap_or_else(|| r.id()), r, batch),
        }
    }

    fn claude_session(&mut self, v: &Value) {
        let s = &mut self.session;
        if s.cwd.is_none() { s.cwd = string(&v["cwd"]); }
        if let Some(version) = string(&v["version"]) { s.version = Some(version); }
        let session = string(&v["sessionId"]).or_else(|| string(&v["session_id"]));
        if let Some(agent) = string(&v["agentId"]) {
            s.id = Some(format!("agent-{agent}"));
            s.parent = session.clone();
            s.root = session;
            s.relation = Some("subagent".into());
            s.unknown.insert("parent_step".into(), "not-reported".into());
        } else if s.id.is_none() { s.id = session.clone(); s.root = session; }
    }

    fn claude_user(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        if let Some(prompt) = string(&v["promptId"]) { self.start_turn(prompt, r, batch); }
        let content = &v["message"]["content"];
        let uuid = string(&v["uuid"]).unwrap_or_else(|| r.id());
        if let Some(text) = content.as_str() {
            if let Some(fields) = notification(text) { return self.claude_notification(&fields, r, batch); }
            let body = Body::Message { role: Some("user".into()), text: r.output("/message/content", content) };
            let step = self.new_step(uuid, body, Status::Completed, r);
            return self.emit(step, batch);
        }
        let Some(blocks) = content.as_array() else { return self.unknown("user".into(), uuid, r, batch) };
        let results = blocks.iter().filter(|b| b["type"] == "tool_result").count();
        for (index, block) in blocks.iter().enumerate() {
            let id = format!("{uuid}:{index}");
            let at = format!("/message/content/{index}");
            let r = &Record { order: r.order + index as u64, ..*r };
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(fields) = block["text"].as_str().and_then(notification) { self.claude_notification(&fields, r, batch); continue; }
                    let body = Body::Message { role: Some("user".into()), text: r.output(&format!("{at}/text"), &block["text"]) };
                    let step = self.new_step(id, body, Status::Completed, r);
                    self.emit(step, batch);
                }
                Some("image") => {
                    let body = Body::Image { path: None, media_type: string(&block["source"]["media_type"]) };
                    let mut step = self.new_step(id, body, Status::Completed, r);
                    step.unknown.insert("path".into(), "inline-data".into());
                    self.emit(step, batch);
                }
                Some("tool_result") => self.claude_result(block, &at, results == 1, r, batch),
                other => self.unknown(format!("user:{}", other.unwrap_or("")), id, r, batch),
            }
        }
    }

    fn claude_assistant(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        let message = &v["message"];
        self.model = string(&message["model"]).or(self.model.take());
        let uuid = string(&v["uuid"]).or_else(|| string(&message["id"])).unwrap_or_else(|| r.id());
        let blocks = message["content"].as_array().map_or(&[][..], Vec::as_slice);
        for (index, block) in blocks.iter().enumerate() {
            let id = format!("{uuid}:{index}");
            let at = format!("/message/content/{index}");
            let r = &Record { order: r.order + index as u64, ..*r };
            match block["type"].as_str() {
                Some("text") => {
                    let failed = v["isApiErrorMessage"] == true || v["error"].is_string();
                    let body = Body::Message { role: Some("assistant".into()), text: r.output(&format!("{at}/text"), &block["text"]) };
                    let step = self.new_step(id, body, if failed { Status::Failed } else { Status::Completed }, r);
                    self.emit(step, batch);
                }
                Some("thinking") => {
                    let text = r.output(&format!("{at}/thinking"), &block["thinking"]).filter(|o| !o.text.is_empty());
                    let mut step = self.new_step(id, Body::Thinking { text, encrypted: block["signature"].is_string().then_some(true) }, Status::Completed, r);
                    if block["thinking"].as_str() == Some("") { step.unknown.insert("text".into(), "not-retained".into()); }
                    self.emit(step, batch);
                }
                Some("tool_use") => self.claude_tool(block, &at, r, batch),
                other => self.unknown(format!("assistant:{}", other.unwrap_or("")), id, r, batch),
            }
        }
        let u = &message["usage"];
        if let (true, Some(msg)) = (u.is_object(), string(&message["id"])) {
            let (input, cache_read, cache_write, output) = (u["input_tokens"].as_u64(), u["cache_read_input_tokens"].as_u64(), u["cache_creation_input_tokens"].as_u64(), u["output_tokens"].as_u64());
            let total = input.and_then(|i| Some(i + cache_read? + cache_write? + output?));
            let r = &Record { order: r.order + blocks.len() as u64, ..*r };
            self.usage(format!("usage:{msg}"), [input, cache_read, cache_write, output, None, total], r, batch);
        }
    }

    fn claude_tool(&mut self, block: &Value, at: &str, r: &Record, batch: &mut Batch) {
        let Some(id) = string(&block["id"]) else { return };
        let name = block["name"].as_str().unwrap_or("");
        let input = &block["input"];
        let input_at = format!("{at}/input");
        let body = match name {
            "Bash" | "PowerShell" | "Monitor" => Body::Shell(Shell { command: string(&input["command"]), cwd: self.cwd.clone(), ..Shell::default() }),
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Body::FileChange {
                files: string(&input["file_path"]).or_else(|| string(&input["notebook_path"])).map(|path| vec![FileEdit { path, action: Some(name.to_ascii_lowercase()), diff: None }]).unwrap_or_default(),
                output: None, stderr: None,
            },
            "Agent" | "Task" => Body::Subagent { action: Some(name.to_ascii_lowercase()), agent: string(&input["subagent_type"]), sessions: vec![], input: r.output(&format!("{input_at}/prompt"), &input["prompt"]), output: None },
            "WebSearch" | "WebFetch" => Body::Web { action: Some(name.to_ascii_lowercase()), query: string(&input["query"]).or_else(|| string(&input["prompt"])), url: string(&input["url"]), output: None, duration_ms: None },
            "AskUserQuestion" => Body::Question { prompt: r.output(&input_at, input), answer: None },
            _ if name.starts_with("mcp__") => {
                let mut parts = name.splitn(3, "__").skip(1);
                Body::Mcp { server: parts.next().map(String::from), tool: parts.next().map(String::from), input: r.output(&input_at, input), output: None, duration_ms: None }
            }
            _ => Body::Tool { name: Some(name.into()), input: r.output(&input_at, input), output: None, duration_ms: None },
        };
        self.tools.insert(id.clone(), Tool { name: name.into(), input: json!({"run_in_background": input["run_in_background"]}), at: r.time });
        let step = self.new_step(id, body, Status::Running, r);
        self.emit(step, batch);
    }

    fn claude_result(&mut self, block: &Value, at: &str, only: bool, r: &Record, batch: &mut Batch) {
        let Some(id) = string(&block["tool_use_id"]) else { return };
        let native = r.v.get("toolUseResult").or_else(|| r.v.get("tool_use_result")).filter(|_| only).unwrap_or(&Value::Null);
        let pointer = if r.v.get("toolUseResult").is_some() { "/toolUseResult" } else { "/tool_use_result" };
        let tool = self.tools.remove(&id);
        let name = tool.as_ref().map_or("", |t| t.name.as_str());
        let content = r.output(&format!("{at}/content"), &block["content"]);
        let failed = block["is_error"] == true;
        let interrupted = native["interrupted"] == true;
        let mut status = if interrupted { Status::Cancelled } else if failed { Status::Failed } else { Status::Completed };
        let elapsed = tool.as_ref().and_then(|t| Some((r.time? - t.at?).max(0) as u64));
        let mut unknown = BTreeMap::new();
        let body = match name {
            "Bash" | "PowerShell" | "Monitor" => {
                let text = content.as_ref().map_or("", |o| o.text.as_str());
                let exit_code = if failed { failed_exit(text) } else { None };
                let background = name == "Monitor" || tool.as_ref().is_some_and(|t| t.input["run_in_background"] == true);
                let task = (!failed).then(|| events::started_task(background, native, &block["content"])).flatten();
                if native["persistedOutputPath"].is_string() { unknown.insert("output_full".into(), "persisted-outside-transcript".into()); }
                let background = task.map(|task| Background { task: Some(task), state: Some("running".into()), output_file: None });
                if background.is_some() { status = Status::Running; }
                // A job's start result says nothing about how long the job runs; its notification does.
                let elapsed = if background.is_some() { None } else { elapsed };
                let mut output = content.clone();
                if let (Some(o), true) = (output.as_mut(), native["persistedOutputPath"].is_string()) { o.truncated = true; o.bytes = native["persistedOutputSize"].as_u64(); }
                Some(Body::Shell(Shell { exit_code, duration_ms: elapsed, output, stdout: r.output(&format!("{pointer}/stdout"), &native["stdout"]), stderr: r.output(&format!("{pointer}/stderr"), &native["stderr"]), background, ..Shell::default() }))
            }
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                let action = if name == "Write" { string(&native["type"]) } else { Some(name.to_ascii_lowercase()) };
                let files = string(&native["filePath"]).map(|path| vec![FileEdit { path, action, diff: r.output(&format!("{pointer}/structuredPatch"), &native["structuredPatch"]) }]).unwrap_or_default();
                Some(Body::FileChange { files, output: content.clone(), stderr: None })
            }
            "Agent" | "Task" => {
                let sessions = string(&native["agentId"]).map(|a| vec![format!("agent-{a}")]).unwrap_or_default();
                if let Some(state) = native["status"].as_str() { status = Status::native(Some(state)).unwrap_or(status); }
                Some(Body::Subagent { action: Some(name.to_ascii_lowercase()), agent: string(&native["agentType"]), sessions, input: None, output: r.output(&format!("{pointer}/content"), &native["content"]).or_else(|| content.clone()) })
            }
            "WebSearch" | "WebFetch" => Some(Body::Web { action: Some(name.to_ascii_lowercase()), query: None, url: None, output: content.clone(), duration_ms: elapsed }),
            "AskUserQuestion" => Some(Body::Question { prompt: None, answer: content.clone() }),
            _ if name.starts_with("mcp__") => Some(Body::Mcp { server: None, tool: None, input: None, output: content.clone(), duration_ms: elapsed }),
            _ => None,
        };
        let body = body.unwrap_or(Body::Tool { name: (!name.is_empty()).then(|| name.into()), input: None, output: content, duration_ms: elapsed });
        let mut step = self.new_step(id, body, status, r);
        step.unknown.extend(unknown);
        if status == Status::Running { step.ended_ms = None; }
        if let Some(open) = self.open.get(&step.id) { step.started_ms = open.started_ms; }
        self.emit(step, batch);
    }

    /// A native notification revises the call that started the job; a job the read has not seen revises by id alone.
    fn claude_notification(&mut self, fields: &BTreeMap<&'static str, String>, r: &Record, batch: &mut Batch) {
        let Some(id) = fields.get("tool-use-id").cloned() else { *batch.covered.entry("queue-operation".into()).or_default() += 1; return };
        let state = fields.get("status").cloned();
        let status = Status::native(state.as_deref()).unwrap_or(Status::Unknown);
        let background = Background { task: fields.get("task-id").cloned(), state, output_file: fields.get("output-file").cloned() };
        let exit_code = fields.get("summary").and_then(|s| leading_number(s.split_once("(exit code ")?.1));
        let mut step = self.open.get(&id).cloned().unwrap_or_else(|| self.new_step(id, Body::tool(), status, r));
        // A job whose start this read has not seen, or a subagent, revises only status and time.
        if let Body::Shell(shell) = &mut step.body {
            shell.background = Some(background);
            if exit_code.is_some() { shell.exit_code = exit_code; }
            if status.terminal() { shell.duration_ms = step.started_ms.zip(r.time).map(|(from, to)| (to - from).max(0) as u64); }
        }
        step.status = status;
        step.ended_ms = status.terminal().then_some(r.time).flatten();
        step.revision = r.revision;
        step.source = r.source();
        self.emit(step, batch);
    }

    fn claude_attachment(&mut self, r: &Record, batch: &mut Batch) {
        let a = &r.v["attachment"];
        let kind = a["type"].as_str().unwrap_or("");
        let id = string(&r.v["uuid"]).unwrap_or_else(|| r.id());
        match kind {
            "hook_success" | "hook_non_blocking_error" | "hook_cancelled" => {
                let status = match kind { "hook_success" => Status::Completed, "hook_cancelled" => Status::Cancelled, _ => Status::Failed };
                let shell = Shell { command: string(&a["command"]), cwd: self.cwd.clone(), exit_code: a["exitCode"].as_i64(), duration_ms: a["durationMs"].as_u64(), stdout: r.output("/attachment/stdout", &a["stdout"]), stderr: r.output("/attachment/stderr", &a["stderr"]), output: r.output("/attachment/content", &a["content"]), origin: Some("hook".into()), ..Shell::default() };
                let step = self.new_step(id, Body::Shell(shell), status, r);
                self.emit(step, batch);
            }
            "edited_text_file" => {
                let files = string(&a["filename"]).map(|path| vec![FileEdit { path, action: Some("external".into()), diff: r.output("/attachment/snippet", &a["snippet"]) }]).unwrap_or_default();
                let step = self.new_step(id, Body::FileChange { files, output: None, stderr: None }, Status::Completed, r);
                self.emit(step, batch);
            }
            "file" => {
                let body = Body::Tool { name: Some("attachment".into()), input: r.output("/attachment/filename", &a["filename"]), output: r.output("/attachment/content/file/content", &a["content"]["file"]["content"]), duration_ms: None };
                let step = self.new_step(id, body, Status::Completed, r);
                self.emit(step, batch);
            }
            "task_status" => {
                let tool = string(&a["shell"]["toolUseId"]).unwrap_or_else(|| format!("task:{}", a["taskId"].as_str().unwrap_or_default()));
                let state = string(&a["status"]);
                let status = Status::native(state.as_deref()).unwrap_or(Status::Unknown);
                let shell = Shell { command: string(&a["shell"]["command"]), cwd: None, background: Some(Background { task: string(&a["taskId"]), state, output_file: string(&a["outputFilePath"]) }), ..Shell::default() };
                let step = self.new_step(tool, Body::Shell(shell), status, r);
                self.emit(step, batch);
            }
            "queued_command" => {
                match a["prompt"].as_str().and_then(notification) {
                    Some(fields) => self.claude_notification(&fields, r, batch),
                    None => {
                        let body = Body::Message { role: Some("user".into()), text: r.output("/attachment/prompt", &a["prompt"]) };
                        let step = self.new_step(id, body, Status::Completed, r);
                        self.emit(step, batch);
                    }
                }
            }
            _ if CLAUDE_CONTEXT.contains(&kind) => *batch.covered.entry(format!("attachment:{kind}")).or_default() += 1,
            _ => self.unknown(format!("attachment:{kind}"), id, r, batch),
        }
    }

    fn claude_system(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        let kind = v["subtype"].as_str().unwrap_or("");
        let id = string(&v["uuid"]).unwrap_or_else(|| r.id());
        match kind {
            "init" => {
                self.session.version = string(&v["claude_code_version"]).or(self.session.version.take());
                self.start_turn(format!("init:{id}"), r, batch);
                *batch.covered.entry("system:init".into()).or_default() += 1;
            }
            "turn_duration" => self.close_turn(Some(Status::Unknown), v["durationMs"].as_u64(), r, batch),
            "compact_boundary" => {
                let m = &v["compactMetadata"];
                let body = Body::Compaction { trigger: string(&m["trigger"]), tokens_before: m["preTokens"].as_u64(), tokens_after: m["postTokens"].as_u64() };
                let step = self.new_step(id, body, Status::Completed, r);
                self.emit(step, batch);
            }
            "task_notification" => {
                let mut fields = BTreeMap::new();
                for (tag, key) in [("task-id", "task_id"), ("tool-use-id", "tool_use_id"), ("output-file", "output_file"), ("status", "status"), ("summary", "summary")] {
                    if let Some(value) = string(&v[key]) { fields.insert(tag, value); }
                }
                self.claude_notification(&fields, r, batch);
            }
            _ if CLAUDE_SYSTEM.contains(&kind) => *batch.covered.entry(format!("system:{kind}")).or_default() += 1,
            _ => self.unknown(format!("system:{kind}"), id, r, batch),
        }
    }

    // Antigravity: `agy` stream and sidecar transcript.
    fn antigravity(&mut self, r: &Record, batch: &mut Batch) {
        let v = r.v;
        if let Some(session) = string(&v["conversation_id"]).or_else(|| string(&v["step_update"]["conversation_id"])).or_else(|| string(&v["result"]["conversation_id"])) { self.session.id = Some(session); }
        match v["event"].as_str() {
            Some("init") => {
                self.session.cwd = string(&v["init"]["cwd"]).or(self.session.cwd.take());
                self.cwd = self.session.cwd.clone();
                self.start_turn(format!("turn:{}", r.order), r, batch);
                *batch.covered.entry("init".into()).or_default() += 1;
                return;
            }
            Some("result") => {
                let status = Status::native(v["result"]["status"].as_str()).unwrap_or(Status::Unknown);
                let duration = v["result"]["duration_seconds"].as_f64().map(|s| (s * 1000.0) as u64);
                return self.close_turn(Some(status), duration, r, batch);
            }
            Some("step_update") => return self.agy_step(&v["step_update"], r, batch),
            _ => {}
        }
        match v["type"].as_str() {
            Some(kind @ ("USER_INPUT" | "PLANNER_RESPONSE" | "SYSTEM_MESSAGE" | "TOOL_RESPONSE")) => self.sidecar(kind, r, batch),
            _ => {
                let record = v["type"].as_str().or(v["event"].as_str()).unwrap_or("").to_string();
                let id = v["step_index"].as_u64().map_or_else(|| r.id(), |i| format!("step:{i}"));
                self.unknown(record, id, r, batch);
            }
        }
    }

    fn agy_tool_body(name: Option<&str>, input: &Value, at: &str, r: &Record, cwd: Option<String>) -> Body {
        if name == Some("run_command") {
            let shell = Shell { command: string(&input["CommandLine"]), cwd: string(&input["Cwd"]).or(cwd), ..Shell::default() };
            return Body::Shell(shell);
        }
        Body::Tool { name: name.map(String::from), input: r.output(at, input), output: None, duration_ms: None }
    }

    fn agy_step(&mut self, s: &Value, r: &Record, batch: &mut Batch) {
        let Some(index) = s["step_index"].as_u64() else { return self.unknown("step_update".into(), r.id(), r, batch) };
        let id = format!("step:{index}");
        let status = Status::native(s["state"].as_str()).unwrap_or(Status::Unknown);
        let duration = s["duration_seconds"].as_f64().map(|d| (d * 1000.0) as u64);
        match s["step_type"].as_str() {
            Some(kind @ ("user_input" | "agent_response" | "system_message")) => {
                let role = match kind { "user_input" => "user", "agent_response" => "assistant", _ => "system" };
                let known = self.open.get(&id).and_then(|o| match &o.body { Body::Message { text, .. } => text.clone(), _ => None });
                let text = match s["text_delta"].as_str() {
                    Some(delta) => {
                        let previous = known.as_ref().map_or(0, |o| o.bytes.unwrap_or(0));
                        let (text, truncated) = bound(format!("{}{delta}", known.map_or_else(String::new, |o| o.text)));
                        Some(Output { text, truncated, bytes: Some(previous + delta.len() as u64), source: None })
                    }
                    None => known,
                };
                let mut step = self.new_step(id, Body::Message { role: Some(role.into()), text: text.clone() }, status, r);
                if text.is_none() { step.unknown.insert("text".into(), "text-not-emitted".into()); }
                self.emit(step, batch);
            }
            Some("tool") => {
                let info = &s["tool_info"];
                let name = s["tool_name"].as_str().or(info["name"].as_str());
                let mut body = Self::agy_tool_body(name, &info["parameters"], "/step_update/tool_info/parameters", r, self.cwd.clone());
                let (key, out) = first(info, &["output", "error"]);
                let output = r.output(&format!("/step_update/tool_info/{key}"), out);
                match &mut body {
                    Body::Shell(shell) => { shell.output = output; shell.duration_ms = duration; }
                    Body::Tool { output: slot, duration_ms, .. } => { *slot = output; *duration_ms = duration; }
                    _ => {}
                }
                let step = self.new_step(id, body, status, r);
                self.emit(step, batch);
            }
            other => self.unknown(format!("step_update:{}", other.unwrap_or("")), id, r, batch),
        }
        let u = &s["usage"];
        if u.is_object() && status.terminal() {
            let total = first(u, &["total_tokens"]).1.as_u64();
            self.usage(format!("usage:{index}"), [u["input_tokens"].as_u64(), u["cache_read_tokens"].as_u64(), u["cache_write_tokens"].as_u64(), u["output_tokens"].as_u64(), u["thinking_tokens"].as_u64(), total], r, batch);
        }
    }

    fn sidecar(&mut self, kind: &str, r: &Record, batch: &mut Batch) {
        let v = r.v;
        let index = v["step_index"].as_u64().unwrap_or(r.order);
        let id = format!("step:{index}");
        let status = Status::native(v["status"].as_str()).unwrap_or(Status::Unknown);
        let truncated = v["truncated_fields"].as_array().is_some_and(|f| !f.is_empty());
        let mut text = r.output("/content", &v["content"]);
        if let (Some(t), true) = (text.as_mut(), truncated) { t.truncated = true; }
        if kind == "USER_INPUT" { self.start_turn(format!("turn:{index}"), r, batch); }
        if kind == "TOOL_RESPONSE" {
            let call = string(&v["tool_call_id"]).unwrap_or(id);
            let body = match self.open.get(&call).map(|s| &s.body) {
                Some(Body::Shell(_)) => Body::Shell(Shell { output: text, ..Shell::default() }),
                _ => Body::Tool { name: None, input: None, output: text, duration_ms: None },
            };
            let step = self.new_step(call, body, status, r);
            return self.emit_result(step, r, batch);
        }
        let role = match kind { "USER_INPUT" => "user", "SYSTEM_MESSAGE" => "system", _ => "assistant" };
        let calls = v["tool_calls"].as_array().map_or(&[][..], Vec::as_slice);
        if text.is_some() || calls.is_empty() {
            let mut step = self.new_step(id.clone(), Body::Message { role: Some(role.into()), text: text.clone() }, status, r);
            if truncated { step.unknown.insert("text_full".into(), "truncated-source".into()); }
            if text.is_none() { step.unknown.insert("text".into(), "text-not-emitted".into()); }
            self.emit(step, batch);
        }
        for (position, call) in calls.iter().enumerate() {
            let name = call["name"].as_str();
            let (key, input) = first(call, &["args", "arguments"]);
            let body = Self::agy_tool_body(name, input, &format!("/tool_calls/{position}/{key}"), r, self.cwd.clone());
            let call_id = string(&call["id"]).unwrap_or_else(|| format!("{id}:{position}"));
            let step = self.new_step(call_id, body, Status::Running, &Record { order: r.order + 1 + position as u64, ..*r });
            self.emit(step, batch);
        }
        if kind == "PLANNER_RESPONSE" {
            if v["input_tokens"].is_u64() || v["output_tokens"].is_u64() {
                self.model = string(&v["model"]).or(self.model.take());
                let r = &Record { order: r.order + 1 + calls.len() as u64, ..*r };
                self.usage(format!("usage:{index}"), [v["input_tokens"].as_u64(), v["cache_read_tokens"].as_u64(), v["cache_write_tokens"].as_u64(), v["output_tokens"].as_u64(), None, None], r, batch);
            }
            if super::ends(v) { self.close_turn(Some(status), None, r, batch); }
        }
    }
}

/// One page of a session. `checkpoint` resumes it; save it only after accepting the page.
#[derive(Debug)]
pub struct StepPage {
    pub format: String,
    pub session: Session,
    pub turns: Vec<Turn>,
    pub steps: Vec<Step>,
    pub covered: BTreeMap<String, u64>,
    pub more: bool,
    pub pending_bytes: usize,
    pub checkpoint: Value,
}

pub(super) fn page(client: Client, format: &str, session: &str, path: &Path, saved: Option<&Value>, until: Option<u64>) -> Res<StepPage> {
    let (mut history, mut reducer) = match saved {
        Some(saved) => {
            if saved["schema"] != 1 { return Err("history checkpoint schema invalid".into()); }
            let history = History::recover(&saved["history"])?;
            let bound = history.checkpoint();
            if bound["format"] != format || bound["session"] != session { return Err("history checkpoint names another session".into()); }
            (history, Reducer::restore(&saved["steps"], client)?)
        }
        None => (History::new(format, "agentq", session)?, Reducer::new(client)),
    };
    let read = history.poll(path)?;
    let mut batch = Batch::default();
    let mut ended = false;
    for event in &read.events {
        let (start, end) = (event["start"].as_u64().unwrap_or(0), event["end"].as_u64().unwrap_or(0));
        if until.is_some_and(|limit| end > limit) { ended = true; break; }
        let raw = event["raw"].as_str().unwrap_or_default();
        let value: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
        reducer.read(&value, raw, start, end, &mut batch);
    }
    let at_limit = until.is_some_and(|limit| history.checkpoint()["offset"].as_u64().unwrap_or(0) >= limit);
    let more = read.more && !ended && !at_limit;
    let checkpoint = json!({"schema": 1, "history": history.checkpoint(), "steps": serde_json::to_value(&reducer).map_err(|e| e.to_string())?});
    Ok(StepPage { format: format.into(), session: reducer.session.clone(), turns: batch.turns, steps: batch.steps, covered: batch.covered, more, pending_bytes: read.pending_bytes, checkpoint })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(steps: Vec<Step>) -> Vec<Value> {
        steps.into_iter().map(|step| {
            let mut value = serde_json::to_value(step).unwrap();
            let map = value.as_object_mut().unwrap();
            for key in ["order", "revision", "source", "unknown"] { map.remove(key); }
            for (_, field) in map.iter_mut() { if let Some(output) = field.as_object_mut() { output.remove("source"); } }
            value
        }).collect()
    }

    // #282 A4 defect: a live stream and a finished source read different shapes. Risk: a client renders two formats.
    #[test]
    fn finished_and_live_share_one_step_shape() {
        for (client, log) in [
            (Client::Codex, include_str!("../../tests/fixtures/steps/codex-0.162.1-app-server.jsonl")),
            (Client::Claude, include_str!("../../tests/fixtures/steps/claude-2.1.295.jsonl")),
        ] {
            let (mut file, mut live) = (Reducer::new(client), Reducer::new(client));
            let (mut from_file, mut from_live) = (Batch::default(), Batch::default());
            let mut start = 0;
            for raw in log.split_inclusive('\n') {
                let value: Value = serde_json::from_str(raw).unwrap();
                file.read(&value, raw, start, start + raw.len() as u64, &mut from_file);
                live.read_live(&value, &mut from_live);
                start += raw.len() as u64;
            }
            let strip_raw = |steps: Vec<Value>| steps.into_iter().map(|mut s| { s.as_object_mut().unwrap().remove("raw"); s }).collect::<Vec<_>>();
            assert_eq!(strip_raw(shape(from_file.steps)), strip_raw(shape(from_live.steps)));
            assert_eq!(from_file.turns.len(), from_live.turns.len());
        }
    }

    #[test]
    fn a_turn_without_a_terminal_record_reads_unknown() {
        let mut reducer = Reducer::new(Client::Codex);
        let mut batch = Batch::default();
        let rows = [
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"one"}}),
            json!({"type":"response_item","payload":{"type":"custom_tool_call","call_id":"c","name":"exec","input":"x"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"two"}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"two","duration_ms":5}}),
        ];
        for (index, row) in rows.iter().enumerate() { reducer.read(row, &row.to_string(), index as u64, index as u64 + 1, &mut batch); }
        assert_eq!(batch.turns[0].status, Status::Unknown, "no terminal record is never completion");
        assert_eq!((batch.turns[1].status, batch.turns[1].duration_ms), (Status::Completed, Some(5)));
        let tool = batch.steps.iter().find(|s| s.id == "c").unwrap();
        assert_eq!((tool.status, tool.unknown["ended_ms"].as_str()), (Status::Unknown, "no-terminal-record"));
        assert!(reducer.open.is_empty(), "a checkpoint keeps only open work");
    }

    #[test]
    fn a_checkpoint_keeps_open_steps_across_pages() {
        let mut reducer = Reducer::new(Client::Claude);
        let mut batch = Batch::default();
        let call = json!({"type":"assistant","uuid":"a","cwd":"/w","timestamp":"2000-01-01T00:00:00.000Z","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}]}});
        reducer.read(&call, &call.to_string(), 0, 10, &mut batch);
        let saved = serde_json::to_value(&reducer).unwrap();
        let mut restored = Reducer::restore(&saved, Client::Claude).unwrap();
        assert!(Reducer::restore(&saved, Client::Codex).is_err());
        let mut batch = Batch::default();
        let result = json!({"type":"user","uuid":"b","timestamp":"2000-01-01T00:00:02.000Z","toolUseResult":{"stdout":"out","stderr":"err"},"message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"out"}]}});
        restored.read(&result, &result.to_string(), 10, 20, &mut batch);
        let Body::Shell(shell) = &batch.steps[0].body else { panic!() };
        assert_eq!((shell.command.as_deref(), shell.cwd.as_deref(), shell.duration_ms), (Some("ls"), Some("/w"), Some(2000)));
        assert_eq!((shell.stdout.as_ref().unwrap().text.as_str(), shell.stderr.as_ref().unwrap().text.as_str()), ("out", "err"));
    }
}
