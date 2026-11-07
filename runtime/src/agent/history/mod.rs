//! Read-only, bounded native history (agentq.md §12; RUNTIME.md history wire contract).
use super::{Res, native};
mod steps;
mod requests;
pub mod sessions;
pub mod stop;
pub mod live;
pub use steps::{Step, StepKind, StepPage, Turn};
use serde_json::{Value, json};
use std::{fs::File, io::{Read, Seek, SeekFrom}, path::Path};

const BUDGET: usize = 16 * 1024 * 1024;
const FORMATS: &[&str] = &["codex-exec", "codex-rollout", "codex-app-server", "claude-stream", "claude-native", "agy-stream", "antigravity-sidecar", "acp", "jsonl-rpc"];

/// Adapter support is distinct from native qualification. Version and OS are caller readback.
pub fn capabilities(format: &str, native_version: Option<&str>, os: &str) -> Res<Value> {
    if !FORMATS.contains(&format) { return Err("history transport unsupported".into()); }
    let sidecar = format == "antigravity-sidecar";
    let app = format == "codex-app-server";
    let mut features = serde_json::Map::new();
    for feature in ["launch", "resume", "session", "input", "files_images", "ask_reply", "tools_outputs", "heartbeat", "cancel_recovery", "model_effort_accounts_permissions", "usage_errors", "history"] {
        let (status, reason) = match feature {
            "history" if format == "agy-stream" => ("limited", "completed steps may omit text; deltas/result only; unknown shapes retained raw"),
            "history" if sidecar => ("limited", "truncated fields, GENERIC author unknown and raw-only attachments; caller selects full source"),
            "history" => ("supported", "bounded source adapter; unknown fields retained raw"),
            "tools_outputs" if sidecar => ("limited", "native calls retained; GENERIC output has unknown tool/actor/call ID; TOOL_RESPONSE schema fixture only"),
            "heartbeat" => ("limited", "source progress and process poll only; no useful-work inference"),
            "files_images" => ("limited", "caller arguments or native typed references only; no media decoding"),
            "ask_reply" if !app => ("unsupported", "no added interactive reply channel; native caller control only"),
            "cancel_recovery" if sidecar => ("limited", "submission kill is not server cancellation; original ownership retained"),
            "model_effort_accounts_permissions" => ("limited", "caller arguments/environment/parameters; no grant or catalogue inference"),
            _ => ("supported", "existing Turn/NativeTurn/AppSession boundary; not a history receipt"),
        };
        features.insert(feature.into(), json!({"status":status,"reason":reason}));
    }
    Ok(json!({"schema":1,"format":format,"native_version":native_version,"os":os,"features":features,
        "native":{"status":"untested","reason":"history API not qualified on a native version/OS tuple"},
        "evidence":["runtime/tests/history.rs","runtime/tests/agent.rs"],
        "live_output":{"status":if app {"supported"}else{"unsupported"},"reason":if app {"item/commandExecution/outputDelta"}else{"no native live console delta proof"}},
        "limits":{"page_bytes":BUDGET,"record_bytes":BUDGET,"page_events":128}}))
}

/// An observation cursor, never execution or stop authority. Protect its checkpoint as private data.
#[derive(Clone, Debug)]
pub struct History {
    format: String,
    source: String,
    session: String,
    native_session: String,
    native_session_alias: Option<String>,
    native_parent: Option<String>,
    offset: u64,
    identity: Option<String>,
    prefix: String,
}

#[derive(Debug)]
pub struct Page {
    pub events: Vec<Value>,
    /// Forward: unconsumed source bytes. Backward: older bytes remain.
    pub more: bool,
    /// Forward: unconsumed read bytes. Backward: excluded incomplete tail bytes.
    pub pending_bytes: usize,
}

impl History {
    pub fn new(format: &str, source: &str, session: &str) -> Res<Self> {
        if super::registry::history_protocol(format).is_none() { return Err("history transport unsupported".into()); }
        if source.is_empty() || session.is_empty() { return Err("history source/session required".into()); }
        Ok(Self {format:format.into(), source:source.into(), session:session.into(), native_session:session.into(), native_session_alias:None, native_parent:None, offset:0,
            identity:None, prefix:crate::sha256_hex(b"")})
    }

    /// Bind a caller-validated copied Claude job; this grants observation only.
    pub fn with_native_session_alias(format:&str, source:&str, session:&str, native_session:&str)->Res<Self> {
        let mut h=Self::new(format,source,session)?;
        if !super::registry::history_protocol(format).is_some_and(|p|p.history_alias) || native_session.is_empty() || native_session==session {
            return Err("history native session alias invalid".into());
        }
        h.native_session_alias=Some(native_session.into());Ok(h)
    }

    pub fn checkpoint(&self) -> Value {
        let mut v=json!({"schema":1,"format":self.format,"source":self.source,"session":self.session,
            "native_session":self.native_session,"native_parent":self.native_parent,
            "offset":self.offset,"identity":self.identity,"prefix":self.prefix});
        if let Some(alias)=&self.native_session_alias {v["native_session_alias"]=json!(alias);}
        v
    }

    pub fn recover(v: &Value) -> Res<Self> {
        if v["schema"] != 1 { return Err("history checkpoint schema invalid".into()); }
        let field = |k: &str| v[k].as_str().ok_or_else(|| format!("history checkpoint {k} invalid"));
        let mut h = Self::new(field("format")?, field("source")?, field("session")?)?;
        match v.get("native_session_alias") {
            None | Some(Value::Null) => {},
            Some(Value::String(alias)) => h=Self::with_native_session_alias(&h.format,&h.source,&h.session,alias)?,
            _ => return Err("history native session alias invalid".into()),
        }
        if let Some(native)=v.get("native_session") {h.native_session=native.as_str().filter(|s|!s.is_empty()).ok_or("history checkpoint native session invalid")?.into();}
        h.native_parent=match &v["native_parent"] {Value::Null=>None,Value::String(s) if !s.is_empty()=>Some(s.clone()),_=>return Err("history checkpoint native parent invalid".into())};
        h.offset = v["offset"].as_u64().ok_or("history checkpoint offset invalid")?;
        h.prefix = field("prefix")?.into();
        if h.prefix.len() != 64 || !h.prefix.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("history checkpoint prefix invalid".into()); }
        h.identity = match &v["identity"] { Value::Null if h.offset == 0 => None,
            Value::String(s) if !s.is_empty() => Some(s.clone()), _ => return Err("history checkpoint identity invalid".into()) };
        Ok(h)
    }

    /// A failed poll leaves the cursor untouched. No source is edited or input submitted.
    pub fn poll(&mut self, path: &Path) -> Res<Page> {
        let (mut file, identity) = self.open(path)?;
        file.seek(SeekFrom::Start(self.offset)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&mut file).take(BUDGET as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let mut consumed = 0;
        let mut events = Vec::new();
        for row in bytes.split_inclusive(|b| *b == b'\n') {
            if events.len() == 128 { break; }
            if row.len() > BUDGET { return Err("history record exceeds 16 MiB".into()); }
            if !row.ends_with(b"\n") || consumed + row.len() > BUDGET { break; }
            let raw = std::str::from_utf8(row).map_err(|_| "history invalid UTF-8")?;
            if raw.trim().is_empty() {consumed+=row.len();continue;}
            let v: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !v.is_object() { return Err("history invalid record".into()); }
            events.push(self.event(&v, raw, self.offset + consumed as u64)?);
            consumed += row.len();
        }
        let end = self.offset + consumed as u64;
        let prefix = native::digest_prefix(&mut file, end)?;
        let more = consumed < bytes.len();
        self.offset = end; self.prefix = prefix; self.identity = Some(identity);
        Ok(Page {events, more, pending_bytes:bytes.len()-consumed})
    }

    /// Read older records before this cursor, or from EOF on a fresh cursor.
    /// Save/recover checkpoints exactly as for `poll`; failure never advances the cursor.
    pub fn poll_before(&mut self, path: &Path) -> Res<Page> {
        let (mut file, identity) = self.open(path)?;
        let bound = if self.identity.is_none() { file.metadata().map_err(|e| e.to_string())?.len() } else { self.offset };
        let start = bound.saturating_sub(BUDGET as u64 + 1);
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        (&mut file).take(bound - start).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != bound - start { return Err("history truncated".into()); }
        let mut end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        let pending_bytes = bytes.len() - end;
        let mut events = Vec::new();
        let mut used = 0;
        while end > 0 && events.len() < 128 {
            let row_start = match bytes[..end-1].iter().rposition(|b| *b == b'\n') {
                Some(i) => i + 1,
                None if start == 0 => 0,
                None => break,
            };
            let row = &bytes[row_start..end];
            if row.len() > BUDGET { return Err("history record exceeds 16 MiB".into()); }
            if used + row.len() > BUDGET { break; }
            let raw = std::str::from_utf8(row).map_err(|_| "history invalid UTF-8")?;
            if raw.trim().is_empty() {used+=row.len();end=row_start;continue;}
            let v: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !v.is_object() { return Err("history invalid record".into()); }
            events.push(self.event(&v, raw, start + row_start as u64)?);
            used += row.len();
            end = row_start;
        }
        if (used == 0 && pending_bytes == 0 && bound > 0) || pending_bytes > BUDGET {
            return Err("history record exceeds 16 MiB".into());
        }
        // Skip only the incomplete EOF tail; never start inside a complete record.
        let offset = start + end as u64;
        let prefix = native::digest_prefix(&mut file, offset)?;
        events.reverse();
        self.offset = offset; self.prefix = prefix; self.identity = Some(identity);
        Ok(Page {events, more:offset > 0, pending_bytes})
    }

    fn open(&self, path: &Path) -> Res<(File, String)> {
        let mut file = File::open(path).map_err(|e| format!("history unavailable: {e}"))?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() { return Err("history not a regular file".into()); }
        let identity = native::file_id(&file)?;
        if self.identity.as_ref().is_some_and(|id| *id != identity) { return Err("history replaced".into()); }
        if native::digest_prefix(&mut file, self.offset)? != self.prefix { return Err("history truncated or rewritten".into()); }
        if self.offset > 0 {
            file.seek(SeekFrom::Start(self.offset - 1)).map_err(|e| e.to_string())?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(|e| e.to_string())?;
            if byte[0] != b'\n' { return Err("history cursor is not a record boundary".into()); }
        }
        Ok((file, identity))
    }

    fn event(&self, v: &Value, raw: &str, offset: u64) -> Res<Value> {
        let protocol=super::registry::history_protocol(&self.format).ok_or("history transport unsupported")?;
        let native_session=requests::session(v).or_else(||(protocol.history_session)(&self.format,v,raw));
        let provenance=(protocol.history_provenance)(&self.format,v,offset);
        if !provenance && native_session.as_deref().is_some_and(|s|s!=self.native_session && Some(s)!=self.native_session_alias.as_deref()) {return Err("history native session differs".into());}
        if self.native_parent.as_ref().is_some_and(|parent|(protocol.history_parent)(v).as_ref().is_some_and(|id|id!=parent)) {return Err("history native parent differs".into());}
        let (author,kind,item,blocks,limited)=(protocol.history_normalize)(&self.format,v);
        let validity=(protocol.history_validity)(v);
        let hash = crate::sha256_hex(raw.as_bytes());
        let id = crate::sha256_hex(json!([self.format,self.source,self.session,offset,hash]).to_string().as_bytes());
        let time = v.get("timestamp").or_else(|| v.get("time"))
            .or_else(|| v.get("emittedAtMs"))
            .or_else(|| if self.format == "antigravity-sidecar" {v.get("created_at")}else{None})
            .cloned().unwrap_or(Value::Null);
        let aliased=native_session.as_deref().is_some_and(|s|Some(s)==self.native_session_alias.as_deref());
        let mut event=json!({"id":id,"source":self.source,"session":self.session,"session_binding":if provenance {"caller"}else if aliased {"alias"} else if native_session.is_some(){"native"}else{"caller"},"provenance":provenance,"format":self.format,
            "order":offset,"start":offset,"end":offset+raw.len() as u64,"sha256":hash,"raw":raw,
            "validity":validity,"assistant_failure":(protocol.assistant_failure)(v),
            "lifecycle":(protocol.lifecycle)(v),"permission_mode":(protocol.permission_mode)(v),
            "time":time,"author":author,"kind":kind,"item_id":item,"blocks":blocks,"limited":limited});
        if aliased {event["native_session"]=json!(native_session);}
        Ok(event)
    }
}

pub(crate) type Normalized=(&'static str,&'static str,Value,Vec<Value>,&'static str);
