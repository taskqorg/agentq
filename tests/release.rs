//! #238 A2 / CoreQ R15: recovery from the real running launcher must dispatch the restored release.
use coreq::{Tool, update, write_durable};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn recovered_running_launcher_dispatches_prior_release() {
    let root = std::env::temp_dir().join(format!("agentq-238-launcher-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let t = Tool { install: root.clone(), ..Tool::new("agentq", "AgentQ", "taskqorg/agentq", "2.0.0") };
    fs::create_dir_all(t.release_exe("1.0.0").parent().unwrap()).unwrap();
    // The prior release is libtest: it rejects --check with a distinct response, without invoking AgentQ or the network.
    fs::copy(std::env::current_exe().unwrap(), t.release_exe("1.0.0")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_agentq"), root.join(t.exe_name())).unwrap();
    fs::write(root.join("current"), "1.0.0").unwrap();
    let recovery = root.join("data/install-recovery");
    fs::create_dir_all(&recovery).unwrap();
    fs::copy(root.join("current"), recovery.join("current")).unwrap();
    fs::copy(root.join(t.exe_name()), recovery.join(t.exe_name())).unwrap();
    let mut saved = json!({"current": true, "previous": false});
    saved[t.exe_name()] = json!(true);
    #[cfg(windows)]
    { saved[t.gui_exe_name()] = json!(false); }
    write_durable(&recovery.join("ready"), saved.to_string()).unwrap();
    update::prepare_install(&t, &fs::read(env!("CARGO_BIN_EXE_agentq")).unwrap()).unwrap();
    update::activate_install(&t).unwrap();

    // A dispatch regression must fail offline too: even a fallthrough download runs libtest as its fake curl.
    fs::copy(std::env::current_exe().unwrap(), root.join(format!("curl{}", std::env::consts::EXE_SUFFIX))).unwrap();
    let out = Command::new(root.join(t.exe_name())).args(["update", "--check"])
        .current_dir(&root).env("AGENTQ_INSTALL", &root).env("PATH", &root)
        .env("HOME", &root).env("USERPROFILE", &root).env("LOCALAPPDATA", &root).env("APPDATA", &root)
        .output().unwrap();
    assert_eq!(t.current().as_deref(), Some("1.0.0"));
    assert!(!recovery.exists());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Unrecognized option: 'check'"), "must reach the prior libtest release: {out:?}");
    fs::remove_dir_all(root).unwrap();
}

// M12.8 / #241 A1: a signed launcher cannot redirect protected authority to an unchecked current release.
#[test]
fn protected_host_uses_checked_launcher_bytes() {
    let root=std::env::temp_dir().join(format!("agentq-241-launcher-{}",std::process::id()));
    fs::create_dir_all(&root).unwrap();let root=root.canonicalize().unwrap();
    let t=Tool{install:root.clone(),..Tool::new("agentq","AgentQ","taskqorg/agentq","2.0.0")};
    fs::create_dir_all(t.release_exe("1.0.0").parent().unwrap()).unwrap();
    fs::copy(std::env::current_exe().unwrap(),t.release_exe("1.0.0")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_agentq"),root.join(t.exe_name())).unwrap();
    fs::write(root.join("current"),"1.0.0").unwrap();
    let out=Command::new(root.join(t.exe_name())).args(["agent","host","--action","invalid-action","--request","{}"])
        .env("AGENTQ_INSTALL",&root).output().unwrap();
    let error=String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("Next: retain the original permit and reconcile; never replay uncertain work"),"must reach checked host dispatcher without another release: {out:?}");
    fs::remove_dir_all(root).unwrap();
}
