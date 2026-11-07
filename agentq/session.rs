//! Shared launch attribution, project discovery and retained session identity; independent of legacy model export.
use ring::digest;
use serde_json::{Map,Value,json};
use std::path::{Path,PathBuf};
use std::time::Duration;

/// Same installation and call ID, same event: a duplicate counts once and a later reading replaces it.
pub fn event_id(installation: &str, call: &str) -> String {
    crate::resource::hex(digest::digest(&digest::SHA256, format!("{installation}:model:{call}").as_bytes()).as_ref())[..32].to_string()
}

/// M6.9: TaskQ or another launcher (PilotQ) started this client; CoreQ `agent` records its turns, so agentq counts none.
pub fn launched(env: impl Fn(&str) -> Option<String>) -> bool {
    ["TASKQ_TASK", "AGENTQ_LAUNCHER"].iter().any(|k| env(k).is_some_and(|v| !v.is_empty()))
}

/// M6.10: `owner/name` of the nearest `taskq.json` `repo` at or above `dir`, else the unknown reason.
pub fn project(dir: &Path) -> Result<String, &'static str> {
    let file = dir.ancestors().map(|d| d.join("taskq.json")).find(|f| f.is_file()).ok_or("no known project")?;
    let v: Value = std::fs::read_to_string(file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    let part = |p: &str| !p.is_empty() && !p.starts_with('.') && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    v["repo"].as_str().filter(|r| r.len() <= 64 && r.split_once('/').is_some_and(|(o, n)| part(o) && part(n))).map(String::from).ok_or("no known project")
}

pub(crate) fn set_project(a: &mut Map<String, Value>, p: Result<String, &str>) {
    match p {
        Ok(code) => a.insert("agentq.project".into(), json!(code)),
        Err(reason) => a.insert("agentq.project_unknown".into(), json!(reason)),
    };
}

/// `data/sessions/<hash>.json`: what the Codex `SessionStart` hook saw of a session the service cannot see.
pub(crate) fn session_file(data: &Path, session: &str) -> PathBuf {
    data.join("sessions").join(format!("{}.json", event_id("session", session)))
}

/// Codex `SessionStart`: record whether a launcher started the session and its project code, never its path.
pub fn session_start(data: &Path, input: &Value, launched: bool, cwd: Option<&Path>) -> Result<(), String> {
    let session = input["session_id"].as_str().filter(|s| !s.is_empty()).ok_or("SessionStart has no session_id")?;
    let mut record = Map::new();
    record.insert("launched".into(), json!(launched));
    set_project(&mut record, input["cwd"].as_str().filter(|c| !c.is_empty()).map(Path::new).or(cwd).ok_or("no working directory").and_then(project));
    std::fs::create_dir_all(data.join("sessions")).map_err(|e| format!("session state: {e}"))?;
    coreq::write_atomic(&session_file(data, session), &Value::Object(record).to_string()).map_err(|e| format!("session state: {e}"))?;
    expire(&data.join("sessions"));
    Ok(())
}

/// Files in `dir` untouched for 7 days go: transcript marks (a backlog of a deleted transcript included) and session records.
pub fn expire(dir: &Path) {
    let week = std::time::SystemTime::now() - Duration::from_secs(7 * 86400);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < week) { let _ = std::fs::remove_file(e.path()); }
    }
}

