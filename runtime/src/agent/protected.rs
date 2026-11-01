//! M12.8: optional host adapter; requests and receipts are match requirements, never authority.
use super::*;
use crate::optimization::{self, Intent, Optimization};
use coreq::process::Identity;

const SCHEMA: &str = "taskq.app-server/1";

/// Operator-selected immutable adapter. Never build this trust anchor from task text or a receipt.
#[derive(Clone)]
pub struct ProtectedHost { executable: PathBuf, sha256: String }

/// Expected operator/native identity, obtained independently of this operation's receipt.
#[derive(Clone)]
pub struct ProtectedIdentity {
    pub profile_sha256: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub account_home: PathBuf,
    pub approver: String,
}

impl ProtectedHost {
    pub fn taskq(executable: &Path, sha256: &str) -> Res<Self> {
        canonical(executable)?;
        if !hash(sha256) { return Err("protected adapter digest required".into()); }
        let host = Self { executable: executable.into(), sha256: sha256.into() };
        host.checked()?;
        Ok(host)
    }

    fn checked(&self) -> Res<()> {
        canonical(&self.executable)?;
        if file_hash(&self.executable)? != self.sha256 { return Err("protected adapter artifact changed; authority unknown".into()); }
        Ok(())
    }

    fn command(&self, action: &str, request: &Value) -> Res<Command> {
        validate_request(request)?;
        self.checked()?;
        let mut command = Command::new(&self.executable);
        command.args(["service", "app-server", "--action", action, "--request", &request.to_string()]);
        Ok(command)
    }

    fn read(&self, action: &str, request: &Value, until: Instant) -> Res<Value> {
        let mut command = self.command(action, request)?;
        let budget = until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
            .ok_or("protected readback deadline; authority unknown")?.min(Duration::from_secs(5));
        let output = coreq::process::output("agent.protected.readback", &mut command, budget)
            .map_err(|e| format!("protected {action} readback unknown: {}", e.class()))?;
        serde_json::from_slice(&output.stdout).map_err(|_| "protected readback invalid; authority unknown".into())
    }

    /// Recovery readback only. A copied receipt cannot be supplied to a transport to authorize input.
    pub fn observe(&self, request: &Value, until: Instant) -> Res<Value> { self.read("observe", request, until) }

    /// Exact host cancellation, never slot release or proof of whole-turn completion.
    pub fn cancel(&self, request: &Value, until: Instant) -> Res<Value> { self.read("cancel", request, until) }

    /// Persist the original operation before calling. Even an error is uncertain; never retry launch.
    pub fn launch(&self, request: Value, expected: ProtectedIdentity, intent: Intent, private_log: &Path) -> Res<ProtectedAppServer> {
        validate_expected(&expected)?;
        let mut command = self.command("launch", &request)?;
        let owner = identity(std::process::id())?;
        if native(&request["slot"]["binding"])? != owner { return Err("protected slot belongs to another consumer incarnation".into()); }
        // Inherited off wins, exactly as in ordinary runtime launches. No caller environment is forwarded.
        let intent = if std::env::var_os(optimization::ENV).is_some_and(|v| v == "off") { Intent::Off } else { intent };
        command.env(optimization::ENV, intent.code());
        let server = AppServer::spawn(command, private_log)?;
        Ok(ProtectedAppServer { host: self.clone(), request, expected, server, binding: None, fault: None, intent })
    }
}

/// A gated transport; the underlying unprotected transport is deliberately not exposed.
pub struct ProtectedAppServer {
    host: ProtectedHost,
    request: Value,
    expected: ProtectedIdentity,
    server: AppServer,
    binding: Option<Value>,
    fault: Option<String>,
    intent: Intent,
}

impl ProtectedAppServer {
    /// Supervisor PID, not the native child's PID. Native incarnation comes from verified readback.
    pub fn id(&self) -> u32 { self.server.id() }

    pub fn optimization(&self) -> Optimization {
        optimization::observe(self.intent, Client::Codex, optimization::install_root().as_deref(), Some(&self.expected.account_home))
    }

    /// No generating request is needed to verify authority. Failed readback never clears an earlier binding.
    pub fn verify(&mut self, until: Instant) -> Res<Value> {
        if let Some(reason) = &self.fault { return Err(reason.clone()); }
        if self.server.poll()?.is_some() { return Err("protected supervisor exited; retain operation".into()); }
        let receipt = self.host.observe(&self.request, until)?;
        let binding = validate_receipt(&receipt, &self.request, &self.expected, &self.host, self.id())?;
        if self.server.poll()?.is_some() { return Err("protected supervisor exited during readback; retain operation".into()); }
        if self.binding.as_ref().is_some_and(|old| old != &binding) {
            let reason = "protected launch binding changed; input fenced, reconcile original operation".to_string();
            self.fault = Some(reason.clone());
            return Err(reason);
        }
        self.binding = Some(binding);
        Ok(receipt)
    }

    /// The host repeats its authority check at actual delivery. An uncertain write permanently fences input.
    pub fn send(&mut self, message: &Value, until: Instant) -> Res<()> {
        self.verify(until)?;
        let result = self.server.send(message, until);
        if result.is_err() { self.fault = Some("protected input delivery unknown; reconcile without replay".into()); }
        result
    }

    pub fn receive(&mut self, until: Instant) -> Res<Option<Value>> { self.server.receive(until) }
    pub fn poll(&mut self) -> Res<Option<i32>> { self.server.poll() }
    pub fn cancel(&self, until: Instant) -> Res<Value> { self.host.cancel(&self.request, until) }
}

fn fields(value: &Value, names: &[&str]) -> bool {
    value.as_object().is_some_and(|o| o.len() == names.len() && names.iter().all(|n| o.contains_key(*n)))
}
fn nonempty(value: &Value) -> bool { value.as_str().is_some_and(|s| !s.trim().is_empty()) }
fn hash(value: &str) -> bool { value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) }
fn canonical(path: &Path) -> Res<()> {
    if !path.is_absolute() || path.canonicalize().ok().as_deref() != Some(path) { return Err("protected canonical path identity unknown".into()); }
    Ok(())
}
fn file_hash(path: &Path) -> Res<String> {
    let mut file = File::open(path).map_err(|_| "protected executable unreadable")?;
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|_| "protected executable read unknown")?;
        if n == 0 { break; }
        hash.update(&bytes[..n]);
    }
    Ok(hash.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}
fn native(value: &Value) -> Res<Identity> {
    Ok(Identity { pid: value["pid"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0 && *n <= i32::MAX as u32)
        .ok_or("protected process identity unknown")?, started: value["started"].as_str().filter(|s| !s.is_empty())
        .ok_or("protected process creation unknown")?.into() })
}
fn identity(pid: u32) -> Res<Identity> {
    coreq::process::identity(pid).map_err(|_| "protected process liveness unknown")?.ok_or_else(|| "protected process absent; retain operation".into())
}
fn validate_request(r: &Value) -> Res<()> {
    let slot = &r["slot"]; let b = &slot["binding"];
    if !fields(r, &["schema", "slot", "policy_revision", "account"]) || r["schema"] != SCHEMA
        || r["policy_revision"].as_u64().is_none_or(|n| n == 0) || !nonempty(&r["account"])
        || !fields(slot, &["binding", "key"]) || !slot["key"].as_str().is_some_and(hash)
        || !fields(b, &["schema", "caller", "op_id", "root", "runtime", "pid", "started", "boot", "expires_at"])
        || b["schema"] != "taskq.slots/1" || b["runtime"] != "codex" || !nonempty(&b["caller"]) || !nonempty(&b["op_id"])
        || b["boot"].as_i64().is_none_or(|n| n <= 0) || b["expires_at"].as_u64().is_none_or(|n| n == 0)
        || r.to_string().len() > 16384 { return Err("protected request invalid; no native launch".into()); }
    native(b)?;
    canonical(Path::new(b["root"].as_str().ok_or("protected project absent")?))
}
fn validate_expected(e: &ProtectedIdentity) -> Res<()> {
    canonical(&e.account_home)?; canonical(&e.executable)?;
    if !hash(&e.profile_sha256) || !hash(&e.executable_sha256) || e.approver.trim().is_empty() || e.approver == "unknown" {
        return Err("protected expected authority incomplete".into());
    }
    if file_hash(&e.executable)? != e.executable_sha256 { return Err("protected native artifact changed".into()); }
    Ok(())
}
fn validate_receipt(r: &Value, request: &Value, expected: &ProtectedIdentity, host: &ProtectedHost, supervisor: u32) -> Res<Value> {
    validate_expected(expected)?;
    let launch = &r["launch"]; let a = &launch["authority"]; let b = &request["slot"]["binding"];
    let effective = &r["verified_effective"];
    if r["schema"] != SCHEMA || r["state"] != "verified-effective" || r["caller"] != b["caller"] || r["operation"] != b["op_id"]
        || launch["schema"] != SCHEMA || launch["state"] != "verified-effective" || launch["cancel_requested"] != false
        || launch["policy_revision"] != request["policy_revision"] || launch["account"] != request["account"]
        || !nonempty(&launch["host_instance"]) || !launch["reason"].is_null()
        || a["project"] != b["root"] || a["runtime"] != "codex" || a["account"] != request["account"]
        || a["account_home"] != json!(expected.account_home) || a["profile_sha256"] != expected.profile_sha256
        || a["executable"] != json!(expected.executable) || a["executable_sha256"] != expected.executable_sha256
        || a["adapter"]["executable"] != json!(host.executable) || a["adapter"]["sha256"] != host.sha256
        || a["saved"]["revision"] != request["policy_revision"] || a["saved"]["grant"]["by"] != expected.approver
        || a["saved"]["grant"]["mode"] != "workspace" || a["saved"]["grant"]["runtime"] != "codex"
        || a["saved"]["grant"]["project"] != b["root"] || a["saved"]["grant"]["at"].as_u64().is_none_or(|n| n == 0)
        || !nonempty(&a["requested"]) || a["resolved"]["effective"] != "workspace"
        || effective["state"] != "verified-effective" || !nonempty(&effective["scope"])
        || effective["workspace_write"] != true || !effective["protected_write_create_unlink_denied"].as_array().is_some_and(|v| !v.is_empty())
        || launch["verified_effective"] != *effective { return Err("protected authority missing, unknown or mismatched; no native input".into()); }
    let owner = native(&launch["supervisor"])?; let child = native(&launch["native"])?;
    if owner.pid != supervisor || owner == child || identity(owner.pid)? != owner || identity(child.pid)? != child
        || identity(b["pid"].as_u64().ok_or("consumer identity absent")? as u32)? != native(b)? {
        return Err("protected native/supervisor/consumer incarnation changed; no native input".into());
    }
    Ok(json!({"authority": a, "supervisor": launch["supervisor"], "native": launch["native"], "host_instance": launch["host_instance"], "verified_effective": effective}))
}
