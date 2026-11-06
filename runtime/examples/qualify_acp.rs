//! Opt-in native qualification. The caller supplies a temporary home with a copied login; never a working profile.
use agentq_runtime::agent::{self,Approval,Event,Launch,NativeSpec,Runtime,Run,State};
use serde_json::json;
use std::{path::{Path,PathBuf},process::Command,time::{Duration,Instant}};

fn finish(runtime:&dyn Runtime,run:&mut Run)->Vec<Event> {
    let mut events=vec![];
    loop {
        events.extend(runtime.progress(run).unwrap());
        let state=runtime.poll(run).unwrap();
        if state!=State::Running {assert_ne!(state,State::Unknown);break;}
        std::thread::yield_now();
    }
    events.extend(runtime.progress(run).unwrap());events
}

fn main() {
    let root=PathBuf::from(std::env::var_os("AGENTQ_ACP_PROOF_ROOT").expect("AGENTQ_ACP_PROOF_ROOT"));
    let home=root.join("home");let gemini=home.join(".gemini");
    assert!(root.canonicalize().unwrap().starts_with(std::env::temp_dir().canonicalize().unwrap()),"temporary root required");
    let runtime=agent::runtime("antigravity").unwrap();let exe=runtime.executable(None).unwrap();
    let version=Command::new(&exe).arg("--version").output().unwrap();
    let mut results=vec![];
    for transport in ["acp","stream"] {
        let work=root.join(transport);std::fs::create_dir_all(&work).unwrap();
        assert!(std::fs::read_dir(&work).unwrap().next().is_none(),"fresh empty workspace required");
        let mut launch=Launch::new(&work,root.join(format!("{transport}.jsonl")),"Read marker.txt using a native tool. Reply only with the file's exact contents. Do not write any files.","agentq-qualification");
        launch.home=Some(gemini.clone());launch.command=vec![exe.clone().into_os_string()];launch.policy.approval=Approval::Bypass;
        launch.env=vec![("HOME".into(),home.clone().into_os_string()),("USERPROFILE".into(),home.clone().into_os_string()),("GEMINI_HOME".into(),gemini.clone().into_os_string())];
        launch.deadline=Some(Instant::now()+Duration::from_secs(180));
        std::fs::write(work.join("marker.txt"),"ACP_NATIVE_PROOF").unwrap();
        let (session,first,events)=if transport=="acp" {
            let mut run=runtime.launch(&launch).unwrap();
            let files=files(&work);assert_eq!(files,["marker.txt"]);
            let events=finish(runtime,&mut run);let r=runtime.report(&run).unwrap();
            (runtime.session_id(&run).unwrap(),agentq_runtime::cli::report_json(&r),events)
        } else {
            let spec=NativeSpec{root:work.clone(),session:"-".into(),resume:false,home:Some(gemini.clone()),args:vec!["--dangerously-skip-permissions".into()],env:launch.env.clone()};
            let mut turn=runtime.turn(Command::new(&exe),&spec,&launch.log,&launch.input,&launch.process).unwrap();
            turn.wait(launch.deadline).unwrap();let r=turn.report();
            let events=runtime.log_events(&std::fs::read_to_string(&launch.log).unwrap());
            (r.session.clone().unwrap(),agentq_runtime::cli::report_json(&r),events)
        };
        assert_eq!(first["outcome"],"ok","{first}");assert!(first["text"].as_str().unwrap_or_default().contains("ACP_NATIVE_PROOF"));
        assert!(events.iter().any(|e|matches!(e,Event::ToolStart{..})),"native tool start required");
        assert!(events.iter().any(|e|matches!(e,Event::ToolEnd{..})),"native tool completion required");
        assert!(first["usage"]["input"].is_u64() && first["usage"]["output"].is_u64());
        launch.input="Without tools, reply only with the exact contents you just read from marker.txt.".into();
        let second=if transport=="acp" {
            let mut run=runtime.resume(&session,&launch).unwrap();let events=finish(runtime,&mut run);
            assert!(!events.iter().any(|e|matches!(e,Event::ToolStart{..})),"load replay must not emit old tools");
            agentq_runtime::cli::report_json(&runtime.report(&run).unwrap())
        }else{
            let spec=NativeSpec{root:work.clone(),session:session.clone(),resume:true,home:Some(gemini.clone()),args:vec!["--dangerously-skip-permissions".into()],env:launch.env.clone()};
            let mut turn=runtime.turn(Command::new(&exe),&spec,&launch.log,&launch.input,&launch.process).unwrap();turn.wait(launch.deadline).unwrap();agentq_runtime::cli::report_json(&turn.report())
        };
        assert_eq!(second["outcome"],"ok","{second}");assert!(second["text"].as_str().unwrap_or_default().contains("ACP_NATIVE_PROOF"));
        launch.input="Reply CANCEL_PROOF. Do not use tools.".into();
        let cancelled=if transport=="acp" {
            let mut run=runtime.resume(&session,&launch).unwrap();runtime.cancel(&mut run).unwrap();finish(runtime,&mut run);
            agentq_runtime::cli::report_json(&runtime.report(&run).unwrap())
        }else{
            let spec=NativeSpec{root:work.clone(),session:session.clone(),resume:true,home:Some(gemini.clone()),args:vec!["--dangerously-skip-permissions".into()],env:launch.env.clone()};
            let mut turn=runtime.turn(Command::new(&exe),&spec,&launch.log,&launch.input,&launch.process).unwrap();turn.kill().unwrap();turn.wait(launch.deadline).unwrap();agentq_runtime::cli::report_json(&turn.report())
        };
        assert_eq!(files(&work),["marker.txt"],"no stray workspace writes");
        results.push(json!({"transport":transport,"session":session,"first":first,"resume":second,"cancel":cancelled,"tool_events":events,"workspace_files":files(&work)}));
    }
    let evidence=json!({"os":std::env::consts::OS,"version":String::from_utf8_lossy(&version.stdout).trim(),"results":results});
    std::fs::write(root.join("evidence.json"),serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    println!("{}",json!({"evidence":root.join("evidence.json"),"os":evidence["os"],"version":evidence["version"],"outcome":"passed"}));
}

fn files(root:&Path)->Vec<String> {
    fn walk(root:&Path,path:&Path,out:&mut Vec<String>) {for entry in std::fs::read_dir(path).unwrap() {let p=entry.unwrap().path();if p.is_dir(){walk(root,&p,out);}else{out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\',"/"));}}}
    let mut files=vec![];walk(root,root,&mut files);files.sort();files
}
