//! M6.14/#297: a registered hook keeps metadata while installation writes are denied; native fallback stays intact.
use std::{fs,path::{Path,PathBuf},process::{Command,Output,Stdio},io::Write};
fn command(root:&Path,args:&[&str])->Command {
    let mut c=Command::new(env!("CARGO_BIN_EXE_agentq"));
    c.args(args).env("HOME",root).env("USERPROFILE",root).env("LOCALAPPDATA",root).env("APPDATA",root)
        .env("XDG_CONFIG_HOME",root.join("config")).env("CODEX_HOME",root.join("profile"))
        .env("CLAUDE_CONFIG_DIR",root.join("claude")).env("HERMES_HOME",root.join("hermes"))
        .env_remove("TASKQ_TASK").env_remove("AGENTQ_LAUNCHER").current_dir(root);
    c
}
fn hook(mut c:Command,input:&str)->Output {
    let mut p=c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    p.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();p.wait_with_output().unwrap()
}
#[test]
fn registered_cli_store_outage_and_unreadable_current_authority_keep_native_answer() {
    let root=std::env::temp_dir().join(format!("agentq297-cli-{}",std::process::id()));let _=fs::remove_dir_all(&root);
    let mut dirs=fs::DirBuilder::new();dirs.recursive(true);
    #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;dirs.mode(0o700);}
    dirs.create(root.join("profile")).unwrap();dirs.create(root.join("spool")).unwrap();
    fs::write(root.join("taskq.json"),r#"{"repo":"invented/hook"}"#).unwrap();
    let root=root.canonicalize().unwrap();let spool=root.join("spool");
    let r=command(&root,&["hook-spool","register","codex","invented/hook",spool.to_str().unwrap()]).output().unwrap();
    assert!(r.status.success(),"{}",String::from_utf8_lossy(&r.stderr));
    let data=if cfg!(windows) {root.join("agentq/data")} else {root.join(".local/share/agentq/data")};
    dirs.create(data.join("observations.sqlite3")).unwrap();
    let input=serde_json::json!({"session_id":"invented","turn_id":"invented-turn","cwd":root}).to_string();
    let output=hook(command(&root,&["hook","codex","Stop"]),&input);
    assert_eq!((output.status.code(),String::from_utf8_lossy(&output.stdout).trim()),(Some(0),"{}"));
    let events=||->Vec<PathBuf> {fs::read_dir(&spool).unwrap().map(|e|e.unwrap().path()).filter(|p|p.file_name().unwrap().to_string_lossy().ends_with(".hook.json")).collect()};
    assert_eq!(events().len(),1);
    let doctor=command(&root,&["doctor"]).output().unwrap();
    assert_eq!(doctor.status.code(),Some(1));
    let report=String::from_utf8(doctor.stdout).unwrap();
    let spool_report:serde_json::Value=serde_json::from_str(report.lines().find_map(|l|l.strip_prefix("hook spool: ")).unwrap()).unwrap();
    assert_eq!(spool_report["pending"],1);assert!(spool_report["oldest_age"].as_u64().is_some());
    #[cfg(target_os="macos")] {
        // Private Seatbelt fixture only; this is not a Codex workspace-write qualification.
        let mut sandbox=Command::new("/usr/bin/sandbox-exec");
        sandbox.args(["-p",&format!("(version 1)(allow default)(deny file-write* (subpath {:?}))",data.to_str().unwrap()),env!("CARGO_BIN_EXE_agentq"),"hook","codex","Stop"])
            .env("HOME",&root).env("CODEX_HOME",root.join("profile")).env_remove("TASKQ_TASK").env_remove("AGENTQ_LAUNCHER").current_dir(&root);
        let next=serde_json::json!({"session_id":"invented","turn_id":"invented-sandbox-turn","cwd":root}).to_string();
        let output=hook(sandbox,&next);
        assert_eq!((output.status.code(),String::from_utf8_lossy(&output.stdout).trim()),(Some(0),"{}"),"{}",String::from_utf8_lossy(&output.stderr));
        assert_eq!(events().len(),2,"sandboxed child writes registered spool while install writes are denied");
    }
    let count=events().len();let index=data.join("observe/hook-spools.json");fs::remove_file(&index).unwrap();fs::create_dir(&index).unwrap();
    let output=hook(command(&root,&["hook","codex","Stop"]),&input);
    assert_eq!((output.status.code(),String::from_utf8_lossy(&output.stdout).trim()),(Some(0),"{}"));assert_eq!(events().len(),count);
    let doctor=command(&root,&["doctor"]).output().unwrap();
    let report=String::from_utf8(doctor.stdout).unwrap();
    let spool_report:serde_json::Value=serde_json::from_str(report.lines().find_map(|l|l.strip_prefix("hook spool: ")).unwrap()).unwrap();
    assert_eq!(spool_report["state"],"unknown");assert!(spool_report["pending"].is_null() && spool_report["oldest_age"].is_null());
    fs::remove_dir_all(root).unwrap();
}
