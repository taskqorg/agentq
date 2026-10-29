//! Telemetry to Langfuse (agentq.md § 6): metadata-only `agentq.*` spans as files in `data/queue/`, one sender in the
//! service, standalone keys in OS protected storage.

use coreq::write_atomic;
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SCHEMA: u64 = 1;
const CAP_BYTES: u64 = 64 * 1024 * 1024;
const CAP_AGE: u64 = 7 * 86400;
const BATCH: usize = 100;
const HEARTBEAT: u64 = 3600;
/// The start heartbeat waits at most this long for proxies the service just started.
const START_WAIT: u64 = 300;
const STORE: &str = "agentq-langfuse";
/// Queue files bound to a DashQ grant (`connect.rs`); standalone spans are the other files.
pub const RECORD: &str = ".rec.json";

pub(crate) fn queue(data: &Path) -> PathBuf {
    data.join("queue")
}

pub fn runtime() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if std::env::var_os("WSL_DISTRO_NAME").is_some() || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists() {
        "wsl"
    } else {
        "linux"
    }
}

/// Unique installation ID, created on first use.
pub fn installation(data: &Path) -> String {
    let p = data.join("installation");
    match std::fs::read_to_string(&p) {
        Ok(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            let id = crate::mcp::new_id();
            let _ = std::fs::create_dir_all(data).map_err(|e| e.to_string()).and_then(|_| write_atomic(&p, &id));
            id
        }
    }
}

/// One OTLP span with the common attributes; the event ID is also the trace ID, so a resend is idempotent. A 32-hex
/// `agentq.event_id` in `attrs` is kept (resource samples), else a new one is made.
pub fn span(name: &str, stage: &str, attrs: Value, at_ns: u128, installation: &str, host: &str) -> Value {
    let id = attrs["agentq.event_id"].as_str().filter(|i| i.len() == 32).map_or_else(|| crate::mcp::new_id().replace('-', ""), String::from);
    let mut a = json!({"agentq.schema": SCHEMA, "agentq.event_id": id, "agentq.stage": stage, "agentq.installation": installation, "host.name": host, "agentq.runtime": runtime(), "agentq.version": env!("CARGO_PKG_VERSION")});
    if let (Some(m), Value::Object(x)) = (a.as_object_mut(), attrs) {
        m.extend(x);
    }
    let attributes: Vec<Value> = a
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| {
            let v = match v {
                Value::Bool(b) => json!({"boolValue": b}),
                Value::Number(n) if n.is_f64() => json!({"doubleValue": n}),
                Value::Number(n) => json!({"intValue": n}),
                Value::String(s) => json!({"stringValue": s}),
                v => json!({"stringValue": v.to_string()}),
            };
            json!({"key": k, "value": v})
        })
        .collect();
    json!({"traceId": id, "spanId": &id[16..], "name": name, "kind": 1, "startTimeUnixNano": at_ns.to_string(), "endTimeUnixNano": at_ns.to_string(), "attributes": attributes})
}

pub fn now_ns() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// Queue one event; never waits for the network and never fails the caller. Returns the event ID when written.
pub fn emit(data: &Path, name: &str, stage: &str, attrs: Value) -> Option<String> {
    emit_at(data, name, stage, attrs, now_ns())
}

/// `emit` at a given time; a queued event with the same ID is replaced, so the queue keeps one file per ID.
pub fn emit_at(data: &Path, name: &str, stage: &str, attrs: Value, ns: u128) -> Option<String> {
    emit_all(data, name, stage, vec![(attrs, ns)]).pop()
}

/// Events per local transaction in `emit_until`: the most a pass runs past its deadline.
pub(crate) const EMIT_CHUNK: usize = 32;

/// `emit_at` for many events: one queue prune for the batch, one file per event. Returns the IDs written.
pub fn emit_all(data: &Path, name: &str, stage: &str, events: Vec<(Value, u128)>) -> Vec<String> {
    emit_until(data, name, stage, events, None).1
}

/// `emit_all` in chunks of `EMIT_CHUNK` events, one local transaction each; after the first chunk, none starts past
/// `deadline`. Returns how many leading events were handled and the IDs written.
pub fn emit_until(data: &Path, name: &str, stage: &str, events: Vec<(Value, u128)>, deadline: Option<std::time::Instant>) -> (usize, Vec<String>) {
    let (inst, host) = (installation(data), crate::host());
    let secs = |ns: u128| (ns / 1_000_000_000).min(u64::MAX as u128) as u64;
    let spans: Vec<(Value, u128)> = events.into_iter().map(|(a, ns)| (span(name, stage, a, ns, &inst, &host), ns)).collect();
    let q = queue(data);
    if spans.is_empty() {
        return (0, vec![]);
    }
    // An unwritable queue still keeps the local copy; its file writes fail one by one.
    let _ = std::fs::create_dir_all(&q);
    let (mut handled, mut written) = (0, std::collections::HashMap::new());
    for chunk in spans.chunks(EMIT_CHUNK) {
        if handled > 0 && deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            break;
        }
        if name != "agentq.compress" {
            let rows: Vec<(&Value, u64)> = chunk.iter().map(|(s, ns)| (s, secs(*ns))).collect();
            if let Err(e) = crate::local::events_at(data, &rows) {
                eprintln!("agentq: local event unknown: {e}");
            }
        }
        for (s, ns) in chunk {
            let Some(id) = s["traceId"].as_str() else { continue };
            let file = q.join(format!("{}-{id}.json", secs(*ns)));
            if write_atomic(&file, &s.to_string()).is_ok() {
                written.insert(id.to_string(), file);
            }
        }
        handled += chunk.len();
    }
    for (_, p, _) in prune(&q, spans[..handled].iter().map(|s| secs(s.1)).max().unwrap_or(0)) {
        let id = p.file_name().and_then(|n| n.to_str()?.strip_suffix(".json")?.split_once('-').map(|x| x.1.to_string()));
        if id.and_then(|i| written.get(&i)).is_some_and(|f| *f != p) {
            let _ = std::fs::remove_file(p);
        }
    }
    (handled, written.into_keys().collect())
}

/// Queued events oldest first: (unix seconds, path, bytes).
pub(crate) fn entries(q: &Path) -> Vec<(u64, PathBuf, u64)> {
    let mut v: Vec<_> = std::fs::read_dir(q)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let at = n.strip_suffix(".json")?.split('-').next()?.parse().ok()?;
            Some((at, e.path(), e.metadata().ok()?.len()))
        })
        .collect();
    v.sort();
    v
}

pub(crate) fn state(q: &Path) -> Value {
    std::fs::read_to_string(q.join("state.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({}))
}

// ponytail: read-modify-write without a lock; concurrent writers may lose a `dropped` increment. Add a lock file if
// the count must be exact.
pub(crate) fn update_state(q: &Path, f: impl FnOnce(&mut Value)) {
    let mut s = state(q);
    f(&mut s);
    let _ = write_atomic(&q.join("state.json"), &s.to_string());
}

/// Drop events past 7 days, then oldest until the queue fits 64 MiB; count them in `dropped`.
pub(crate) fn prune(q: &Path, now: u64) -> Vec<(u64, PathBuf, u64)> {
    let mut v = entries(q);
    let mut bytes: u64 = v.iter().map(|e| e.2).sum();
    let mut dropped = 0;
    v.retain(|(at, p, len)| {
        if (now.saturating_sub(*at) > CAP_AGE || bytes > CAP_BYTES) && std::fs::remove_file(p).is_ok() {
            bytes -= len;
            dropped += 1;
            return false;
        }
        true
    });
    if dropped > 0 {
        update_state(q, |s| s["dropped"] = json!(s["dropped"].as_u64().unwrap_or(0) + dropped));
    }
    v
}

/// `status.json` `queue` and `telemetry status`.
pub fn status(data: &Path, now: u64) -> Value {
    let q = queue(data);
    let (v, s) = (entries(&q), state(&q));
    json!({"pending": v.len(), "oldest_age": v.first().map(|e| now.saturating_sub(e.0)), "last_delivery": s["last_delivery"], "dropped": s["dropped"].as_u64().unwrap_or(0), "error": s["error"]})
}

pub struct Keys {
    pub host: String,
    public: String,
    secret: String,
}

impl Keys {
    /// `host public secret`; one safe charset keeps them quotable; `https://`, or `http://` on loopback only.
    pub fn parse(s: &str) -> Result<Keys, String> {
        let w: Vec<&str> = s.split_whitespace().collect();
        let [host, public, secret] = w[..] else { return Err("need the host, public key and secret key".into()) };
        if !w.iter().all(|x| x.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))) {
            return Err("host and keys take letters, digits and . _ : / - only".into());
        }
        let loopback = host.strip_prefix("http://").and_then(|r| r.split(['/', ':']).next()).is_some_and(|h| h == "127.0.0.1" || h == "localhost");
        if !host.starts_with("https://") && !loopback {
            return Err("the host must be https:// (http:// only on loopback)".into());
        }
        Ok(Keys { host: host.trim_end_matches('/').into(), public: public.into(), secret: secret.into() })
    }

    fn line(&self) -> String {
        format!("{} {} {}", self.host, self.public, self.secret)
    }
}

/// stdout of a command fed `input` on stdin, when it succeeds; else its start error or exit status. Its stderr is
/// dropped: a key store may echo the secret input there.
fn feed(c: &mut Command, input: &str) -> Result<String, String> {
    let mut p = coreq::quiet(c).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    p.stdin.take().ok_or("no stdin")?.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
    let o = p.wait_with_output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).into_owned()) } else { Err(o.status.to_string()) }
}

/// The optional key store; a miss or a missing tool falls back to the file without a log line (agentq.md § 5).
fn os_read() -> Option<String> {
    if cfg!(target_os = "macos") {
        feed(Command::new("security").args(["find-generic-password", "-s", STORE, "-a", "agentq", "-w"]), "").ok()
    } else if cfg!(target_os = "linux") {
        feed(Command::new("secret-tool").args(["lookup", "service", STORE]), "").ok()
    } else {
        None
    }
}

/// A failed write is logged with its command; the caller falls back to the file.
fn os_write(line: &str) -> Option<&'static str> {
    let (mut c, input, store) = if cfg!(target_os = "macos") {
        // `security -i` reads the command from stdin: the secret stays out of argv
        let mut c = Command::new("security");
        c.arg("-i");
        (c, format!("add-generic-password -U -s {STORE} -a agentq -w \"{line}\"\n"), "Keychain")
    } else if cfg!(target_os = "linux") {
        let mut c = Command::new("secret-tool");
        c.args(["store", "--label=agentq Langfuse", "service", STORE]);
        (c, line.to_string(), "Secret Service")
    } else {
        return None;
    };
    feed(&mut c, &input).map_err(|e| crate::host::log(&c, &e)).ok().map(|_| store)
}

// ponytail: Windows uses the per-user file; add a Credential Manager binding (CredRead) when Windows hosts need it.
fn file_write(data: &Path, line: &str) -> Result<(), String> {
    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let p = data.join("langfuse");
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    #[cfg(unix)]
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Keychain or Secret Service, read back to confirm; else `data/langfuse` mode 0600. Returns where the keys went.
pub fn save(data: &Path, k: &Keys) -> Result<&'static str, String> {
    if let Some(store) = os_write(&k.line())
        && os_read().is_some_and(|s| s.trim() == k.line())
    {
        let _ = std::fs::remove_file(data.join("langfuse"));
        return Ok(store);
    }
    file_write(data, &k.line()).map(|_| "data/langfuse (mode 0600)")
}

pub fn load(data: &Path) -> Option<Keys> {
    os_read().and_then(|s| Keys::parse(&s).ok()).or_else(|| Keys::parse(&std::fs::read_to_string(data.join("langfuse")).ok()?).ok())
}

/// `agentq langfuse`: three lines from stdin, the secret without echo on a terminal.
pub fn read_keys() -> Result<Keys, String> {
    let tty = std::io::stdin().is_terminal();
    let mut words = vec![];
    for (prompt, hide) in [("Langfuse host: ", false), ("Public key: ", false), ("Secret key: ", true)] {
        if tty {
            eprint!("{prompt}");
        }
        let echo = |on: bool| {
            if hide && tty && cfg!(unix) {
                let _ = Command::new("stty").arg(if on { "echo" } else { "-echo" }).status();
            }
        };
        echo(false);
        let mut line = String::new();
        let r = std::io::stdin().read_line(&mut line);
        echo(true);
        if hide && tty {
            eprintln!();
        }
        r.map_err(|e| e.to_string())?;
        words.push(line.trim().to_string());
    }
    Keys::parse(&words.join(" "))
}

/// curl with the credentials in its stdin config, never in argv (§ 6).
fn curl(config: &str, k: &Keys) -> Result<String, String> {
    let mut c = Command::new("curl");
    c.args(["-sS", "--fail", "--max-time", "30", "-K", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut p = coreq::quiet(&mut c).spawn().map_err(|e| format!("curl: {e}"))?;
    let cfg = format!("{config}user = {}\n", quote(&format!("{}:{}", k.public, k.secret)));
    p.stdin.take().ok_or("curl: no stdin")?.write_all(cfg.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    let o = p.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).into_owned()) } else { Err(format!("Langfuse: {}", String::from_utf8_lossy(&o.stderr).trim())) }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Post queued spans in batches of 100, deleting each after a 2xx; the last outcome goes to `state.json`. Grant-bound
/// records keep their own destination (`connect.rs`).
pub fn send(data: &Path, k: &Keys, now: u64) -> Result<usize, String> {
    if crate::control::host_disabled(data) != Ok(false) { return Err("integration_disabled: export suspended".into()); }
    let q = queue(data);
    let body = q.join(format!("body-{}.tmp", std::process::id()));
    let mut sent = 0;
    let start = std::time::Instant::now();
    let res = loop {
        if crate::control::host_disabled(data) != Ok(false) { break Err("integration_disabled: export suspended".into()); }
        let batch: Vec<_> = prune(&q, now).into_iter().filter(|e| !e.1.to_string_lossy().ends_with(RECORD)).take(BATCH).collect();
        if batch.is_empty() {
            break Ok(sent);
        }
        let spans=batch.iter().map(|e| {
            let text=std::fs::read_to_string(&e.1).map_err(|_|"queue event unreadable; file kept".to_string())?;
            serde_json::from_str::<Value>(&text).map_err(|_|"queue event malformed; file kept".to_string())
        }).collect::<Result<Vec<_>,_>>();
        let spans=match spans { Ok(v)=>v, Err(e)=>break Err(e) };
        if let Some(e)=spans.iter().find_map(|s| {
            let at=s["startTimeUnixNano"].as_str().and_then(|s|s.parse::<u128>().ok()).map(|n|(n/1_000_000_000).min(u64::MAX as u128) as u64);
            match at {
                Some(at)=>crate::local::event(data,s,at).err(),
                None=>Some("queue event time unknown; file kept".into()),
            }
        }) { break Err(format!("local archive unavailable: {e}")); }
        let otlp = json!({"resourceSpans": [{"resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "agentq"}}]}, "scopeSpans": [{"scope": {"name": "agentq", "version": env!("CARGO_PKG_VERSION")}, "spans": spans}]}]});
        let step = std::time::Instant::now();
        let posted = std::fs::write(&body, otlp.to_string()).map_err(|e| e.to_string()).and_then(|_| {
            curl(&format!("url = {}\nheader = \"Content-Type: application/json\"\ndata-binary = {}\n", quote(&format!("{}/api/public/otel/v1/traces", k.host)), quote(&format!("@{}", body.display()))), k)
        });
        crate::profile::trace(data, "export.langfuse", "batch", step.elapsed(), posted.as_ref().err().map(|e| crate::profile::class(e)));
        let _ = std::fs::remove_file(&body);
        if let Err(e) = posted {
            break Err(e);
        }
        if batch.iter().any(|e| std::fs::remove_file(&e.1).is_err() && e.1.exists()) {
            break Err("queue: cannot delete sent events".into());
        }
        sent += batch.len();
    };
    let took = start.elapsed();
    update_state(&q, |s| {
        match &res {
            Ok(n) => {
                if *n > 0 {
                    s["last_delivery"] = json!(now);
                }
                s["error"] = Value::Null;
            }
            Err(e) => s["error"] = json!(e),
        }
    });
    // An empty pass posted nothing: no sample.
    if res != Ok(0) {
        crate::profile::sample(data, "export.langfuse", now, took, res.as_ref().err().map(|e|crate::profile::class(e)));
    }
    res
}

/// `agentq.routing_problems` and `agentq.hook_failures` (#132): each problem, `none` when there is none.
fn problems(p: Vec<String>) -> String {
    if p.is_empty() { "none".into() } else { p.join(", ") }
}

/// `agentq.installation` attributes. `agents`: status.json `agents`, exported whole as `agentq.routing` (#134) beside
/// its problem summary; `agentq.hooks` per hook with its last failure.
fn heartbeat(install: &Path, data: &Path, now: u64, agents: Value) -> Value {
    let q = status(data, now);
    let observed: Value = std::fs::read_to_string(install.join("status.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let rtk = if crate::engines::rtk_path(install).exists() { crate::engines::RTK } else { "unavailable" };
    json!({"agentq.rtk_version": rtk, "agentq.headroom_pin": crate::engines::HEADROOM, "agentq.queue_pending": q["pending"], "agentq.queue_dropped": q["dropped"],
        "agentq.routing_problems": problems(crate::routing::problems(&agents)), "agentq.routing": agents,
        "agentq.hook_failures": problems(crate::executor::hook_failures(data)), "agentq.hooks": crate::executor::hooks(data),
        "agentq.release": crate::release::project(&observed["release"], now), "agentq.model_backlog": crate::model::backlog(data),
        "agentq.ids_unknown": "an installation event has no session"})
}

/// Whether the start heartbeat waits: an agent still reads `unknown` (its proxy is binding) and `START_WAIT` has not
/// passed since `start`. Hourly heartbeats never wait.
fn start_waits(agents: &Value, beat: u64, start: u64, now: u64) -> bool {
    beat == 0 && now < start + START_WAIT && agents.as_object().into_iter().flatten().any(|(_, a)| a["routing"] == "unknown")
}

/// The service's one sender: `agentq.installation` at start and hourly, a resource sample per bucket, then a send pass
/// every `every` seconds.
pub fn sender(install: &Path, every: u64) -> ! {
    let data = install.join("data");
    let start = crate::now();
    let mut beat = 0;
    let mut sampler = crate::resource::Sampler::new(&data);
    let mut dashq = crate::connect::Sender::default();
    loop {
        if crate::control::host_disabled(&data) != Ok(false) {
            // § 4: the DashQ snapshot alone keeps sending, so DashQ names the host disabled (#169)
            crate::connect::pass(&data, crate::now(), &mut dashq);
            std::thread::sleep(Duration::from_secs(every));
            continue;
        }
        sampler.tick(&data);
        if crate::control::disabled(&data, "claude") == Ok(false) {
            crate::model::drain(&data, std::time::Instant::now() + Duration::from_secs(10));
        }
        let now = crate::now();
        crate::connect::pass(&data, now, &mut dashq);
        if now >= beat + HEARTBEAT {
            let agents = crate::routing::agents(&data);
            if !start_waits(&agents, beat, start, now) {
                beat = now;
                emit(&data, "agentq.installation", "heartbeat", heartbeat(install, &data, now, agents));
            }
        }
        match load(&data) {
            Some(k) => {
                let _ = send(&data, &k, now);
            }
            None => update_state(&queue(&data), |s| s["error"] = json!("no Langfuse keys; run `agentq langfuse`")),
        }
        std::thread::sleep(Duration::from_secs(every));
    }
}

/// `telemetry verify`: queue one event, send now, read its span back (Langfuse ingests asynchronously). v2
/// observations, since Langfuse v4 events-only servers answer 404 on `/api/public/traces`. A miss names the query and
/// its last answer, so not ingested yet differs from a 401 (keys) or a network error.
pub fn verify(data: &Path, k: &Keys, tries: u32, wait: Duration) -> Result<String, String> {
    let id = emit(data, "agentq.verify", "verify", json!({"agentq.ids_unknown": "synthetic event"})).ok_or("queue not writable")?;
    send(data, k, crate::now())?;
    let get = format!("{}/api/public/v2/observations?traceId={id}&limit=1", k.host);
    let mut last = String::new();
    for _ in 0..tries {
        std::thread::sleep(wait);
        match curl(&format!("url = {}\n", quote(&get)), k) {
            Ok(b) if b.contains(&format!("\"traceId\":\"{id}\"")) => return Ok(id),
            Ok(_) => last = "no observation yet".into(),
            Err(e) => last = e,
        }
    }
    Err(format!("sent {id}; GET {get} for {} s, last answer: {last}", tries as u64 * wait.as_secs()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-tel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn attr(s: &Value, k: &str) -> Value {
        s["attributes"].as_array().unwrap().iter().find(|a| a["key"] == k).map(|a| a["value"].clone()).unwrap_or(Value::Null)
    }

    /// A1: schema, ID, time, host, runtime, installation and version on every event.
    #[test]
    fn event_fields() {
        let s = span("agentq.run", "end", json!({"agentq.exit_code": 3, "agentq.retained": true}), 5_000_000_000, "inst", "h");
        for k in ["agentq.schema", "agentq.event_id", "agentq.stage", "agentq.installation", "host.name", "agentq.runtime", "agentq.version"] {
            assert!(!attr(&s, k).is_null(), "{k}");
        }
        assert_eq!(attr(&s, "agentq.schema"), json!({"intValue": SCHEMA}));
        assert_eq!(attr(&s, "agentq.retained"), json!({"boolValue": true}));
        assert_eq!(s["traceId"].as_str().unwrap().len(), 32);
        assert_eq!(s["spanId"].as_str().unwrap().len(), 16);
        assert_eq!(s["startTimeUnixNano"], "5000000000");
        assert_ne!(span("x", "end", json!({}), 0, "", "")["traceId"], span("x", "end", json!({}), 0, "", "")["traceId"]);
        let d = dir("inst");
        let id = emit(&d, "agentq.verify", "verify", json!({})).unwrap();
        emit(&d, "agentq.verify", "verify", json!({}));
        let e = entries(&queue(&d));
        assert_eq!(e.len(), 2);
        let first: Value = serde_json::from_str(&std::fs::read_to_string(&e.iter().find(|e| e.1.to_string_lossy().contains(&id)).unwrap().1).unwrap()).unwrap();
        assert_eq!(attr(&first, "agentq.installation")["stringValue"], std::fs::read_to_string(d.join("installation")).unwrap(), "one installation ID");
        let _ = std::fs::remove_dir_all(d);
    }

    /// Chain #134 A1: per-client routing with reason and intent, and a hook's last failure, reach the exported
    /// `agentq.installation` span, and the failure stays after the client's next successful run. Risk: a heartbeat that
    /// drops the failure before dashq Attention reads it, reports a corrupt retained failure as `ok`, exports routing only as a summary, or exports a proxy still
    /// binding at service start as an hour of `unknown` routing.
    #[test]
    fn heartbeat_carries_routing_and_hooks() {
        let d = dir("beat");
        crate::executor::hook_failed(&d, "codex", "Stop", 100, "stdin: stream did not contain valid UTF-8");
        let agents = json!({"codex": {"routing": "direct", "reason": "config.toml selects no agentq provider", "intended": "routed", "intended_reason": "onboarded"},
            "claude": {"routing": "direct", "reason": "no ANTHROPIC_BASE_URL", "intended": "direct", "intended_reason": "routing not shipped"}});
        let beat = |agents: &Value| span("agentq.installation", "heartbeat", heartbeat(&d, &d, 0, agents.clone()), 0, "inst", "h");
        // #141 A2: actual status -> heartbeat OTLP JSON preserves the source time and selection.
        let release = json!({"at": 0, "selected": "2.0.0", "running": "1.0.0", "coreq_version": "0.11.0", "lifecycle": "waiting_for_activation", "reason": "selection_changed"});
        std::fs::write(d.join("status.json"), json!({"release": release}).to_string()).unwrap();
        let s = beat(&agents);
        let projected: Value = serde_json::from_str(attr(&s, "agentq.release")["stringValue"].as_str().unwrap()).unwrap();
        assert_eq!(projected, crate::release::project(&release, 0));
        assert_eq!(projected["selected"], "2.0.0");
        let routing: Value = serde_json::from_str(attr(&s, "agentq.routing")["stringValue"].as_str().unwrap()).unwrap();
        assert_eq!(routing, agents);
        assert_eq!(attr(&s, "agentq.routing_problems")["stringValue"], "codex direct (config.toml selects no agentq provider)");
        let hooks = |s: &Value| serde_json::from_str::<Value>(attr(s, "agentq.hooks")["stringValue"].as_str().unwrap()).unwrap();
        let failure = json!({"at": 100, "error": "stdin: stream did not contain valid UTF-8"});
        assert_eq!(hooks(&s), json!([{"agent": "codex", "event": "Stop", "state": "failed", "last_ok_at": null, "last_failure": failure}]));
        crate::executor::stamp(&d, "codex", "Stop", 200);
        let s = beat(&json!({}));
        assert_eq!(hooks(&s), json!([{"agent": "codex", "event": "Stop", "state": "ok", "last_ok_at": 200, "last_failure": failure}]));
        assert_eq!((attr(&s, "agentq.hook_failures")["stringValue"].as_str(), attr(&s, "agentq.routing")["stringValue"].as_str()), (Some("none"), Some("{}")));
        // a corrupt retained failure after that success reads unknown, never ok
        std::fs::write(d.join("evidence").join("codex.Stop.last_failure"), "{").unwrap();
        let s = beat(&json!({}));
        assert_eq!(hooks(&s), json!([{"agent": "codex", "event": "Stop", "state": "unknown", "reason": "unreadable failure record", "last_ok_at": 200, "last_failure": {"at": null, "error": "unreadable failure record"}}]));
        // a present but unreadable success stamp after a failure reads unknown, never ok; the entry stays
        std::fs::write(d.join("evidence").join("codex.Stop.last_failure"), failure.to_string()).unwrap();
        std::fs::write(d.join("evidence").join("codex.Stop"), "{").unwrap();
        let s = beat(&json!({}));
        assert_eq!(hooks(&s), json!([{"agent": "codex", "event": "Stop", "state": "unknown", "reason": "unreadable success stamp", "last_ok_at": null, "last_failure": failure}]));
        // the start heartbeat waits for a binding proxy, at most START_WAIT; hourly ones never wait
        let binding = json!({"codex": {"routing": "unknown", "reason": "proxy on 127.0.0.1:1 not answering"}});
        assert!(start_waits(&binding, 0, 1000, 1000 + START_WAIT - 1));
        assert!(!start_waits(&binding, 0, 1000, 1000 + START_WAIT));
        assert!(!start_waits(&agents, 0, 1000, 1000));
        assert!(!start_waits(&binding, 1000, 1000, 1000 + HEARTBEAT));
        let _ = std::fs::remove_dir_all(d);
    }

    /// A2: the 7-day and 64 MiB caps drop oldest events first and count them.
    #[test]
    fn caps() {
        let d = dir("caps");
        let q = queue(&d);
        std::fs::create_dir_all(&q).unwrap();
        let now = 100 * 86400;
        std::fs::write(q.join(format!("{}-old.json", now - CAP_AGE - 1)), "{}").unwrap();
        let mb = vec![b' '; 1024 * 1024];
        for i in 0..65 {
            std::fs::write(q.join(format!("{}-{i:02}.json", now - 100 + i)), &mb).unwrap();
        }
        let left = prune(&q, now);
        assert_eq!(left.len(), 64, "64 MiB fit");
        assert!(left[0].1.ends_with(format!("{}-01.json", now - 99)), "the old one and the oldest MiB went first");
        assert_eq!(status(&d, now)["dropped"], 2);
        assert_eq!(status(&d, now)["pending"], 64);
        assert_eq!(status(&d, now)["oldest_age"], 99);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn keys() {
        assert!(Keys::parse("https://lf.example.com/ pk-lf-1 sk-lf-2").is_ok_and(|k| k.host == "https://lf.example.com"));
        assert!(Keys::parse("http://127.0.0.1:3000 pk sk").is_ok());
        for bad in ["http://lf.example.com pk sk", "http://localhost.evil.com pk sk", "https://h pk", "https://h pk s\"k", "https://h pk sk extra"] {
            assert!(Keys::parse(bad).is_err(), "{bad}");
        }
        let d = dir("keys");
        file_write(&d, "https://h pk sk").unwrap();
        file_write(&d, "https://h2 pk sk").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("langfuse")).unwrap(), "https://h2 pk sk", "rewrite truncates");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(d.join("langfuse")).unwrap().permissions()) & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(d);
    }

    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    /// Loopback Langfuse stand-in: answers 200 and records each request (head, body); a v2 observations read finds
    /// the span only when `ingested`.
    fn server() -> (String, Seen) {
        serve(false)
    }

    fn serve(ingested: bool) -> (String, Seen) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let got = Arc::new(Mutex::new(vec![]));
        let g = got.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut head, mut len) = (String::new(), 0);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; len];
                let _ = r.read_exact(&mut body);
                let id = head.split("traceId=").nth(1).and_then(|r| r.split(['&', ' ']).next()).unwrap_or("");
                let found = if ingested { format!(r#"{{"traceId":"{id}"}}"#) } else { String::new() };
                let answer = if head.starts_with("GET ") { format!(r#"{{"data":[{found}],"meta":{{}}}}"#) } else { String::new() };
                g.lock().unwrap().push((head, String::from_utf8_lossy(&body).into_owned()));
                let _ = (&s).write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}", answer.len()).as_bytes());
            }
        });
        (url, got)
    }

    #[test]
    fn malformed_queue_event_is_not_silently_deleted() {
        let d=dir("malformed-queue");std::fs::create_dir_all(queue(&d)).unwrap();
        let p=queue(&d).join(format!("{}-{}.json",crate::now(),"1".repeat(32)));std::fs::write(&p,"{bad").unwrap();
        let keys=Keys::parse("http://127.0.0.1:1 pk-lf-test sk-lf-test").unwrap();
        assert!(send(&d,&keys,crate::now()).unwrap_err().contains("malformed; file kept"));
        assert_eq!(std::fs::read_to_string(&p).unwrap(),"{bad");
        std::fs::write(&p,json!({"traceId":"1".repeat(32),"name":"agentq.verify","startTimeUnixNano":"unknown"}).to_string()).unwrap();
        assert!(send(&d,&keys,crate::now()).unwrap_err().contains("time unknown; file kept"));
        assert!(p.is_file());std::fs::remove_dir_all(d).unwrap();
    }

    /// A2: an outage keeps the queue and records the error; the next pass drains it in batches with Basic auth.
    #[test]
    fn outage_then_drain() {
        let d = dir("drain");
        for _ in 0..150 {
            emit(&d, "agentq.run", "end", json!({"agentq.command": "cargo test"})).unwrap();
        }
        let down = Keys::parse("http://127.0.0.1:1 pk-lf-test sk-lf-test").unwrap();
        crate::control::save(&d, "integration.json", &json!({"schema": 1, "generation": 1, "host_disabled": true, "profiles": {}})).unwrap();
        assert!(send(&d, &down, crate::now()).unwrap_err().contains("integration_disabled"));
        assert_eq!(status(&d, crate::now())["pending"], 150, "host disable retains every queued event");
        std::fs::remove_file(d.join("integration.json")).unwrap();
        assert!(send(&d, &down, crate::now()).is_err());
        let s = status(&d, crate::now());
        assert_eq!(s["pending"], 150, "an outage keeps every event");
        assert!(s["error"].as_str().unwrap().starts_with("Langfuse:"));
        assert!(s["last_delivery"].is_null());

        let (url, got) = server();
        let up = Keys::parse(&format!("{url} pk-lf-test sk-lf-test")).unwrap();
        assert_eq!(send(&d, &up, crate::now()), Ok(150));
        let s = status(&d, crate::now());
        assert_eq!((s["pending"].clone(), s["error"].clone()), (json!(0), Value::Null));
        assert!(s["last_delivery"].is_u64());
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 2, "batches of 100");
        assert!(got[0].0.starts_with("POST /api/public/otel/v1/traces "));
        assert!(got[0].0.contains("Basic cGstbGYtdGVzdDpzay1sZi10ZXN0"), "Basic pk-lf-test:sk-lf-test");
        let archived=crate::local::export(&d).unwrap();
        assert_eq!(archived["events"].as_array().unwrap().len(),150,"successful remote delivery must not delete local event history");
        let spans = |b: &str| serde_json::from_str::<Value>(b).unwrap()["resourceSpans"][0]["scopeSpans"][0]["spans"].as_array().unwrap().len();
        assert_eq!(spans(&got[0].1) + spans(&got[1].1), 150);
        assert!(!got[0].1.contains("sk-lf-test"), "no secret in the payload");
        let _ = std::fs::remove_dir_all(d);
    }

    /// #64 A2: a read-back miss names the query and Langfuse's answer, not a bare timeout.
    #[test]
    fn verify_names_the_query() {
        let d = dir("verify");
        let (url, got) = server();
        let k = Keys::parse(&format!("{url} pk sk")).unwrap();
        let e = verify(&d, &k, 2, Duration::ZERO).unwrap_err();
        assert!(e.contains(&format!("GET {url}/api/public/v2/observations?traceId=")) && e.contains("no observation yet"), "{e}");
        assert_eq!(got.lock().unwrap().iter().filter(|r| r.0.starts_with("GET /api/public/v2/observations?traceId=")).count(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    /// #80: Langfuse v4 events-only servers drop `/api/public/traces`; verify reads the span from v2 observations.
    #[test]
    fn verify_reads_v2_observations() {
        let d = dir("verify-v2");
        let (url, _) = serve(true);
        let k = Keys::parse(&format!("{url} pk sk")).unwrap();
        let id = verify(&d, &k, 1, Duration::ZERO).unwrap();
        assert_eq!(id.len(), 32);
        let _ = std::fs::remove_dir_all(d);
    }
}
