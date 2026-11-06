//! `agentq agent turn` (agentq.md § 12, docs/spec/RUNTIME.md § JSON command): one turn from a JSON request, for
//! callers that are not Rust. Same `Spec`, `run` and `Report` as the library.
use crate::agent::{self, Client, Context, Report, Spec};
use crate::optimization::{self, Intent};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

fn text<'a>(v: &'a Value, k: &str) -> Res<&'a str> { v[k].as_str().filter(|s| !s.is_empty()).ok_or(format!("{k} requires a non-empty string")) }

fn path(v: &Value, k: &str) -> Res<Option<PathBuf>> {
    match &v[k] {
        Value::Null => Ok(None),
        Value::String(s) if Path::new(s).is_absolute() => Ok(Some(PathBuf::from(s))),
        _ => Err(format!("{k} requires an absolute path or null")),
    }
}

fn strings(v: &Value, k: &str) -> Res<Vec<String>> {
    match &v[k] {
        Value::Null => Ok(vec![]),
        Value::Array(a) => a.iter().map(|s| s.as_str().map(String::from).ok_or(format!("{k} requires strings"))).collect(),
        _ => Err(format!("{k} requires an array of strings")),
    }
}

/// Explicit ownership import for callers outside Rust; never a launch or a login operation.
pub fn adopt(request:&str)->Res<Value> {
    let v:Value=serde_json::from_str(request).map_err(|e|format!("invalid request: {e}"))?;
    let runtime=agent::runtime(text(&v,"client")?).ok_or("unknown client")?;
    let native:Vec<agent::NativeProcess>=if v["known_native"].is_null() {vec![]} else {serde_json::from_value(v["known_native"].clone()).map_err(|e|format!("invalid known_native: {e}"))?};
    serde_json::to_value(runtime.adopt(text(&v,"session")?,&preflight_launch(&v)?,&native)?).map_err(|e|e.to_string())
}

/// Read launch evidence without a model call. Repair requires the caller's existing ownership journal.
pub fn preflight(request: &str, repair: bool, journal: Option<&Path>) -> Res<Value> {
    let v:Value=serde_json::from_str(request).map_err(|e|format!("invalid request: {e}"))?;
    let runtime=agent::runtime(text(&v,"client")?).ok_or("unknown client")?;
    let launch=preflight_launch(&v)?;
    let repair=agent::Repair {workspace:repair,hooks:repair,journal:journal.map(Path::to_path_buf)};
    let mut readiness=runtime.preflight(&launch,&repair);
    native_arguments_readiness(&mut readiness,&v)?;
    Ok(json!({"readiness":readiness,"allowed":readiness.permit(launch.readiness).is_ok()}))
}

fn native_arguments_readiness(readiness: &mut agent::Readiness,v:&Value)->Res<()> {
    if !strings(v,"args")?.is_empty() {
        readiness.checks[4]=agent::LaunchCheck {check:"permissions".into(),state:"unknown".into(),cause:Some("native-policy-unchecked".into()),detail:"Native arguments remain caller-owned".into(),next:"use Runtime::launch for an abstract policy".into()};
    }
    Ok(())
}

fn preflight_launch(v: &Value) -> Res<agent::Launch> {
    let mut launch=agent::Launch::new(path(v,"root")?.ok_or("root requires an absolute path")?,path(v,"log")?.ok_or("log requires an absolute path")?,v["input"].as_str().unwrap_or(""),"agentq");
    if !v["policy"].is_null() {launch.policy=serde_json::from_value(v["policy"].clone()).map_err(|e|format!("invalid policy: {e}"))?;}
    if let Some(store)=path(v,"session_store")? {launch.session_store=agent::SessionStore::new(store);}
    launch.home=path(v,"home")?;launch.store=path(v,"store")?;
    launch.model=v["model"].as_str().map(String::from);
    launch.command=strings(v,"command")?.into_iter().map(OsString::from).collect();
    launch.env=match &v["env"] {Value::Null=>vec![],Value::Object(env)=>env.iter().map(|(k,v)|Ok((k.into(),OsString::from(v.as_str().ok_or("env values require strings")?)))).collect::<Res<_>>()?,_=>return Err("env requires an object".into())};
    launch.readiness=match v["readiness"].as_str() {None|Some("continue")=>agent::ReadinessMode::Continue,Some("strict")=>agent::ReadinessMode::Strict,_=>return Err("readiness requires continue or strict".into())};
    launch.mcp=path(v,"mcp")?;launch.instructions=path(v,"instructions")?;
    Ok(launch)
}

/// Run one turn from `request` and return its JSON report. A request error is `Err`; a failed turn is a report.
/// `install` is the AgentQ installation the optimization state is read from (None: the default one).
pub fn turn(request: &str, install: Option<&Path>) -> Res<Value> {
    let v: Value = serde_json::from_str(request).map_err(|e| format!("invalid request: {e}"))?;
    if !v.is_object() { return Err("request requires a JSON object".into()); }
    let client = Client::parse(text(&v, "client")?).ok_or("client requires claude, codex or antigravity")?;
    let root = path(&v, "root")?.ok_or("root requires an absolute path")?;
    let log = path(&v, "log")?.ok_or("log requires an absolute path")?;
    let resume = match &v["resume"] { Value::Null => false, Value::Bool(b) => *b, _ => return Err("resume requires a boolean".into()) };
    let session = match &v["session"] { Value::Null if !resume => agent::new_session(client), Value::String(s) if !s.is_empty() => s.clone(), _ => return Err("session requires a string; a resume names it".into()) };
    let env = match &v["env"] {
        Value::Null => vec![],
        Value::Object(m) => m.iter().map(|(k, s)| s.as_str().map(|s| (OsString::from(k), OsString::from(s))).ok_or("env values require strings")).collect::<Result<_, _>>()?,
        _ => return Err("env requires an object of strings".into()),
    };
    let intent = match &v["optimization"] { Value::Null => Intent::Inherit, Value::String(s) => Intent::parse(s).ok_or("optimization requires inherit, off or on")?, _ => return Err("optimization requires inherit, off or on".into()) };
    let input = v["input"].as_str().ok_or("input requires a string")?;
    let deadline = match &v["timeout_s"] { Value::Null => None, t => Some(Instant::now() + Duration::from_secs(t.as_u64().filter(|s| (1..=86_400).contains(s)).ok_or("timeout_s requires 1-86400")?)) };
    let c = &v["context"];
    let task = match &c["task"] { Value::Null => None, Value::Array(a) if a.len() == 2 => Some((a[0].as_u64().ok_or("context.task requires [task, attempt]")?, a[1].as_u64().ok_or("context.task requires [task, attempt]")?)), _ => return Err("context.task requires [task, attempt] or null".into()) };
    let ctx = Context { product: text(c, "product")?, version: text(c, "version")?, project: c["project"].as_str().unwrap_or("other"),
        task, route: c["route"].as_str(), account: c["account"].as_str().unwrap_or(agent::DEFAULT) };
    let store = path(&v, "store")?;
    // The caller's base command (a sandbox wrapper or another executable), else the client's own executable.
    let base = match strings(&v, "command")?.split_first() {
        Some((exe, rest)) => { let mut c = Command::new(exe); c.args(rest); c }
        None => Command::new(agent::runtime(client.code()).ok_or("unknown runtime")?.executable(env.iter().rev().find(|(k,_)|k=="PATH").map(|(_,v)|v.as_os_str()))?),
    };
    let mut spec = Spec { client, root, session, resume, args: strings(&v, "args")?.into_iter().map(OsString::from).collect(), home: path(&v, "home")?, env };
    agent::registry::validate_native_args(client,&spec.args)?;
    let optimization = optimization::prepare(intent, ctx.product, &mut spec, install.map(Path::to_path_buf).or_else(optimization::install_root).as_deref());
    let launch=preflight_launch(&v)?;
    if launch.policy!=agent::Policy::default() {return Err("native JSON turn arguments require the default policy; use Runtime::launch for abstract policies".into());}
    let mut readiness=agent::runtime(client.code()).ok_or("unknown runtime")?.preflight(&launch,&agent::Repair::default());
    native_arguments_readiness(&mut readiness,&v)?;
    readiness.permit(launch.readiness)?;
    let owner=agent::runtime(client.code()).ok_or("unknown runtime")?.session_owner(resume.then_some(spec.session.as_str()),&launch)?;
    let report = agent::with_environment_caller(|| match &store {
        Some(p) => agent::run(base, &spec, &log, input, deadline, &ctx, &crate::telemetry::Store::open(p)?, readiness.clone(),owner.clone()),
        None => { let mut t = agent::Turn::start(base, &spec, &log, input, &owner.process_hooks(&agent::Process::default()))?.owned(owner.clone()).prepared(readiness.clone()); t.wait(deadline)?; Ok(t.report()) }
    })?;
    let mut out = report_json(&report);
    out["readiness"]=json!(readiness);
    out["recorded"] = match (&store, &report.recorded) { (None, _) => json!("not-requested"), (Some(_), Ok(())) => json!("ok"), (Some(_), Err(e)) => json!(e) };
    out["optimization"] = optimization.json();
    Ok(out)
}

/// The report as JSON: codes, numbers and the final text; every null in `usage` and `limit` has its `unknown` reason.
pub fn report_json(r: &Report) -> Value {
    let u = &r.usage;
    json!({"schema": 1, "readiness": r.readiness, "owner":r.owner, "session": r.session, "outcome": r.outcome, "error": r.error, "reason": r.reason, "detail": r.detail, "text": r.text, "exit": r.exit, "termination": r.termination,
        "started_ms": r.started_ms, "duration_us": r.duration_us,
        "usage": {"calls": u.calls, "input": u.input, "cache_read": u.cache_read, "cache_write": u.cache_write, "output": u.output, "reasoning": u.reasoning, "model": u.model, "unknown": u.unknown},
        "process": {"user_cpu_us": r.process.user_cpu_us, "system_cpu_us": r.process.system_cpu_us, "peak_rss_bytes": r.process.peak_rss_bytes, "unknown": r.process.unknown},
        "limit": {"used_percent": r.limit.used_percent, "resets_at": r.limit.resets_at, "window": r.limit.window, "unknown": r.limit.unknown}, "limits": r.limits,
        "background_job": r.background_job, "live_background": r.live_background, "reached": r.reached.map(|reset| json!({"reset": reset}))})
}
