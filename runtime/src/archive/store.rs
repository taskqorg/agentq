use super::*;
use crate::agent::{self, history};
use coreq::enroll::{self, Request, Wire};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    time::{SystemTime, UNIX_EPOCH},
};

fn root(data: &Path) -> PathBuf {
    data.join("archive")
}
fn connection(data: &Path) -> PathBuf {
    data.join("connection")
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Res<T> {
    serde_json::from_slice(&fs::read(path).map_err(|_| "archive storage unreadable")?)
        .map_err(|_| "archive state invalid".into())
}
fn write(path: &Path, value: &impl Serialize) -> Res<()> {
    // The caller holds this state file's lock. A stale uncommitted temporary never replaces the committed file.
    let pending = path.with_file_name(format!(
        "{}.pending",
        path.file_name()
            .ok_or("archive state path invalid")?
            .to_string_lossy()
    ));
    if pending.exists() {
        fs::remove_file(pending).map_err(|_| "archive storage unavailable")?;
    }
    coreq::write_durable(
        path,
        serde_json::to_vec(value).map_err(|_| "archive state invalid")?,
    )
    .map_err(|_| "archive storage unavailable".into())
}
fn private(path: &Path) -> Res<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|_| "archive storage unavailable")?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "archive storage not protected")?;
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path).map_err(|_| "archive storage unavailable")?;
        let who = coreq::quiet(&mut std::process::Command::new("whoami"))
            .output()
            .map_err(|_| "archive storage identity unavailable")?;
        if !who.status.success() {
            return Err("archive storage identity unavailable".into());
        }
        let user =
            String::from_utf8(who.stdout).map_err(|_| "archive storage identity unavailable")?;
        if user.trim().is_empty() {
            return Err("archive storage identity unavailable".into());
        }
        let result = coreq::quiet(std::process::Command::new("icacls").arg(path).args([
            "/inheritance:r",
            "/grant:r",
            &format!("{}:(OI)(CI)F", user.trim()),
            "/q",
        ]))
        .output()
        .map_err(|_| "archive storage not protected")?;
        if !result.status.success() {
            return Err("archive storage not protected".into());
        }
    }
    Ok(())
}
fn active(data: &Path) -> Res<Option<Value>> {
    if !connection(data).join("connection.json").exists() {
        return Ok(None);
    }
    enroll::active(&connection(data)).map_err(|_| "archive grant unreadable".into())
}
fn enabled(data: &Path) -> Res<bool> {
    let path = data.join("integration.json");
    if !path.exists() {
        return Ok(true);
    }
    let state: Value = read(&path).map_err(|_| "archive lifecycle unknown")?;
    if state["schema"] != 1 || !state["profiles"].is_object() {
        return Err("archive lifecycle unknown".into());
    }
    Ok(state["host_disabled"] != true)
}
fn identity(grant: &Value) -> Value {
    json!({"destination":grant["destination"],"workspace":grant["workspace"],"installation":grant["installation"],"generation":grant["generation"]})
}
fn allowed(grant: &Value, capture: &str) -> bool {
    grant["schemas"]
        .as_array()
        .is_some_and(|a| a.contains(&json!(SCHEMA)))
        && (capture == "launched"
            || capture == "manual"
                && grant["capabilities"]
                    .as_array()
                    .is_some_and(|a| a.contains(&json!(MANUAL))))
}

pub(super) fn register(data: &Path, mut capture: Capture) -> Res<()> {
    let Some(grant) = active(data)? else {
        return Ok(());
    };
    if !allowed(&grant, &capture.capture) || !enabled(data)? {
        return Ok(());
    }
    if excluded(&capture.root, &capture.root)? {
        return Ok(());
    }
    let dir = root(data).join("captures");
    private(&dir)?;
    let id = crate::sha256_hex(capture.id.as_bytes());
    let path = dir.join(format!("{id}.json"));
    let _lock = coreq::file_lock::lock(&dir.join(format!("{id}.lock")))
        .map_err(|_| "archive storage locked")?;
    let previous = if path.exists() {
        Some(read::<Capture>(&path)?)
    } else {
        None
    };
    capture.grant = previous
        .as_ref()
        .map(|c| c.grant.clone())
        .unwrap_or_else(|| identity(&grant));
    for source in &mut capture.sources {
        if source.start == u64::MAX {
            source.start = previous
                .as_ref()
                .and_then(|c| c.sources.iter().find(|s| s.path == source.path))
                .map(|s| s.start)
                .unwrap_or_else(|| fs::metadata(&source.path).map_or(0, |m| m.len()));
        }
        if source.end == Some(u64::MAX) {
            source.end = fs::metadata(&source.path).ok().map(|m| m.len());
        }
    }
    capture.manifest["host"] = read::<Value>(&root(data).join("host.json"))
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .map(Value::String)
        .unwrap_or_else(|| {
            json!(
                std::env::var("COMPUTERNAME")
                    .or_else(|_| std::env::var("HOSTNAME"))
                    .unwrap_or_else(|_| "unknown".into())
            )
        });
    write(&path, &capture)
}

pub(super) fn append_stream(path: &Path, bytes: &[u8]) -> Res<()> {
    let data = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("archive stream path invalid")?;
    if active(data)?.is_none_or(|g| !allowed(&g, "launched")) || !enabled(data)? {
        return Ok(());
    }
    let dir = path.parent().ok_or("archive stream path invalid")?;
    private(dir)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "archive stream unavailable")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "archive stream unavailable".into())
}

/// Literal per-project exclusions; invalid rules stop capture, never widen it.
fn excluded(root: &Path, path: &Path) -> Res<bool> {
    let file = root.join(".agentq-archive.json");
    if !file
        .try_exists()
        .map_err(|_| "archive exclusions unreadable")?
    {
        return Ok(false);
    }
    let rules: Value = read(&file).map_err(|_| "archive exclusions invalid")?;
    let list = rules["exclude"]
        .as_array()
        .ok_or("archive exclusions invalid")?;
    let resolved = path.canonicalize().unwrap_or_else(|_| path.into());
    for rule in list {
        let rule = rule
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("archive exclusions invalid")?;
        let relative = Path::new(rule);
        if relative.is_absolute()
            || relative.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err("archive exclusions invalid".into());
        }
        let excluded = root.join(relative);
        let canonical = excluded.canonicalize().unwrap_or_else(|_| excluded.clone());
        if path.starts_with(&excluded) || resolved.starts_with(&canonical) {
            return Ok(true);
        }
    }
    Ok(false)
}

struct Machine<'a>(&'a mut dyn Wire);
impl Wire for Machine<'_> {
    fn post(&mut self, d: &str, r: &str, a: Option<&str>, b: &Value) -> Res<(u16, Value)> {
        self.0.post(d, &format!("/machine{r}"), a, b)
    }
    fn now(&self) -> u64 {
        self.0.now()
    }
    fn sleep(&mut self, s: u64) {
        self.0.sleep(s)
    }
    fn show(&mut self, u: &str, c: &str) {
        self.0.show(u, c)
    }
}
pub fn enroll(
    data: &Path,
    destination: &str,
    installation: &str,
    host: &str,
    manual: bool,
    wire: &mut dyn Wire,
) -> Res<()> {
    private(&root(data))?;
    let previous = active(data)?.unwrap_or(Value::Null);
    let mut schemas: Vec<String> = previous["schemas"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if !schemas.iter().any(|s| s == SCHEMA) {
        schemas.push(SCHEMA.into());
    }
    let mut capabilities: Vec<String> = previous["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if manual && !capabilities.iter().any(|s| s == MANUAL) {
        capabilities.push(MANUAL.into());
    }
    let schemas: Vec<_> = schemas.iter().map(String::as_str).collect();
    let capabilities: Vec<_> = capabilities.iter().map(String::as_str).collect();
    let req = Request {
        installation: Some(installation),
        product: "agentq",
        host_label: host,
        schemas: &schemas,
        scope: None,
        capabilities: &capabilities,
    };
    enroll::pair(&connection(data), destination, &req, &mut Machine(wire))?;
    write(&root(data).join("host.json"), &host)
}
pub fn disconnect(data: &Path, wire: &mut dyn Wire) -> Res<()> {
    if !connection(data).join("connection.json").exists() {
        return Ok(());
    }
    enroll::disconnect(&connection(data), &mut Machine(wire))
}

#[derive(Default, Serialize, Deserialize)]
struct Summary {
    at: u64,
    error: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Queued {
    body: Value,
    replay: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct Checkpoint {
    source: String,
    previous: Vec<String>,
    grant: Value,
    identity: String,
    prefix: String,
    raw: u64,
    sent: u64,
    manifest: Value,
    manifest_hash: String,
    capture: String,
    format: String,
    pending: Vec<Queued>,
    last: Option<Value>,
    turns: Vec<Value>,
    bound: Vec<String>,
    observed: u64,
    uploaded: u64,
    scrubbed: u64,
    replaced: u64,
    last_upload: Option<u64>,
}
fn files(dir: &Path) -> Res<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut paths = fs::read_dir(dir)
        .map_err(|_| "archive storage unreadable")?
        .map(|p| {
            p.map(|p| p.path())
                .map_err(|_| "archive storage unreadable".to_string())
        })
        .collect::<Res<Vec<_>>>()?;
    paths.retain(|p| p.extension().is_some_and(|s| s == "json"));
    paths.sort();
    Ok(paths)
}

/// One independent archive pass. No telemetry queue, backoff, or agent-work lock is used.
pub fn pass(data: &Path, wire: &mut dyn Wire) -> Res<()> {
    let Some(grant) = active(data)? else {
        return Ok(());
    };
    if !allowed(&grant, "launched") || !enabled(data)? {
        return Ok(());
    }
    let dir = root(data);
    private(&dir)?;
    let _lock = coreq::file_lock::try_lock(&dir.join("shipper.lock"))
        .map_err(|_| "archive shipper busy")?;
    let summary_path = dir.join("status.json");
    let mut summary: Summary = if summary_path.exists() {
        read(&summary_path)?
    } else {
        Summary::default()
    };
    summary.at = wire.now();
    summary.error = None;
    let mut captures = Vec::new();
    for path in files(&dir.join("captures"))? {
        match read::<Capture>(&path) {
            Ok(c) if c.grant == identity(&grant) && allowed(&grant, &c.capture) => captures.push(c),
            Ok(_) => {}
            Err(e) => summary.error = Some(e),
        }
    }
    // A source can fail without hiding any other registered session.
    for mut capture in captures {
        if let Err(e) = session(data, &grant, &mut capture, wire) {
            summary.error = Some(e);
        }
    }
    write(&summary_path, &summary)
}

fn session(data: &Path, grant: &Value, c: &mut Capture, wire: &mut dyn Wire) -> Res<()> {
    if excluded(&c.root, &c.root)? {
        return Ok(());
    }
    let rt = agent::runtime(&c.runtime).ok_or("archive runtime unsupported")?;
    if c.manifest["session"].as_str().is_none() {
        let source = c.sources.first().ok_or("archive session unknown")?;
        let mut file = fs::File::open(&source.path).map_err(|_| "archive source unavailable")?;
        file.seek(SeekFrom::Start(source.start))
            .map_err(|_| "archive source unavailable")?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| "archive source unavailable")?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "archive session unknown")?;
        c.manifest["session"] = json!(rt.classify(text).session.ok_or("archive session unknown")?);
    }
    let session = c.manifest["session"]
        .as_str()
        .ok_or("archive session unknown")?
        .to_string();
    let mut error = None;
    for (index, source) in c.sources.iter().enumerate() {
        let mut source = source.clone();
        if source.format == "native" {
            let reader = rt.history(&session, c.home.as_deref(), Some(&source.path), None)?;
            source.format = reader.checkpoint()["format"]
                .as_str()
                .ok_or("archive format unknown")?
                .into();
        }
        let mut manifest = c.manifest.clone();
        if let Err(e) = upload(
            data,
            grant,
            c,
            &source,
            &mut manifest,
            if index == 0 { c.turn.as_deref() } else { None },
            wire,
        ) {
            error = Some(e);
        }
    }
    let sources = rt.history_sessions(c.home.as_deref())?;
    let mut family = vec![session.clone()];
    loop {
        let old = family.len();
        for source in &sources {
            if source.parent.as_ref().is_some_and(|p| family.contains(p))
                && !family.contains(&source.session)
            {
                family.push(source.session.clone());
            }
        }
        if family.len() == old {
            break;
        }
    }
    for source in sources.iter().filter(|v| family.contains(&v.session)) {
        // Full native transcripts can span callers. Conditions belong to the exact per-turn log receipts;
        // without a native byte binding the shared transcript must not guess them from the latest launch.
        let mut manifest = c.manifest.clone();
        for key in [
            "task",
            "attempt",
            "cli_version",
            "model",
            "effort",
            "policy",
            "account_label",
            "instructions_hash",
        ] {
            manifest[key] = Value::Null;
        }
        manifest["project"] = json!("other");
        manifest["actor"] = json!("unknown");
        manifest["entry"] = json!("unknown");
        manifest["session"] = json!(source.session);
        manifest["parent_session"] = json!(source.parent);
        let src = Source {
            path: source.path.clone(),
            format: source.format.clone(),
            start: 0,
            end: None,
        };
        if let Err(e) = upload(data, grant, c, &src, &mut manifest, None, wire) {
            error = Some(e);
        }
        // Media is discovered only by the history adapter; no URL or caller-supplied path is fetched here.
    }
    error.map_or(Ok(()), Err)
}

fn upload(
    data: &Path,
    grant: &Value,
    c: &Capture,
    src: &Source,
    manifest: &mut Value,
    turn: Option<&str>,
    wire: &mut dyn Wire,
) -> Res<()> {
    if excluded(&c.root, &src.path)? {
        return Ok(());
    }
    let dir = root(data).join("sources");
    private(&dir)?;
    let key = crate::sha256_hex(
        json!([identity_of(grant), manifest["session"], src.path, src.start])
            .to_string()
            .as_bytes(),
    );
    let mut path = dir.join(format!("{key}.json"));
    let scrubbed_manifest = super::scrub::manifest(manifest);
    let hash = enroll::digest(manifest);
    let mut prior_error = None;
    let mut predecessor = None;
    if let Some(turn) = turn.filter(|_| path.exists()) {
        let mut live: Checkpoint = read(&path)?;
        if live.raw > 0 && live.manifest_hash != hash {
            // Terminal attribution cannot rewrite already accepted live ranges.
            prior_error = send(data, grant, c, &mut live, &path, wire).err();
            predecessor = Some(live.source);
            path = dir.join(format!(
                "{}.json",
                crate::sha256_hex(json!([key, turn]).to_string().as_bytes())
            ));
        }
    }
    let mut saved = if path.exists() {
        Some(read::<Checkpoint>(&path)?)
    } else {
        None
    };
    // Capture and delivery are independent: an offline sink must not stop spooling later appends.
    let mut file = match fs::File::open(&src.path) {
        Ok(file) => file,
        Err(_) => {
            if let Some(st) = &mut saved {
                send(data, grant, c, st, &path, wire)?;
            }
            return Err("archive source unavailable".into());
        }
    };
    if !file
        .metadata()
        .map_err(|_| "archive source unavailable")?
        .is_file()
    {
        return Err("archive source is not a regular file".into());
    }
    let identity = agent::archive_file_identity(&file)?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|_| "archive source unavailable")?;
    let end = src.end.unwrap_or(raw.len() as u64).min(raw.len() as u64) as usize;
    let base = usize::try_from(src.start).map_err(|_| "archive source truncated")?;
    if base > end {
        return Err("archive source truncated".into());
    }
    let raw = &raw[base..end];
    let end = raw.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let raw = &raw[..end];
    let mut st = saved.unwrap_or_else(|| {
        fresh(
            grant,
            &identity,
            manifest,
            &c.capture,
            &src.format,
            wire.now(),
        )
    });
    if st.identity != identity
        || st.format != src.format
        || st.raw > raw.len() as u64
        || crate::sha256_hex(&raw[..st.raw.min(raw.len() as u64) as usize]) != st.prefix
    {
        let old = st;
        st = fresh(
            grant,
            &identity,
            manifest,
            &c.capture,
            &src.format,
            wire.now(),
        );
        st.previous = old.previous;
        st.previous.push(old.source);
        st.pending = old.pending;
        st.uploaded = old.uploaded;
        st.scrubbed = old.scrubbed;
        st.replaced = old.replaced + 1;
        st.last_upload = old.last_upload;
        write(&path, &st)?;
    }
    if let Some(predecessor) = predecessor {
        if !st.previous.contains(&predecessor) {
            st.previous.push(predecessor);
        }
    }
    if st.manifest_hash != hash && st.raw < raw.len() as u64 {
        st.manifest = manifest.clone();
        st.manifest_hash = hash;
    }
    if let Some(event) = turn {
        if !st.bound.iter().any(|id| id == event)
            && !st.turns.iter().any(|v| v["event_id"] == event)
        {
            st.turns.push(json!({"event_id":event,"start":0}));
        }
    }
    // Rescan the native prefix so a PEM block continuing in a later append remains scrubbed.
    let (scrubbed, count) = super::scrub::bytes_from(raw, st.raw as usize);
    let bytes = &scrubbed[st.raw as usize..];
    let mut offset = 0;
    let raw_start = st.raw;
    while offset < bytes.len() {
        let start = raw_start + offset as u64;
        let mut body = json!({"protocol":1,"schema":SCHEMA,"manifest":st.manifest,"manifest_sha256":st.manifest_hash,
            "capture":st.capture,"source":st.source,"format":st.format,"start":start,"end":start,
            "sha256":"","bytes":"","scrubbed":if offset==0{count+scrubbed_manifest}else{0},"observed_at":wire.now()});
        let available = enroll::MAX_BODY
            .checked_sub(body.to_string().len() + 256)
            .ok_or("archive manifest exceeds sink body limit")?;
        let length = (available / 4 * 3).min(bytes.len() - offset);
        if length == 0 {
            return Err("archive manifest exceeds sink body limit".into());
        }
        let chunk = &bytes[offset..offset + length];
        let end = start + length as u64;
        body["end"] = json!(end);
        body["bytes"] = json!(agent::encode_base64(chunk));
        body["sha256"] = json!(crate::sha256_hex(chunk));
        let turns: Vec<_> = st
            .turns
            .iter()
            .filter(|t| t["start"].as_u64().is_some_and(|p| p < end))
            .cloned()
            .collect();
        if !turns.is_empty() {
            body["turns"] = json!(turns);
        }
        if body.to_string().len() > enroll::MAX_BODY {
            return Err("archive request exceeds sink body limit".into());
        }
        st.pending.push(Queued {
            body,
            replay: false,
        });
        offset += length;
    }
    if !bytes.is_empty() {
        st.raw = raw.len() as u64;
        st.prefix = crate::sha256_hex(raw);
        st.observed = wire.now();
    } else if !st.turns.is_empty()
        && !st.pending.iter().any(|queued| {
            st.turns.iter().all(|turn| {
                queued.body["turns"]
                    .as_array()
                    .is_some_and(|turns| turns.contains(turn))
            })
        })
    {
        // DashQ permits adding an earlier binding on an otherwise exact range replay.
        if let Some(mut last) = st.last.clone() {
            last["turns"] = json!(st.turns);
            st.pending.push(Queued {
                body: last,
                replay: true,
            });
        }
    }
    write(&path, &st)?;
    send(data, grant, c, &mut st, &path, wire)?;
    prior_error.map_or(Ok(()), Err)
}
fn identity_of(grant: &Value) -> Value {
    identity(grant)
}
fn fresh(
    grant: &Value,
    identity: &str,
    manifest: &Value,
    capture: &str,
    format: &str,
    at: u64,
) -> Checkpoint {
    let mut manifest = manifest.clone();
    super::scrub::manifest(&mut manifest);
    Checkpoint {
        source: crate::incident::uuid(),
        previous: vec![],
        grant: identity_of(grant),
        identity: identity.into(),
        prefix: crate::sha256_hex(b""),
        raw: 0,
        sent: 0,
        manifest_hash: enroll::digest(&manifest),
        manifest,
        capture: capture.into(),
        format: format.into(),
        pending: vec![],
        last: None,
        turns: vec![],
        bound: vec![],
        observed: at,
        uploaded: 0,
        scrubbed: 0,
        replaced: 0,
        last_upload: None,
    }
}
fn send(
    data: &Path,
    grant: &Value,
    c: &Capture,
    st: &mut Checkpoint,
    path: &Path,
    wire: &mut dyn Wire,
) -> Res<()> {
    while let Some(queued) = st.pending.first().cloned() {
        let body = queued.body;
        let current = active(data)?.ok_or("archive disconnected")?;
        if identity(&current) != identity(grant)
            || !allowed(&current, &c.capture)
            || !enabled(data)?
        {
            return Err("archive grant changed".into());
        }
        if excluded(&c.root, &c.root)? {
            return Err("archive excluded".into());
        }
        let response = wire.post(
            grant["destination"]
                .as_str()
                .ok_or("archive destination invalid")?,
            "/machine/archive/1",
            grant["credential"].as_str(),
            &body,
        );
        let (code, receipt) = response.map_err(|_| "archive offline")?;
        enroll::check(&receipt).map_err(|_| "archive receipt invalid")?;
        if code != 200 || receipt["state"] != "accepted" {
            return Err(match receipt["error"].as_str() {
                Some("manual_disabled") => "archive manual disabled",
                Some("scope") => "archive scope refused",
                Some("replay_mismatch") => "archive replay mismatch",
                Some("range_gap") => "archive range gap",
                Some("deleted") => "archive deleted",
                Some("expired") => "archive expired",
                Some("revoked" | "invalid_credential") => "archive revoked",
                _ => "archive refused",
            }
            .into());
        }
        if receipt["schema"] != SCHEMA
            || receipt["id"].as_str().is_none_or(str::is_empty)
            || ["source", "start", "end", "sha256", "manifest_sha256"]
                .iter()
                .any(|k| receipt[*k] != body[*k])
            || receipt["complete"] != true
        {
            return Err("archive receipt mismatch".into());
        }
        let length = body["end"].as_u64().ok_or("archive receipt invalid")?
            - body["start"].as_u64().ok_or("archive receipt invalid")?;
        if !queued.replay {
            st.uploaded += length;
            st.scrubbed += body["scrubbed"].as_u64().unwrap_or(0);
        }
        st.last_upload = Some(wire.now());
        if body["source"] == st.source {
            for turn in body["turns"].as_array().into_iter().flatten() {
                if let Some(id) = turn["event_id"].as_str() {
                    if !st.bound.iter().any(|bound| bound == id) {
                        st.bound.push(id.into());
                    }
                }
            }
            st.turns.retain(|t| {
                t["event_id"]
                    .as_str()
                    .is_none_or(|id| !st.bound.iter().any(|bound| bound == id))
            });
            st.sent = st
                .sent
                .max(body["end"].as_u64().ok_or("archive receipt invalid")?);
            st.last = Some(body);
        }
        st.pending.remove(0);
        write(path, st)?;
    }
    Ok(())
}

pub fn status(data: &Path) -> Value {
    let grant = match active(data) {
        Ok(g) => g,
        Err(e) => return json!({"state":"unknown","reason":e}),
    };
    let Some(grant) = grant.filter(|g| allowed(g, "launched")) else {
        return json!({"state":"off","manual":false,"reason":"no archive grant"});
    };
    match enabled(data) {
        Ok(false) => return json!({"state":"off","reason":"host disabled"}),
        Err(e) => return json!({"state":"unknown","reason":e}),
        _ => {}
    }
    let file = root(data).join("status.json");
    let summary: Summary = match read(&file) {
        Ok(v) => v,
        Err(e) => {
            return json!({"state":"unknown","manual":allowed(&grant,"manual"),"reason":if file.exists(){e}else{"no archive shipper observation".into()}});
        }
    };
    let mut pending = 0;
    let mut oldest = None;
    let mut uploaded = 0;
    let mut scrubbed = 0;
    let mut replaced = 0;
    let mut last_upload: Option<u64> = None;
    let sources = match files(&root(data).join("sources")) {
        Ok(v) => v,
        Err(e) => return json!({"state":"unknown","reason":e}),
    };
    for path in sources {
        let st: Checkpoint = match read(&path) {
            Ok(v) => v,
            Err(e) => return json!({"state":"unknown","reason":e}),
        };
        if st.grant != identity(&grant) {
            continue;
        }
        uploaded += st.uploaded;
        scrubbed += st.scrubbed;
        replaced += st.replaced;
        last_upload = match (last_upload, st.last_upload) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        for queued in st.pending {
            let body = queued.body;
            pending += body["end"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(body["start"].as_u64().unwrap_or(0));
            oldest = Some(oldest.map_or(st.observed, |v: u64| v.min(st.observed)));
        }
    }
    json!({"state":if summary.error.is_some(){"unknown"}else{"observed"},"manual":allowed(&grant,"manual"),"observed_at":summary.at,"shipper_age":now().saturating_sub(summary.at),
        "last_upload":last_upload,"bytes":uploaded,"scrubbed":scrubbed,"replaced":replaced,"drops":0,
        "pending_bytes":pending,"lag":oldest.map(|at|now().saturating_sub(at)),"reason":summary.error})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Sink {
        requests: Vec<Value>,
        offline: bool,
        lose: bool,
        mismatch: bool,
    }
    impl Wire for Sink {
        fn post(
            &mut self,
            _: &str,
            route: &str,
            _: Option<&str>,
            body: &Value,
        ) -> Res<(u16, Value)> {
            assert_eq!(route, "/machine/archive/1");
            if self.offline {
                return Err("offline".into());
            }
            self.requests.push(body.clone());
            if self.lose {
                self.lose = false;
                return Err("lost response".into());
            }
            let mut receipt = json!({"protocol":1,"schema":SCHEMA,"state":"accepted","id":"sink-session","source":body["source"],"start":body["start"],"end":body["end"],"sha256":body["sha256"],"manifest_sha256":body["manifest_sha256"],"complete":true});
            if self.mismatch {
                receipt["sha256"] = json!("wrong");
            }
            Ok((200, receipt))
        }
        fn now(&self) -> u64 {
            1800000000
        }
        fn sleep(&mut self, _: u64) {
            panic!("archive adds no retry delay");
        }
        fn show(&mut self, _: &str, _: &str) {}
    }
    pub(super) fn setup(tag: &str) -> (PathBuf, Capture) {
        let d = crate::scratch(tag);
        let data = d.join("data");
        private(&root(&data)).unwrap();
        private(&connection(&data)).unwrap();
        write(&connection(&data).join("connection.json"),&json!({"installation":"fixture","active":{"destination":"https://fixture.invalid","workspace":"w","installation":"fixture","generation":1,"credential":"fixture-credential","schemas":[SCHEMA],"capabilities":[]},"pending":null,"revoke":[]})).unwrap();
        let project = d.join("project");
        fs::create_dir_all(&project).unwrap();
        let home = d.join("native-home");
        fs::create_dir_all(&home).unwrap();
        let log = project.join("turn.jsonl");
        fs::write(&log,b"{\"type\":\"assistant\",\"session_id\":\"s-1\",\"message\":{\"content\":\"sk-fixture_secret\"}}\n").unwrap();
        let capture = Capture {
            id: "turn-1".into(),
            root: project,
            runtime: "claude".into(),
            home: Some(home),
            manifest: json!({"project":"fixture/project","task":283,"attempt":1,"actor":"worker","entry":"fixture","runtime":"claude","cli_version":null,"model":null,"effort":null,"policy":null,"account_label":null,"host":"fixture","session":"s-1","parent_session":null,"instructions_hash":null}),
            capture: "launched".into(),
            sources: vec![Source {
                path: log,
                format: "claude-stream".into(),
                start: 0,
                end: None,
            }],
            turn: None,
            grant: Value::Null,
        };
        (data, capture)
    }
    // A4/A6: lost response and process restart replay precisely the persisted scrubbed request.
    // A changed/truncated native file gets a different source; accepted bytes remain immutable.
    #[test]
    fn offline_replay_replacement_and_late_turn() {
        let (data, mut capture) = setup("archive-resume");
        register(&data, capture.clone()).unwrap();
        let mut sink = Sink {
            lose: true,
            ..Sink::default()
        };
        pass(&data, &mut sink).unwrap();
        assert_eq!(status(&data)["state"], "unknown");
        assert!(status(&data)["pending_bytes"].as_u64().unwrap() > 0);
        assert!(!sink.requests[0].to_string().contains("fixture_secret"));
        let first = sink.requests[0].clone();
        pass(&data, &mut sink).unwrap();
        assert_eq!(sink.requests[1], first);
        capture.turn = Some("exact-event".into());
        register(&data, capture.clone()).unwrap();
        pass(&data, &mut sink).unwrap();
        assert_eq!(
            sink.requests.last().unwrap()["turns"],
            json!([{"event_id":"exact-event","start":0}])
        );
        assert_eq!(status(&data)["pending_bytes"], 0);
        let bytes = status(&data)["bytes"].as_u64().unwrap();
        assert_eq!(bytes, first["end"].as_u64().unwrap());
        fs::write(&capture.sources[0].path, b"{\"new\":true}\n").unwrap();
        pass(&data, &mut sink).unwrap();
        let last = sink.requests.last().unwrap();
        assert_ne!(last["source"], first["source"]);
        assert_eq!(last["start"], 0);
        assert_eq!(status(&data)["replaced"], 1);
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
    // A3: late caller conditions must not bind a turn to the earlier unknown manifest.
    #[test]
    fn frozen_attribution_preserves_accepted_live_bytes() {
        let (data, mut capture) = setup("archive-attribution");
        capture.manifest["project"] = json!("other");
        capture.manifest["task"] = Value::Null;
        register(&data, capture.clone()).unwrap();
        let mut sink = Sink::default();
        pass(&data, &mut sink).unwrap();
        let live = sink.requests[0].clone();
        capture.manifest["project"] = json!("fixture/project");
        capture.manifest["task"] = json!(283);
        capture.turn = Some("frozen-turn".into());
        register(&data, capture).unwrap();
        pass(&data, &mut sink).unwrap();
        let attributed = sink.requests.last().unwrap();
        assert_ne!(attributed["source"], live["source"]);
        assert_eq!(attributed["bytes"], live["bytes"]);
        assert_eq!(attributed["manifest"]["task"], 283);
        assert_eq!(
            attributed["turns"],
            json!([{"event_id":"frozen-turn","start":0}])
        );
        let n = sink.requests.len();
        pass(&data, &mut sink).unwrap();
        assert_eq!(sink.requests.len(), n, "attribution replay is idempotent");
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
    // A4/A6: chunk boundaries and a restart cannot lose the secret's prefix and expose its remainder.
    #[test]
    fn chunks_are_scrubbed_and_durable_before_sending() {
        let (data, capture) = setup("archive-chunks");
        let text = format!(
            "{{\"text\":\"{}sk-{}\"}}\n",
            "a".repeat(enroll::MAX_BODY),
            "secret".repeat(enroll::MAX_BODY / 6)
        );
        fs::write(&capture.sources[0].path, &text).unwrap();
        register(&data, capture.clone()).unwrap();
        let mut sink = Sink {
            offline: true,
            ..Sink::default()
        };
        pass(&data, &mut sink).unwrap();
        // The original file disappears while offline: every scrubbed chunk was already persisted.
        fs::write(&capture.sources[0].path, b"{\"replacement\":true}\n").unwrap();
        sink.offline = false;
        pass(&data, &mut sink).unwrap();
        assert!(sink.requests.len() > 2);
        assert!(
            sink.requests
                .iter()
                .all(|r| r.to_string().len() <= enroll::MAX_BODY)
        );
        let old = &sink.requests[0]["source"];
        let mut next = 0;
        for request in sink.requests.iter().filter(|r| &r["source"] == old) {
            assert_eq!(request["start"].as_u64(), Some(next));
            next = request["end"].as_u64().unwrap();
            assert_eq!(
                request["manifest_sha256"],
                enroll::digest(&request["manifest"])
            );
        }
        assert_eq!(next, text.len() as u64);
        assert_eq!(status(&data)["pending_bytes"], 0);
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
    // A1/A6/A7: manual needs its own capability; exclusions and disconnect stop pending delivery.
    #[test]
    fn authority_exclusions_and_receipt_fencing() {
        let (data, mut capture) = setup("archive-authority");
        let mut sink = Sink::default();
        capture.capture = "manual".into();
        register(&data, capture.clone()).unwrap();
        pass(&data, &mut sink).unwrap();
        assert!(sink.requests.is_empty());
        capture.capture = "launched".into();
        register(&data, capture.clone()).unwrap();
        sink.mismatch = true;
        pass(&data, &mut sink).unwrap();
        assert_eq!(status(&data)["bytes"], 0);
        assert!(status(&data)["pending_bytes"].as_u64().unwrap() > 0);
        fs::write(
            capture.root.join(".agentq-archive.json"),
            b"{\"exclude\":[\"turn.jsonl\"]}",
        )
        .unwrap();
        let n = sink.requests.len();
        sink.mismatch = false;
        pass(&data, &mut sink).unwrap();
        assert_eq!(sink.requests.len(), n);
        fs::write(capture.root.join(".agentq-archive.json"), b"invalid").unwrap();
        pass(&data, &mut sink).unwrap();
        assert_eq!(status(&data)["reason"], "archive exclusions invalid");
        let connection_file = connection(&data).join("connection.json");
        let mut value: Value = read(&connection_file).unwrap();
        value["active"] = Value::Null;
        write(&connection_file, &value).unwrap();
        pass(&data, &mut sink).unwrap();
        assert_eq!(sink.requests.len(), n);
        assert_eq!(status(&data)["state"], "off");
        let _ = fs::remove_dir_all(data.parent().unwrap());
    }
}

#[cfg(test)]
#[path = "local_sink.rs"]
mod local_sink;
