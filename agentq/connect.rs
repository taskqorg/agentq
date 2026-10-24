//! DashQ connection (docs/spec/ENROLLMENT.md): coreq `enroll` owns pairing, the protected grant and revocation in
//! `data/connection/`; allowlisted snapshots travel as grant-bound records in the existing telemetry queue, and
//! `agentq.configure` operations (docs/spec/CONFIGURATION.md A4) run through the guarded executor on the same pass.

use crate::{configure, executor, profiles};
use crate::telemetry::{RECORD, entries, prune, queue, update_state};
use coreq::enroll::{self, Backoff, Delivery, Request, Wire};
use coreq::write_atomic;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "agentq-snapshot/1";
/// DashQ C5: a snapshot every 60 s while connected; first connection completes within 60 s, receipt polled at 2 s.
const HEARTBEAT: u64 = 60;
const COMPLETE: u64 = 60;
const POLL: u64 = 2;

fn dir(data: &Path) -> PathBuf {
    data.join("connection")
}

fn read(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// coreq makes a random installation on first use; AgentQ keeps `data/installation` as the identity.
fn seed(data: &Path) -> Result<PathBuf, String> {
    let d = dir(data);
    let file = d.join("connection.json");
    if !file.exists() {
        // coreq protects the folder first, so the seed is born inside it
        enroll::active(&d)?;
        let s = json!({"installation": crate::telemetry::installation(data), "pending": null, "active": null, "revoke": []});
        let tmp = d.join("connection.json.seed");
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&tmp).and_then(|mut f| f.write_all(s.to_string().as_bytes()).and_then(|_| f.sync_all())).and_then(|_| std::fs::rename(&tmp, &file)).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(d)
}

/// One allowlisted snapshot of `status.json` and the newest resource sample, bound to the active grant and queued.
/// An unsent snapshot of the same grant and resource sample adds no history: the new one replaces it.
fn queue_snapshot(data: &Path, now: u64) -> Result<(Value, PathBuf), String> {
    let status = data.parent().and_then(|i| read(&i.join("status.json")));
    let payload = crate::snapshot::project(&crate::telemetry::installation(data), crate::telemetry::runtime(), status.as_ref(), read(&data.join("resource.json")).as_ref(), now);
    let id = crate::mcp::new_id().replace('-', "");
    let rec = enroll::stamp(&dir(data), SCHEMA, &id, now, payload)?;
    let q = queue(data);
    std::fs::create_dir_all(&q).map_err(|e| format!("{}: {e}", q.display()))?;
    for (_, p, _) in entries(&q).into_iter().filter(|e| e.1.to_string_lossy().ends_with(RECORD)) {
        if read(&p).is_some_and(|r| r["grant"] == rec["grant"] && r["payload"]["resource"]["event_id"] == rec["payload"]["resource"]["event_id"]) {
            let _ = std::fs::remove_file(p);
        }
    }
    let file = q.join(format!("{now}-{id}{RECORD}"));
    write_atomic(&file, &rec.to_string())?;
    Ok((rec, file))
}

/// The service sender's connection state. `clients`: the configurable clients, `None` for the real ones.
#[derive(Default)]
pub struct Sender {
    backoff: Backoff,
    wait_until: u64,
    snapshot_at: u64,
    clients: Option<Vec<profiles::Client>>,
}

/// DashQ serves every machine route under `/machine/` (dashq#110); coreq posts `destination + route`, so the
/// destination stays the DashQ origin the person named and only the route gains the prefix.
struct Machine<'a>(&'a mut dyn Wire, Option<&'a Path>);

impl Wire for Machine<'_> {
    fn post(&mut self, destination: &str, route: &str, auth: Option<&str>, body: &Value) -> Result<(u16, Value), String> {
        // Host disable (§ 4): only snapshot records (`/ingest`) still travel, so DashQ can name the host disabled.
        if route != "/ingest" && self.1.is_some_and(|data| crate::control::host_disabled(data) != Ok(false)) { return Err("integration_disabled: export suspended".into()); }
        self.0.post(destination, &format!("/machine{route}"), auth, body)
    }
    fn now(&self) -> u64 {
        self.0.now()
    }
    fn sleep(&mut self, secs: u64) {
        self.0.sleep(secs);
    }
    fn show(&mut self, verification_uri: &str, user_code: &str) {
        self.0.show(verification_uri, user_code);
    }
}

/// One service pass over HTTPS.
pub fn pass(data: &Path, now: u64, s: &mut Sender) {
    pass_with(data, now, s, &mut enroll::Https { browser: false });
}

/// Retry queued revokes, queue a snapshot every 60 s while connected, deliver grant-bound records. Never enrolled:
/// nothing. Host disable (§ 4): snapshots only, no revoke retry and no operation.
// ponytail: offline revokes take one 10 s try each per pass, at most 16; skip them for a while if passes run late
fn pass_with(data: &Path, now: u64, s: &mut Sender, wire: &mut dyn Wire) {
    let wire = &mut Machine(wire, Some(data));
    let d = dir(data);
    if !d.join("connection.json").exists() {
        return;
    }
    let disabled = crate::control::host_disabled(data) != Ok(false);
    if !disabled {
        let _ = enroll::retry_revokes(&d, wire);
    }
    if now >= s.snapshot_at + HEARTBEAT && queue_snapshot(data, now).is_ok() {
        s.snapshot_at = now;
    }
    if now >= s.wait_until {
        deliver(data, now, s, wire);
    }
    if !disabled && now >= s.wait_until {
        operate(data, now, s, wire);
    }
}

/// Results of executed operations, kept until DashQ's receipt and after it, so a lost reply, a restart or a duplicate
/// delivery posts the same result and never runs the operation again.
fn results(data: &Path) -> PathBuf {
    dir(data).join("results")
}

/// Posted results kept for duplicate deliveries; an unposted one is never pruned.
const KEEP: usize = 64;

/// CONFIGURATION A4: one `/operations` pull per pass when the active grant holds `agentq.configure`. A new operation
/// runs once through the executor (lifecycle lock, plan/apply, readback); a known one with the same digest posts its
/// stored result, a changed payload under the same ID is `payload_mismatch`. Operations coreq rejects locally never
/// run; those of this grant get a `rejected` result with the reason.
fn operate(data: &Path, now: u64, s: &mut Sender, wire: &mut dyn Wire) {
    let d = dir(data);
    let Ok(Some(a)) = enroll::active(&d) else { return };
    if !a["capabilities"].as_array().is_some_and(|c| c.contains(&json!(configure::CAPABILITY))) {
        return;
    }
    let mut direct = vec![];
    let error = match enroll::pull(&d, configure::CAPABILITY, &mut s.backoff, wire) {
        Ok(p) => {
            for r in p.rejected {
                // a malformed operation or another grant's has no result here
                if !["invalid", "installation", "stale_generation"].contains(&r["reason"].as_str().unwrap_or("invalid")) {
                    let op = json!({"operation": r["operation"], "digest": r["digest"], "installation": a["installation"], "generation": a["generation"]});
                    direct.push((op, configure::result(r["operation"].as_str(), "rejected", r["reason"].as_str(), None)));
                }
            }
            for op in p.operations {
                let file = results(data).join(format!("{}.json", op["operation"].as_str().unwrap_or_default()));
                match read(&file) {
                    Some(mut st) if st["op"]["digest"] == op["digest"] => {
                        st["posted"] = json!(false);
                        let _ = write_atomic(&file, &st.to_string());
                    }
                    Some(_) => direct.push((op.clone(), configure::result(op["operation"].as_str(), "rejected", Some("payload_mismatch"), None))),
                    None => {
                        let (result, readback) = run(data, &a, &op, s, now);
                        let op = json!({"operation": op["operation"], "digest": op["digest"], "installation": op["installation"], "generation": op["generation"], "actor": op["authority"]["actor"]});
                        let st = json!({"op": op, "body": {"result": result, "readback": readback}, "at": now, "posted": false});
                        let _ = std::fs::create_dir_all(results(data)).map_err(|e| e.to_string()).and_then(|_| write_atomic(&file, &st.to_string()));
                    }
                }
            }
            None
        }
        Err(Delivery::Retry(secs)) => {
            s.wait_until = now + secs;
            Some(format!("no reply; retry in {secs} s"))
        }
        Err(Delivery::Stopped(e)) => Some(e),
    };
    let error = error.or_else(|| post(data, now, s, wire, direct));
    update_state(&queue(data), |st| st["connection"]["operations_error"] = json!(error));
}

/// Posts `direct` results, then every stored unposted one; a lost reply keeps it for the next pass unchanged.
fn post(data: &Path, now: u64, s: &mut Sender, wire: &mut dyn Wire, direct: Vec<(Value, Value)>) -> Option<String> {
    let d = dir(data);
    for (op, r) in direct {
        if let Err(Delivery::Retry(secs)) = enroll::post_result(&d, &op, json!({"result": r, "readback": []}), &mut s.backoff, wire) {
            s.wait_until = now + secs;
            return Some(format!("no reply; retry in {secs} s"));
        }
    }
    let mut stored: Vec<(PathBuf, Value)> = std::fs::read_dir(results(data)).into_iter().flatten().flatten().map(|e| e.path()).filter_map(|p| read(&p).map(|v| (p, v))).collect();
    stored.sort_by_key(|e| std::cmp::Reverse(e.1["at"].as_u64().unwrap_or(0)));
    for (p, st) in stored.iter_mut().filter(|e| e.1["posted"] == false) {
        let receipt = match enroll::post_result(&d, &st["op"], st["body"].clone(), &mut s.backoff, wire) {
            Ok(r) => r,
            Err(Delivery::Retry(secs)) => {
                s.wait_until = now + secs;
                return Some(format!("no reply; retry in {secs} s"));
            }
            // another grant's result never enters this one; ends its posting
            Err(Delivery::Stopped(e)) if e == "grant" => json!({"state": "rejected", "reason": "grant"}),
            Err(Delivery::Stopped(e)) => return Some(e),
        };
        st["posted"] = json!(true);
        st["receipt"] = receipt;
        let _ = write_atomic(p, &st.to_string());
    }
    for (p, _) in stored.iter().filter(|e| e.1["posted"] == true).skip(KEEP) {
        let _ = std::fs::remove_file(p);
    }
    None
}

/// One pulled operation through the executor with this grant's authority: its installation and generation, the
/// workspace DashQ bound to the grant, the actor and profiles of the operation's server-set `authority` (coreq checked
/// it on pull) and the operation's expiry. Payload claims never add authority: the executor rejects a payload actor,
/// workspace or profile outside it. Returns the result and the sanitized A2 projection of the profiles it names.
fn run(data: &Path, a: &Value, op: &Value, s: &Sender, now: u64) -> (Value, Value) {
    let install = data.parent().unwrap_or(data);
    let clients = s.clients.clone().unwrap_or_else(profiles::clients);
    let au = &op["authority"];
    let grant = json!({"capability": configure::CAPABILITY, "actor": au["actor"], "workspace": a["workspace"], "installation": a["installation"],
        "generation": a["generation"], "profiles": au["profiles"], "expires_at": op["expires_at"]});
    let exe = install.join(format!("agentq{}", std::env::consts::EXE_SUFFIX));
    let h = executor::Host { data: data.to_path_buf(), exe, clients, installation: crate::telemetry::installation(data), generation: a["generation"].as_u64().unwrap_or(0), grant };
    let r = executor::execute(&h, &op["payload"].to_string(), now);
    let named: Vec<&Value> = r["profiles"].as_array().into_iter().flatten().map(|p| &p["id"]).collect();
    let all = crate::snapshot::configuration(Some(&json!({"configuration": profiles::status_with(&h.clients, data, now)})), now);
    let readback: Value = all["profiles"].as_array().into_iter().flatten().filter(|p| named.contains(&&p["id"])).cloned().collect();
    // ponytail: an over-limit readback is dropped, the result alone fits; DashQ reads the snapshot instead
    let fits = json!({"result": r, "readback": readback}).to_string().len() <= enroll::MAX_PAYLOAD;
    (r, if fits { readback } else { json!([]) })
}

/// Up to 100 records, oldest first. A durable `accepted` deletes a record; `rejected` (scope, replay, another grant)
/// deletes and counts it in `dropped`; no reply keeps every record for a retry with the same event ID.
fn deliver(data: &Path, now: u64, s: &mut Sender, wire: &mut dyn Wire) {
    let q = queue(data);
    let files: Vec<PathBuf> = prune(&q, now).into_iter().map(|e| e.1).filter(|p| p.to_string_lossy().ends_with(RECORD)).take(enroll::MAX_RECORDS).collect();
    if files.is_empty() {
        return;
    }
    // an unreadable record is null: no grant, so coreq rejects it unsent
    let recs: Vec<Value> = files.iter().map(|p| read(p).unwrap_or_default()).collect();
    let (mut accepted, mut rejected) = (0, 0);
    let start = std::time::Instant::now();
    let mut class = None;
    let error = match enroll::deliver(&dir(data), &recs, &mut s.backoff, wire) {
        Ok(receipts) => {
            for (p, r) in files.iter().zip(&receipts) {
                match r["state"].as_str() {
                    Some("accepted" | "projected") => accepted += 1,
                    Some("rejected") => rejected += 1,
                    _ => continue,
                }
                let _ = std::fs::remove_file(p);
            }
            if rejected > 0 {
                class = Some("rejected");
            }
            (rejected > 0).then(|| format!("{rejected} records rejected"))
        }
        Err(Delivery::Retry(secs)) => {
            s.wait_until = now + secs;
            class = Some("no_reply");
            Some(format!("no reply; retry in {secs} s"))
        }
        Err(Delivery::Stopped(e)) => {
            class = Some("stopped");
            Some(e)
        }
    };
    let took = start.elapsed();
    crate::profile::trace(data, "export.dashq", "batch", took, class);
    crate::profile::sample(data, "export.dashq", now, took, class);
    update_state(&q, |st| {
        st["dropped"] = json!(st["dropped"].as_u64().unwrap_or(0) + rejected);
        if accepted > 0 {
            st["connection"]["last_delivery"] = json!(now);
        }
        st["connection"]["error"] = json!(error);
    });
}

/// `agentq enroll`: pair, then deliver one real snapshot and read its projection back. `Ok` only on a `projected`
/// receipt that matches this installation, generation, schema and event.
pub fn enroll_with(data: &Path, destination: &str, wire: &mut dyn Wire) -> Result<String, String> {
    if !destination.starts_with("https://") {
        return Err("destination must be https".into());
    }
    let wire = &mut Machine(wire, None);
    let d = seed(data)?;
    let host = crate::host();
    let req = Request { installation: None, product: "agentq", host_label: &host, schemas: &[SCHEMA], scope: None, capabilities: &[configure::CAPABILITY] };
    enroll::pair(&d, destination, &req, wire)?;
    let (rec, file) = queue_snapshot(data, wire.now())?;
    let id = rec["event_id"].as_str().unwrap_or_default().to_string();
    let (until, mut backoff, mut stage) = (wire.now() + COMPLETE, Backoff::default(), "delivery");
    while wire.now() < until {
        if stage == "delivery" {
            match enroll::deliver(&d, std::slice::from_ref(&rec), &mut backoff, wire) {
                Ok(r) if r[0]["state"] == "rejected" => return Err(format!("snapshot rejected: {}", r[0]["reason"].as_str().unwrap_or("unknown"))),
                Ok(_) => {
                    let _ = std::fs::remove_file(&file);
                    stage = "projection";
                }
                Err(Delivery::Stopped(e)) => return Err(e),
                Err(Delivery::Retry(_)) => {}
            }
        }
        if stage == "projection" {
            match enroll::receipt(&d, &rec, wire) {
                Ok("projected") => {
                    let a = enroll::active(&d)?.unwrap_or_default();
                    write_atomic(&d.join("verified.json"), &json!({"destination": a["destination"], "generation": a["generation"], "event_id": id, "at": wire.now()}).to_string())?;
                    return Ok(id);
                }
                Err(e) if e.contains("mismatch") => return Err(e),
                _ => {}
            }
        }
        wire.sleep(POLL);
    }
    Err(format!("not verified: snapshot {stage} unconfirmed after {COMPLETE} s; the service keeps sending"))
}

/// `agentq disconnect`: stop sending at once; the revoke retries until DashQ confirms it.
pub fn disconnect_with(data: &Path, wire: &mut dyn Wire) -> Result<&'static str, String> {
    let wire = &mut Machine(wire, None);
    let d = dir(data);
    let s = read(&d.join("connection.json")).unwrap_or_default();
    if !s["active"].is_object() && !s["pending"].is_object() && s["revoke"].as_array().is_none_or(Vec::is_empty) {
        return Ok("not connected");
    }
    let _ = std::fs::remove_file(d.join("verified.json"));
    if s["pending"].is_object() {
        let _ = enroll::cancel(&d, wire);
    }
    match enroll::disconnect(&d, wire) {
        Ok(()) => Ok("disconnected; revoked"),
        Err(e) if e.ends_with("revoke pending") => Ok("Disconnected locally; revocation pending"),
        Err(e) => Err(e),
    }
}

pub fn enroll(data: &Path, destination: &str, browser: bool) -> Result<String, String> {
    enroll_with(data, destination, &mut enroll::Https { browser })
}

pub fn disconnect(data: &Path) -> Result<&'static str, String> {
    disconnect_with(data, &mut enroll::Https { browser: false })
}

/// `agentq connection status`: destination, connection, projection, export and revocation; never a credential.
pub fn status(data: &Path) -> Value {
    let d = dir(data);
    let Some(s) = read(&d.join("connection.json")) else {
        let state = if d.join("connection.json").exists() { "unreadable" } else { "not enrolled" };
        return json!({"state": state, "next": "run `agentq enroll <https-url>`"});
    };
    let (a, v) = (&s["active"], read(&d.join("verified.json")).unwrap_or_default());
    let verified = a.is_object() && v["destination"] == a["destination"] && v["generation"] == a["generation"];
    let (state, next) = match () {
        _ if verified => ("connected", None),
        _ if a.is_object() => ("unverified", Some("run `agentq enroll <https-url>` again to verify")),
        _ if s["pending"].is_object() => ("pending approval", Some("run `agentq enroll <https-url>` again and approve in the browser")),
        _ => ("disconnected", Some("run `agentq enroll <https-url>` to connect")),
    };
    let q = queue(data);
    let records = entries(&q).iter().filter(|e| e.1.to_string_lossy().ends_with(RECORD)).count();
    let c = &crate::telemetry::state(&q)["connection"];
    json!({
        "state": state,
        "destination": a["destination"].as_str().or(s["pending"]["destination"].as_str()),
        "generation": a["generation"],
        "projected": verified.then(|| v["event_id"].clone()),
        "export": {"pending": records, "last_delivery": c["last_delivery"], "error": c["error"]},
        "configure": {"granted": a["capabilities"].as_array().is_some_and(|c| c.contains(&json!(configure::CAPABILITY))), "error": c["operations_error"]},
        "revocation_pending": s["revoke"].as_array().map_or(0, Vec::len),
        "next": next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn data(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agentq-conn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let data = d.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("installation"), "inst-1").unwrap();
        data
    }

    /// In-memory DashQ under wire v1: auto-approves the shown code, projects on acceptance unless told otherwise.
    #[derive(Default)]
    struct Dash {
        now: u64,
        offline: bool,
        /// Process the next request on this route, then lose its reply.
        lose: Option<&'static str>,
        /// Schemas outside the grant.
        refuse: Vec<String>,
        approved: bool,
        generation: u64,
        staged: Option<(String, u64)>,
        creds: HashMap<String, (u64, bool)>,
        events: HashMap<(u64, String), Value>,
        sent: Vec<(String, Value)>,
        /// Capabilities the browser approves of those asked for.
        approve: Vec<&'static str>,
        /// Queued operations and their stored results.
        ops: Vec<Value>,
        results: HashMap<String, Value>,
    }

    impl Dash {
        fn handle(&mut self, route: &str, auth: Option<&str>, b: &Value) -> (u16, Value) {
            let live = auth.and_then(|c| self.creds.get(c)).filter(|c| !c.1).map(|c| c.0);
            match route {
                "/machine/connect/device" => (200, json!({"protocol": 1, "device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://dash.example/connect", "expires_in": 600, "interval": 5})),
                "/machine/connect/token" if !self.approved && self.staged.is_none() => (400, json!({"protocol": 1, "error": "authorization_pending"})),
                "/machine/connect/token" => {
                    let (cred, g) = self.staged.get_or_insert_with(|| {
                        self.generation += 1;
                        (format!("cred{}", self.generation), self.generation)
                    }).clone();
                    self.approved = false;
                    (200, json!({"protocol": 1, "credential": cred, "installation": "inst-1", "generation": g, "product": "agentq", "workspace": "ws", "schemas": [SCHEMA], "capabilities": self.approve}))
                }
                "/machine/connect/ack" => {
                    if let Some((c, g)) = self.staged.take() {
                        self.creds.insert(c, (g, false));
                    }
                    let g = live.or_else(|| auth.and_then(|c| self.creds.get(c)).map(|c| c.0));
                    (200, json!({"protocol": 1, "state": "active", "generation": g}))
                }
                "/machine/connection/revoke" => {
                    let c = self.creds.get_mut(auth.unwrap_or_default()).unwrap();
                    c.1 = true;
                    (200, json!({"protocol": 1, "state": "revoked", "generation": c.0}))
                }
                _ if live.is_none() => (401, json!({"protocol": 1, "error": "revoked"})),
                "/machine/ingest" => {
                    let g = live.unwrap();
                    let receipts: Vec<Value> = b["records"].as_array().unwrap().iter().map(|r| {
                        let id = r["event_id"].as_str().unwrap().to_string();
                        if self.refuse.contains(&r["schema"].as_str().unwrap().to_string()) {
                            return json!({"event_id": id, "state": "rejected", "reason": "scope"});
                        }
                        match self.events.entry((g, id.clone())).or_insert_with(|| r.clone()) {
                            old if *old != *r => json!({"event_id": id, "state": "rejected", "reason": "replay_mismatch"}),
                            _ => json!({"event_id": id, "state": "accepted"}),
                        }
                    }).collect();
                    (200, json!({"protocol": 1, "receipts": receipts}))
                }
                "/machine/operations" => {
                    let g = live.unwrap();
                    let pending: Vec<Value> = self.ops.iter().filter(|o| o["generation"] == g && !self.results.contains_key(o["operation"].as_str().unwrap())).cloned().collect();
                    (200, json!({"protocol": 1, "operations": pending}))
                }
                "/machine/operations/result" => {
                    let id = b["operation"].as_str().unwrap().to_string();
                    let Some(op) = self.ops.iter().find(|o| o["operation"] == id) else { return (200, json!({"protocol": 1, "operation": id, "state": "rejected", "reason": "unknown_operation"})) };
                    if op["digest"] != b["digest"] {
                        return (200, json!({"protocol": 1, "operation": id, "state": "rejected", "reason": "digest_mismatch"}));
                    }
                    match self.results.entry(id.clone()).or_insert_with(|| b["result"].clone()) {
                        old if *old != b["result"] => (200, json!({"protocol": 1, "operation": id, "state": "rejected", "reason": "replay_mismatch"})),
                        _ => (200, json!({"protocol": 1, "operation": id, "state": "accepted"})),
                    }
                }
                "/machine/connection/receipt" => {
                    let g = live.unwrap();
                    match self.events.get(&(g, b["event_id"].as_str().unwrap().to_string())) {
                        Some(r) => (200, json!({"protocol": 1, "installation": "inst-1", "generation": g, "schema": r["schema"], "event_id": r["event_id"], "state": "projected"})),
                        None => (404, json!({"protocol": 1, "error": "unknown_event"})),
                    }
                }
                _ => (404, json!({"protocol": 1, "error": "not_found"})),
            }
        }

        fn ingested(&self) -> Vec<String> {
            self.sent.iter().filter(|s| s.0.ends_with("/machine/ingest")).flat_map(|s| s.1["records"].as_array().unwrap().iter().map(|r| r["event_id"].as_str().unwrap().to_string()).collect::<Vec<_>>()).collect()
        }
    }

    impl Wire for Dash {
        fn post(&mut self, destination: &str, route: &str, auth: Option<&str>, body: &Value) -> Result<(u16, Value), String> {
            if self.offline {
                return Err("no reply".into());
            }
            self.sent.push((format!("{destination}{route}"), body.clone()));
            let r = self.handle(route, auth, body);
            if self.lose == Some(route) {
                self.lose = None;
                return Err("no reply".into());
            }
            Ok(r)
        }
        fn now(&self) -> u64 {
            self.now
        }
        fn sleep(&mut self, secs: u64) {
            self.now += secs;
        }
        fn show(&mut self, _: &str, _: &str) {
            self.approved = true;
        }
    }

    /// A Desktop Chat client root beside `data`, not yet set up.
    fn desktop(data: &Path) -> Vec<profiles::Client> {
        let root = data.parent().unwrap().join("desktop");
        std::fs::create_dir_all(&root).unwrap();
        vec![profiles::Client { kind: "claude-desktop-chat", agent: "claude-desktop", files: vec![root.join("claude_desktop_config.json")], root }]
    }

    /// Queues one operation of `generation` whose payload is the AgentQ request `kind` with `changes`, authorized for
    /// actor `u1` on the profiles it changes.
    fn queue_op(w: &mut Dash, id: &str, kind: &str, generation: u64, changes: Value, plan: Option<&Value>) -> Value {
        let mut profiles: Vec<Value> = vec![];
        for c in changes.as_array().into_iter().flatten() {
            if !profiles.contains(&c["profile"]) {
                profiles.push(c["profile"].clone());
            }
        }
        let mut payload = json!({"schema": 1, "operation": id, "kind": kind, "actor": "u1", "workspace": "ws", "installation": "inst-1", "generation": generation, "issued_at": w.now, "expires_at": w.now + 300, "changes": changes});
        if let Some(p) = plan {
            payload["plan"] = json!({"revision": p["revision"], "digest": p["digest"]});
        }
        let op = json!({"operation": id, "capability": configure::CAPABILITY, "installation": "inst-1", "generation": generation, "expires_at": w.now + 300, "digest": enroll::digest(&payload), "payload": payload,
            "authority": {"actor": "u1", "workspace": "ws", "profiles": profiles}});
        w.ops.push(op.clone());
        op
    }

    fn records(data: &Path) -> usize {
        entries(&queue(data)).iter().filter(|e| e.1.to_string_lossy().ends_with(RECORD)).count()
    }

    /// A1, A2: one approval keeps `data/installation` and owned edits; a real snapshot is projected, then deleted.
    #[test]
    fn enroll_keeps_identity_and_verifies() {
        let d = data("enroll");
        std::fs::write(d.join("owned.json"), "[1]").unwrap();
        std::fs::write(d.join("resource.json"), crate::telemetry::span("agentq.resource", "sample", json!({"agentq.resource.cpu_unknown": "warmup"}), 5, "inst-1", "h").to_string()).unwrap();
        let mut w = Dash { now: 1000, ..Dash::default() };
        let id = enroll_with(&d, "https://dash.example", &mut w).unwrap();
        assert_eq!(w.sent[0].1["installation"], "inst-1", "existing identity");
        assert_eq!(w.sent[0].1["schemas"], json!([SCHEMA]));
        assert!(w.sent.iter().all(|s| s.0.starts_with("https://dash.example/machine/")), "machine routes only");
        let rec = &w.events[&(1, id.clone())];
        assert_eq!((rec["payload"]["schema"].clone(), rec["payload"]["installation"].clone()), (json!(1), json!("inst-1")));
        assert_eq!(rec["payload"]["resource"]["cpu_unknown"], "warmup", "the newest sample, not a fake one");
        assert_eq!(records(&d), 0, "deleted after durable acceptance");
        let s = status(&d);
        assert_eq!((s["state"].clone(), s["destination"].clone(), s["projected"].clone()), (json!("connected"), json!("https://dash.example"), json!(id)));
        assert!(!s.to_string().contains("cred"), "no credential in status");
        assert_eq!(std::fs::read_to_string(d.join("owned.json")).unwrap(), "[1]");
        assert_eq!(std::fs::read_to_string(d.join("installation")).unwrap(), "inst-1");
        // a repeat to the same destination reuses the grant and verifies a new snapshot
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        assert_eq!(w.generation, 1);
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// #169: a host disable keeps the DashQ snapshot (and only it) flowing, so DashQ can name the host disabled
    /// instead of reading a silent host; operations and revoke retries stay off.
    #[test]
    fn host_disable_keeps_only_snapshots() {
        let d = data("disabled");
        let mut w = Dash { now: 1000, approve: vec![configure::CAPABILITY], ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        crate::control::save(&d, "integration.json", &json!({"schema": 1, "generation": 1, "host_disabled": true, "profiles": {}})).unwrap();
        queue_op(&mut w, "d0000000000000000000000000000009", "plan", 1, json!([]), None);
        let (sent, events) = (w.sent.len(), w.events.len());
        w.now += HEARTBEAT;
        pass_with(&d, w.now, &mut Sender::default(), &mut w);
        let routes: Vec<_> = w.sent[sent..].iter().map(|s| s.0.as_str()).collect();
        assert_eq!(routes, ["https://dash.example/machine/ingest"], "snapshot only, no operation pull");
        assert_eq!((w.events.len(), records(&d)), (events + 1, 0), "a new snapshot is accepted and deleted");
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// A3, A4: lost ack and ingest replies; a crash after queueing. One grant; the same event ID is retried and the
    /// record lives until a durable receipt.
    #[test]
    fn lost_replies_and_crash_recover() {
        let d = data("lost");
        let mut w = Dash { now: 1000, lose: Some("/machine/connect/ack"), ..Dash::default() };
        assert!(enroll_with(&d, "https://dash.example", &mut w).is_err());
        assert_eq!(status(&d)["state"], "pending approval");
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        assert_eq!(w.generation, 1, "no second grant");

        // crash after the record was queued: the next service pass sends it
        let mut s = Sender { snapshot_at: u64::MAX / 2, ..Sender::default() };
        let (rec, file) = queue_snapshot(&d, w.now).unwrap();
        w.lose = Some("/machine/ingest");
        pass_with(&d, w.now, &mut s, &mut w);
        assert!(file.exists(), "no reply: kept");
        assert!(status(&d)["export"]["error"].as_str().unwrap().starts_with("no reply"));
        pass_with(&d, s.wait_until, &mut s, &mut w);
        assert!(!file.exists(), "accepted: deleted");
        let id = rec["event_id"].as_str().unwrap();
        assert_eq!(w.ingested().iter().filter(|e| *e == id).count(), 2, "retried with the same event ID");
        assert!(status(&d)["export"]["error"].is_null());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// A3, A4: offline disconnect stops sending at once and retries the revoke; a server revoke stops the sender.
    #[test]
    fn offline_revoke() {
        let d = data("revoke");
        let mut w = Dash { now: 1000, ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        w.offline = true;
        assert_eq!(disconnect_with(&d, &mut w), Ok("Disconnected locally; revocation pending"));
        let s = status(&d);
        assert_eq!((s["state"].clone(), s["revocation_pending"].clone()), (json!("disconnected"), json!(1)));
        w.offline = false;
        let before = w.ingested().len();
        pass_with(&d, w.now, &mut Sender::default(), &mut w);
        assert_eq!(w.ingested().len(), before, "no snapshot after disconnect");
        assert_eq!(status(&d)["revocation_pending"], 0);
        assert!(w.creds["cred1"].1, "revoked on the server");

        // reconnect needs a new approval and generation; a DashQ-side revoke ends sending
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        w.creds.get_mut("cred2").unwrap().1 = true;
        let mut s = Sender::default();
        pass_with(&d, w.now, &mut s, &mut w);
        assert_eq!(status(&d)["state"], "disconnected");
        assert_eq!(status(&d)["export"]["error"], "revoked");
        assert_eq!(records(&d), 1, "kept: never sent elsewhere, pruned by the queue caps");
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// A3, A4: a schema outside the grant is rejected and counted; old backlog keeps its grant across a destination
    /// switch and never reaches the new one; standalone sending skips grant-bound records.
    #[test]
    fn wrong_scope_and_old_grant() {
        let d = data("scope");
        let mut w = Dash { now: 1000, ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        w.refuse = vec![SCHEMA.into()];
        pass_with(&d, w.now, &mut Sender::default(), &mut w);
        assert_eq!(records(&d), 0);
        assert_eq!(crate::telemetry::status(&d, w.now)["dropped"], 1);
        assert_eq!(status(&d)["export"]["error"], "1 records rejected");
        w.refuse.clear();

        w.offline = true;
        let (old, _) = queue_snapshot(&d, w.now).unwrap();
        let down = crate::telemetry::Keys::parse("http://127.0.0.1:1 pk sk").unwrap();
        assert_eq!(crate::telemetry::send(&d, &down, w.now), Ok(0), "standalone never sends a record");
        w.offline = false;
        enroll_with(&d, "https://other.example", &mut w).unwrap();
        assert!(status(&d)["state"] == "connected" && w.generation == 2);
        pass_with(&d, w.now, &mut Sender::default(), &mut w);
        let id = old["event_id"].as_str().unwrap();
        assert!(!w.sent.iter().any(|s| s.0.starts_with("https://other.example") && s.1.to_string().contains(id)), "old record never reaches the new destination");
        assert_eq!(records(&d), 0, "rejected locally by grant");
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// #114 A1-A4: a granted host pulls a plan and an apply on its pass, applies under the lock and posts the result
    /// with plan digest, configured state and readback. A lost reply, a restart and a duplicate delivery post the same
    /// result without a second run; a changed payload under the same ID is refused.
    #[test]
    fn operations_plan_apply_and_duplicates() {
        let d = data("ops");
        let mut w = Dash { now: 1000, approve: vec![configure::CAPABILITY], ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        assert_eq!(w.sent[0].1["capabilities"], json!([configure::CAPABILITY]), "asked for separately");
        assert_eq!(status(&d)["configure"]["granted"], true);
        crate::control::save(&d, "integration.json", &json!({"schema": 1, "generation": 1, "host_disabled": true, "profiles": {}})).unwrap();
        let sent = w.sent.len();
        assert!(Machine(&mut w, Some(&d)).post("https://dash.example", "/operations", None, &json!({})).unwrap_err().contains("integration_disabled"));
        assert_eq!(w.sent.len(), sent, "a pending pass may not start another remote request after host disable");
        assert_eq!(status(&d)["configure"]["granted"], true, "disable preserves enrollment for enable");
        std::fs::remove_file(d.join("integration.json")).unwrap();
        let clients = desktop(&d);
        let file = clients[0].files[0].clone();
        let id = profiles::status_with(&clients, &d, w.now)["profiles"][0]["id"].clone();
        let mcp = json!([{"profile": id, "field": "mcp", "value": "registered"}]);
        let mut s = Sender { snapshot_at: u64::MAX / 2, clients: Some(clients.clone()), ..Sender::default() };

        queue_op(&mut w, "d0000000000000000000000000000001", "plan", 1, mcp.clone(), None);
        pass_with(&d, w.now, &mut s, &mut w);
        let plan = w.results["d0000000000000000000000000000001"]["result"].clone();
        assert_eq!((plan["kind"].as_str(), plan["writable"].as_bool()), (Some("plan"), Some(true)), "{plan}");
        assert!(!file.exists(), "a plan writes nothing");

        let op = queue_op(&mut w, "d0000000000000000000000000000002", "apply", 1, mcp.clone(), Some(&plan));
        w.lose = Some("/machine/operations/result");
        pass_with(&d, w.now, &mut s, &mut w);
        assert!(file.exists(), "applied");
        let posted = w.results["d0000000000000000000000000000002"].clone();
        let r = &posted["result"];
        assert_eq!((r["operation"].clone(), r["outcome"].as_str(), r["plan"].clone()), (op["operation"].clone(), Some("applied"), plan["digest"].clone()), "{r}");
        assert_eq!(posted["readback"][0]["mcp"], "registered");
        assert_eq!(posted["readback"][0]["verification"]["state"], "unknown", "configured, not verified");
        assert!(!posted.to_string().contains(&d.display().to_string()), "no path in the result");
        assert!(status(&d)["configure"]["error"].as_str().unwrap().starts_with("no reply"));

        // the reply was lost: DashQ stored it, the host posts it again unchanged, then a restart and a redelivery
        // post the same result and never run it twice
        let before = std::fs::read_to_string(&file).unwrap();
        w.results.remove("d0000000000000000000000000000002");
        std::fs::write(&file, "{}").unwrap();
        let mut s = Sender { snapshot_at: u64::MAX / 2, clients: Some(clients.clone()), ..Sender::default() };
        pass_with(&d, w.now + 60, &mut s, &mut w);
        assert_eq!(w.results["d0000000000000000000000000000002"], posted, "same result after restart");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}", "not run again");
        std::fs::write(&file, before).unwrap();
        assert!(status(&d)["configure"]["error"].is_null());

        // a changed payload under the same ID
        let mut changed = queue_op(&mut w, "d0000000000000000000000000000002", "plan", 1, mcp, None);
        w.ops.retain(|o| o["operation"] != "d0000000000000000000000000000002" || o["payload"]["kind"] == "plan");
        w.results.remove("d0000000000000000000000000000002");
        changed["payload"]["kind"] = json!("plan");
        pass_with(&d, w.now + 61, &mut s, &mut w);
        assert_eq!(w.results["d0000000000000000000000000000002"]["result"]["reason"], "payload_mismatch");
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// #114 A1, A4: expired, wrong-generation, unsupported and ungranted or revoked operations never apply; those of
    /// this grant get a rejected result with the reason.
    #[test]
    fn operations_rejected_never_apply() {
        let d = data("opsno");
        let clients = desktop(&d);
        let file = clients[0].files[0].clone();
        let id = profiles::status_with(&clients, &d, 1000)["profiles"][0]["id"].clone();
        let mcp = json!([{"profile": id, "field": "mcp", "value": "registered"}]);
        let mut s = Sender { snapshot_at: u64::MAX / 2, clients: Some(clients), ..Sender::default() };

        // observation-only grant: nothing pulled
        let mut w = Dash { now: 1000, ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        queue_op(&mut w, "e0000000000000000000000000000001", "plan", 1, mcp.clone(), None);
        pass_with(&d, w.now, &mut s, &mut w);
        assert!(!w.sent.iter().any(|x| x.0.ends_with("/machine/operations")), "no pull without agentq.configure");
        assert_eq!(status(&d)["configure"]["granted"], false);

        w.approve = vec![configure::CAPABILITY];
        w.ops.clear();
        enroll_with(&d, "https://other.example", &mut w).unwrap();
        let g = w.generation;
        // a late (clock-skewed) delivery, an operation altered in transit, another generation's, an unsupported field
        let late = queue_op(&mut w, "e0000000000000000000000000000002", "apply", g, mcp.clone(), Some(&json!({"revision": "0000000000000000", "digest": "0".repeat(64)})));
        w.ops[0]["expires_at"] = json!(w.now);
        w.ops[0]["digest"] = json!(enroll::digest(&late["payload"]));
        queue_op(&mut w, "e0000000000000000000000000000003", "plan", g, mcp.clone(), None);
        w.ops[1]["payload"]["changes"][0]["field"] = json!("hooks");
        queue_op(&mut w, "e0000000000000000000000000000004", "plan", g - 1, mcp.clone(), None);
        queue_op(&mut w, "e0000000000000000000000000000005", "plan", g, json!([{"profile": id, "field": "model", "value": "x"}]), None);
        pass_with(&d, w.now, &mut s, &mut w);
        let reason = |w: &Dash, op: &str| w.results.get(op).map(|r| r["result"]["reason"].clone());
        assert_eq!(reason(&w, "e0000000000000000000000000000002"), Some(json!("expired")));
        assert_eq!(reason(&w, "e0000000000000000000000000000003"), Some(json!("digest_mismatch")), "altered in transit");
        assert_eq!(reason(&w, "e0000000000000000000000000000004"), None, "another generation's operation gets no result here");
        assert_eq!(reason(&w, "e0000000000000000000000000000005"), Some(json!("unsupported_field")));
        assert!(!file.exists(), "nothing applied");

        // revoked by DashQ: the pass stops, a queued apply never runs
        w.creds.values_mut().for_each(|c| c.1 = true);
        queue_op(&mut w, "e0000000000000000000000000000007", "plan", g, mcp, None);
        pass_with(&d, w.now, &mut s, &mut w);
        pass_with(&d, w.now + 60, &mut s, &mut w);
        assert!(!w.results.contains_key("e0000000000000000000000000000007"));
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    /// #118 A1-A3: the actor comes from the operation's server-set authority, never the payload. A mismatching payload
    /// actor is `wrong_actor`, a missing one `invalid`, a missing authority `authority`: each posted, none applied. A
    /// matching one applies, and the stored result and operation record keep the authority's actor.
    #[test]
    fn operations_actor_from_authority() {
        let d = data("actor");
        let mut w = Dash { now: 1000, approve: vec![configure::CAPABILITY], ..Dash::default() };
        enroll_with(&d, "https://dash.example", &mut w).unwrap();
        let clients = desktop(&d);
        let file = clients[0].files[0].clone();
        let id = profiles::status_with(&clients, &d, w.now)["profiles"][0]["id"].clone();
        let mcp = json!([{"profile": id, "field": "mcp", "value": "registered"}]);
        let mut s = Sender { snapshot_at: u64::MAX / 2, clients: Some(clients), ..Sender::default() };
        // the last queued operation's payload claims `edit`, under a digest DashQ computed for it
        let claim = |w: &mut Dash, edit: &dyn Fn(&mut Value)| {
            let op = w.ops.last_mut().unwrap();
            edit(&mut op["payload"]);
            op["digest"] = json!(enroll::digest(&op["payload"]));
        };

        queue_op(&mut w, "f0000000000000000000000000000001", "plan", 1, mcp.clone(), None);
        pass_with(&d, w.now, &mut s, &mut w);
        let plan = w.results["f0000000000000000000000000000001"]["result"].clone();
        assert_eq!(plan["kind"], "plan", "matching actor: {plan}");

        queue_op(&mut w, "f0000000000000000000000000000002", "apply", 1, mcp.clone(), Some(&plan));
        claim(&mut w, &|p| p["actor"] = json!("u2"));
        queue_op(&mut w, "f0000000000000000000000000000003", "apply", 1, mcp.clone(), Some(&plan));
        claim(&mut w, &|p| drop(p.as_object_mut().unwrap().shift_remove("actor")));
        queue_op(&mut w, "f0000000000000000000000000000004", "apply", 1, mcp.clone(), Some(&plan));
        w.ops.last_mut().unwrap().as_object_mut().unwrap().shift_remove("authority");
        pass_with(&d, w.now, &mut s, &mut w);
        let reason = |w: &Dash, op: &str| w.results[op]["result"]["reason"].clone();
        assert_eq!(reason(&w, "f0000000000000000000000000000002"), "wrong_actor");
        assert_eq!(reason(&w, "f0000000000000000000000000000003"), "invalid");
        assert_eq!(reason(&w, "f0000000000000000000000000000004"), "authority");
        assert!(!file.exists(), "nothing applied");
        let stored = |op: &str| read(&results(&d).join(format!("{op}.json"))).unwrap();
        assert_eq!(stored("f0000000000000000000000000000002")["op"]["actor"], "u1", "the authority's actor, not the payload's");

        queue_op(&mut w, "f0000000000000000000000000000005", "apply", 1, mcp, Some(&plan));
        pass_with(&d, w.now, &mut s, &mut w);
        assert_eq!(w.results["f0000000000000000000000000000005"]["result"]["outcome"], "applied");
        assert!(file.exists());
        assert_eq!(stored("f0000000000000000000000000000005")["op"]["actor"], "u1");
        let rec = read(&d.join("operations").join("f0000000000000000000000000000005.json")).unwrap();
        assert_eq!((rec["actor"].clone(), rec["workspace"].clone()), (json!("u1"), json!("ws")), "operation record keeps the grant's actor");
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }
}
