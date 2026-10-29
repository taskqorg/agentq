//! Legacy Node install removal (agentq.md § 10). The Node journal gives the file list only: a user edit made after
//! it never blocks. Each file is backed up, then only legacy entries are stripped; the install folder moves aside.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Legacy proxy ports (codex, claude, hermes; Windows +100), as the Node `servicePort` assigns them.
const PORTS: [u16; 6] = [18787, 18788, 18790, 18887, 18888, 18890];
const PACKAGE: &str = "@alexkirs/agent-context";
const LABEL: &str = "com.taskqorg.contextq-monitor";
const TASK: &str = "Agent Context Monitor Native";
/// Files the Node install may edit, relative to its home; a journal path outside them is ignored.
const FILES: [&str; 12] = [
    ".codex/config.toml",
    ".codex/hooks.json",
    ".codex/AGENTS.md",
    ".codex/langfuse.json",
    ".claude.json",
    ".claude/settings.json",
    ".claude/CLAUDE.md",
    ".hermes/config.yaml",
    ".hermes/.env",
    ".hermes/AGENTS.md",
    ".hermes/plugins/agent-context/plugin.yaml",
    ".hermes/plugins/agent-context/__init__.py",
];

/// Runs an external command, true on success; tests record the calls instead.
pub type Run<'a> = &'a dyn Fn(&str, &[&str]) -> bool;

fn legacy_url(s: &str) -> bool {
    PORTS.iter().any(|p| s.strip_prefix(&format!("http://127.0.0.1:{p}")).is_some_and(|r| r.is_empty() || r.starts_with('/')))
}

/// The launcher, the npm package path and the install folder all name it.
fn ours(s: &str) -> bool {
    s.contains("launcher.mjs") || s.contains("agent-context")
}

/// `.agent-context` lives in `$AGENT_CONTEXT_HOME`, else the home folder; the Node install edited configs there.
pub fn remove(dir: &Path, dry: bool, today: &str, run: Run) -> Result<Vec<String>, String> {
    if !dir.exists() {
        // ponytail: a global npm package left without the folder stays; `npm uninstall -g @alexkirs/agent-context` removes it
        return Ok(vec!["legacy: already removed".into()]);
    }
    let would = |a: &'static str, b: &'static str| if dry { a } else { b };
    let mut out = vec![];
    let home = dir.parent().unwrap_or(dir);
    let plugin = home.join(".hermes/plugins/agent-context");
    let files: Vec<PathBuf> = journal_files(dir, home).into_iter().filter(|f| f.exists()).collect();
    // every copy first: a failed backup writes nothing; a rerun keeps the first backup
    for f in &files {
        let to = dir.join("config-backup").join(f.strip_prefix(home).unwrap_or(f));
        if !dry && !to.exists() {
            std::fs::create_dir_all(to.parent().unwrap_or(dir)).and_then(|_| std::fs::copy(f, &to)).map_err(|e| format!("back up {}: {e}", f.display()))?;
        }
        out.push(format!("{}: {}", would("would back up", "backed up"), f.display()));
    }
    for f in &files {
        if f.starts_with(&plugin) {
            if !dry {
                std::fs::remove_file(f).map_err(|e| format!("{}: {e}", f.display()))?;
            }
            out.push(format!("{}: {}", would("would remove", "removed"), f.display()));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let new = strip(f, &text);
        if new == text {
            continue;
        }
        if !dry {
            // the original mode before the rename: `.hermes/.env` holds keys
            let tmp = f.with_extension("agentq-tmp");
            let perm = std::fs::metadata(f).map(|m| m.permissions());
            perm.and_then(|p| std::fs::write(&tmp, &new).and_then(|_| std::fs::set_permissions(&tmp, p))).and_then(|_| std::fs::rename(&tmp, f)).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        out.push(format!("{}: {}", would("would strip", "stripped"), f.display()));
    }
    if !dry && plugin.exists() {
        let _ = std::fs::remove_dir_all(plugin.join("__pycache__"));
        let _ = std::fs::remove_dir(&plugin);
    }
    out.extend(monitor(home, dry, run));
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    out.push(if dry || run(npm, &["uninstall", "-g", "--ignore-scripts", PACKAGE]) {
        format!("{}: global npm package {PACKAGE}", would("would remove", "removed"))
    } else {
        format!("kept: global npm package {PACKAGE}; run `npm uninstall -g {PACKAGE}`")
    });
    out.extend(proxies(dir, run));
    let to = (1..).map(|n| PathBuf::from(format!("{}.retired-{today}{}", dir.display(), if n == 1 { String::new() } else { format!("-{n}") }))).find(|p| !p.exists()).unwrap_or_default();
    let kept = if dry { vec![] } else { retire(dir, &to)? };
    if kept.is_empty() {
        out.push(format!("{}: {} -> {}", would("would move", "moved"), dir.display(), to.display()));
    }
    out.extend(kept);
    Ok(out)
}

/// Marks a folder that could not move as a whole; its rest is held by running legacy processes.
pub const RETIRED: &str = ".retired";

/// The install is `launcher.mjs` in a folder not marked retired.
pub fn installed(dir: &Path) -> bool {
    dir.join("launcher.mjs").exists() && !dir.join(RETIRED).exists()
}

/// Files left in a retired folder, the marker excluded.
pub fn locked(dir: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) { walk(&e.path(), out) } else if e.file_name() != RETIRED { out.push(e.path()) }
        }
    }
    let mut out = vec![];
    if dir.join(RETIRED).exists() {
        walk(dir, &mut out);
    }
    out.sort();
    out
}

/// Moves `dir` to `to`. Windows refuses while running proxies hold files in it (access denied): then copy what can be
/// read, remove each copied file and emptied folder that can go, mark the rest with `.retired` and list it.
fn retire(dir: &Path, to: &Path) -> Result<Vec<String>, String> {
    let Err(e) = std::fs::rename(dir, to) else { return Ok(vec![]) };
    fn copy_out(from: &Path, to: &Path) {
        for e in std::fs::read_dir(from).into_iter().flatten().flatten() {
            let (p, t) = (e.path(), to.join(e.file_name()));
            if e.file_type().is_ok_and(|f| f.is_dir()) {
                copy_out(&p, &t);
                let _ = std::fs::remove_dir(&p);
            } else if std::fs::create_dir_all(to).and_then(|_| std::fs::copy(&p, &t)).is_ok() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    copy_out(dir, to);
    std::fs::write(dir.join(RETIRED), to.display().to_string()).map_err(|w| format!("move {}: {e}; mark {RETIRED}: {w}", dir.display()))?;
    let mut out = vec![format!("kept: {} ({e}); copied to {}, the rest is marked {RETIRED}", dir.display(), to.display())];
    out.extend(locked(dir).iter().map(|f| format!("locked: {}; delete it after the legacy processes stop", f.display())));
    Ok(out)
}

/// `selftest` fixture: a legacy folder whose log a running proxy holds is retired, not refused. ponytail: Windows only;
/// elsewhere an open file never blocks the move.
pub fn selftest_locked() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let h = std::env::temp_dir().join(format!("agentq-selftest-{}", std::process::id()));
        let (dir, to) = (h.join(".agent-context"), h.join(".agent-context.retired"));
        let _ = std::fs::remove_dir_all(&h);
        let made = std::fs::create_dir_all(dir.join("logs")).and_then(|_| std::fs::write(dir.join("launcher.mjs"), "")).and_then(|_| std::fs::write(dir.join("logs/proxy.log"), "x"));
        // FILE_SHARE_READ only: readable, not deletable, as a Node proxy's open log
        let held = made.and_then(|_| std::fs::OpenOptions::new().read(true).share_mode(1).open(dir.join("logs/proxy.log")));
        let ok = held.is_ok()
            && retire(&dir, &to).is_ok_and(|l| l.iter().any(|l| l.starts_with("locked:")))
            && !installed(&dir)
            && to.join("launcher.mjs").exists()
            && to.join("logs/proxy.log").exists()
            && locked(&dir) == [dir.join("logs").join("proxy.log")];
        drop(held);
        let _ = std::fs::remove_dir_all(&h);
        ok
    }
    #[cfg(not(windows))]
    true
}

fn journal_files(dir: &Path, home: &Path) -> Vec<PathBuf> {
    let allowed: Vec<PathBuf> = FILES.iter().map(|f| home.join(f)).collect();
    let mut out: Vec<PathBuf> = vec![];
    for e in std::fs::read_dir(dir.join("backups")).into_iter().flatten().flatten() {
        let j: Value = std::fs::read_to_string(e.path().join("journal.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for f in j["files"].as_array().into_iter().flatten().filter_map(|f| f["file"].as_str()).map(PathBuf::from) {
            if allowed.contains(&f) && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

/// Unloads the `contextq-monitor` launch agent, Windows task or systemd unit.
fn monitor(home: &Path, dry: bool, run: Run) -> Vec<String> {
    let would = if dry { "would unload" } else { "unloaded" };
    if cfg!(windows) {
        if !run("schtasks", &["/Query", "/TN", TASK]) {
            return vec![];
        }
        if !dry {
            run("schtasks", &["/End", "/TN", TASK]);
            if !run("schtasks", &["/Delete", "/TN", TASK, "/F"]) {
                return vec![format!("kept: task `{TASK}`; run `schtasks /Delete /TN \"{TASK}\" /F`")];
            }
        }
        return vec![format!("{would}: task {TASK}")];
    }
    let (unit, stop): (PathBuf, Vec<String>) = if cfg!(target_os = "macos") {
        let unit = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
        let uid = match crate::host::run(std::process::Command::new("id").arg("-u")) {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(e) => return if unit.exists() { vec![format!("kept: {}; {e}", unit.display())] } else { vec![] },
        };
        (unit, vec!["launchctl".into(), "bootout".into(), format!("gui/{uid}/{LABEL}")])
    } else {
        let u = "agent-context-monitor.service";
        (home.join(".config/systemd/user").join(u), vec!["systemctl".into(), "--user".into(), "disable".into(), "--now".into(), u.into()])
    };
    if !unit.exists() {
        return vec![];
    }
    if !dry {
        // not loaded is fine: the unit file goes either way
        run(&stop[0], &stop[1..].iter().map(String::as_str).collect::<Vec<_>>());
        if let Err(e) = std::fs::remove_file(&unit) {
            return vec![format!("kept: {}: {e}", unit.display())];
        }
    }
    vec![format!("{would}: {}", unit.display())]
}

/// Running legacy proxies keep serving open sessions; they are listed, not stopped.
fn proxies(dir: &Path, run: Run) -> Vec<String> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir.join("services")).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let s: Value = std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for pid in [&s["pid"], &s["childPid"]].into_iter().filter_map(Value::as_u64).map(|p| p.to_string()) {
            let alive = if cfg!(windows) { run("powershell", &["-NoProfile", "-Command", &format!("Get-Process -Id {pid} | Out-Null")]) } else { run("kill", &["-0", &pid]) };
            if alive {
                out.push(format!("running: legacy proxy pid {pid} ({}); left for you to stop", p.file_stem().unwrap_or_default().to_string_lossy()));
            }
        }
    }
    out
}

/// Strips only legacy entries; everything else, later user edits included, stays.
pub fn strip(f: &Path, text: &str) -> String {
    let name = f.file_name().unwrap_or_default().to_string_lossy();
    if name.ends_with(".json") {
        strip_json(text)
    } else if name.ends_with(".toml") {
        strip_toml(text)
    } else if name.ends_with(".yaml") {
        strip_yaml(text)
    } else if name.ends_with(".md") {
        strip_md(text)
    } else {
        strip_env(text)
    }
}

/// Hooks, MCP servers and env routed to the proxy; Codex `langfuse.json` gets its own `base_url` back.
fn strip_json(text: &str) -> String {
    let Ok(mut doc) = serde_json::from_str::<Value>(text) else { return text.into() };
    let before = doc.clone();
    if let Some(events) = doc.get_mut("hooks").and_then(Value::as_object_mut) {
        events.retain(|_, rules| {
            let Some(a) = rules.as_array_mut() else { return true };
            let n = a.len();
            a.retain_mut(|r| {
                let Some(h) = r.get_mut("hooks").and_then(Value::as_array_mut) else { return true };
                let m = h.len();
                h.retain(|x| !ours(&x.to_string()));
                h.len() == m || !h.is_empty()
            });
            a.len() == n || !a.is_empty()
        });
    }
    if let Some(m) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) {
        m.retain(|_, v| !ours(&v.to_string()));
    }
    if let Some(m) = doc.get_mut("env").and_then(Value::as_object_mut) {
        m.retain(|_, v| !v.as_str().is_some_and(legacy_url));
    }
    if doc["base_url"].as_str().is_some_and(legacy_url)
        && let Some(o) = doc.as_object_mut()
    {
        match o.shift_remove("agent_context_base_url") {
            Some(u) => o.insert("base_url".into(), u),
            None => o.shift_remove("base_url"),
        };
    }
    if doc == before { text.into() } else { crate::onboard::dump(&doc, text) }
}

/// `model_provider = "agent_context"`, `[model_providers.agent_context]` and `[mcp_servers.*]` tables that name the
/// launcher, with their subtables. ponytail: tables only; an inline `name = {..}` under `[mcp_servers]` stays.
fn strip_toml(text: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut dropped: Vec<String> = vec![];
    let (mut i, mut top) = (0, true);
    while i < lines.len() {
        let t = lines[i].trim_start();
        if !t.starts_with('[') {
            let provider = t.strip_prefix("model_provider").is_some_and(|r| r.trim_start().starts_with('=') && r.contains("agent_context"));
            if !(top && provider) {
                out.push_str(lines[i]);
            }
            i += 1;
            continue;
        }
        top = false;
        let end = (i + 1..lines.len()).find(|&j| lines[j].trim_start().starts_with('[')).unwrap_or(lines.len());
        let parts: Vec<String> = t.trim_start_matches('[').split(']').next().unwrap_or_default().split('.').map(|p| p.trim().trim_matches(['"', '\'']).to_string()).collect();
        let base = parts.iter().take(2).cloned().collect::<Vec<_>>().join(".");
        let drop = match (parts[0].as_str(), parts.get(1).map(String::as_str)) {
            ("model_providers", Some("agent_context")) => true,
            ("mcp_servers", Some(_)) if parts.len() == 2 => ours(&lines[i..end].concat()),
            ("mcp_servers", Some(_)) => dropped.contains(&base),
            _ => false,
        };
        if drop {
            dropped.push(base);
        } else {
            out.push_str(&lines[i..end].concat());
        }
        i = end;
    }
    if out == text { out } else { out.trim_end().to_string() + "\n" }
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn blank(l: &str) -> bool {
    let t = l.trim();
    t.is_empty() || t.starts_with('#')
}

/// End (exclusive) of the block whose key line is `i`; a compact `- item` list at the key's indent belongs to it.
fn block_end(lines: &[String], i: usize) -> usize {
    let n = indent(&lines[i]);
    let mut end = i + 1;
    for (j, l) in lines.iter().enumerate().skip(i + 1) {
        if blank(l) {
            continue;
        }
        if indent(l) > n || indent(l) == n && l.trim_start().starts_with('-') {
            end = j + 1;
        } else {
            break;
        }
    }
    end
}

/// Lines of the direct children of `parent` (None: top level).
fn children(lines: &[String], parent: Option<usize>) -> Vec<usize> {
    let (from, to) = parent.map_or((0, lines.len()), |p| (p + 1, block_end(lines, p)));
    let Some(n) = lines[from..to].iter().find(|l| !blank(l)).map(|l| indent(l)) else { return vec![] };
    (from..to).filter(|&i| !blank(&lines[i]) && indent(&lines[i]) == n).collect()
}

fn key(l: &str) -> &str {
    l.trim_start().split(':').next().unwrap_or_default().trim_matches(['"', '\''])
}

fn child(lines: &[String], parent: Option<usize>, name: &str) -> Option<usize> {
    children(lines, parent).into_iter().find(|&i| key(&lines[i]) == name)
}

/// Hermes `config.yaml`: `mcp_servers` entries that name the launcher, `plugins.enabled` item and
/// `plugins.entries` block `agent-context`. ponytail: block style only, as the Node install wrote it.
fn strip_yaml(text: &str) -> String {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(String::from).collect();
    let cut = |lines: &mut Vec<String>, i: usize, empty: &str| {
        let parent = (0..i).rev().find(|&p| !blank(&lines[p]) && indent(&lines[p]) < indent(&lines[i]));
        lines.drain(i..block_end(lines, i));
        // an emptied parent stays a mapping or a list, not null
        if let Some(p) = parent.filter(|&p| block_end(lines, p) == p + 1 && lines[p].trim_end().ends_with(':')) {
            lines[p] = format!("{} {empty}\n", lines[p].trim_end());
        }
    };
    while let Some(i) = child(&lines, None, "mcp_servers").and_then(|m| children(&lines, Some(m)).into_iter().find(|&i| ours(&lines[i..block_end(&lines, i)].concat()))) {
        cut(&mut lines, i, "{}");
    }
    let plugins = child(&lines, None, "plugins");
    if let Some(i) = plugins.and_then(|p| child(&lines, Some(p), "entries")).and_then(|e| child(&lines, Some(e), "agent-context")) {
        cut(&mut lines, i, "{}");
    }
    let plugins = child(&lines, None, "plugins");
    if let Some(e) = plugins.and_then(|p| child(&lines, Some(p), "enabled"))
        && let Some(i) = (e + 1..block_end(&lines, e)).find(|&i| lines[i].trim_start().strip_prefix('-').is_some_and(|v| v.trim().trim_matches(['"', '\'']) == "agent-context"))
    {
        cut(&mut lines, i, "[]");
    }
    lines.concat()
}

/// The instruction block between the markers and the blank line before it.
fn strip_md(text: &str) -> String {
    let (start, end) = ("<!-- agent-context:start -->", "<!-- agent-context:end -->");
    let mut s = text.to_string();
    while let Some(a) = s.find(start)
        && let Some(b) = s[a..].find(end).map(|b| a + b + end.len())
    {
        let (head, tail) = (s[..a].trim_end_matches(['\n', '\r']), s[b..].trim_start_matches(['\n', '\r']));
        s = match (head.is_empty(), tail.is_empty()) {
            (true, _) => tail.into(),
            (false, true) => format!("{head}\n"),
            (false, false) => format!("{head}\n\n{tail}"),
        };
    }
    s
}

/// Hermes `.env`: lines routed to the proxy.
fn strip_env(text: &str) -> String {
    let routed = |l: &str| l.split_once('=').is_some_and(|(_, v)| legacy_url(v.trim().trim_matches(['"', '\''])));
    text.split_inclusive('\n').filter(|l| !routed(l)).collect()
}

/// Built-in fixture for `selftest`: a Claude settings file edited after the Node journal.
pub fn selftest() -> bool {
    let edited = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:18788","MINE":"1"},"hooks":{"SessionStart":[{"hooks":[{"command":"node /h/.agent-context/launcher.mjs lifecycle claude"},{"command":"mine"}]}]},"model":"edited later"}"#;
    strip_json(edited) == "{\n  \"env\": {\n    \"MINE\": \"1\"\n  },\n  \"hooks\": {\n    \"SessionStart\": [\n      {\n        \"hooks\": [\n          {\n            \"command\": \"mine\"\n          }\n        ]\n      }\n    ]\n  },\n  \"model\": \"edited later\"\n}"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn selftest_fixture_passes() {
        assert!(selftest());
        assert!(selftest_locked());
    }

    /// A move that fails (here: the target exists) copies, removes what it can and marks the folder retired.
    #[test]
    fn failed_move_copies_and_marks() {
        let h = std::env::temp_dir().join(format!("agentq-retire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        let (dir, to) = (h.join(".agent-context"), h.join(".agent-context.retired"));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("other"), "").unwrap();
        std::fs::write(dir.join("launcher.mjs"), "l").unwrap();
        std::fs::write(dir.join("logs/proxy.log"), "x").unwrap();
        let out = retire(&dir, &to).unwrap();
        assert!(out[0].starts_with("kept:"), "{out:?}");
        assert_eq!(std::fs::read_to_string(to.join("logs/proxy.log")).unwrap(), "x");
        assert!(to.join("launcher.mjs").exists() && !dir.join("logs").exists());
        assert!(!installed(&dir) && locked(&dir).is_empty());
        let _ = std::fs::remove_dir_all(h);
    }

    /// A journal whose configs the user edited after install: the edits survive, legacy entries go, nothing blocks.
    #[test]
    fn edited_after_journal_is_stripped_not_refused() {
        let h = std::env::temp_dir().join(format!("agentq-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        let dir = h.join(".agent-context");
        let l = dir.join("launcher.mjs").display().to_string();
        // JSON and TOML basic strings: Windows backslashes must be escaped.
        let q = serde_json::to_string(&l).unwrap();
        let le = &q[1..q.len() - 1];
        let w = |f: &str, t: &str| {
            let p = h.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        let files: Vec<String> = FILES.iter().map(|f| h.join(f).display().to_string()).chain([h.join("elsewhere.json").display().to_string()]).collect();
        w(".agent-context/backups/1/journal.json", &serde_json::json!({"status": "installed", "files": files.iter().map(|f| serde_json::json!({"file": f})).collect::<Vec<_>>()}).to_string());
        w(".agent-context/services/claude.json", r#"{"pid": 4242, "childPid": 4243}"#);
        w("Library/LaunchAgents/com.taskqorg.contextq-monitor.plist", "");
        w(".config/systemd/user/agent-context-monitor.service", "");
        w(".claude/settings.json", &format!("{{\n  \"env\": {{\n    \"ANTHROPIC_BASE_URL\": \"http://127.0.0.1:18788\",\n    \"MINE\": \"edited later\"\n  }},\n  \"hooks\": {{\n    \"SessionStart\": [\n      {{\n        \"hooks\": [\n          {{\n            \"command\": \"'node' '{le}' lifecycle claude\"\n          }}\n        ]\n      }}\n    ],\n    \"Stop\": [\n      {{\n        \"hooks\": [\n          {{\n            \"command\": \"mine\"\n          }}\n        ]\n      }}\n    ]\n  }}\n}}\n"));
        w(".claude.json", &format!("{{\n  \"mcpServers\": {{\n    \"agent_context\": {{\n      \"command\": \"node\",\n      \"args\": [\"{le}\", \"tools\", \"mcp\", \"claude\"]\n    }},\n    \"mine\": {{\n      \"command\": \"x\"\n    }}\n  }}\n}}\n"));
        w(".codex/langfuse.json", "{\n  \"base_url\": \"http://127.0.0.1:18787\",\n  \"agent_context_base_url\": \"https://lf.example\",\n  \"public_key\": \"pk\"\n}\n");
        w(".codex/config.toml", &format!("model_provider = \"agent_context\"\nmodel = \"edited later\"\n\n[mcp_servers.mine]\ncommand = \"x\"\n\n[model_providers.agent_context]\nbase_url = \"http://127.0.0.1:18787/v1\"\n\n[mcp_servers.agent_context]\ncommand = \"node\"\nargs = [\"{le}\", \"tools\", \"mcp\", \"codex\"]\n\n[mcp_servers.agent_context.tools.index_document]\napproval_mode = \"approve\"\n"));
        w(".codex/AGENTS.md", "# Mine\n\n<!-- agent-context:start -->\n# Agent Context\n<!-- agent-context:end -->\n");
        w(".hermes/config.yaml", &format!("model: edited later\nplugins:\n  enabled:\n    - agent-context\n  entries:\n    agent-context:\n      settings:\n        launcher: {l}\nmcp_servers:\n  supercompress:\n    command: node\n    args:\n      - /n/@alexkirs/agent-context/bin/agent-context.js\n  mine:\n    command: x\n  agent_context:\n    command: node\n    args:\n      - {l}\n"));
        w(".hermes/.env", "MINE=1\nHERMES_LANGFUSE_BASE_URL=http://127.0.0.1:18790\n");
        #[cfg(unix)]
        std::fs::set_permissions(h.join(".hermes/.env"), std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        w(".hermes/plugins/agent-context/plugin.yaml", "name: agent-context\n");
        w("elsewhere.json", "{\"hooks\": \"launcher.mjs\"}");
        let before: Vec<Option<String>> = files.iter().map(|f| std::fs::read_to_string(f).ok()).collect();
        let calls = RefCell::new(vec![]);
        let run = |c: &str, a: &[&str]| {
            calls.borrow_mut().push(format!("{c} {}", a.join(" ")));
            !a.iter().any(|x| x.contains("4243"))
        };
        let dry = remove(&dir, true, "2026-10-13", &run).unwrap();
        assert!(dry.iter().any(|l| l.starts_with("would strip:")), "{dry:?}");
        assert_eq!(files.iter().map(|f| std::fs::read_to_string(f).ok()).collect::<Vec<_>>(), before, "dry run writes nothing");
        assert!(dir.exists());
        let out = remove(&dir, false, "2026-10-13", &run).unwrap();
        let r = |f: &str| std::fs::read_to_string(h.join(f)).unwrap();
        assert_eq!(r(".claude/settings.json"), "{\n  \"env\": {\n    \"MINE\": \"edited later\"\n  },\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\n            \"command\": \"mine\"\n          }\n        ]\n      }\n    ]\n  }\n}\n");
        assert_eq!(r(".claude.json"), "{\n  \"mcpServers\": {\n    \"mine\": {\n      \"command\": \"x\"\n    }\n  }\n}\n");
        assert_eq!(r(".codex/langfuse.json"), "{\n  \"base_url\": \"https://lf.example\",\n  \"public_key\": \"pk\"\n}\n");
        assert_eq!(r(".codex/config.toml"), "model = \"edited later\"\n\n[mcp_servers.mine]\ncommand = \"x\"\n");
        assert_eq!(r(".codex/AGENTS.md"), "# Mine\n");
        assert_eq!(r(".hermes/config.yaml"), "model: edited later\nplugins:\n  enabled: []\n  entries: {}\nmcp_servers:\n  mine:\n    command: x\n");
        assert_eq!(r(".hermes/.env"), "MINE=1\n");
        #[cfg(unix)]
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(h.join(".hermes/.env")).unwrap().permissions()) & 0o777, 0o600);
        assert!(!h.join(".hermes/plugins/agent-context").exists());
        assert_eq!(r("elsewhere.json"), "{\"hooks\": \"launcher.mjs\"}", "outside the known files: untouched");
        let retired = h.join(".agent-context.retired-2026-10-13");
        assert!(!dir.exists() && retired.exists(), "{out:?}");
        assert_eq!(std::fs::read_to_string(retired.join("config-backup/.claude/settings.json")).ok(), before[5]);
        let unit = if cfg!(target_os = "macos") { "Library/LaunchAgents/com.taskqorg.contextq-monitor.plist" } else { ".config/systemd/user/agent-context-monitor.service" };
        assert!(cfg!(windows) || !h.join(unit).exists(), "monitor unit removed");
        let calls = calls.borrow().clone();
        assert!(calls.iter().any(|c| c.contains("uninstall -g --ignore-scripts @alexkirs/agent-context")), "{calls:?}");
        assert!(out.iter().any(|l| l.contains("pid 4242")) && !out.iter().any(|l| l.contains("pid 4243")), "{out:?}");
        assert_eq!(remove(&dir, false, "2026-10-13", &run).unwrap(), ["legacy: already removed"]);
        let _ = std::fs::remove_dir_all(h);
    }
}
