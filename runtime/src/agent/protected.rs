//! M12.8: optional host adapter; requests and receipts are match requirements, never authority.
use super::*;
use crate::optimization::{self, Intent, Optimization};
use coreq::process::Identity;
use super::app::AppServer;

const SCHEMA: &str = "taskq.app-server/1";

/// Operator-selected immutable adapter. Never build this trust anchor from task text or a receipt.
#[derive(Clone)]
pub struct ProtectedHost { executable: PathBuf, sha256: String, signature: Option<(String, String)> }

/// Expected operator/native identity, obtained independently of this operation's receipt.
#[derive(Clone)]
pub struct ProtectedIdentity {
    pub profile_sha256: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub account_home: PathBuf,
    pub approver: String,
}

/// Private independent preparation, not a grant. Persist before launch, never rebuild from observation.
#[derive(Clone)]
pub struct ProtectedPreparation { request: Value, prepared: Value, expected: ProtectedIdentity, receipt: Value }

impl ProtectedPreparation {
    pub fn request(&self) -> &Value { &self.request }
    pub fn checkpoint(&self) -> Value { json!({if self.request["schema"] == SCHEMA { "slot" } else { "permit" }:permit(&self.request),"receipt":self.receipt}) }
}

impl ProtectedHost {
    pub fn new(executable: &Path, sha256: &str) -> Res<Self> {
        canonical(executable)?;
        if !hash(sha256) { return Err("protected adapter digest required".into()); }
        let host = Self { executable: executable.into(), sha256: sha256.into(), signature: None };
        host.checked()?;
        Ok(host)
    }

    /// Independent adapter trust is pinned by the operator, never learned from a receipt.
    pub fn signed(executable: &Path, sha256: &str, public_key: &str, signature: &str) -> Res<Self> {
        let host = Self { executable: executable.into(), sha256: sha256.into(), signature: Some((public_key.into(), signature.into())) };
        host.checked()?;
        Ok(host)
    }

    fn schema(&self) -> &'static str { if self.signature.is_some() { "agentq.app-server/1" } else { SCHEMA } }
    fn permit_field(&self) -> &'static str { if self.signature.is_some() { "permit" } else { "slot" } }

    fn checked(&self) -> Res<()> {
        canonical(&self.executable)?;
        if file_hash(&self.executable)? != self.sha256 { return Err("protected adapter artifact changed; authority unknown".into()); }
        if let Some((key, signature)) = &self.signature {
            super::host::verify_signature(key, signature, "agentq.protected-adapter/1", self.sha256.as_bytes())?;
        }
        Ok(())
    }

    fn command(&self, action: &str, request: &Value) -> Res<Command> {
        if action == "prepare" {
            if !fields(request, &["schema", self.permit_field()]) || request["schema"] != self.schema() { return Err("protected preparation request invalid".into()); }
            validate_slot(permit(request))?;
        } else { validate_request(request)?; }
        if request["schema"] != self.schema() { return Err("protected protocol mismatch; no fallback".into()); }
        self.checked()?;
        let mut command = Command::new(&self.executable);
        if self.signature.is_some() { command.args(["agent", "host"]); } else { command.args(["service", "app-server"]); }
        command.args(["--action", action, "--request", &request.to_string()]);
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

    /// Reconcile operator-authenticated retained bytes through the independent host, never the old runtime.
    pub fn reconcile_retained(&self, checkpoint: &str, receipt: &str, until: Instant) -> Res<Value> {
        self.retained("retained",checkpoint,receipt,until)
    }

    /// Exact native cancellation using the original authenticated retained identity; never completion or release.
    pub fn cancel_retained(&self, checkpoint: &str, receipt: &str, until: Instant) -> Res<Value> {
        self.retained("retained-cancel",checkpoint,receipt,until)
    }

    fn retained(&self, action:&str, checkpoint:&str, receipt:&str, until:Instant) -> Res<Value> {
        if self.signature.is_none() || checkpoint.len()>1024*1024 || receipt.len()>1024*1024 {return Err("signed retained recovery required".into());}
        self.checked()?;
        let request=json!({"checkpoint":checkpoint,"receipt":receipt});
        let mut command=Command::new(&self.executable);
        command.args(["agent","host","--action",action,"--request",&request.to_string()]);
        let budget=until.checked_duration_since(Instant::now()).filter(|d|!d.is_zero()).ok_or("retained recovery deadline")?.min(Duration::from_secs(5));
        let output=coreq::process::output("agent.protected.retained",&mut command,budget).map_err(|_|"retained recovery unknown; occupancy retained")?;
        let answer:Value=serde_json::from_slice(&output.stdout).map_err(|_|"retained recovery invalid")?;
        if answer["schema"]!="agentq.retained-operation/1"||answer["checkpoint"]!=checkpoint||answer["receipt"]!=receipt
            || !matches!(answer["state"].as_str(),Some("unknown"|"completed"))
            || answer["state"]=="completed"&&answer["cleanup"]["cleanup_verified"]!=true {return Err("retained recovery binding changed; occupancy retained".into());}
        Ok(answer)
    }

    /// Exact host cancellation, never slot release or proof of whole-turn completion.
    pub fn cancel(&self, request: &Value, until: Instant) -> Res<Value> { self.read("cancel", request, until) }

    /// Non-launching host resolution of existing authority for the actual reserved consumer incarnation.
    pub fn prepare(&self, slot: &Value, until: Instant) -> Res<ProtectedPreparation> {
        validate_slot(slot)?;
        if native(&slot["binding"])? != identity(std::process::id())? { return Err("protected preparation belongs to another consumer incarnation".into()); }
        let receipt = self.read("prepare", &json!({"schema":self.schema(),self.permit_field():slot}), until)?;
        let preparation = self.recover_preparation(&json!({self.permit_field():slot,"receipt":receipt}))?;
        validate_expected(&preparation.expected)?;
        Ok(preparation)
    }

    /// Restore only a caller-retained independent checkpoint. No dispatch or current-caller requirement.
    pub fn recover_preparation(&self, checkpoint: &Value) -> Res<ProtectedPreparation> {
        self.checked()?;
        let receipt = &checkpoint["receipt"]; let prepared = &receipt["prepared"]; let a = &prepared["authority"];
        if !fields(checkpoint, &[self.permit_field(), "receipt"]) || receipt["schema"] != self.schema() || receipt["state"] != "prepared"
            || !fields(prepared, &["schema", "binding", "authority", "host_instance"])
            || prepared["schema"] != preparation_schema(self.schema()) || prepared["binding"] != permit(checkpoint)["binding"]
            || !nonempty(&prepared["host_instance"]) || !receipt["preparation"].as_str().is_some_and(hash)
            || receipt["preparation"] != super::host::digest(prepared.to_string().as_bytes()) {
            return Err("protected independent preparation missing or mismatched".into());
        }
        let request = json!({"schema":self.schema(),self.permit_field():permit(checkpoint),"policy_revision":receipt["policy_revision"],
            "account":receipt["account"],"preparation":receipt["preparation"]});
        validate_request(&request)?;
        let path = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(PathBuf::from).ok_or("protected prepared path unknown");
        let expected = ProtectedIdentity { profile_sha256:a["profile_sha256"].as_str().unwrap_or("").into(),
            executable:path(&a["executable"])?, executable_sha256:a["executable_sha256"].as_str().unwrap_or("").into(),
            account_home:path(&a["account_home"])?, approver:a["saved"]["grant"]["by"].as_str().unwrap_or("").into() };
        if !hash(&expected.profile_sha256) || !hash(&expected.executable_sha256) || expected.approver.trim().is_empty()
            || expected.approver == "unknown" || !expected.executable.is_absolute() || !expected.account_home.is_absolute() {
            return Err("protected prepared authority incomplete".into());
        }
        validate_authority(a, &request, &expected, self)?;
        Ok(ProtectedPreparation { request, prepared:prepared.clone(), expected, receipt:receipt.clone() })
    }

    /// Persist checkpoint first. Errors retain the original operation; never retry native launch.
    pub fn launch_prepared(&self, preparation: &ProtectedPreparation, intent: Intent, private_log: &Path) -> Res<ProtectedAppServer> {
        let preparation = self.recover_preparation(&preparation.checkpoint())?;
        let mut server = self.launch(preparation.request.clone(), preparation.expected.clone(), intent, private_log)?;
        server.prepared = Some(preparation.prepared);
        Ok(server)
    }

    /// Fresh same-operation host reconciliation only; never releases capacity or proves external effects.
    pub fn reconcile(&self, preparation: &ProtectedPreparation, until: Instant) -> Res<Value> {
        let preparation = self.recover_preparation(&preparation.checkpoint())?;
        let receipt = self.read("reconcile", &preparation.request, until)?;
        validate_completion(&receipt, &preparation)?;
        Ok(receipt)
    }

    /// Persist the original operation before calling. Even an error is uncertain; never retry launch.
    pub fn launch(&self, request: Value, expected: ProtectedIdentity, intent: Intent, private_log: &Path) -> Res<ProtectedAppServer> {
        validate_expected(&expected)?;
        let mut command = self.command("launch", &request)?;
        let owner = identity(std::process::id())?;
        if native(&permit(&request)["binding"])? != owner { return Err("protected slot belongs to another consumer incarnation".into()); }
        // Inherited off wins, exactly as in ordinary runtime launches; only validated CoreQ metadata crosses the host.
        let intent = if std::env::var_os(optimization::ENV).is_some_and(|v| v == "off") { Intent::Off } else { intent };
        command.env(optimization::ENV, intent.code());
        let server = AppServer::spawn(command, private_log, &super::Process::default())?;
        Ok(ProtectedAppServer { host: self.clone(), request, expected, server, binding: None, fault: None, intent, prepared: None })
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
    prepared: Option<Value>,
}

impl ProtectedAppServer {
    /// Adapter frontend PID; legacy adapters also supervise. Scope/native identities come from readback.
    pub fn id(&self) -> u32 { self.server.id() }

    pub fn optimization(&self) -> Optimization {
        optimization::observe(self.intent, Client::Codex, optimization::install_root().as_deref(), Some(&self.expected.account_home))
    }

    /// No generating request is needed to verify authority. Failed readback never clears an earlier binding.
    pub fn verify(&mut self, until: Instant) -> Res<Value> {
        if let Some(reason) = &self.fault { return Err(reason.clone()); }
        if self.server.poll()?.is_some() { return Err("protected frontend exited; retain operation".into()); }
        let receipt = self.host.observe(&self.request, until)?;
        let binding = validate_receipt(&receipt, &self.request, &self.expected, &self.host, self.id())?;
        if self.binding.as_ref().is_some_and(|old| old != &binding) {
            let reason = "protected launch binding changed; input fenced, reconcile original operation".to_string();
            self.fault = Some(reason.clone());
            return Err(reason);
        }
        if let Some(prepared) = &self.prepared {
            let launch = &receipt["launch"]; let scope = &launch["scope"];
            if launch["authority"] != prepared["authority"] || launch["host_instance"] != prepared["host_instance"]
                || launch["frontend"].is_null() || launch["input_closed"] != false || scope["dispatch_ack"] != true
                || !scope_binding(&scope["binding"], &permit(&self.request)["binding"]) {
                return Err("protected prepared launch binding changed; no native input".into());
            }
        }
        if self.server.poll()?.is_some() { return Err("protected frontend exited during readback; retain operation".into()); }
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

    /// Open this verified frontend as a Codex app-server `Run` (`Launch.mode` `AppServer`); `session` resumes a thread.
    /// Every later write repeats the authority check; a failed handshake leaves the frontend with its host.
    pub fn open(self, session: Option<&str>, launch: &super::Launch) -> Res<super::Run> { super::adapter::open_protected(self, session, launch) }

    /// Protected-host protocol only: take pending server requests, preserving their original native IDs.
    pub fn requests(run: &mut super::Run) -> Res<Vec<Value>> { super::adapter::protected_requests(run) }

    /// Reply only to a protected host request; every write retains the host's authority verification.
    pub fn reply(run: &mut super::Run, reply: &Value, until: Instant) -> Res<()> { super::adapter::protected_reply(run, reply, until) }
}

pub(super) fn permit(r: &Value) -> &Value { r.get("permit").unwrap_or(&r["slot"]) }
fn preparation_schema(schema: &str) -> &'static str { if schema == SCHEMA { "taskq.app-server-preparation/1" } else { "agentq.app-server-preparation/1" } }
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
    let field = if r["schema"] == SCHEMA { "slot" } else { "permit" };
    if !(fields(r, &["schema", field, "policy_revision", "account"])
        || fields(r, &["schema", field, "policy_revision", "account", "preparation"]) && r["preparation"].as_str().is_some_and(hash)) || (r["schema"] != SCHEMA && r["schema"] != "agentq.app-server/1")
        || r["policy_revision"].as_u64().is_none_or(|n| n == 0) || !nonempty(&r["account"])
        || r.to_string().len() > 16384 { return Err("protected request invalid; no native launch".into()); }
    validate_slot(permit(r))
}
pub(super) fn validate_slot(slot: &Value) -> Res<()> {
    let b = &slot["binding"];
    if !(fields(slot, &["binding", "key"]) && b["schema"] == "taskq.slots/1"
        || fields(slot, &["binding", "key", "signature"]) && b["schema"] == "pipe.permits/1" && slot["signature"].as_str().is_some_and(|s| super::host::hex(s, 64).is_ok())) || !slot["key"].as_str().is_some_and(hash)
        || !fields(b, &["schema", "caller", "op_id", "root", "runtime", "pid", "started", "boot", "expires_at"])
        || b["runtime"] != "codex" || !nonempty(&b["caller"]) || !nonempty(&b["op_id"])
        || b["boot"].as_i64().is_none_or(|n| n <= 0) || b["expires_at"].as_u64().is_none_or(|n| n == 0)
        || slot.to_string().len() > 16384 { return Err("protected slot invalid; no native launch".into()); }
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
fn validate_authority(a: &Value, request: &Value, expected: &ProtectedIdentity, host: &ProtectedHost) -> Res<()> {
    let b = &permit(&request)["binding"];
    if a["project"] != b["root"] || a["runtime"] != "codex" || a["account"] != request["account"]
        || a["account_home"] != json!(expected.account_home) || a["profile_sha256"] != expected.profile_sha256
        || a["executable"] != json!(expected.executable) || a["executable_sha256"] != expected.executable_sha256
        || a["adapter"]["executable"] != json!(host.executable) || a["adapter"]["sha256"] != host.sha256
        || a["saved"]["revision"] != request["policy_revision"] || a["saved"]["grant"]["by"] != expected.approver
        || a["saved"]["grant"]["mode"] != "workspace" || a["saved"]["grant"]["runtime"] != "codex"
        || a["saved"]["grant"]["project"] != b["root"] || a["saved"]["grant"]["at"].as_u64().is_none_or(|n| n == 0)
        || !nonempty(&a["requested"]) || a["resolved"]["effective"] != "workspace" {
        return Err("protected authority missing, unknown or mismatched; no native input".into());
    }
    Ok(())
}
fn validate_receipt(r: &Value, request: &Value, expected: &ProtectedIdentity, host: &ProtectedHost, frontend: u32) -> Res<Value> {
    validate_expected(expected)?;
    let launch = &r["launch"]; let a = &launch["authority"]; let b = &permit(&request)["binding"];
    validate_authority(a, request, expected, host)?;
    let effective = &r["verified_effective"];
    if r["schema"] != request["schema"] || r["state"] != "verified-effective" || r["caller"] != b["caller"] || r["operation"] != b["op_id"]
        || launch["schema"] != request["schema"] || launch["state"] != "verified-effective" || launch["cancel_requested"] != false
        || launch["policy_revision"] != request["policy_revision"] || launch["account"] != request["account"]
        || !nonempty(&launch["host_instance"]) || !launch["reason"].is_null()
        || effective["state"] != "verified-effective" || !nonempty(&effective["scope"])
        || effective["workspace_write"] != true || !effective["protected_write_create_unlink_denied"].as_array().is_some_and(|v| !v.is_empty())
        || launch["verified_effective"] != *effective { return Err("protected authority missing, unknown or mismatched; no native input".into()); }
    let owner = native(&launch["supervisor"])?; let child = native(&launch["native"])?;
    let transport = if launch["frontend"].is_null() { owner.clone() } else { native(&launch["frontend"])? };
    if transport.pid != frontend || identity(transport.pid)? != transport || transport == child
        || owner == child || identity(owner.pid)? != owner || identity(child.pid)? != child
        || identity(b["pid"].as_u64().ok_or("consumer identity absent")? as u32)? != native(b)? {
        return Err("protected native/supervisor/consumer incarnation changed; no native input".into());
    }
    Ok(json!({"authority": a, "frontend":launch["frontend"], "supervisor": launch["supervisor"], "native": launch["native"],
        "scope":launch["scope"]["binding"], "host_instance": launch["host_instance"], "verified_effective": effective}))
}

fn scope_binding(scope: &Value, slot: &Value) -> bool {
    scope["coalition"].as_u64().is_some_and(|n| n > 0)
        && scope["boot"].as_i64().zip(slot["boot"].as_i64()).is_some_and(|(a,b)| a > 0 && b > 0 && a.abs_diff(b) <= 2)
}
fn validate_completion(r: &Value, preparation: &ProtectedPreparation) -> Res<()> {
    let request = &preparation.request; let b = &permit(&request)["binding"]; let launch = &r["launch"];
    if r["schema"] != request["schema"] || r["caller"] != b["caller"] || r["operation"] != b["op_id"] {
        return Err("protected completion operation unknown; retain original slot".into());
    }
    if r["state"] == "unknown" { return Ok(()); }
    let scope = &launch["scope"]; let cert = &scope["certificate"];
    if r["state"] != "completed" || launch["schema"] != request["schema"] || launch["policy_revision"] != request["policy_revision"]
        || launch["account"] != request["account"] || launch["authority"] != preparation.prepared["authority"]
        || launch["host_instance"] != preparation.prepared["host_instance"] || launch["input_closed"] != true
        || scope["dispatch_ack"] != true || scope["cleanup_intent"] != true || scope["bootout_ack"] != true || scope["cleanup_verified"] != true
        || cert["schema"] != if request["schema"] == SCHEMA { "taskq.native-completion/1" } else { "agentq.native-completion/1" } || !scope_binding(&scope["binding"], b)
        || cert["scope"] != scope["binding"] || cert["native"] != launch["native"] || cert["supervisor"] != launch["supervisor"]
        || cert["counts"].as_array().is_none_or(|a| a.len() != 2)
        || cert["counts"][0].as_u64().zip(cert["counts"][1].as_u64()).is_none_or(|(a,b)| a.checked_sub(b) != Some(1)) {
        return Err("protected descendant completion or cleanup unproved; retain original slot".into());
    }
    if native(&launch["native"])? == native(&launch["supervisor"])? { return Err("protected completion incarnation mismatched; retain original slot".into()); }
    Ok(())
}
