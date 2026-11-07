//! #286: no real account sign-in. Scripted native protocols use only fixture homes.
use super::*;
use std::io::{BufRead,Write};
use agent::{Cause,Failure,LoginFlow,LoginOptions,LoginPhase,LoginSession,LoginStatus,Runtime};
const LINK:&str="https://claude.ai/oauth/authorize?code=true&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode&state=private";
fn options(root:&Path,client:&str,mode:&str,flow:LoginFlow)->LoginOptions {
    let home=root.join(client);std::fs::create_dir_all(&home).unwrap();provision_home(&home);
    let mut options=LoginOptions::new(Some(home),flow);options.executable=Some(exe().into());
    let authority=root.join("callback-authority");std::fs::create_dir_all(&authority).unwrap();provision_home(&authority);options.callback_authority=Some(authority);
    options.env=vec![("AGENTQ_FAKE_LOGIN".into(),mode.into()),("AGENTQ_LOGIN_CLIENT".into(),client.into()),("AGENTQ_LOGIN_ROOT".into(),root.as_os_str().into())];options
}

fn provision_home(path:&Path) {
    #[cfg(windows)] {
        // Elevated Windows defaults new directories to Administrators; that is not a proven account owner.
        let output=Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command", "$ErrorActionPreference='Stop'; $p=$env:AGENTQ_LOGIN_TEST_HOME; $acl=Get-Acl -LiteralPath $p; if($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -ne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value){$acl.SetOwner([Security.Principal.WindowsIdentity]::GetCurrent().User); Set-Acl -LiteralPath $p -AclObject $acl}"]).env("AGENTQ_LOGIN_TEST_HOME",path).output().unwrap();
        assert!(output.status.success(),"temporary home owner preparation failed ({:?}): {}",output.status.code(),String::from_utf8_lossy(&output.stderr));
    }
    #[cfg(not(windows))] let _=path;
}
fn until(runtime:&dyn Runtime,session:&LoginSession,predicate:impl Fn(&LoginStatus)->bool)->LoginStatus {
    let until=Instant::now()+Duration::from_secs(10);
    loop {let status=runtime.login_status(session).unwrap();if predicate(&status) {return status;}assert!(Instant::now()<until,"login status: {status:?}");std::thread::yield_now();}
}
fn terminal(status:&LoginStatus)->bool {status.finished && matches!(status.phase,LoginPhase::Authenticated|LoginPhase::Cancelled|LoginPhase::Failed|LoginPhase::Unknown)}
fn cancel_and_wait(runtime:&dyn Runtime,session:&LoginSession) {runtime.login_cancel(session).unwrap();assert_eq!(until(runtime,session,terminal).phase,LoginPhase::Cancelled);}
pub fn pipes() {
    let root=scratch("login-pipes");
    let claude=agent::runtime("claude").unwrap();let codex=agent::runtime("codex").unwrap();
    let opts=options(&root,"claude","code",LoginFlow::Browser);let session=claude.login_start(&opts).unwrap();
    let waiting=until(claude,&session,|s|s.phase==LoginPhase::Waiting);let prompt=waiting.prompt.unwrap();
    assert_eq!(prompt.url,LINK);assert!(prompt.needs_code);assert!(!format!("{prompt:?}").contains("private"));
    assert!(codex.login_status(&session).is_err());assert!(codex.login_code(&session,"secret").is_err());assert!(codex.login_cancel(&session).is_err());
    assert!(claude.login_start(&opts).is_err());assert!(claude.login_code(&session,"bad\ncode").is_err());
    claude.login_code(&session,"private-code#state").unwrap();assert!(claude.login_code(&session,"duplicate").is_err());
    assert_eq!(until(claude,&session,terminal).phase,LoginPhase::Authenticated);
    assert_eq!(std::fs::read_to_string(root.join("received")).unwrap(),"private-code#state\n");
    assert!(!std::fs::read_to_string(root.join("argv")).unwrap().contains("private-code"));
    assert!(claude.login_cancel(&session).is_err());
    let mut missing=opts.clone();missing.home=Some(root.join("missing"));assert!(claude.login_start(&missing).is_err());
    let mut provider=opts.clone();provider.env.push(("OPENAI_API_KEY".into(),"do-not-use".into()));assert!(claude.login_start(&provider).is_err());
    #[cfg(windows)] {
        let mut lower=options(&root,"codex","hold",LoginFlow::Device);let home=lower.home.take().unwrap();lower.env.push(("cOdEx_HoMe".into(),home.into_os_string()));
        let session=codex.login_start(&lower).unwrap();until(codex,&session,|s|s.phase==LoginPhase::Waiting);cancel_and_wait(codex,&session);drop(session);
        lower.env.push(("oPeNaI_aPi_KeY".into(),"never-used".into()));assert!(codex.login_start(&lower).is_err());
    }
    // Descriptor agreement must reach the real isolated protocol, including every unsupported mode.
    for runtime in agent::runtimes() {for (flow,capability) in [(LoginFlow::Browser,"login-browser"),(LoginFlow::Device,"login-device")] {
        let name=runtime.descriptor().name;let opts=options(&root,name,"hold",flow);
        match runtime.login_start(&opts) {
            Ok(session)=>{
                assert!(runtime.descriptor().has(capability),"{name}: undeclared {capability} accepted");
                let status=until(runtime,&session,|s|s.phase==LoginPhase::Waiting || terminal(s));assert_eq!(status.phase,LoginPhase::Waiting,"{name}: {capability} did not reach its native protocol");
                let prompt=status.prompt.unwrap();assert_eq!(prompt.user_code.is_some(),flow==LoginFlow::Device);
                if !prompt.needs_code {assert!(runtime.login_code(&session,"must-not-send").is_err());}cancel_and_wait(runtime,&session);drop(session);
            },
            Err(Failure::Refused(Cause("login-flow-unsupported")))=>assert!(!runtime.descriptor().has(capability),"{name}: declared {capability} refused"),
            Err(error)=>panic!("{name}: {capability}: {error:?}"),
        }
    }}
    teardown(root);
}
pub fn cancel_guards() {
    let root=scratch("login-cancel");let runtime=agent::runtime("claude").unwrap();
    for mode in ["changed","missing","unreadable"] {
        let opts=options(&root,"claude",mode,LoginFlow::Browser);let path=opts.home.as_ref().unwrap().join(".credentials.json");std::fs::write(&path,"old").unwrap();
        let session=runtime.login_start(&opts).unwrap();until(runtime,&session,|s|s.phase==LoginPhase::Waiting);
        match mode {"changed"=>std::fs::write(&path,"new").unwrap(),"missing"=>std::fs::remove_file(&path).unwrap(),_=>{std::fs::remove_file(&path).unwrap();std::fs::create_dir(&path).unwrap();}}
        assert!(runtime.login_cancel(&session).is_err());assert!(runtime.login_start(&opts).is_err());
        let observed=runtime.login_status(&session).unwrap();assert_eq!(observed.credentials_changed,if mode=="changed" {Some(true)}else{None});
        if mode=="unreadable" {std::fs::remove_dir(&path).unwrap();}
        std::fs::write(&path,"old").unwrap();cancel_and_wait(runtime,&session);drop(session);
    }
    teardown(root);
}
pub fn transport_cleanup() {
    let root=scratch("login-cleanup");let runtime=agent::runtime("claude").unwrap();let opts=options(&root,"claude","changed",LoginFlow::Browser);
    let session=runtime.login_start(&opts).unwrap();let waiting=until(runtime,&session,|s|s.phase==LoginPhase::Waiting);
    let path=opts.home.as_ref().unwrap().join(".credentials.json");std::fs::write(path,"new").unwrap();drop(session);
    assert!(runtime.login_start(&opts).is_err(),"drop with changed credentials retains the live native lease");
    let original_pid=waiting.pid.unwrap();
    std::fs::write(root.join("release"),"").unwrap();
    let end=Instant::now()+Duration::from_secs(10);
    // The actual credential writer and both pipe readers finish before a later lease is admitted.
    let mut session=None;while session.is_none() {session=runtime.login_start(&opts).ok();assert!(Instant::now()<end);std::thread::yield_now();}
    let session=session.unwrap();let finished=until(runtime,&session,terminal);assert_ne!(finished.pid,Some(original_pid));drop(session);teardown(root);
}

pub fn startup_and_pipes() {
    let root=scratch("login-startup-pipes");let runtime=agent::runtime("claude").unwrap();
    let opts=options(&root,"claude","pipe-holder",LoginFlow::Browser);
    let session=runtime.login_start(&opts).unwrap();until(runtime,&session,|s|s.phase==LoginPhase::Waiting);wait_for(&root.join("pipe-held"));wait_for(&root.join("leader-finished"));
    assert!(!terminal(&runtime.login_status(&session).unwrap()),"native exit is not transport EOF");assert!(runtime.login_start(&opts).is_err());
    std::fs::write(root.join("release"),"").unwrap();assert_eq!(until(runtime,&session,terminal).phase,LoginPhase::Unknown);drop(session);std::fs::remove_file(root.join("release")).unwrap();
    let mut failed=opts.clone();failed.executable=Some(root.join("missing-executable"));let session=runtime.login_start(&failed).unwrap();
    let result=until(runtime,&session,terminal);assert_eq!(result.reason,Some("login-start-failed"));assert_eq!(result.pid,None);drop(session);
    let early=options(&root,"claude","hold",LoginFlow::Browser);let session=runtime.login_start(&early).unwrap();cancel_and_wait(runtime,&session);drop(session);
    std::fs::remove_file(root.join("pipe-held")).unwrap();
    let late=options(&root,"claude","cancel-pipe-holder",LoginFlow::Browser);let session=runtime.login_start(&late).unwrap();until(runtime,&session,|s|s.phase==LoginPhase::Waiting);wait_for(&root.join("pipe-held"));
    runtime.login_cancel(&session).unwrap();assert!(!terminal(&runtime.login_status(&session).unwrap()));assert!(runtime.login_start(&late).is_err());
    std::fs::write(late.home.as_ref().unwrap().join(".credentials.json"),"late-native-write").unwrap();std::fs::write(root.join("release"),"").unwrap();
    let result=until(runtime,&session,terminal);assert_eq!(result.phase,LoginPhase::Unknown);assert_eq!(result.reason,Some("login-cancel-outcome-unknown"));assert_eq!(result.credentials_changed,Some(true));drop(session);
    let codex=agent::runtime("codex").unwrap();let unknown=options(&root,"codex","store-unverified",LoginFlow::Browser);let session=codex.login_start(&unknown).unwrap();
    let result=until(codex,&session,terminal);assert_eq!(result.phase,LoginPhase::Unknown);assert_eq!(result.reason,Some("login-credentials-unverified"));assert_eq!(result.pid,None);drop(session);teardown(root);
}
pub fn fake() {
    let args:Vec<_>=std::env::args().skip(1).collect();let root=PathBuf::from(std::env::var_os("AGENTQ_LOGIN_ROOT").unwrap());let client=std::env::var("AGENTQ_LOGIN_CLIENT").unwrap();
    let home=PathBuf::from(std::env::var_os(if client=="claude" {"CLAUDE_CONFIG_DIR"}else{"CODEX_HOME"}).unwrap());
    if args.first().is_some_and(|s|s=="--login-keep-pipe") {std::fs::write(root.join("pipe-held"),"").unwrap();return wait_for(&root.join("release"));}
    if args.iter().any(|s|s=="status") {
        let present=home.join(if client=="claude" {".credentials.json"}else{"auth.json"}).is_file();
        if client=="claude" {println!("{}",json!({"loggedIn":present,"authMethod":"claude.ai","apiProvider":"firstParty"}));}else{println!("{}",if present {"Logged in using ChatGPT"}else{"Not logged in"});}return;
    }
    if args.first().is_some_and(|s|s=="app-server") {
        for line in std::io::stdin().lock().lines() {let v:Value=serde_json::from_str(&line.unwrap()).unwrap();match v["method"].as_str() {
            Some("initialize")=>println!("{}",json!({"id":v["id"],"result":{}})),
            Some("config/read")=>println!("{}",json!({"id":v["id"],"result":{"config":{"cli_auth_credentials_store":if std::env::var("AGENTQ_FAKE_LOGIN").as_deref()==Ok("store-unverified") {"keyring"}else{"file"}}}})),
            Some("account/login/start")=>{std::fs::write(root.join(format!("auth-start-{}",home.file_name().unwrap().to_string_lossy())),"").unwrap();println!("{}",json!({"id":v["id"],"result":{"loginId":"login-1","authUrl":"https://auth.openai.com/oauth/authorize?state=private&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"}}));},
            _=>{},
        }std::io::stdout().flush().unwrap();}return;
    }
    std::fs::write(root.join("argv"),format!("{args:?}")).unwrap();
    if client=="codex" {println!("Open https://auth.openai.com/codex/device\n2. Enter this one-time code (expires in 15 minutes)\n AB12-CD34\n");}else {
        // Fake doc URL and split streams must never form an accepted prompt.
        print!("https://claude.ai/oauth/auth");std::io::stdout().flush().unwrap();eprintln!("orize?code=true&redirect_uri=https%3A%2F%2Fconsole.anthropic.com ");
        print!("\n\x1b[34m{}",&LINK[..40]);std::io::stdout().flush().unwrap();print!("{}\x1b[0m\n",&LINK[40..]);
    }std::io::stdout().flush().unwrap();
    if matches!(std::env::var("AGENTQ_FAKE_LOGIN").as_deref(),Ok("pipe-holder"|"cancel-pipe-holder")) {
        let _child=Command::new(std::env::current_exe().unwrap()).arg("--login-keep-pipe").spawn().unwrap();std::fs::write(root.join("leader-finished"),"").unwrap();if std::env::var("AGENTQ_FAKE_LOGIN").as_deref()==Ok("cancel-pipe-holder") {wait_for(&root.join("leader-release"));}return;
    }
    if std::env::var("AGENTQ_FAKE_LOGIN").as_deref()==Ok("code") {
        let mut line=String::new();std::io::stdin().read_line(&mut line).unwrap();std::fs::write(root.join("received"),line).unwrap();std::fs::write(home.join(".credentials.json"),"fixture credentials").unwrap();return;
    }
    wait_for(&root.join("release"));
}
pub fn native_proof() {
    let client=std::env::var("AGENTQ_LOGIN_CLIENT").unwrap();let home=PathBuf::from(std::env::var_os("AGENTQ_LOGIN_HOME").unwrap());assert!(home.is_dir());assert!(std::fs::read_dir(&home).unwrap().next().is_none(),"native proof requires an empty caller-created temp home");
    provision_home(&home);
    let runtime=agent::runtime(&client).unwrap();let flow=if std::env::var("AGENTQ_LOGIN_FLOW").as_deref()==Ok("device") {LoginFlow::Device}else{LoginFlow::Browser};
    let mut options=LoginOptions::new(Some(home.clone()),flow);options.executable=std::env::var_os("AGENTQ_LOGIN_EXECUTABLE").map(PathBuf::from);
    // Even native clients' OS-home writes stay in this proof's disposable home.
    options.env=vec![("HOME".into(),home.as_os_str().into()),("USERPROFILE".into(),home.as_os_str().into())];
    if client=="codex" && flow==LoginFlow::Browser {
        let authority=PathBuf::from(std::env::var_os("AGENTQ_LOGIN_CALLBACK_AUTHORITY").expect("native callback proof needs explicit caller authority"));
        provision_home(&authority);options.callback_authority=Some(authority);
    }
    let session=runtime.login_start(&options).unwrap();
    if std::env::var_os("AGENTQ_LOGIN_EXPECT_BUSY").is_some() {
        let status=until(runtime,&session,terminal);assert_eq!(status.reason,Some("login-browser-callback-busy"));assert_eq!(status.pid,None);println!("native_busy_callback_refused_before_login=true");return;
    }
    let mut qualified=false;
    loop {let status=runtime.login_status(&session).unwrap();
        let browser=client!="claude" || std::fs::read_dir(&home).unwrap().filter_map(Result::ok).any(|entry|entry.file_name().to_string_lossy().ends_with(".called"));
        if let Some(prompt)=&status.prompt {if browser {
            qualified=true;println!("native_prompt client={client} flow={flow:?} https={} needs_code={} device_code={} browser_suppressed={browser}",prompt.url.starts_with("https://"),prompt.needs_code,prompt.user_code.is_some());
            runtime.login_cancel(&session).unwrap();break;
        }}
        if home.join("cancel-proof").exists() {runtime.login_cancel(&session).unwrap();break;}
        assert!(!terminal(&status),"native URL not qualified: {status:?}");std::thread::sleep(Duration::from_millis(20));
    }
    loop {let status=runtime.login_status(&session).unwrap();if terminal(&status) {println!("native_cancel client={client} phase={:?} credentials_changed={:?}",status.phase,status.credentials_changed);assert_eq!(status.phase,LoginPhase::Cancelled);break;}std::thread::sleep(Duration::from_millis(20));}
    assert!(qualified,"native prompt/browser suppression remains unqualified");
    assert!(!home.join("auth.json").exists());assert!(!home.join(".credentials.json").exists());
}


pub fn controller() {
    let root=PathBuf::from(std::env::args_os().nth(2).unwrap());let account=std::env::args().nth(3).unwrap();let mode=std::env::args().nth(4).unwrap();
    let runtime=agent::runtime("codex").unwrap();let mut opts=options(&root,"codex","hold",LoginFlow::Browser);
    let home=root.join(&account);std::fs::create_dir_all(&home).unwrap();provision_home(&home);opts.home=Some(home);
    let session=runtime.login_start(&opts).unwrap();let state=until(runtime,&session,|s|s.phase==LoginPhase::Waiting || terminal(s));
    coreq::write_durable(&root.join(format!("{account}.json")),serde_json::to_vec(&json!({"phase":format!("{:?}",state.phase),"reason":state.reason,"pid":state.pid})).unwrap()).unwrap();
    if state.phase==LoginPhase::Waiting {
        if mode=="hold" {wait_for(&root.join(format!("release-{account}")));}
        if mode=="uncertain" {std::fs::write(root.join("callback-authority/codex-oauth-127.0.0.1-1455.json.pending"),"held failed-write fixture").unwrap();}
        runtime.login_cancel(&session).unwrap();let final_state=until(runtime,&session,|s|terminal(s) || s.reason==Some("login-helper-cleanup-unknown"));
        if mode=="uncertain" {assert_eq!(final_state.reason,Some("login-helper-cleanup-unknown"));assert!(final_state.exited);}else{assert_eq!(final_state.phase,LoginPhase::Cancelled);}
    }
}
pub fn callback_processes() {
    let root=scratch("login-callback-processes");let authority=root.join("callback-authority");std::fs::create_dir_all(&authority).unwrap();provision_home(&authority);
    let spawn=|account:&str,mode:&str|Command::new(std::env::current_exe().unwrap()).args(["--login-controller",root.to_str().unwrap(),account,mode]).spawn().unwrap();
    let status=|account:&str|->Value {let path=root.join(format!("{account}.json"));wait_for(&path);serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()};
    let mut first=spawn("first","hold");assert_eq!(status("first")["phase"],"Waiting");
    let original=std::fs::read(authority.join("codex-oauth-127.0.0.1-1455.json")).unwrap();
    let mut second=spawn("second","finish");let denied=status("second");assert_eq!(denied["reason"],"login-callback-authority-unavailable");assert!(denied["pid"].is_null());assert!(second.wait().unwrap().success());assert!(!root.join("auth-start-second").exists());assert_eq!(std::fs::read(authority.join("codex-oauth-127.0.0.1-1455.json")).unwrap(),original);
    std::fs::write(root.join("release-first"),"").unwrap();assert!(first.wait().unwrap().success());
    let mut third=spawn("third","finish");assert_eq!(status("third")["phase"],"Waiting");assert!(third.wait().unwrap().success());assert!(root.join("auth-start-third").exists());
    let pending=authority.join("codex-oauth-127.0.0.1-1455.json.pending");let settled=std::fs::read(authority.join("codex-oauth-127.0.0.1-1455.json")).unwrap();std::fs::write(&pending,"held failed-write fixture").unwrap();
    let mut prewrite=spawn("prewrite","finish");let denied=status("prewrite");assert_eq!(denied["reason"],"login-callback-write-unknown");assert!(denied["pid"].is_null());assert!(prewrite.wait().unwrap().success());assert!(!root.join("auth-start-prewrite").exists());assert_eq!(std::fs::read(authority.join("codex-oauth-127.0.0.1-1455.json")).unwrap(),settled);std::fs::remove_file(&pending).unwrap();
    let mut uncertain=spawn("uncertain","uncertain");assert_eq!(status("uncertain")["phase"],"Waiting");assert!(uncertain.wait().unwrap().success());
    let mut refused=spawn("refused","finish");let denied=status("refused");assert_eq!(denied["reason"],"login-callback-owner-unknown");assert!(denied["pid"].is_null());assert!(refused.wait().unwrap().success());assert!(!root.join("auth-start-refused").exists());
    let marker:Value=serde_json::from_slice(&std::fs::read(authority.join("codex-oauth-127.0.0.1-1455.json")).unwrap()).unwrap();assert_eq!(marker["phase"],"starting");teardown(root);
}


pub fn home_controller() {
    let root=PathBuf::from(std::env::args_os().nth(2).unwrap());let name=std::env::args().nth(3).unwrap();
    let runtime=agent::runtime("claude").unwrap();let opts=options(&root,"claude","hold",LoginFlow::Browser);
    let session=match runtime.login_start(&opts) {
        Ok(session)=>session,Err(error)=>{coreq::write_durable(&root.join(format!("{name}.json")),json!({"refused":format!("{error:?}")}).to_string()).unwrap();return;},
    };
    let state=until(runtime,&session,|s|s.phase==LoginPhase::Waiting || terminal(s));
    coreq::write_durable(&root.join(format!("{name}.json")),json!({"phase":format!("{:?}",state.phase),"pid":state.pid,"reason":state.reason}).to_string()).unwrap();
    if state.phase==LoginPhase::Waiting {
        if name=="original" {wait_for(&root.join("controller-release"));}
        cancel_and_wait(runtime,&session);
    }
}
pub fn home_processes() {
    let root=scratch("login-home-owner-loss");let spawn=|name:&str|Command::new(std::env::current_exe().unwrap()).args(["--login-home-controller",root.to_str().unwrap(),name]).spawn().unwrap();
    let status=|name:&str|->Value {let path=root.join(format!("{name}.json"));wait_for(&path);serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()};
    let mut original=spawn("original");let started=status("original");assert_eq!(started["phase"],"Waiting");let pid=started["pid"].as_u64().unwrap() as u32;
    let identity=coreq::process::identity(pid).unwrap().unwrap();original.kill().unwrap();original.wait().unwrap();
    let original_still_live=coreq::process::identity(pid).unwrap().as_ref()==Some(&identity);
    let mut replacement=spawn("replacement");let result=status("replacement");let replacement_exit=replacement.wait().unwrap();
    std::fs::write(root.join("release"),"").unwrap();
    let end=Instant::now()+Duration::from_secs(10);loop {match coreq::process::identity(pid) {Ok(Some(now)) if now!=identity=>break,Ok(None)=>break,_=>{assert!(Instant::now()<end,"owned original native identity remains unconfirmed");std::thread::yield_now();}}}
    assert!(replacement_exit.success());assert!(original_still_live,"counterexample requires original native writer after controller death");
    assert!(result["refused"].as_str().is_some_and(|s|s.contains("login-owner-unknown")),"replacement admission with original writer live: {result}");teardown(root);
}
