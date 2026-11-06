//! Persistent native sessions; submission, accepted input, terminal evidence and process exit stay separate.
use super::*;

/// Claude assigns the job/session on a start. Resume never overrides its saved settings or permissions.
pub(crate) fn background_command(mut base: Command, spec: &Spec) -> Res<Command> {
    if spec.client != Client::Claude { return Err("background requires Claude".into()); }
    super::prepare::refuse_bypass(&spec.args)?;
    base.arg("--bg");
    if spec.resume {
        if !plain_id(&spec.session) { return Err("background session invalid".into()); }
        base.args(["--resume", &spec.session]);
    } else { base.args(&spec.args); }
    Ok(environment(base, spec))
}

/// Only the launcher exchange. Any uncertain response requires observation of the original submission, never replay.
/// `reference` points to caller-protected, durably bound input: Claude forwards stdin to child argv.
pub(crate) fn background_launch(base: Command, spec: &Spec, reference: &str, budget: Duration) -> Res<Result<std::process::Output, crate::process::Failure>> {
    Ok(crate::process::exchange("agent.background-launch", &mut background_command(base, spec)?, reference.as_bytes(), budget))
}

/// The job ID from the launcher's first line `backgrounded · <job>[ · <name>]`; the command hints after it are not IDs.
pub(crate) fn background_job_id(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?;
    line.split('·').nth(1).map(str::trim).filter(|t| plain_id(t)).map(String::from)
}

/// `filtered` adds `--cwd <root>`; Claude's filter misses roots under `.claude/worktrees` (#222), so callers retry
/// unfiltered and `BackgroundJob::read` still binds the exact root.
pub(crate) fn background_jobs(mut base: Command, spec: &Spec, filtered: bool, budget: Duration) -> Res<Vec<Value>> {
    if spec.client != Client::Claude { return Err("background requires Claude".into()); }
    base.args(["agents", "--json", "--all"]);
    if filtered { base.arg("--cwd").arg(&spec.root); }
    let output = crate::process::output("agent.background-jobs", &mut environment(base, spec), budget).map_err(|e| e.to_string())?;
    serde_json::from_slice::<Vec<Value>>(&output.stdout).map_err(|e| format!("background jobs invalid: {e}"))
}

/// Validated native readiness. This is neither accepted input nor an execution grant.
#[derive(Clone, Debug)]
pub struct BackgroundJob {
    pub id: String,
    pub session: String,
    pub pid: Option<u32>,
    root: PathBuf,
    scope: String,
}

impl BackgroundJob {
    pub(crate) fn read(row: &Value, spec: &Spec, job: &str) -> Res<Self> {
        if spec.client != Client::Claude || !plain_id(job) || row["id"] != job || row["kind"] != "background" {
            return Err("background job identity differs".into());
        }
        let root = spec.root.canonicalize().map_err(|e| e.to_string())?;
        if row["cwd"].as_str().and_then(|p| Path::new(p).canonicalize().ok()).as_ref() != Some(&root) {
            return Err("background project differs".into());
        }
        let session = row["sessionId"].as_str().filter(|s| plain_id(s)).ok_or("background session unknown")?;
        if spec.resume && session != spec.session { return Err("background resumed another session".into()); }
        row["state"].as_str().filter(|s| matches!(*s, "working" | "idle" | "done" | "blocked" | "failed" | "crashed" | "stopped"))
            .ok_or("background state unknown")?;
        let pid = if row["pid"].is_null() { None } else { Some(row["pid"].as_u64().and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0).ok_or("background pid invalid")?) };
        Ok(Self { id: job.into(), session: session.into(), pid, root, scope:scope(spec) })
    }
}

/// Caller must already own this execution. Acknowledges only a stop request; inspect jobs/process identity for exit.
pub(crate) fn background_stop(mut base: Command, spec: &Spec, job: &BackgroundJob, budget: Duration) -> Res<std::process::Output> {
    if spec.client != Client::Claude || spec.root.canonicalize().ok().as_ref() != Some(&job.root) || scope(spec)!=job.scope
        || (spec.resume && spec.session != job.session) { return Err("background stop scope differs".into()); }
    base.args(["stop", &job.id]);
    crate::process::output("agent.background-stop", &mut environment(base, spec), budget).map_err(|e| e.to_string())
}

/// The caller owns app presence/UI and a durable submission binding. A failed response never permits replay.
pub(crate) fn app_queue(mut base: Command, spec: &Spec, reference: &str, budget: Duration) -> Res<std::process::Output> {
    if spec.client != Client::Codex || !plain_id(&spec.session) || spec.session == "-" { return Err("app thread invalid".into()); }
    base.args(["queue", "--thread", &spec.session, "--message", reference]);
    // Queue uses the app's saved policy; forward only the per-launch optimizer override, never CLI permissions.
    let key = format!("model_providers.agentq.http_headers.{}=", crate::optimization::HEADER);
    for pair in spec.args.windows(2) {
        if (pair[0] == "-c" || pair[0] == "--config") && pair[1].to_str().is_some_and(|s| s.starts_with(&key)) { base.args(pair); }
    }
    crate::process::output("agent.app-queue", &mut environment(base, spec), budget).map_err(|e| e.to_string())
}

/// A native Remote Control session URL.
pub(super) fn remote_url(url: &str) -> bool {
    url.strip_prefix("https://claude.ai/code/session_").is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The Remote Control URL of a native bridge session `cse_<id>`.
fn bridge_url(id: &str) -> Option<String> {
    let url = format!("https://claude.ai/code/session_{}", id.strip_prefix("cse_")?);
    remote_url(&url).then_some(url)
}

/// A Claude link absence: the cause and its native evidence text.
pub(crate) type Absence = (Cause, Option<String>);

/// Claude Remote Control link of `session` from native records only (RUNTIME.md Session observation): `seen` from the
/// transcript, then the background job state, then the debug log. `requested` is the launch's `--remote-control`.
pub(crate) fn claude_link(home: Option<&Path>, session: &str, job: Option<&str>, mut requested: bool, seen: Option<String>) -> Result<String, Absence> {
    if let Some(url) = seen { return Ok(url); }
    let unreadable = |what: &str, e: &dyn std::fmt::Display| (Cause("evidence-unreadable"), Some(format!("{what}: {e}")));
    let read = |path: PathBuf, what: &str| -> Result<Option<Vec<u8>>, Absence> {
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(unreadable(what, &e)),
        };
        // ponytail: last 16 MiB only; a bridge line older than that reads as the newer evidence or `pending`.
        let size = file.metadata().map_err(|e| unreadable(what, &e))?.len();
        file.seek(SeekFrom::Start(size.saturating_sub(16 << 20))).map_err(|e| unreadable(what, &e))?;
        let mut bytes = vec![];
        file.read_to_end(&mut bytes).map_err(|e| unreadable(what, &e))?;
        Ok(Some(bytes))
    };
    let Some(home) = home else { return Err((Cause("evidence-unreadable"), Some("no Claude home".into()))) };
    if let Some(job) = job.filter(|j| plain_id(j)) {
        if let Some(bytes) = read(home.join("jobs").join(job).join("state.json"), "job state")? {
            let state: Value = serde_json::from_slice(&bytes).map_err(|e| unreadable("job state", &e))?;
            if let Some(url) = state["bridgeSessionId"].as_str().and_then(bridge_url) { return Ok(url); }
            requested |= state["respawnFlags"].as_array().is_some_and(|a| a.iter().any(|f| f == "--remote-control"));
        }
    }
    if !plain_id(session) { return Err((Cause("session-unknown"), None)); }
    let Some(log) = read(home.join("debug").join(format!("{session}.txt")), "debug log")? else {
        return Err((Cause(if requested { "unlogged" } else { "not-requested" }), None));
    };
    let failure = |detail: &str| {
        let d = detail.to_ascii_lowercase();
        let refused = ["polic", "disabled", "turned off", "subscription", "login", "logged in", "only available", "not available"].iter().any(|w| d.contains(w));
        Err((Cause(if refused { "disabled" } else { "failed" }), Some(detail.trim().to_string()).filter(|d| !d.is_empty())))
    };
    let mut last = None;
    for line in String::from_utf8_lossy(&log).lines() {
        // Only Claude's own logger prefix (`<time> [<LEVEL>] `) may precede the marker, so logged content cannot spoof it.
        let Some(at) = line.find("[bridge:repl] ") else { continue };
        let prefix = &line[..at];
        if !(prefix.is_empty() || prefix.ends_with("] ") && prefix.split_whitespace().count() <= 2) { continue; }
        let rest = &line[at + 14..];
        let found = if let Some(id) = ["Hook initialized, session=", "Mirror initialized, session="].iter().find_map(|p| rest.strip_prefix(p)) {
            bridge_url(id.trim()).map(Ok)
        } else if let Some(d) = rest.strip_prefix("Init failed: ") {
            Some(failure(d.split("; consecutive failures").next().unwrap_or(d)))
        } else if let Some(d) = rest.strip_prefix("notifyBridgeFailed detail=\"") {
            Some(failure(d.split("\" outboundOnly=").next().unwrap_or(d)))
        } else if rest.starts_with("Org policy now refuses the connected bridge") {
            Some(failure(rest))
        } else { None };
        if found.is_some() { last = found; }
    }
    last.unwrap_or(Err((Cause(if requested { "pending" } else { "not-requested" }), None)))
}

pub(super) fn plain_id(id: &str) -> bool { !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') }

pub(super) fn scope(spec: &Spec) -> String {
    let mut bytes = spec.client.code().as_bytes().to_vec();bytes.push(u8::from(spec.resume));
    bytes.extend_from_slice(&(spec.args.len() as u64).to_le_bytes());
    for value in &spec.args {
        let value=value.as_encoded_bytes();bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());bytes.extend_from_slice(value);
    }
    bytes.extend_from_slice(&(spec.env.len() as u64).to_le_bytes());
    for (key,value) in &spec.env {
        for part in [key,value] {let part=part.as_encoded_bytes();bytes.extend_from_slice(&(part.len() as u64).to_le_bytes());bytes.extend_from_slice(part);}
    }
    bytes.extend_from_slice(format!("{:?}", spec.home.clone().or_else(|| base_home(spec.client))).as_bytes());
    crate::sha256_hex(&bytes)
}

pub(super) fn file_id(file: &File) -> Res<String> {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata().map_err(|e| e.to_string())?;
        Ok(format!("{}:{}",m.dev(),m.ino()))
    }
    #[cfg(windows)] {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)] struct Id {volume:u64, id:[u8;16]}
        #[link(name="kernel32")] unsafe extern "system" {
            fn GetFileInformationByHandleEx(handle:*mut std::ffi::c_void, kind:i32, data:*mut std::ffi::c_void, size:u32)->i32;
        }
        let mut id = Id {volume:0,id:[0;16]};
        // SAFETY: this file owns the handle; FileIdInfo writes exactly the live Id buffer.
        if unsafe {GetFileInformationByHandleEx(file.as_raw_handle(),18,(&mut id as *mut Id).cast(),std::mem::size_of::<Id>() as u32)} == 0 {
            return Err(format!("native history identity unavailable: {}",std::io::Error::last_os_error()));
        }
        Ok(format!("{}:{:x?}",id.volume,id.id))
    }
}

/// An observation cursor, never a process owner. Save its checkpoint before submitting the bound input.
#[derive(Clone)]
pub(crate) struct NativeTurn {
    spec: Spec,
    path: PathBuf,
    input: String,
    start: u64,
    prefix: String,
    started_ms: i64,
    events: Vec<Value>,
    accepted: bool,
    turn: Option<String>,
    ended: bool,
    aborted: bool,
    remote: Option<String>,
    duration: Option<u64>,
    identity: Option<String>,
    protected_until: u64,
    protected_hash: String,
    cursor: u64,
    current: Option<String>,
    scope: String,
}

impl NativeTurn {
    /// Capture the pre-submission boundary. A missing file is allowed only for a new native session.
    pub fn before(spec: &Spec, path: &Path, input: &str) -> Res<Self> {
        if !plain_id(&spec.session) || spec.session == "-" { return Err("native session unknown".into()); }
        let (start,prefix,identity) = match File::open(path) {
            Ok(mut file) => {
                let size = file.metadata().map_err(|e| e.to_string())?.len();
                if size > 0 {
                    file.seek(SeekFrom::Start(size-1)).map_err(|e|e.to_string())?;
                    let mut last = [0]; file.read_exact(&mut last).map_err(|e|e.to_string())?;
                    if last != [b'\n'] { return Err("native baseline incomplete".into()); }
                }
                (size,digest_prefix(&mut file,size)?,Some(file_id(&file)?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !spec.resume => (0,crate::sha256_hex(&[]),None),
            Err(e) => return Err(format!("native baseline unavailable: {e}")),
        };
        Ok(Self { spec: spec.clone(), path: path.into(), input: input.into(), start,
            prefix: prefix.clone(), started_ms: unix_ms(), events: vec![], accepted: false, turn: None,
            ended: false, aborted: false, remote: None, duration: None,
            identity, protected_until:start, protected_hash:prefix, cursor:start, current:None, scope:scope(spec) })
    }

    /// Private recovery data: caller protects and persists it; no submission is performed by recovery.
    pub fn checkpoint(&self) -> Value {
        json!({"schema":"coreq-native-turn/1", "client":self.spec.client.code(), "root":self.spec.root,
            "session":self.spec.session, "home":self.spec.home, "path":self.path, "input":self.input,
            "start":self.start, "prefix":self.prefix, "started_ms":self.started_ms, "scope":self.scope, "identity":self.identity,
            "protected_until":self.protected_until, "protected_hash":self.protected_hash})
    }

    pub fn recover(spec: &Spec, saved: &Value) -> Res<Self> {
        if saved["schema"] != "coreq-native-turn/1" || saved["client"] != spec.client.code() || saved["session"] != spec.session
            || saved["root"] != json!(spec.root) || saved["home"] != json!(spec.home) || saved["scope"] != scope(spec) { return Err("native recovery scope differs".into()); }
        let path = PathBuf::from(saved["path"].as_str().ok_or("native recovery path missing")?);
        let prefix = saved["prefix"].as_str().filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())).ok_or("native recovery prefix invalid")?;
        let start=saved["start"].as_u64().ok_or("native recovery boundary missing")?;
        let protected_until=saved["protected_until"].as_u64().filter(|n|*n>=start).ok_or("native recovery protected boundary invalid")?;
        let protected_hash=saved["protected_hash"].as_str().filter(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())).ok_or("native recovery protected prefix invalid")?;
        Ok(Self { spec: spec.clone(), path, input: saved["input"].as_str().ok_or("native recovery input missing")?.into(),
            start, prefix: prefix.into(),
            started_ms: saved["started_ms"].as_i64().filter(|s| *s >= 0).ok_or("native recovery time invalid")?,
            events: vec![], accepted: false, turn: None, ended: false, aborted: false, remote: None, duration: None,
            identity:saved["identity"].as_str().map(String::from), scope:scope(spec),
            protected_until, protected_hash:protected_hash.into(),cursor:start,current:None })
    }

    /// Bind a new launch to its verified native job; no mode/account or submission changes are made.
    pub fn attach_background(&mut self, job: &BackgroundJob, transcript: &Path) -> Res<()> {
        if self.spec.client != Client::Claude || self.spec.resume || self.start != 0 || self.accepted
            || self.spec.root.canonicalize().ok().as_ref() != Some(&job.root) || self.scope != job.scope {
            return Err("native background attachment differs".into());
        }
        self.spec.session = job.session.clone(); self.path = transcript.into(); self.identity = None;
        Ok(())
    }

    #[cfg(test)]
    fn accepted(&self) -> bool { self.accepted }
    pub(crate) fn path(&self) -> &Path { &self.path }
    pub(crate) fn start(&self) -> u64 { self.start }
    pub(crate) fn session(&self) -> &str { &self.spec.session }
    pub fn remote_url(&self) -> Option<&str> { self.remote.as_deref() }

    /// Read at most 8 MiB of new records per pass; incomplete/corrupt evidence never retires ownership.
    pub fn poll(&mut self) -> Res<Option<Report>> {
        let mut next=self.clone();
        let report=next.poll_inner()?;
        *self=next;
        Ok(report)
    }

    fn poll_inner(&mut self) -> Res<Option<Report>> {
        let mut file = File::open(&self.path).map_err(|e| format!("native history unavailable: {e}"))?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("native history not a regular file".into()); }
        let identity = file_id(&file)?;
        if self.identity.as_ref().is_some_and(|id| *id != identity) { return Err("native history file replaced".into()); }
        if digest_prefix(&mut file,self.start)? != self.prefix || digest_prefix(&mut file,self.protected_until)? != self.protected_hash {
            return Err("native history replaced or rewritten".into());
        }
        if self.spec.client == Client::Codex {
            file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
            let mut first=String::new();
            BufReader::new((&mut file).take(8*1024*1024+1)).read_line(&mut first).map_err(|e|e.to_string())?;
            if first.len()>8*1024*1024 || !first.ends_with('\n') {return Err("native session metadata incomplete or oversized".into());}
            self.verify_meta(&serde_json::from_str(&first).map_err(|_|"native session metadata invalid")?)?;
        }
        file.seek(SeekFrom::Start(self.cursor)).map_err(|e|e.to_string())?;
        let mut reader=BufReader::new((&mut file).take(8*1024*1024+1));
        let mut consumed=0;
        let mut complete=true;
        loop {
            let mut line=String::new();
            let size=reader.read_line(&mut line).map_err(|e|format!("native history read: {e}"))?;
            if size==0 {break;}
            if size>8*1024*1024 {return Err("native history record exceeds 8 MiB".into());}
            if consumed+size>8*1024*1024 {complete=false;break;}
            if !line.ends_with('\n') {return Err("native history incomplete; retry observation".into());}
            let v: Value = serde_json::from_str(&line).map_err(|_| "native history record invalid")?;
            if v["type"].as_str().is_none() { return Err("native history shape invalid".into()); }
            match self.spec.client {
                Client::Claude => {
                    if v["sessionId"].as_str().or(v["session_id"].as_str()).is_some_and(|s| s != self.spec.session) {
                        return Err("native Claude session differs".into());
                    }
                    if v["subtype"] == "bridge_status" {
                        if let Some(url) = v["url"].as_str().filter(|s| remote_url(s)) { self.remote = Some(url.into()); }
                    }
                    if v["type"] == "user" {
                        let content = &v["message"]["content"];
                        let input = content.as_str().or_else(|| content.as_array().filter(|a| a.len() == 1 && a[0]["type"] == "text").and_then(|a| a[0]["text"].as_str()));
                        if input == Some(self.input.as_str()) {
                            if v["sessionId"].as_str().or(v["session_id"].as_str()) != Some(self.spec.session.as_str()) {
                                return Err("native Claude input session unproved".into());
                            }
                            if self.accepted { return Err("native input appears more than once".into()); }
                            self.accepted = true;
                        } else if input.is_some() && self.accepted && !self.ended { return Err("another input superseded the native turn".into()); }
                    }
                    if self.accepted && !self.ended {
                        if matches!(v["type"].as_str(),Some("assistant"|"rate_limit_event")) {self.events.push(v.clone());}
                        if v["type"] == "system" && v["subtype"] == "turn_duration" {
                            self.ended = true;
                            self.duration = v["durationMs"].as_u64().and_then(|n| n.checked_mul(1000));
                        }
                    }
                }
                Client::Codex => {
                    if v["type"] == "session_meta" { self.verify_meta(&v)?; }
                    if v["type"] != "event_msg" { self.cursor+=size as u64;consumed+=size;continue; }
                    let p = &v["payload"];
                    if !p.is_object() {return Err("native app record shape invalid".into());}
                    match p["type"].as_str() {
                        Some("task_started") => self.current = p["turn_id"].as_str().filter(|s| plain_id(s)).map(String::from),
                        Some("user_message" | "item_completed") => {
                            let item = p["type"] == "item_completed" && p["item"]["type"] == "UserMessage";
                            let value = if item { p["item"]["content"].as_array().filter(|a| a.len() == 1 && a[0]["type"] == "text").and_then(|a| a[0]["text"].as_str()) } else { p["message"].as_str() };
                            if value.is_some_and(|s| s == self.input || item && s.strip_suffix('\n') == Some(self.input.as_str())) {
                                if self.accepted { return Err("native input appears more than once".into()); }
                                let id = self.current.clone().ok_or("native input lacks turn identity")?;
                                if item && (p["thread_id"] != self.spec.session || p["turn_id"].as_str() != Some(&id))
                                    || p["turn_id"].as_str().is_some_and(|s| s != id) { return Err("native input identity differs".into()); }
                                self.accepted = true; self.turn = Some(id);
                            }
                        }
                        Some("task_complete" | "turn_aborted") if self.accepted && p["turn_id"].as_str() == self.turn.as_deref() => {
                            let aborted = p["type"] == "turn_aborted";
                            if self.ended && self.aborted != aborted { return Err("native terminal outcomes conflict".into()); }
                            self.ended = true; self.aborted = aborted;
                        }
                        _ => {}
                    }
                    if self.accepted && !self.ended && self.current == self.turn {
                        if p["type"] == "token_count" && p["turn_id"].as_str() == self.turn.as_deref() {
                            let u = &p["info"]["last_token_usage"];
                            append_codex_usage(&mut self.events,u,&p["info"]["total_token_usage"],false);
                        }
                        if p["type"] == "agent_message" && p["turn_id"].as_str().is_none_or(|t|Some(t)==self.turn.as_deref()) {
                            self.events.push(json!({"type":"item.completed","item":{"type":"agent_message","text":p["message"]}}));
                        }
                        if !p["rate_limits"].is_null() && p["turn_id"].as_str() == self.turn.as_deref() {
                            let mut limits=p.clone();
                            // A relative reset without a native timestamp cannot become a stable reset on recovery.
                            if let Some(windows)=limits.get_mut("rate_limits").and_then(Value::as_object_mut) {
                                for key in ["primary","secondary"] {if let Some(w)=windows.get_mut(key).and_then(Value::as_object_mut) {w.remove("resets_in_seconds");}}
                            }
                            self.events.push(limits);
                        }
                    }
                }
                Client::Antigravity => {
                    if matches!(v["type"].as_str(), Some("USER_INPUT")) {
                        let content = v["content"].as_str().unwrap_or_default();
                        if content == self.input {
                            if self.accepted { return Err("native input appears more than once".into()); }
                            self.accepted = true;
                        } else if !content.is_empty() && self.accepted && !self.ended {
                            return Err("another input superseded the native turn".into());
                        }
                    } else if self.accepted {
                        if v["type"] == "PLANNER_RESPONSE" {
                            self.events.push(v.clone());
                            if v["status"] == "CANCELLED" {
                                self.ended = true;
                                self.aborted = true;
                            } else if ends(&v) {
                                self.ended = true;
                            } else {
                                self.ended = false;
                            }
                        } else if v["type"] == "TOOL_RESPONSE" {
                            self.ended = false;
                        }
                    }
                }
            }
            self.cursor+=size as u64;consumed+=size;
        }
        drop(reader);
        if self.cursor>=self.protected_until {self.protected_hash=digest_prefix(&mut file,self.cursor)?;self.protected_until=self.cursor;}
        self.identity = Some(identity);
        if !complete || !self.ended { return Ok(None); }
        let mut report = Report::read(self.spec.client, "", &self.events, None);
        report.session = Some(self.spec.session.clone()); report.started_ms = self.started_ms; report.duration_us = self.duration.unwrap_or(0);
        if self.spec.client == Client::Claude {
            report.text = self.events.iter().rev().filter(|v|v["type"] == "assistant").find_map(|v|v["message"]["content"].as_array()?.iter().rev().find_map(|b|(b["type"]=="text").then(||b["text"].as_str().map(String::from)).flatten()));
        }
        (report.outcome, report.error) = if self.aborted { ("cancelled", "interrupted") }
            else if self.events.iter().any(|e| e["isApiErrorMessage"] == true || e["error"].is_string()) { ("failed", "client-error") } else { ("ok", "none") };
        if report.reached.is_some() { (report.outcome, report.error) = ("refused", "usage-limit"); }
        Ok(Some(report))
    }

    fn verify_meta(&self, v: &Value) -> Res<()> {
        let m = &v["payload"];
        let root=self.spec.root.canonicalize().map_err(|_|"native project identity unavailable")?;
        if v["type"] != "session_meta" || m["id"] != self.spec.session || !m["forked_from_id"].is_null()
            || m["cwd"].as_str().and_then(|p| Path::new(p).canonicalize().ok()) != Some(root) {
            return Err("native app session identity differs or is copied".into());
        }
        Ok(())
    }

    /// Idempotent FULL-commit append. Recovery uses the same native boundary and immutable payload.
    pub fn record(&mut self, ctx: &Context, store: &crate::telemetry::Store) -> Res<Option<Report>> {
        let Some(mut report) = self.poll()? else { return Ok(None) };
        report.recorded = self.snapshot(&report, ctx).and_then(|f| f.append(store));
        Ok(Some(report))
    }

    /// The immutable records of `report`, this turn's terminal report, without a store or native read.
    pub(crate) fn snapshot(&self, report: &Report, ctx: &Context) -> Res<super::Frozen> {
        let mut observation = report.observation(self.spec.client, ctx);
        if self.duration.is_none() { observation["duration_us"] = Value::Null; observation["unknown"]["duration_us"] = json!("not-reported"); }
        let key = crate::sha256_hex(format!("{}\0{}\0{}\0{}\0{}\0{}", self.spec.client.code(), self.spec.root.display(), self.scope, self.spec.session, self.start, self.input).as_bytes());
        super::capture::freeze("agent.native", &key, &observation, report, self.spec.client, ctx)
    }
}

// `last` is one model-call increment, not the whole turn. Repeated total snapshots carry no new usage.
pub(super) fn append_codex_usage(events:&mut Vec<Value>,last:&Value,total:&Value,camel:bool) {
    let fields=if camel {["inputTokens","cachedInputTokens","outputTokens"]} else {["input_tokens","cached_input_tokens","output_tokens"]};
    let reasoning=&last[if camel {"reasoningOutputTokens"} else {"reasoning_output_tokens"}];
    let totals:Vec<_>=fields.iter().map(|k|total[k].as_u64()).collect();
    let increments:Vec<_>=fields.iter().map(|k|last[k].as_u64()).collect();
    let key=json!(totals);
    if totals.iter().all(Option::is_some) && events.iter().any(|e|e["total_usage"]==key) {return;}
    let prior=events.iter().rev().find_map(|e|e.get("total_usage").and_then(Value::as_array));
    let consistent=totals.iter().all(Option::is_some) && increments.iter().all(Option::is_some)
        && prior.is_none_or(|previous|totals.iter().zip(&increments).zip(previous).all(|((total,increment),old)|
            total.zip(old.as_u64()).and_then(|(t,o)|t.checked_sub(o))==*increment));
    let value=|i|if consistent {json!(increments[i])} else {Value::Null};
    events.push(json!({"type":"turn.completed","call":true,"total_usage":key,"usage":{"input_tokens":value(0),"cached_input_tokens":value(1),"output_tokens":value(2),"reasoning_output_tokens":reasoning}}));
}

pub(super) fn digest_prefix(file:&mut File,length:u64)->Res<String> {
    file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
    let until=Instant::now()+Duration::from_secs(5);
    let mut hash=ring::digest::Context::new(&ring::digest::SHA256);
    let mut left=length;
    let mut buffer=[0;65536];
    while left>0 {
        if Instant::now()>=until {return Err("native prefix verification exceeded 5 seconds; ownership retained".into());}
        let size=buffer.len().min(left as usize);
        let n=file.read(&mut buffer[..size]).map_err(|e|e.to_string())?;
        if n==0 {return Err("native history truncated".into());}
        hash.update(&buffer[..n]);left-=n as u64;
    }
    Ok(hash.finish().as_ref().iter().map(|b|format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append(path: &Path, events: &[Value]) {
        let mut f = File::options().create(true).append(true).open(path).unwrap();
        for event in events { writeln!(f, "{event}").unwrap(); }
    }

    fn observations(store: &crate::telemetry::Store) -> Vec<Value> {
        store.export(0, 0, 100).unwrap()["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()).filter(|o| o["kind"] == "agent.turn").collect()
    }

    // #213 A3 native proof, not CI: reads a live background job of this host through the same reader.
    // Run: AGENTQ_LIVE_CLAUDE_JOB=<job id> cargo test -p agentq-runtime --lib live_claude_job_link -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_claude_job_link() {
        let job = std::env::var("AGENTQ_LIVE_CLAUDE_JOB").expect("AGENTQ_LIVE_CLAUDE_JOB");
        let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).expect("home")).join(".claude");
        let link = claude_link(Some(&home), "-", Some(&job), false, None);
        println!("live job {job}: {link:?}");
        assert!(link.is_ok_and(|u| remote_url(&u)), "a live Remote Control job names its URL");
    }

    // #213 A1-A4: Claude link from native records only. Risk: a missing link reads as a guess or `unknown` instead of the
    // runtime's logged cause, or unreadable evidence reads as pending.
    #[test]
    fn claude_link_names_url_or_logged_cause() {
        let home=crate::scratch("claude-link");
        let link=|job:Option<&str>,requested:bool| claude_link(Some(&home),"s-1",job,requested,None);
        let cause=|r:Result<String,Absence>| r.unwrap_err();
        assert_eq!(claude_link(Some(&home),"s-1",None,true,Some("https://claude.ai/code/session_seen1".into())).unwrap(),"https://claude.ai/code/session_seen1");
        assert_eq!(cause(link(None,false)),(Cause("not-requested"),None));
        assert_eq!(cause(link(None,true)),(Cause("unlogged"),None),"no runtime log is unknown, never pending");
        let log=home.join("debug/s-1.txt");std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log,"2026-11-03T09:00:00Z [DEBUG] other line\n").unwrap();
        assert_eq!(cause(link(None,true)),(Cause("pending"),None));
        let write=|line:&str| {use std::io::Write;writeln!(File::options().append(true).open(&log).unwrap(),"2026-11-03T09:00:01Z [DEBUG] {line}").unwrap();};
        write("[bridge:repl] Init failed: Remote Control is disabled by your organization's policy.; consecutive failures: 1");
        assert_eq!(cause(link(None,true)),(Cause("disabled"),Some("Remote Control is disabled by your organization's policy.".into())));
        write("[bridge:repl] notifyBridgeFailed detail=\"Session ingress unreachable\" outboundOnly=false wasConnected=false");
        assert_eq!(cause(link(None,false)),(Cause("failed"),Some("Session ingress unreachable".into())),"the latest logged event wins");
        write("[DEBUG] tool output quoted [bridge:repl] Hook initialized, session=cse_01Spoof");
        assert_eq!(cause(link(None,true)).0,Cause("failed"),"a marker inside logged content is not evidence");
        write("[bridge:repl] Hook initialized, session=cse_01Abc");
        assert_eq!(link(None,true).unwrap(),"https://claude.ai/code/session_01Abc");
        let state=home.join("jobs/c37eb75a/state.json");std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(&state,"{\"bridgeSessionId\":\"cse_01Job\"}").unwrap();
        assert_eq!(link(Some("c37eb75a"),false).unwrap(),"https://claude.ai/code/session_01Job","background job state carries the URL");
        std::fs::write(&state,"{\"respawnFlags\":[\"--remote-control\"]}").unwrap();std::fs::remove_file(&log).unwrap();
        assert_eq!(cause(link(Some("c37eb75a"),false)).0,Cause("unlogged"),"a background job's saved flags request Remote Control");
        std::fs::write(&state,"{").unwrap();
        assert_eq!(cause(link(Some("c37eb75a"),true)).0,Cause("evidence-unreadable"));
        std::fs::remove_file(&state).unwrap();std::fs::create_dir_all(&log).unwrap();
        let (c,detail)=cause(link(None,true));
        assert!(c==Cause("evidence-unreadable") && detail.is_some_and(|d|d.starts_with("debug log: ")),"an unreadable log names its error");
        assert_eq!(cause(claude_link(None,"s-1",None,true,None)).0,Cause("evidence-unreadable"));
        std::fs::remove_dir_all(home).unwrap();
    }

    // R48/R49: launcher readiness is separate from exact input and its native terminal; cold observation never replays.
    // Risk: stale/foreign/partial native history releases owned work or charges a turn twice across restart.
    #[test]
    fn native_turns_bind_input_terminal_and_recovery() {
        for client in [Client::Claude,Client::Codex] {
            let root=crate::scratch(client.code());
            let spec=Spec {client,root:root.clone(),session:"native-session".into(),resume:false,args:vec![],home:Some(root.join("account")),env:vec![]};
            let path=root.join("native.jsonl");
            if client==Client::Codex {append(&path,&[json!({"type":"session_meta","payload":{"id":spec.session,"cwd":root}})]);}
            let mut turn=NativeTurn::before(&spec,&path,"owned input").unwrap();
            let checkpoint=turn.checkpoint();
            let terminal=if client==Client::Claude {json!({"type":"system","subtype":"turn_duration","sessionId":spec.session,"durationMs":10})}
                else {json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"owned-turn"}})};
            append(&path,&[terminal.clone()]);
            assert!(turn.poll().unwrap().is_none(),"stale terminal without own input is not completion");
            assert!(!turn.accepted());
            let input=if client==Client::Claude {json!({"type":"user","sessionId":spec.session,"message":{"content":"owned input"}})}
                else {json!({"type":"event_msg","payload":{"type":"user_message","message":"owned input","turn_id":"owned-turn"}})};
            if client==Client::Codex {append(&path,&[json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"owned-turn"}})]);}
            append(&path,&[input.clone()]);
            assert!(turn.poll().unwrap().is_none()); assert!(turn.accepted());
            if client==Client::Claude {
                append(&path,&[json!({"type":"system","subtype":"bridge_status","sessionId":spec.session,"url":"https://claude.ai/code/session_fixture123"}),
                    json!({"type":"assistant","sessionId":spec.session,"message":{"content":[{"type":"tool_use","id":"tool1","name":"Read","input":{}}]}}),
                    json!({"type":"user","sessionId":spec.session,"message":{"content":[{"type":"tool_result","tool_use_id":"tool1","content":"file content"}]}}),
                    json!({"type":"assistant","sessionId":spec.session,"message":{"id":"m1","model":"fixture-model","content":[{"type":"text","text":"answer"}],
                    "usage":{"input_tokens":3,"cache_read_input_tokens":20,"cache_creation_input_tokens":4,"output_tokens":5}}}),
                    json!({"type":"rate_limit_event","rate_limit_info":{"utilization":0.25,"resetsAt":1793700000u64,"rateLimitType":"five_hour"}})]);
            } else {
                append(&path,&[json!({"type":"event_msg","payload":{"type":"token_count","turn_id":"foreign","info":{"last_token_usage":{"input_tokens":999,"cached_input_tokens":0,"output_tokens":999}}}})]);
            }
            append(&path,&[terminal]);
            let mut recovered=NativeTurn::recover(&spec,&checkpoint).unwrap();
            let store=crate::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
            let ctx=Context {product:"fixtureq",version:"1.0.0",project:"other",task:Some((9,1)),route:None,account:"work"};
            let report=recovered.record(&ctx,&store).unwrap().unwrap(); report.recorded.unwrap();
            assert_eq!(report.outcome,"ok");
            let saved=recovered.checkpoint();
            recovered.record(&ctx,&store).unwrap().unwrap().recorded.unwrap();
            let mut cold=NativeTurn::recover(&spec,&checkpoint).unwrap();
            cold.record(&ctx,&store).unwrap().unwrap().recorded.unwrap();
            assert_eq!(observations(&store).len(),1,"one durable observation even from a pre-ack checkpoint");
            if client==Client::Claude {
                assert_eq!(report.usage.input,Some(3)); assert_eq!(report.text.as_deref(),Some("answer"));
                assert_eq!(recovered.remote_url(),Some("https://claude.ai/code/session_fixture123"));
            } else {assert_eq!(report.usage.input,None,"foreign usage never becomes this turn's usage");assert_eq!(report.usage.unknown,Some("no-usage"));}
            let payload=observations(&store)[0].to_string();
            for private in ["owned input","native.jsonl","session_fixture123","answer"] {assert!(!payload.contains(private));}
            // A3 names the session id as an `agent.turn` field; no process is owned here.
            let turn=&observations(&store)[0];
            assert_eq!(turn["session_id"],"native-session");
            assert!(turn["peak_rss_bytes"].is_null()&&turn["exit_code"].is_null()&&turn["unknown"]["peak_rss_bytes"]=="not-owned");
            let changed=Spec {home:Some(root.join("foreign-account")),..spec.clone()};
            assert!(NativeTurn::recover(&changed,&saved).is_err());
            let changed=Spec {resume:!spec.resume,..spec.clone()};assert!(NativeTurn::recover(&changed,&saved).is_err());
            use std::io::Write;
            std::fs::File::options().append(true).open(&path).unwrap().write_all(b"{\"type\":").unwrap();
            assert!(recovered.poll().is_err(),"partial record cannot establish completion");
            std::fs::write(&path,"{}\n").unwrap();
            assert!(recovered.poll().is_err(),"truncated or rewritten history stays unknown");
            drop(store);std::fs::remove_dir_all(root).unwrap();
        }
    }
}
