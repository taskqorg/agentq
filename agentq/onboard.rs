//! Owned config edits for Codex, Claude Code and Hermes with the ownership journal (agentq.md § 4, § 8, § 9).
//!
//! ponytail: MCP registration, hooks and Codex routing. Claude Code routing, telemetry env and the
//! instruction block join with their own tasks.

use coreq::write_atomic;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

pub const AGENTS: [&str; 3] = ["codex", "claude", "hermes"];

// Conversations retain this provider ID; no base_url lets Codex use its native authenticated endpoint.
pub const CODEX_DIRECT_PROVIDER: &str = "# agentq: direct OpenAI for saved conversations\n[model_providers.agentq]\nname = \"OpenAI\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = true\n";

fn direct_codex_provider(text: &str) -> bool {
    text.matches("[model_providers.agentq]").count() == 1
        && !text.contains("[model_providers.agentq.")
        && text.split_once(CODEX_DIRECT_PROVIDER).is_some_and(|(_, tail)| {
            tail.lines().take_while(|l| !l.trim_start().starts_with('[')).all(|l| l.trim().is_empty())
        })
}

/// One owned edit. `.json`: `value` at `key`, or appended to the array there. `.toml`, `.yaml`: a rendered block at
/// `key`; a one-part `.toml` key is a top-level `key = value` line. `routing` names the agent of a routing edit.
pub struct Want {
    file: PathBuf,
    key: Vec<String>,
    value: Value,
    append: bool,
    routing: Option<String>,
    agent: String,
    instruction: bool,
}

impl Want {
    /// The configuration field this edit serves (CONFIGURATION A3): `routing`, `hooks` or `mcp`.
    pub fn field(&self) -> &'static str {
        if self.instruction { return "instructions"; }
        match (&self.routing, self.key[0].as_str()) {
            (Some(_), _) => "routing",
            (None, "hooks") => "hooks",
            (None, "plugins" | "$file") => "plugin",
            _ => "mcp",
        }
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

/// The client's own env, else the default home folder (§ 4).
pub fn root(agent: &str) -> PathBuf {
    let (var, dir) = match agent {
        "codex" => ("CODEX_HOME", ".codex"),
        "claude" => ("CLAUDE_CONFIG_DIR", ".claude"),
        _ => ("HERMES_HOME", ".hermes"),
    };
    std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| home().join(dir))
}

/// Claude Code keeps `.claude.json` in `CLAUDE_CONFIG_DIR` when set, else in the home folder.
pub fn claude_json() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map(|d| PathBuf::from(d).join(".claude.json")).unwrap_or_else(|| home().join(".claude.json"))
}

pub fn wants(agent: &str, root: &Path, claude_json: &Path, exe: &Path) -> Vec<Want> {
    let exe = exe.display().to_string();
    let hook = |event: &str| json!({"type": "command", "command": format!("\"{exe}\" hook {agent} {event}"), "timeout": 5});
    let tools: Vec<Value> = crate::mcp::tools().as_array().into_iter().flatten().map(|t| t["name"].clone()).collect();
    let w = |file: PathBuf, key: &[&str], value: Value, append: bool| Want { file, key: key.iter().map(|k| k.to_string()).collect(), value, append, routing: None, agent: agent.into(), instruction: false };
    match agent {
        "codex" => vec![
            w(root.join("config.toml"), &["mcp_servers", "agentq"], json!({"command": exe, "args": ["mcp", "codex"], "enabled_tools": tools}), false),
            w(root.join("hooks.json"), &["hooks", "SessionStart"], json!({"hooks": [hook("SessionStart")]}), true),
            w(root.join("hooks.json"), &["hooks", "Stop"], json!({"hooks": [hook("Stop")]}), true),
        ],
        "claude" => vec![
            w(root.join("settings.json"), &["hooks", "SessionStart"], json!({"hooks": [hook("SessionStart")]}), true),
            w(root.join("settings.json"), &["hooks", "PreToolUse"], json!({"matcher": "Bash", "hooks": [hook("PreToolUse")]}), true),
            w(root.join("settings.json"), &["hooks", "PostToolUse"], json!({"matcher": "Bash", "hooks": [hook("PostToolUse")]}), true),
            w(root.join("settings.json"), &["hooks", "Stop"], json!({"hooks": [hook("Stop")]}), true),
            w(claude_json.into(), &["mcpServers", "agentq"], json!({"type": "stdio", "command": exe, "args": ["mcp", "claude"]}), false),
        ],
        // Desktop Chat is MCP-only (CONFIGURATION A1), with its own control identity.
        "claude-desktop" => vec![w(root.join("claude_desktop_config.json"), &["mcpServers", "agentq"], json!({"command": exe, "args": ["mcp", "claude-desktop"]}), false)],
        _ => vec![
            w(root.join("plugins/agentq/__init__.py"), &["$file"], json!(include_str!("../integrations/hermes/__init__.py").replace("@AGENTQ_EXE@", &json!(exe).to_string())), false),
            w(root.join("plugins/agentq/plugin.yaml"), &["$file"], json!(include_str!("../integrations/hermes/plugin.yaml")), false),
            w(root.join("config.yaml"), &["plugins", "enabled"], json!("agentq"), true),
            w(root.join("config.yaml"), &["mcp_servers", "agentq"], json!({"command": exe, "args": ["mcp", "hermes"]}), false),
        ],
    }
}

/// Codex routing edits to the agent's loopback port (§ 4); `agentq direct` removes only these.
pub fn routing_wants(agent: &str, root: &Path, port: u16) -> Vec<Want> {
    let w = |key: &[&str], value: Value| Want { file: root.join("config.toml"), key: key.iter().map(|k| k.to_string()).collect(), value, append: false, routing: Some(agent.into()), agent: agent.into(), instruction: false };
    match agent {
        "codex" => vec![
            w(&["model_provider"], "agentq".into()),
            w(&["model_providers", "agentq"], json!({"name": "AgentQ", "base_url": crate::routing::base_url(port), "wire_api": "responses", "requires_openai_auth": true, "supports_websockets": true})),
        ],
        _ => vec![],
    }
}

pub(crate) fn load(journal: &Path) -> Vec<Value> {
    std::fs::read_to_string(journal).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// `orig` copies configs that may hold env secrets: owner-only, gone with the install folder on uninstall.
fn save(journal: &Path, j: &[Value]) -> Result<(), String> {
    write_atomic(journal, &serde_json::to_string_pretty(j).unwrap_or_default())?;
    #[cfg(unix)]
    std::fs::set_permissions(journal, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(|e| format!("{}: {e}", journal.display()))?;
    Ok(())
}

fn entry(key: &[String], kind: &str, after: Value) -> Value {
    json!({"key": key, "kind": kind, "before": null, "after": after})
}

fn read_config(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// File by file: a conflict leaves that file untouched; every edit is journaled before the write. Repeat is safe.
pub fn apply(wants: &[Want], journal: &Path) -> Result<Vec<String>, String> {
    // Do not enable a plugin whose source files cannot be installed safely.
    for w in wants.iter().filter(|w| w.key == ["$file"]) {
        let old = read_config(&w.file)?;
        place(old.as_deref().unwrap_or(""), w)?;
    }
    let mut out = vec![];
    let mut files: Vec<&Path> = wants.iter().map(|w| w.file.as_path()).collect();
    files.sort();
    files.dedup();
    for f in files {
        let old = read_config(f)?;
        let mut text = old.clone().unwrap_or_default();
        let mut entries = vec![];
        let placed = wants.iter().filter(|w| w.file == f).try_for_each(|w| {
            let (t, e) = place(&text, w)?;
            entries.extend(e.map(|mut e| {
                e["agent"] = w.agent.clone().into();
                if let Some(a) = &w.routing {
                    e["routing"] = a.as_str().into();
                }
                e
            }));
            text = t;
            Ok::<(), String>(())
        });
        match placed {
            Err(e) => out.push(format!("conflict: {} {e}; file unchanged", f.display())),
            Ok(()) if entries.is_empty() => out.push(format!("kept: {}", f.display())),
            Ok(()) => {
                for e in &mut entries {
                    e["file"] = f.display().to_string().into();
                }
                match &old {
                    None => entries[0]["created"] = true.into(),
                    // restore writes these bytes back once the doc matches them again
                    Some(o) if entries[0]["kind"] != "text" => entries[0]["orig"] = o.as_str().into(),
                    Some(_) => {}
                }
                let n = entries.len();
                let mut j = load(journal);
                j.extend(entries);
                if let Some(d) = journal.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                save(journal, &j)?;
                if let Some(parent) = f.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                write_atomic(f, &text)?;
                let bom = if old.as_deref().is_some_and(|o| o.starts_with(BOM) && !text.starts_with(BOM)) { "; UTF-8 BOM stripped" } else { "" };
                out.push(format!("edited: {} ({n} owned keys{bom})", f.display()));
            }
        }
    }
    Ok(out)
}

/// Select supported owned fields, expanding old JSON parent receipts to the owned leaves only.
/// This lets a later foreign MCP/hook share the same parent without being removed or blocking its sibling.
pub fn capture(wants: &[Want], journal: &Path) -> Result<Vec<Value>, String> {
    let j = match std::fs::read(journal) {
        Ok(b) => serde_json::from_slice::<Vec<Value>>(&b).map_err(|_| "owned.json unreadable")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(_) => return Err("owned.json unreadable".into()),
    };
    let mut out = vec![];
    for e in j {
        let file = e["file"].as_str().ok_or("invalid owned entry")?;
        if !wants.iter().any(|w| w.file == Path::new(file)) { continue; }
        let keys: Vec<String> = e["key"].as_array().ok_or("invalid owned key")?.iter()
            .map(|k| k.as_str().map(String::from).ok_or("invalid owned key")).collect::<Result<_, _>>()?;
        let source = crate::configure::digest(&e);
        let mut matched = false;
        for w in wants.iter().filter(|w| w.file == Path::new(file) && w.key.starts_with(&keys)) {
            if e["agent"].as_str().is_some_and(|a| a != w.agent) { continue; }
            let mut n = e.clone();
            n.as_object_mut().ok_or("invalid owned entry")?.shift_remove("orig");
            if w.instruction && !e["after"].as_str().is_some_and(|s| s.trim().starts_with("<!-- agentq:start -->") && s.trim().ends_with("<!-- agentq:end -->")
                && s.matches("<!-- agentq:start -->").count() == 1 && s.matches("<!-- agentq:end -->").count() == 1) {
                return Err("instruction receipt contains foreign text".into());
            }
            if e["kind"] == "set" && Path::new(file).extension().is_some_and(|x| x == "json") {
                let mut v = &e["after"];
                for key in &w.key[keys.len()..] { v = &v[key]; }
                if v.is_null() { continue; }
                n["key"] = json!(w.key);
                n["after"] = v.clone();
                if w.append {
                    let items = v.as_array().ok_or("invalid owned hook array")?;
                    for item in items {
                        let mut item_entry = n.clone();
                        item_entry["kind"] = "append".into();
                        item_entry["after"] = item.clone();
                        item_entry["source"] = source.clone().into();
                        item_entry["field"] = w.field().into();
                        item_entry["agent"] = w.agent.clone().into();
                        out.push(item_entry);
                    }
                    matched = true;
                    continue;
                }
            } else if w.key != keys { continue; }
            if w.key != ["$file"] && matches!(w.file.extension().and_then(|s| s.to_str()), Some("yaml" | "yml")) {
                let after = e["after"].as_str().ok_or("invalid owned YAML")?;
                if w.key == ["plugins", "enabled"] {
                    let line = after.split_inclusive('\n').find(|l| l.trim() == "- agentq").ok_or("unresolved plugin receipt")?;
                    n["after"] = line.into();
                } else if w.key == ["mcp_servers", "agentq"] {
                    let at = after.find("agentq:").ok_or("unresolved MCP receipt")?;
                    let start = after[..at].rfind('\n').map_or(0, |n| n + 1);
                    n["after"] = after[start..].into();
                } else { return Err("unresolved YAML scope".into()); }
                n["kind"] = "text".into();
                n["before"] = Value::Null;
                n["scoped_yaml"] = true.into();
            }
            n["source"] = source.clone().into();
            n["field"] = w.field().into();
            n["agent"] = w.agent.clone().into();
            out.push(n);
            matched = true;
        }
        if !matched { return Err("unresolved owned scope".into()); }
    }
    Ok(out)
}

/// Reapply the current canonical writer only for the fields present in the saved owned receipts.
pub fn saved_wants(wants: Vec<Want>, entries: &[Value]) -> Result<Vec<Want>, String> {
    if entries.iter().any(|e| !wants.iter().any(|w| {
        let kind = if w.instruction { e["kind"] == "text" }
            else if w.key == ["$file"] { e["kind"] == "file" }
            else if w.file.extension().is_some_and(|s| s == "json") { e["kind"] == if w.append { "append" } else { "set" } }
            else { e["kind"] == "text" || e["kind"] == "replace" };
        e["file"] == w.file.display().to_string() && e["key"] == json!(w.key) && e["field"] == w.field() && kind
    })) {
        return Err("saved profile scope changed".into());
    }
    Ok(wants.into_iter().filter(|w| entries.iter().any(|e| e["file"] == w.file.display().to_string() && e["key"] == json!(w.key))).collect())
}

pub fn instruction_wants(agent: &str, root: &Path, entries: &[Value]) -> Vec<Want> {
    if agent == "claude-desktop" { return vec![]; }
    let file = root.join(if agent == "claude" { "CLAUDE.md" } else { "AGENTS.md" });
    let mut out = vec![];
    for e in entries.iter().filter(|e| e["file"] == file.display().to_string()) {
        let Some(keys) = e["key"].as_array() else { continue };
        let Some(key) = keys.iter().map(Value::as_str).collect::<Option<Vec<_>>>() else { continue };
        let key: Vec<_> = key.into_iter().map(String::from).collect();
        if key.is_empty() || out.iter().any(|w: &Want| w.key == key) { continue; }
        out.push(Want { file: file.clone(), key, value: crate::mcp::INSTRUCTIONS.into(), append: false, routing: None,
            agent: agent.into(), instruction: true });
    }
    out
}

pub fn unowned_present(wants: &[Want], entries: &[Value]) -> Result<bool, String> {
    for w in wants {
        if w.key == ["$file"] || entries.iter().any(|e| e["file"] == w.file.display().to_string() && e["key"] == json!(w.key)) { continue; }
        let Some(text) = read_config(&w.file)? else { continue };
        if w.field() == "routing" && !text.lines().take_while(|l| !l.trim_start().starts_with('[')).any(|l| {
            l.split_once('=').is_some_and(|(key, v)| key.trim() == "model_provider" && ["\"agentq\"", "'agentq'"].iter().any(|id| v.trim_start().starts_with(id)))
        }) { continue; }
        match place(&text, w) {
            Ok((_, None)) => return Ok(true),
            Err(_) if w.field() != "routing" || text.contains("agentq") => return Err("foreign or unreadable integration registration".into()),
            _ => {},
        }
    }
    Ok(false)
}

pub fn present(wants: &[Want]) -> Result<bool, String> {
    for w in wants {
        let Some(text) = read_config(&w.file)? else { return Ok(false); };
        match place(&text, w) {
            Ok((_, None)) => {},
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub fn preflight(wants: &[Want]) -> Result<(), String> {
    let mut files: std::collections::BTreeMap<PathBuf, String> = Default::default();
    for w in wants {
        let text = if let Some(t) = files.get(&w.file) { t.clone() } else { read_config(&w.file)?.unwrap_or_default() };
        let (after, _) = place(&text, w)?;
        files.insert(w.file.clone(), after);
    }
    Ok(())
}

/// Readback/restore for suspended receipts. Missing values after a crash count only as detached,
/// never as evidence that a client reloaded. Later foreign values fail closed.
pub fn detached(e: &Value) -> Result<bool, String> {
    let path = Path::new(e["file"].as_str().ok_or("invalid saved file")?);
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(_) => return Err("config unreadable".into()),
    };
    detached_in(e, &text)
}

fn detached_in(e: &Value, text: &str) -> Result<bool, String> {
    if e["kind"] == "file" { return Ok(false); }
    if e["kind"] == "text" || e["kind"] == "replace" {
        if text.contains(e["after"].as_str().ok_or("invalid saved block")?) { return Ok(false); }
        if e["key"] == json!(["model_providers", "agentq"]) && direct_codex_provider(text) { return Ok(true); }
        let keys: Vec<_> = e["key"].as_array().ok_or("invalid saved key")?.iter().filter_map(Value::as_str).collect();
        let still_registered = match keys.as_slice() {
            ["model_provider"] => text.lines().take_while(|l| !l.trim_start().starts_with('['))
                .any(|l| l.split('=').next().is_some_and(|k| k.trim() == "model_provider") && l.contains("agentq")),
            ["mcp_servers", "agentq"] | ["model_providers", "agentq"] => text.contains(&format!("[{}]", keys.join(".")))
                || text.lines().any(|l| l.trim_start().starts_with("agentq:")),
            ["plugins", "enabled"] => text.contains("agentq"),
            _ if e["field"] == "instructions" => text.contains("<!-- agentq:start -->") || text.contains("<!-- agentq:end -->"),
            _ => true,
        };
        return if still_registered { Err("owned block changed".into()) } else { Ok(true) };
    }
    let mut v: Value = serde_json::from_str(no_bom(&text)).map_err(|_| "config unreadable")?;
    for k in e["key"].as_array().ok_or("invalid saved key")? {
        v = v.get(k.as_str().ok_or("invalid saved key")?).cloned().unwrap_or(Value::Null);
    }
    if e["kind"] == "append" {
        if v.as_array().is_some_and(|a| a.contains(&e["after"])) { return Ok(false); }
        let mut strings = vec![];
        fn collect<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
            match v {
                Value::String(s) => out.push(s),
                Value::Array(a) => a.iter().for_each(|v| collect(v, out)),
                Value::Object(o) => o.values().for_each(|v| collect(v, out)),
                _ => {},
            }
        }
        collect(&v, &mut strings);
        let mut old = vec![];
        collect(&e["after"], &mut old);
        if old.iter().filter_map(|s| s.split_once(" hook ").map(|s| s.0)).any(|prefix| !prefix.is_empty() && strings.iter().any(|s| s.starts_with(prefix))) {
            return Err("owned hook changed".into());
        }
        return Ok(true);
    }
    Ok(v.is_null())
}

pub fn detach(journal: &Path, entries: &[Value], dry: bool) -> Result<Vec<String>, String> {
    let mut out = vec![];
    let mut files: std::collections::BTreeMap<PathBuf, (String, String)> = Default::default();
    for e in entries.iter().rev() {
        let path = PathBuf::from(e["file"].as_str().ok_or("invalid saved file")?);
        let provider = e["key"] == json!(["model_providers", "agentq"]);
        let (before, text) = if let Some(t) = files.get(&path) { t.clone() } else {
            let Some(t) = read_config(&path)? else {
                if provider { files.insert(path, (String::new(), CODEX_DIRECT_PROVIDER.into())); }
                continue;
            };
            (t.clone(), t)
        };
        // Older receipts see the simulated restore, including a file already scheduled for removal.
        if files.contains_key(&path) && text.is_empty() && !provider { continue; }
        if detached_in(e, &text)? {
            if provider && !direct_codex_provider(&text) {
                if text.contains("[model_providers]") || text.contains("model_providers.agentq") {
                    return Err("foreign or unreadable provider definition; kept".into());
                }
                files.insert(path, (before, format!("{text}{}{CODEX_DIRECT_PROVIDER}", sep(&text))));
            }
            continue;
        }
        let after = unplace(&text, e).ok_or("owned value changed")?;
        files.insert(path, (before, after));
    }
    if dry { return Ok(files.keys().map(|p| format!("would detach: {}", p.display())).collect()); }
    for (path, (before, after)) in files {
        if read_config(&path)?.unwrap_or_default() != before { return Err("config changed during detach".into()); }
        if after.is_empty() { std::fs::remove_file(&path).map_err(|e| e.to_string())?; }
        else { write_atomic(&path, &after)?; }
        out.push(format!("detached: {}", path.display()));
    }
    if !entries.iter().all(|e| detached(e) == Ok(true)) { return Err("detach readback mismatch".into()); }
    let mut j: Vec<Value> = match std::fs::read(journal) {
        Ok(b) => serde_json::from_slice(&b).map_err(|_| "owned.json unreadable")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(_) => return Err("owned.json unreadable".into()),
    };
    j.retain(|e| !entries.iter().any(|n| n["source"].as_str() == Some(crate::configure::digest(e).as_str())));
    if journal.exists() { save(journal, &j)?; }
    Ok(out)
}

/// Restores journaled edits newest first when the current value is still ours; later foreign edits stay. `routing`:
/// only the routing edits of these agents, and the restored ones leave the journal (`agentq direct`).
pub fn restore(journal: &Path, dry: bool, routing: Option<&[&str]>) -> Result<Vec<String>, String> {
    let pick = |_: usize, e: &Value| routing.is_none_or(|r| e["routing"].as_str().is_some_and(|a| r.contains(&a)));
    restore_where(journal, dry, routing.is_some(), &pick)
}

/// `restore` of the entries `pick` selects by journal index; `forget` drops the restored ones from the journal.
pub fn restore_where(journal: &Path, dry: bool, forget: bool, pick: &dyn Fn(usize, &Value) -> bool) -> Result<Vec<String>, String> {
    let mut out = vec![];
    // a dry run restores in memory, so later entries of one file see the earlier ones
    let mut seen: std::collections::HashMap<PathBuf, String> = Default::default();
    let mut j = load(journal);
    let mut done = vec![];
    for (i, e) in j.iter().enumerate().rev() {
        if !pick(i, e) {
            continue;
        }
        let f = PathBuf::from(e["file"].as_str().unwrap_or_default());
        let what = format!("{} {}", f.display(), e["key"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("."));
        let current = seen.get(&f).cloned().or_else(|| std::fs::read_to_string(&f).ok());
        let Some(mut text) = current.and_then(|t| unplace(&t, e)) else {
            out.push(format!("conflict: {what} changed since onboard; kept"));
            continue;
        };
        if let Some(o) = e["orig"].as_str().filter(|o| serde_json::from_str::<Value>(no_bom(o)).ok().is_some_and(|v| Some(v) == serde_json::from_str(&text).ok())) {
            text = o.into();
        }
        seen.insert(f.clone(), text.clone());
        if !dry {
            if e["created"] == true && matches!(text.trim(), "" | "{}") {
                std::fs::remove_file(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            } else {
                write_atomic(&f, &text)?;
            }
        }
        out.push(format!("{}: {what}", if dry { "would restore" } else { "restored" }));
        done.push(i);
    }
    if forget && !dry && !done.is_empty() {
        for i in done {
            j.remove(i);
        }
        save(journal, &j)?;
    }
    Ok(out)
}

fn place(text: &str, w: &Want) -> Result<(String, Option<Value>), String> {
    if w.instruction {
        let wanted = w.value.as_str().ok_or("invalid instruction block")?;
        if text.contains(wanted) { return Ok((text.into(), None)); }
        if text.contains("<!-- agentq:start -->") || text.contains("<!-- agentq:end -->") { return Err("instruction block has foreign content".into()); }
        let after = format!("{}{wanted}\n", sep(text));
        return Ok((format!("{text}{after}"), Some(entry(&w.key, "text", after.into()))));
    }
    if w.key == ["$file"] {
        let wanted = w.value.as_str().ok_or("plugin content requires text")?;
        return if text == wanted { Ok((text.into(), None)) }
            else if text.is_empty() { Ok((wanted.into(), Some(entry(&w.key, "file", wanted.into())))) }
            else { Err("plugin file has foreign content".into()) };
    }
    if w.key == ["plugins", "enabled"] { return enable_plugin(text, w); }

    match w.file.extension().and_then(|e| e.to_str()) {
        Some("json") => place_json(text, w),
        Some("toml") => place_toml(text, w),
        _ => place_yaml(text, w),
    }
}

fn unplace(text: &str, e: &Value) -> Option<String> {
    let after = &e["after"];
    if e["scoped_yaml"] == true {
        let a = after.as_str()?;
        if !text.contains(a) { return None; }
        let text = text.replacen(a, "", 1);
        let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
        let head = if e["key"][0] == "plugins" { "enabled:" } else { "mcp_servers:" };
        let parent = if head == "enabled:" { "plugins:" } else { "mcp_servers:" };
        let start = lines.iter().position(|l| l.trim_end() == parent);
        let end = start.map(|start| (start + 1..lines.len()).find(|&i| !lines[i].trim().is_empty() && !lines[i].starts_with([' ', '#'])).unwrap_or(lines.len()));
        let wanted_indent = if head == "enabled:" { (a.len() - a.trim_start().len()).checked_sub(2)? } else { 0 };
        let index = start.zip(end).and_then(|(start, end)| (start..end).find(|&i| lines[i].trim() == head && lines[i].len() - lines[i].trim_start().len() == wanted_indent));
        if let Some(i) = index {
            let indent = lines[i].len() - lines[i].trim_start().len();
            let children: Vec<_> = lines[i+1..].iter().take_while(|l| l.trim().is_empty() || l.trim_start().starts_with('#') || l.len()-l.trim_start().len() > indent).collect();
            if children.iter().all(|l| l.trim().is_empty() || l.trim_start().starts_with('#')) {
                lines[i] = format!("{}{head} {}\n", " ".repeat(indent), if head == "enabled:" { "[]" } else { "{}" });
            }
        }
        return Some(lines.concat());
    }
    if e["kind"] == "file" {
        return (text == after.as_str()?).then(|| String::new());
    }
    if e["kind"] == "replace" {
        let a = after.as_str()?;
        return text.contains(a).then(|| text.replacen(a, e["before"].as_str().unwrap_or(""), 1));
    }
    if e["kind"] == "text" {
        let a = after.as_str()?;
        return text.contains(a).then(|| {
            if e["key"] == json!(["model_providers", "agentq"]) {
                let direct = format!("{}{CODEX_DIRECT_PROVIDER}", &a[..a.len() - a.trim_start().len()]);
                text.replacen(a, &direct, 1)
            } else { text.replacen(a, "", 1) }
        });
    }
    let mut doc: Value = serde_json::from_str(no_bom(text)).ok()?;
    let key: Vec<&str> = e["key"].as_array()?.iter().filter_map(Value::as_str).collect();
    let (last, path) = key.split_last()?;
    let parent = path.iter().try_fold(&mut doc, |v, k| v.get_mut(*k))?.as_object_mut()?;
    if e["kind"] == "append" {
        let a = parent.get_mut(*last)?.as_array_mut()?;
        let i = a.iter().position(|v| v == after)?;
        a.remove(i);
    } else if parent.get(*last) == Some(after) {
        parent.shift_remove(*last);
    } else {
        return None;
    }
    Some(dump(&doc, no_bom(text)))
}

const BOM: char = '\u{feff}';

/// Windows editors may save JSON with a UTF-8 BOM, which serde_json rejects as `expected value at line 1 column 1`.
fn no_bom(text: &str) -> &str {
    text.strip_prefix(BOM).unwrap_or(text)
}

/// Pretty JSON with the original's indent (2 spaces for a new file); a trailing newline only when the original had one.
/// ponytail: indent only; other foreign layout (compact arrays, spacing) is re-pretty while onboarded, restore brings its bytes back.
pub(crate) fn dump(doc: &Value, orig: &str) -> String {
    let indent = orig.lines().nth(1).map(|l| &l[..l.len() - l.trim_start().len()]).filter(|i| !i.is_empty()).unwrap_or("  ");
    // JSON strings hold no raw newline, so each line's leading spaces are indent only
    let s = serde_json::to_string_pretty(doc).unwrap_or_default().lines().map(|l| {
        let t = l.trim_start();
        indent.repeat((l.len() - t.len()) / 2) + t
    }).collect::<Vec<_>>().join("\n");
    if orig.is_empty() || orig.ends_with('\n') { s + "\n" } else { s }
}

/// The first missing key gets the whole remaining subtree, so restore removes exactly what onboard created.
fn place_json(text: &str, w: &Want) -> Result<(String, Option<Value>), String> {
    let text = no_bom(text);
    let mut doc: Value = if text.trim().is_empty() { json!({}) } else { serde_json::from_str(text).map_err(|e| format!("is not JSON: {e}"))? };
    let mut cur = &mut doc;
    for (i, k) in w.key.iter().enumerate() {
        let obj = cur.as_object_mut().ok_or(format!("{} is not an object", w.key[..i].join(".")))?;
        if !obj.contains_key(k) {
            let mut v = if w.append { json!([w.value]) } else { w.value.clone() };
            for k2 in w.key[i + 1..].iter().rev() {
                v = Value::Object(Map::from_iter([(k2.clone(), v)]));
            }
            obj.insert(k.clone(), v.clone());
            return Ok((dump(&doc, text), Some(entry(&w.key[..=i], "set", v))));
        }
        cur = obj.get_mut(k).unwrap();
    }
    if w.append {
        let a = cur.as_array_mut().ok_or(format!("{} is not an array", w.key.join(".")))?;
        if a.contains(&w.value) {
            return Ok((text.into(), None));
        }
        a.push(w.value.clone());
        return Ok((dump(&doc, text), Some(entry(&w.key, "append", w.value.clone()))));
    }
    if *cur == w.value { Ok((text.into(), None)) } else { Err(format!("{} has a foreign value", w.key.join("."))) }
}

fn fields(w: &Want) -> impl Iterator<Item = (&String, &Value)> {
    w.value.as_object().into_iter().flatten()
}

fn sep(text: &str) -> &'static str {
    if text.is_empty() {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

/// Appends `[mcp_servers.agentq]`; JSON strings and arrays are valid TOML values. A top-level key goes first in
/// the file, where it cannot fall into a table.
fn place_toml(text: &str, w: &Want) -> Result<(String, Option<Value>), String> {
    let name = w.key.join(".");
    if let [k] = &w.key[..] {
        let line = format!("{k} = {}\n", w.value);
        let top: Vec<&str> = text.lines().take_while(|l| !l.trim_start().starts_with('[')).collect();
        if top.iter().any(|l| l.trim() == line.trim()) {
            return Ok((text.into(), None));
        }
        if top.iter().any(|l| l.split('=').next().is_some_and(|x| x.trim() == k)) {
            return Err(format!("{k} has a foreign value"));
        }
        return Ok((format!("{line}{text}"), Some(entry(&w.key, "text", line.into()))));
    }
    let block = format!("[{name}]\n{}", fields(w).map(|(k, v)| format!("{k} = {v}\n")).collect::<String>());
    if text.contains(&block) {
        return Ok((text.into(), None));
    }
    if w.key == ["model_providers", "agentq"] && direct_codex_provider(text) {
        let mut e = entry(&w.key, "replace", block.clone().into());
        e["before"] = CODEX_DIRECT_PROVIDER.into();
        return Ok((text.replacen(CODEX_DIRECT_PROVIDER, &block, 1), Some(e)));
    }
    // ponytail: substring check; an inline `agentq = {..}` under `[mcp_servers]` slips through
    if text.contains(&name) {
        return Err(format!("{name} exists"));
    }
    let after = format!("{}{block}", sep(text));
    Ok((format!("{text}{after}"), Some(entry(&w.key, "text", after.into()))))
}

/// Inserts `agentq` under a block `mcp_servers:` with its indent, else appends both; JSON scalars are YAML.
/// Preserve the user's plugin list; journal only our inserted item or empty mapping expansion.
fn enable_plugin(text: &str, w: &Want) -> Result<(String, Option<Value>), String> {
    let name = w.value.as_str().ok_or("plugin name requires text")?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(head) = lines.iter().position(|l| l.starts_with("plugins:")) else {
        let after = format!("{}plugins:\n  enabled:\n    - {name}\n", sep(text));
        return Ok((format!("{text}{after}"), Some(entry(&w.key, "text", after.into()))));
    };
    if lines[head].trim() == "plugins: {}" {
        let before = lines[head];
        let after = format!("plugins:\n  enabled:\n    - {name}\n");
        let mut e = entry(&w.key, "replace", after.clone().into()); e["before"] = before.into();
        return Ok((text.replacen(before, &after, 1), Some(e)));
    }
    if lines[head].trim() != "plugins:" { return Err("plugins is not a block mapping".into()); }
    let end = (head+1..lines.len()).find(|&i| !lines[i].trim().is_empty() && !lines[i].starts_with(' ') && !lines[i].starts_with('#')).unwrap_or(lines.len());
    let enabled = (head+1..end).find(|&i| lines[i].trim_start().starts_with("enabled:"));
    let (at, after) = if let Some(i) = enabled {
        let indent = lines[i].len() - lines[i].trim_start().len();
        if lines[i].trim() == "enabled: []" {
            let before = lines[i]; let after = format!("{}enabled:\n{}- {name}\n", " ".repeat(indent), " ".repeat(indent+2));
            let mut e = entry(&w.key, "replace", after.clone().into()); e["before"] = before.into();
            return Ok((text.replacen(before, &after, 1), Some(e)));
        }
        if lines[i].trim() != "enabled:" { return Err("plugins.enabled is not a block list".into()); }
        let last = (i+1..end).find(|&j| !lines[j].trim().is_empty() && lines[j].len()-lines[j].trim_start().len() <= indent).unwrap_or(end);
        if lines[i+1..last].iter().any(|l| l.trim().trim_start_matches('-').trim().trim_matches(['\"','\'']) == name) { return Ok((text.into(), None)); }
        (i+1, format!("{}- {name}\n", " ".repeat(indent+2)))
    } else { (head+1, format!("  enabled:\n    - {name}\n")) };
    let offset: usize = lines[..at].iter().map(|l|l.len()).sum();
    Ok((format!("{}{after}{}", &text[..offset], &text[offset..]), Some(entry(&w.key,"text",after.into()))))
}

fn place_yaml(text: &str, w: &Want) -> Result<(String, Option<Value>), String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let head = lines.iter().position(|l| l.starts_with(&format!("{}:", w.key[0])));
    let indent = head
        .and_then(|h| lines[h + 1..].iter().find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')))
        .map(|l| l.len() - l.trim_start().len())
        .filter(|n| *n > 0)
        .unwrap_or(2);
    let i = " ".repeat(indent);
    let block = format!("{i}{}:\n{}", w.key[1], fields(w).map(|(k, v)| format!("{i}{i}{k}: {v}\n")).collect::<String>());
    if text.contains(&block) {
        return Ok((text.into(), None));
    }
    if lines.iter().any(|l| l.trim_start().starts_with(&format!("{}:", w.key[1]))) {
        return Err(format!("{} exists", w.key.join(".")));
    }
    let (text, after) = match head {
        None => {
            let after = format!("{}{}:\n{block}", sep(text), w.key[0]);
            (format!("{text}{after}"), after)
        }
        Some(h) if lines[h].trim_end() == format!("{}:", w.key[0]) && lines[h].ends_with('\n') => {
            let at: usize = lines[..=h].iter().map(|l| l.len()).sum();
            (format!("{}{block}{}", &text[..at], &text[at..]), block)
        }
        Some(h) if lines[h].trim_end() == format!("{}: {{}}", w.key[0]) => {
            let before = lines[h];
            let after = format!("{}:\n{block}", w.key[0]);
            let mut e = entry(&w.key, "replace", after.clone().into());
            e["before"] = before.into();
            return Ok((text.replacen(before, &after, 1), Some(e)));
        }
        Some(_) => return Err(format!("{} is not a block mapping", w.key[0])),
    };
    Ok((text, Some(entry(&w.key, "text", after.into()))))
}

/// Built-in fixture for `selftest`: a Claude settings file saved with a BOM gets the hook and loses the BOM.
pub fn selftest() -> bool {
    let w = &wants("claude", Path::new("s"), Path::new("c.json"), Path::new("agentq"))[0];
    place_json("\u{feff}{}", w).is_ok_and(|(t, e)| t.starts_with("{\n  \"hooks\"") && e.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(files: &[PathBuf]) -> Vec<Option<String>> {
        files.iter().map(|f| std::fs::read_to_string(f).ok()).collect()
    }

    /// A1: onboard edits only owned keys, records each edit, and uninstall restores every file byte for byte.
    #[test]
    fn onboard_then_restore_leaves_no_diff() {
        let d = std::env::temp_dir().join(format!("agentq-onboard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let (codex, claude, hermes) = (d.join("codex"), d.join("claude"), d.join("hermes"));
        for r in [&codex, &claude, &hermes] {
            std::fs::create_dir_all(r).unwrap();
        }
        std::fs::write(codex.join("config.toml"), "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"").unwrap();
        // 4-space indent and a compact array: onboard keeps the indent, uninstall brings back every byte
        std::fs::write(claude.join("settings.json"), "{\n    \"model\": \"opus\",\n    \"allow\": [\"a\", \"b\"],\n    \"hooks\": {\n        \"SessionStart\": [\n            {\n                \"hooks\": []\n            }\n        ]\n    }\n}").unwrap();
        std::fs::write(claude.join(".claude.json"), "{\n  \"mcpServers\": {\n    \"other\": {}\n  },\n  \"tips\": 3\n}\n").unwrap();
        std::fs::write(hermes.join("config.yaml"), "model: x\nmcp_servers:\n    other:\n        command: y\nplugins: {}\n").unwrap();
        let exe = d.join("install").join("agentq");
        let journal = d.join("install").join("data").join("owned.json");
        let all = || -> Vec<Want> {
            let mut w = wants("codex", &codex, &claude.join(".claude.json"), &exe);
            w.extend(wants("claude", &claude, &claude.join(".claude.json"), &exe));
            w.extend(wants("hermes", &hermes, &claude.join(".claude.json"), &exe));
            w
        };
        let files: Vec<PathBuf> = {
            let mut f: Vec<PathBuf> = all().into_iter().map(|w| w.file).collect();
            f.dedup();
            f
        };
        let before = snapshot(&files);

        let out = apply(&all(), &journal).unwrap();
        assert!(out.iter().all(|l| l.starts_with("edited:")), "{out:?}");
        assert_eq!(load(&journal).len(), 12, "one journal entry per edit, including Claude PostToolUse and Stop");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&journal).unwrap().permissions()) & 0o777, 0o600);
        let toml = std::fs::read_to_string(codex.join("config.toml")).unwrap();
        assert!(toml.starts_with("model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"\n\n[mcp_servers.agentq]\n"), "{toml}");
        let yaml = std::fs::read_to_string(hermes.join("config.yaml")).unwrap();
        assert!(yaml.starts_with("model: x\nmcp_servers:\n    agentq:\n        command: "), "{yaml}");
        let settings = std::fs::read_to_string(claude.join("settings.json")).unwrap();
        assert!(settings.starts_with("{\n    \"model\": \"opus\",\n    \"allow\": [\n        \"a\","), "{settings}");
        let settings: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(settings["hooks"]["SessionStart"].as_array().unwrap().len(), 2, "foreign hook kept");
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(settings["hooks"]["PostToolUse"][0]["matcher"], "Bash");
        assert!(settings["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap().ends_with(" hook claude Stop"), "Stop reads model calls");
        let hooks: Value = serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap()).unwrap();
        assert!(hooks["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap().ends_with(" hook codex Stop"));

        let edited = snapshot(&files);
        assert!(apply(&all(), &journal).unwrap().iter().all(|l| l.starts_with("kept:")), "repeat is safe");
        assert_eq!(load(&journal).len(), 12);
        assert_eq!(snapshot(&files), edited);

        assert!(restore(&journal, true, None).unwrap().iter().all(|l| l.starts_with("would restore:")));
        assert_eq!(snapshot(&files), edited, "dry run writes nothing");
        assert!(restore(&journal, false, None).unwrap().iter().all(|l| l.starts_with("restored:")));
        assert_eq!(snapshot(&files), before, "uninstall leaves no diff");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hermes_keeps_foreign_enabled_and_disabled_plugins_on_install_and_restore() {
        let d = std::env::temp_dir().join(format!("agentq-plugin-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap();
        let before = "plugins:\n  enabled:\n    - tight-langfuse\n    - web/tavily\n  disabled:\n    - observability/langfuse\n";
        std::fs::write(d.join("config.yaml"), before).unwrap();
        let journal = d.join("owned.json");
        let wants = wants("hermes", &d, &d.join("unused"), Path::new("/agentq"));
        apply(&wants, &journal).unwrap();
        let after = std::fs::read_to_string(d.join("config.yaml")).unwrap();
        assert!(after.contains("    - agentq\n"));
        assert!(after.contains("    - tight-langfuse\n    - web/tavily\n  disabled:\n    - observability/langfuse\n"));
        assert!(apply(&wants, &journal).unwrap().iter().all(|l| l.starts_with("kept:")));
        restore(&journal, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("config.yaml")).unwrap(), before);
        assert!(!d.join("plugins/agentq/__init__.py").exists());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn foreign_plugin_prevents_activation_before_any_write() {
        let d = std::env::temp_dir().join(format!("agentq-plugin-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let source = d.join("plugins/agentq/__init__.py");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "foreign plugin").unwrap();
        let config = "model: native\nplugins:\n  enabled:\n    - other\n";
        std::fs::write(d.join("config.yaml"), config).unwrap();
        let journal = d.join("owned.json");
        assert!(apply(&wants("hermes", &d, &d.join("unused"), Path::new("/agentq")), &journal).is_err());
        assert_eq!(std::fs::read_to_string(d.join("config.yaml")).unwrap(), config);
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "foreign plugin");
        assert!(!journal.exists());
        std::fs::remove_dir_all(d).unwrap();
    }

    /// #71 A2: a BOM is reported and stripped, not `is not JSON`; uninstall brings the original bytes back.
    #[test]
    fn json_bom_is_stripped_and_restored() {
        let d = std::env::temp_dir().join(format!("agentq-bom-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let orig = "\u{feff}{\n  \"model\": \"opus\"\n}\n";
        std::fs::write(d.join("settings.json"), orig).unwrap();
        let journal = d.join("owned.json");
        let w: Vec<Want> = wants("claude", &d, &d.join(".claude.json"), Path::new("/x/agentq")).into_iter().take(2).collect();
        let out = apply(&w, &journal).unwrap();
        assert!(out[0].starts_with("edited:") && out[0].ends_with("; UTF-8 BOM stripped)"), "{out:?}");
        let edited = std::fs::read_to_string(d.join("settings.json")).unwrap();
        assert!(edited.starts_with("{\n  \"model\""), "{edited}");
        assert!(selftest());
        restore(&journal, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("settings.json")).unwrap(), orig);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn foreign_values_stop_the_file_and_survive_restore() {
        let d = std::env::temp_dir().join(format!("agentq-conflict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let cj = d.join(".claude.json");
        let foreign = "{\n  \"mcpServers\": {\n    \"agentq\": {\n      \"command\": \"node\"\n    }\n  }\n}\n";
        std::fs::write(&cj, foreign).unwrap();
        let foreign_yaml = "mcp_servers: {agentq: {command: foreign-tool}}\n";
        std::fs::write(d.join("config.yaml"), foreign_yaml).unwrap();
        let journal = d.join("owned.json");
        let mut w = wants("claude", &d, &cj, Path::new("/x/agentq"));
        w.extend(wants("hermes", &d, &cj, Path::new("/x/agentq")));
        let out = apply(&w, &journal).unwrap();
        assert!(out.iter().any(|l| l.starts_with("conflict:") && l.contains("mcpServers.agentq has a foreign value")), "{out:?}");
        assert!(out.iter().any(|l| l.starts_with("conflict:") && l.contains("not a block mapping")), "{out:?}");
        assert_eq!(std::fs::read_to_string(&cj).unwrap(), foreign);
        assert_eq!(std::fs::read_to_string(d.join("config.yaml")).unwrap(), foreign_yaml);

        // a later foreign edit of an owned key is kept and reported
        let s = d.join("settings.json");
        let edited = std::fs::read_to_string(&s).unwrap().replace("hook claude PreToolUse", "my-hook");
        std::fs::write(&s, &edited).unwrap();
        let out = restore(&journal, false, None).unwrap();
        assert!(out.iter().any(|l| l.starts_with("conflict:") && l.contains("hooks.PreToolUse")), "{out:?}");
        assert!(std::fs::read_to_string(&s).unwrap().contains("my-hook"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// #76 A1: Codex routes to the loopback provider; `direct` removes only the routing edits, onboard routes again,
    /// Direct and uninstall keep old conversations resumable without the AgentQ proxy; foreign edits stay.
    #[test]
    fn codex_routing_then_direct() {
        let d = std::env::temp_dir().join(format!("agentq-routing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let cfg = d.join("config.toml");
        let orig = "model = \"gpt-5\"\n\n[projects.\"/p\"]\ntrust_level = \"trusted\"\n";
        std::fs::write(&cfg, orig).unwrap();
        let journal = d.join("owned.json");
        let all = || -> Vec<Want> {
            let mut w = wants("codex", &d, &d.join(".claude.json"), Path::new("/x/agentq"));
            w.extend(routing_wants("codex", &d, 41234));
            w
        };
        apply(&all(), &journal).unwrap();
        let routed = std::fs::read_to_string(&cfg).unwrap();
        assert!(routed.starts_with("model_provider = \"agentq\"\nmodel = \"gpt-5\"\n"), "{routed}");
        assert!(routed.contains("[model_providers.agentq]\nname = \"AgentQ\"\nbase_url = \"http://127.0.0.1:41234/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = true\n"), "{routed}");
        assert_eq!(crate::routing::state("codex", &d, Some(41234)).0, "unknown", "routed config, no worker yet");
        assert!(crate::routing::drift(&journal).is_empty());

        let out = restore(&journal, false, Some(&["codex"])).unwrap();
        assert_eq!(out.len(), 2, "{out:?}");
        let direct = std::fs::read_to_string(&cfg).unwrap();
        assert!(direct.starts_with(orig) && direct.contains("[mcp_servers.agentq]") && direct_codex_provider(&direct), "{direct}");
        assert!(!direct.lines().any(|l| l.starts_with("model_provider =")));
        assert_eq!(crate::routing::state("codex", &d, Some(41234)).0, "direct");
        assert_eq!(load(&journal).len(), 3, "MCP and hook entries stay");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&journal).unwrap().permissions()) & 0o777, 0o600);
        assert!(restore(&journal, false, Some(&["codex"])).unwrap().is_empty(), "repeat is safe");

        for edit in ["base_url = \"https://foreign.invalid/v1\"\n", "[model_providers.agentq.http_headers]\nx-foreign = \"kept\"\n"] {
            let foreign = direct.replacen(CODEX_DIRECT_PROVIDER, &format!("{CODEX_DIRECT_PROVIDER}{edit}"), 1);
            std::fs::write(&cfg, &foreign).unwrap();
            let out = apply(&all(), &journal).unwrap();
            assert!(out.iter().any(|l| l.starts_with("conflict:")), "{out:?}");
            assert_eq!(std::fs::read_to_string(&cfg).unwrap(), foreign, "compatibility edits must not be reclaimed");
        }
        std::fs::write(&cfg, &direct).unwrap();

        crate::routing::set_direct(&d, &["codex"], true).unwrap();
        assert_eq!(crate::routing::intended(&d, "codex", &d), ("direct", "`agentq direct`"));
        crate::routing::set_direct(&d, &["codex"], false).unwrap();
        apply(&all(), &journal).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), routed, "onboard routes again");
        assert_eq!(crate::routing::intended(&d, "codex", &d), ("routed", "onboarded"), "#132 A1: re-onboarded Codex is intended routed");
        // a user who drops the provider line is reported as drift
        std::fs::write(&cfg, routed.replacen("model_provider = \"agentq\"\n", "", 1)).unwrap();
        assert_eq!(crate::routing::drift(&journal).len(), 1);
        std::fs::write(&cfg, &routed).unwrap();
        restore(&journal, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), format!("{orig}\n{CODEX_DIRECT_PROVIDER}"), "uninstall keeps only native config and the independent compatibility definition");

        std::fs::write(&cfg, "model_provider = \"openai\"\n").unwrap();
        let out = apply(&all(), &d.join("owned2.json")).unwrap();
        assert!(out[0].starts_with("conflict:") && out[0].contains("model_provider has a foreign value"), "{out:?}");

        let native = format!("model = \"native\"\n\n{CODEX_DIRECT_PROVIDER}");
        std::fs::write(&cfg, &native).unwrap();
        let provider_only: Vec<_> = routing_wants("codex", &d, 41234).into_iter().skip(1).collect();
        let journal = d.join("provider-only.json");
        apply(&provider_only, &journal).unwrap();
        let foreign = "\n[projects.\"/later\"]\ntrust_level = \"trusted\"\n";
        std::fs::write(&cfg, std::fs::read_to_string(&cfg).unwrap() + foreign).unwrap();
        restore(&journal, false, None).unwrap();
        assert_eq!(std::fs::read_to_string(&cfg).unwrap(), native + foreign, "a TOML replacement receipt cannot restore an old whole-file snapshot over later edits");
        let _ = std::fs::remove_dir_all(&d);
    }
}
