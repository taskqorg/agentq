//! Opt-in Windows native cleanup proof: empty temporary home, no model prompt or login.
#[cfg(not(windows))] fn main() {eprintln!("Windows native cleanup qualification only");}
#[cfg(windows)] fn main() {
    use agentq_runtime::agent::{self,Launch};
    use serde_json::{Value,json};
    use std::{process::Command,sync::{Arc,Mutex},time::{Duration,Instant}};
    let root=std::env::temp_dir().join(format!("agentq-native-cleanup-{}",std::process::id()));
    let work=root.join("workspace");let home=root.join("home");let gemini=home.join(".gemini");std::fs::create_dir_all(&work).unwrap();std::fs::create_dir_all(&gemini).unwrap();
    let runtime=agent::runtime("antigravity").unwrap();let exe=runtime.executable(None).unwrap();
    let owned=Arc::new(Mutex::new(Vec::<coreq::process::Identity>::new()));let observed=owned.clone();
    // Empty input refuses after initialize/new, so this cannot submit a model turn.
    let mut launch=Launch::new(&work,root.join("native.jsonl"),"","agentq-qualification");launch.command=vec![exe.into_os_string()];launch.home=Some(gemini.clone());
    launch.deadline=Some(Instant::now()+Duration::from_secs(30));
    launch.env=[("HOME",home.clone()),("USERPROFILE",home.clone()),("APPDATA",home.join("AppData/Roaming")),("LOCALAPPDATA",home.join("AppData/Local")),("GEMINI_HOME",gemini)].into_iter().map(|(k,v)|(k.into(),v.into_os_string())).collect();
    let version=Command::new(&launch.command[0]).arg("--version").envs(launch.env.iter().cloned()).output().unwrap();assert!(version.status.success());
    launch.process.spawned=Some(Arc::new(move |child|{
        let query=format!("$p=Get-CimInstance Win32_Process; @({}; $p | Where-Object ParentProcessId -eq {} | ForEach-Object ProcessId) | ConvertTo-Json -Compress",child.id(),child.id());
        let output=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",&query]).output().map_err(|e|e.to_string())?;
        if !output.status.success(){return Err("native process snapshot failed".into());}
        let pids:Value=serde_json::from_slice(&output.stdout).map_err(|e|e.to_string())?;
        for pid in pids.as_array().ok_or("native process snapshot shape")? {if let Some(id)=coreq::process::identity(pid.as_u64().ok_or("native PID shape")? as u32).map_err(|e|e.to_string())? {observed.lock().unwrap().push(id);}}
        Ok(())
    }));
    let refusal=runtime.launch(&launch).err().expect("empty input or isolated native authentication must refuse");
    assert!(refusal.contains("Authentication") || refusal.contains("input-empty"),"{refusal}");
    let owned=owned.lock().unwrap();assert!(owned.len()>=2,"cmd and native Node must both be observed before input");
    let until=Instant::now()+Duration::from_secs(5);
    for identity in owned.iter() {while coreq::process::identity(identity.pid).unwrap().as_ref()==Some(identity) {assert!(Instant::now()<until,"native process survived cleanup: {}",identity.pid);std::thread::yield_now();}}
    assert!(std::fs::read_dir(&work).unwrap().next().is_none(),"native workspace write");
    println!("{}",json!({"os":"windows","version":String::from_utf8_lossy(&version.stdout).trim(),"observed_processes":owned.len(),"all_original_processes_gone":true,"workspace_writes":false,"model_prompt_submitted":false,"native_refusal":refusal}));
    std::fs::remove_dir_all(root).unwrap();
}
