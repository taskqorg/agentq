//! Explicit native account operations; no worker input, credential export or rollback.
use super::*;
use std::sync::{Arc,Mutex,mpsc};
#[path="signin/home.rs"]
mod home;
pub(super) use home::Home;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum LoginFlow {Browser,Device}
#[derive(Clone)]
pub struct LoginOptions {
    pub executable:Option<PathBuf>,
    pub home:Option<PathBuf>,
    pub env:Vec<(OsString,OsString)>,
    pub flow:LoginFlow,
    /// Shared caller-owned directory for native callback resources; required by Codex Browser.
    pub callback_authority:Option<PathBuf>,
}
impl LoginOptions {
    pub fn new(home:Option<PathBuf>,flow:LoginFlow)->Self {Self {executable:None,home,env:vec![],flow,callback_authority:None}}
}
#[derive(Clone,PartialEq,Eq)]
pub struct LoginPrompt {pub url:String,pub user_code:Option<String>,pub needs_code:bool}
impl std::fmt::Debug for LoginPrompt {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.debug_struct("LoginPrompt").field("url",&"[redacted]").field("user_code",&self.user_code.as_ref().map(|_|"[redacted]")).field("needs_code",&self.needs_code).finish()}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum LoginPhase {Starting,Waiting,CodeQueued,CodeSubmitted,CancelRequested,Authenticated,Cancelled,Failed,Unknown}
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct LoginStatus {
    pub phase:LoginPhase,pub prompt:Option<LoginPrompt>,pub reason:Option<&'static str>,pub pid:Option<u32>,pub exit:Option<i32>,
    pub credentials_changed:Option<bool>,pub restart_required:Option<bool>,pub exited:bool,pub finished:bool,
}
impl LoginStatus {fn starting()->Self {Self {phase:LoginPhase::Starting,prompt:None,reason:None,pid:None,exit:None,credentials_changed:None,restart_required:None,exited:false,finished:false}}}
pub struct LoginSession {runtime:&'static str,events:mpsc::Sender<Event>,status:Arc<Mutex<LoginStatus>>}
impl Drop for LoginSession {fn drop(&mut self) {let _=self.events.send(Event::Cancel(None));}}

pub(super) struct Context {pub home:PathBuf,pub explicit:bool,pub callback_authority:Option<PathBuf>,program:PathBuf,env:Vec<(OsString,OsString)>,home_var:&'static str,pin:Option<PathBuf>}
impl Context {
    pub fn env_value(&self,key:&str)->Option<OsString> {self.env.iter().rev().find(|(k,_)|env_key(k,key)).map(|(_,v)|v.clone()).or_else(||std::env::var_os(key))}
    pub fn command(&self)->Command {
        let mut command=Command::new(&self.program);command.envs(self.env.iter().cloned());
        super::super::clean_environment(&mut command,true);command.current_dir(&self.home);
        if let Some(home)=&self.pin {command.env(self.home_var,home);}crate::quiet(&mut command);command
    }
}
pub(super) struct Protocol {
    pub executable:fn(PathBuf)->Res<PathBuf>,
    pub configure:fn(&Context,LoginFlow,&mut Command)->Result<Box<dyn Conversation>,&'static str>,
    pub credentials:fn(&Context)->Res<Vec<Option<String>>>,
    pub verify:fn(&Context)->Check,
}
pub(super) trait Conversation:Send {
    fn admission(&self)->Option<Arc<dyn Send+Sync>> {None}
    fn initial(&mut self)->Vec<Vec<u8>> {vec![]}
    fn output(&mut self,stream:usize,bytes:&[u8])->Res<Vec<Action>>;
    fn code(&mut self,_code:&str)->Res<Vec<u8>> {Err("login-code-unsupported".into())}
    fn completed(&self,exit:Option<i32>)->bool;
    fn cleanup(&mut self)->Res<()> {Ok(())}
}
pub(super) enum Action {Prompt(LoginPrompt),Write(Vec<u8>),Complete,Failure(&'static str)}
enum Event {Bytes(usize,Vec<u8>),Closed(usize,bool),Exited(Res<Option<i32>>),Written(bool,bool),Code(String,mpsc::Sender<Res<()>>),Cancel(Option<mpsc::Sender<Res<()>>>)}
enum Input {Open(std::process::ChildStdin),Write(Vec<u8>,bool),Close}

pub(super) fn start(adapter:&Adapter,options:&LoginOptions)->Result<LoginSession,Failure> {
    let capability=match options.flow {LoginFlow::Browser=>"login-browser",LoginFlow::Device=>"login-device"};
    if !adapter.descriptor.has(capability) {return Err(Failure::Refused(Cause("login-flow-unsupported")));}
    let protocol=adapter.ops.sign_in.ok_or(Failure::Refused(Cause("login-unsupported")))?;
    if options.env.iter().any(|(key,_)|registry::auth_vars().iter().any(|k|env_key(key,k))) {return Err(Failure::Refused(Cause("login-auth-env-unsupported")));}
    let selected=options.home.clone().or_else(||options.env.iter().rev().find(|(key,_)|env_key(key,adapter.ops.home_var)).map(|(_,value)|PathBuf::from(value)))
        .or_else(||std::env::var_os(adapter.ops.home_var).map(PathBuf::from));
    if selected.is_none() && options.env.iter().any(|(key,_)|env_key(key,"HOME") || env_key(key,"USERPROFILE")) {return Err(Failure::Refused(Cause("login-home-unverified")));}
    let explicit=selected.is_some();let home=selected.or_else(||(adapter.ops.home)()).ok_or(Failure::Refused(Cause("login-home-missing")))?;
    let path=options.env.iter().rev().find(|(key,_)|env_key(key,"PATH")).map(|(_,value)|value.clone()).unwrap_or_else(||std::env::var_os("PATH").unwrap_or_default());
    let program=options.executable.clone().map(Ok).unwrap_or_else(||(adapter.ops.executable)(&path)).map_err(Failure::Failed)?;
    let program=(protocol.executable)(program).map_err(Failure::Failed)?;
    let guard=Arc::new(Home::open(&home).map_err(Failure::Failed)?);
    let context=Context {home,explicit,callback_authority:options.callback_authority.clone(),program,env:options.env.clone(),home_var:adapter.ops.home_var,pin:options.home.clone()};
    let (events,receive)=mpsc::channel();let status=Arc::new(Mutex::new(LoginStatus::starting()));
    let result=status.clone();let send=events.clone();let flow=options.flow;let lease=guard.clone();let on_start_failure=guard.clone();
    std::thread::Builder::new().name("agentq-login".into()).spawn(move|| {
        let outcome=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||run(context,guard,protocol,flow,send,receive,&result)));
        if let Err(mut reason)=outcome.unwrap_or(Err("login-controller-panicked")) {
            let mut state=result.lock().unwrap();if state.pid.is_none() {if let Err(error)=lease.finish() {reason=error;}}else if !state.exited {std::mem::forget(lease);}
            state.phase=LoginPhase::Unknown;state.reason=Some(reason);state.finished=state.pid.is_none() && reason!="login-helper-cleanup-unknown";
        }
    }).map_err(|_|Failure::Failed(on_start_failure.finish().err().unwrap_or("login-start-failed").into()))?;
    Ok(LoginSession {runtime:adapter.client.code(),events,status})
}
pub(super) fn status(runtime:&str,session:&LoginSession)->Result<LoginStatus,Failure> {if session.runtime!=runtime {return Err(Failure::Refused(Cause("foreign-login")));}Ok(session.status.lock().unwrap().clone())}
pub(super) fn code(runtime:&str,session:&LoginSession,code:&str)->Result<(),Failure> {
    status(runtime,session)?;let (send,answer)=mpsc::channel();
    session.events.send(Event::Code(code.into(),send)).map_err(|_|Failure::Refused(Cause("login-not-running")))?;
    answer.recv().map_err(|_|Failure::Failed("login-code-outcome-unknown".into()))?.map_err(Failure::Failed)
}
pub(super) fn cancel(runtime:&str,session:&LoginSession)->Result<LoginStatus,Failure> {
    let observed=status(runtime,session)?;
    if observed.phase==LoginPhase::Authenticated {return Err(Failure::Refused(Cause("login-already-authenticated")));}
    let (send,answer)=mpsc::channel();
    if session.events.send(Event::Cancel(Some(send))).is_err() {return Ok(observed);}
    answer.recv().map_err(|_|Failure::Failed("login-cancel-outcome-unknown".into()))?.map_err(Failure::Failed)?;
    status(runtime,session)
}

fn run(context:Context,guard:Arc<Home>,protocol:&'static Protocol,flow:LoginFlow,events:mpsc::Sender<Event>,receive:mpsc::Receiver<Event>,state:&Arc<Mutex<LoginStatus>>)->Result<(),&'static str> {
    guard.check(&context.home).map_err(|_|"login-home-changed")?;
    let before=(protocol.credentials)(&context);
    if before.is_err() {return Err("login-credentials-unverified");}
    guard.check(&context.home).map_err(|_|"login-home-changed")?;
    let mut command=context.command();let mut conversation=(protocol.configure)(&context,flow,&mut command)?;
    let guard=Arc::new((guard,conversation.admission()));
    std::thread::scope(|scope| {
    // All I/O workers exist before native spawn; partial worker startup cannot strand a native login.
    let (input_send,input)=mpsc::channel();let send=events.clone();let lease=guard.clone();
    let writing=std::thread::Builder::new().name("agentq-login-input".into()).spawn_scoped(scope,move|| {
        let _lease=lease;let mut pipe=None;
        while let Ok(message)=input.recv() {match message {
            Input::Open(stdin)=>pipe=Some(stdin),Input::Close=>break,
            Input::Write(bytes,code)=>{let ok=pipe.as_mut().is_some_and(|stdin:&mut std::process::ChildStdin|stdin.write_all(&bytes).and_then(|()|stdin.flush()).is_ok());let _=send.send(Event::Written(code,ok));if !ok {break;}}
        }}
    }).map_err(|_|"login-io-start-failed")?;
    let (out_send,out)=mpsc::channel::<std::process::ChildStdout>();let send=events.clone();let lease=guard.clone();
    let output=std::thread::Builder::new().name("agentq-login-stdout".into()).spawn_scoped(scope,move|| {let _lease=lease;if let Ok(pipe)=out.recv() {read(pipe,0,send);}}).map_err(|_|"login-io-start-failed")?;
    let (err_send,errors)=mpsc::channel::<std::process::ChildStderr>();let send=events.clone();let lease=guard.clone();
    let error=std::thread::Builder::new().name("agentq-login-stderr".into()).spawn_scoped(scope,move|| {let _lease=lease;if let Ok(pipe)=errors.recv() {read(pipe,1,send);}}).map_err(|_|"login-io-start-failed")?;
    let (child_send,children)=mpsc::channel::<Child>();let send=events.clone();let lease=guard.clone();
    let waiting=std::thread::Builder::new().name("agentq-login-exit".into()).spawn_scoped(scope,move|| {let mut lease=Some(lease);if let Ok(mut child)=children.recv() {let result=child.wait().map(|s|s.code()).map_err(|_|"login-wait-unknown".into());if result.is_err() {std::mem::forget(lease.take());}let _=send.send(Event::Exited(result));}}).map_err(|_|"login-io-start-failed")?;
    if guard.0.check(&context.home).is_err() {return Err("login-home-changed");}
    // A queued owner cancel before spawn must not start a native credential operation.
    for event in receive.try_iter() {match event {
        Event::Cancel(answer)=>{
            let change=if guard.0.check(&context.home).is_ok() {changed(&before,&(protocol.credentials)(&context))}else{None};
            drop(input_send);drop(out_send);drop(err_send);drop(child_send);
            for thread in [writing,output,error,waiting] {thread.join().map_err(|_|"login-transport-panicked")?;}
            guard.0.check(&context.home).map_err(|_|"login-home-changed")?;
            conversation.cleanup().map_err(|_|"login-helper-cleanup-unknown")?;guard.0.finish()?;drop(conversation);drop(guard);
            let reason=if change==Some(false) {None}else{Some(if change==Some(true) {"login-credentials-changed"}else{"login-credentials-unverified"})};
            let mut s=state.lock().unwrap();s.phase=if reason.is_none() {LoginPhase::Cancelled}else{LoginPhase::Unknown};s.reason=reason;s.finished=true;s.credentials_changed=change;s.restart_required=change;
            if let Some(answer)=answer {let _=answer.send(reason.map_or(Ok(()),|reason|Err(reason.into())));}return Ok(());
        },
        Event::Code(_,answer)=>{let _=answer.send(Err("login-code-not-accepted".into()));},_=>{},
    }}
    let mut child=match command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {Ok(c)=>c,Err(_)=>{conversation.cleanup().map_err(|_|"login-helper-cleanup-unknown")?;return Err("login-start-failed");}};
    state.lock().unwrap().pid=Some(child.id());let native=crate::process::identity(child.id()).ok().flatten();
    let input_ok=input_send.send(Input::Open(child.stdin.take().unwrap())).is_ok();
    let output_ok=out_send.send(child.stdout.take().unwrap()).is_ok();
    let error_ok=err_send.send(child.stderr.take().unwrap()).is_ok();
    if let Err(error)=child_send.send(child) {
        let mut child=error.0;let _=child.kill();
        // A failed wait leaves the home fenced; absence was not proven.
        if child.wait().is_err() {std::mem::forget(guard.clone());}
        return Err("login-transport-unknown");
    }
    for bytes in conversation.initial() {let _=input_send.send(Input::Write(bytes,false));}
    let (mut exited,mut closed,mut code_sent,mut cancelling,mut completed)=(None,[!output_ok,!error_ok],false,false,false);
    let mut failure=(!(input_ok && output_ok && error_ok)).then_some("login-transport-unknown");
    while exited.is_none() || !closed.iter().all(|v|*v) {
        let event=receive.recv().map_err(|_|"login-transport-unknown")?;
        match event {
            Event::Bytes(stream,bytes)=>match conversation.output(stream,&bytes) {
                Ok(actions)=>for action in actions {match action {
                    Action::Prompt(prompt) if !cancelling && !completed && failure.is_none() && exited.is_none()=>{let mut s=state.lock().unwrap();s.phase=LoginPhase::Waiting;s.prompt=Some(prompt);},
                    Action::Prompt(_)=>{},
                    Action::Write(bytes) if failure.is_none() && !cancelling && exited.is_none()=>{let _=input_send.send(Input::Write(bytes,false));},
                    Action::Write(_)=>{},
                    Action::Complete=>{completed=true;let _=input_send.send(Input::Close);},
                    Action::Failure(reason)=>{failure=Some(reason);let mut s=state.lock().unwrap();s.phase=LoginPhase::Unknown;s.reason=Some(reason);drop(s);let _=input_send.send(Input::Close);},
                }},Err(_)=>{failure=Some("login-output-unverified");let mut s=state.lock().unwrap();s.phase=LoginPhase::Unknown;s.reason=Some("login-output-unverified");},
            },
            Event::Closed(stream,ok)=>{closed[stream]=true;if !ok {failure=Some("login-output-unreadable");let mut s=state.lock().unwrap();s.phase=LoginPhase::Unknown;s.reason=Some("login-output-unreadable");}},
            Event::Exited(result)=>{let mut s=state.lock().unwrap();s.exited=result.is_ok();if let Ok(exit)=&result {s.exit=*exit;}drop(s);exited=Some(result);let _=input_send.send(Input::Close);},
            Event::Written(code,ok)=>{if !ok {failure=Some("login-input-outcome-unknown");let mut s=state.lock().unwrap();s.phase=LoginPhase::Unknown;s.reason=Some("login-input-outcome-unknown");}else if code && failure.is_none() && !cancelling && !completed && exited.is_none() {state.lock().unwrap().phase=LoginPhase::CodeSubmitted;}},
            Event::Code(code,answer)=>{
                let result=(|| {guard.0.check(&context.home)?;if code_sent || completed || cancelling || failure.is_some() || exited.is_some() {return Err("login-code-not-accepted".into());}
                    let bytes=conversation.code(&code)?;code_sent=true;state.lock().unwrap().phase=LoginPhase::CodeQueued;
                    input_send.send(Input::Write(bytes,true)).map_err(|_|"login-input-outcome-unknown".into())})();let _=answer.send(result);
            },
            Event::Cancel(answer)=>{
                let result=(|| {if cancelling {return Ok(());}guard.0.check(&context.home)?;
                    if completed || exited.is_some() {return Err("login-already-completed".into());}
                    let now=(protocol.credentials)(&context);let changed=changed(&before,&now);
                    {let mut s=state.lock().unwrap();s.credentials_changed=changed;s.restart_required=changed;}
                    if changed!=Some(false) {return Err(if changed==Some(true) {"login-credentials-changed"}else{"login-credentials-unverified"}.into());}
                    if exited.is_none() {let native=native.as_ref().ok_or("login-process-unverified")?;crate::process::terminate(native).map_err(|_|"login-stop-unverified")?;}
                    cancelling=true;state.lock().unwrap().phase=LoginPhase::CancelRequested;let _=input_send.send(Input::Close);Ok(())
                })();if let Some(answer)=answer {let _=answer.send(result);}
            },
        }
    }
    let _=input_send.send(Input::Close);
    for thread in [writing,output,error,waiting] {thread.join().map_err(|_|"login-transport-panicked")?;}
    guard.0.check(&context.home).map_err(|_|"login-home-changed")?;
    let exit=exited.unwrap().map_err(|_|"login-wait-unknown")?;
    conversation.cleanup().map_err(|_|"login-helper-cleanup-unknown")?;
    let unchanged_home=guard.0.check(&context.home).is_ok();
    let after=if unchanged_home {(protocol.credentials)(&context)}else{Err("login-home-changed".into())};
    let change=changed(&before,&after);
    let verified=unchanged_home && failure.is_none() && (!cancelling || completed) && conversation.completed(exit) && (protocol.verify)(&context).state=="ok";
    let cancelled=cancelling && change==Some(false) && !completed && failure.is_none();
    guard.0.finish()?;
    drop(conversation);drop(guard);
    let mut s=state.lock().unwrap();s.finished=true;s.exit=exit;s.credentials_changed=change;s.restart_required=if change==Some(true) {Some(true)}else if verified || cancelled {change}else{None};
    s.prompt=None;
    s.phase=if verified {LoginPhase::Authenticated}else if cancelled {LoginPhase::Cancelled}else if cancelling {LoginPhase::Unknown}else if failure.is_some() || exit.is_some_and(|n|n!=0) {LoginPhase::Failed}else{LoginPhase::Unknown};
    s.reason=if verified || cancelled {None}else if cancelling {Some("login-cancel-outcome-unknown")}else{Some(failure.unwrap_or(if !unchanged_home {"login-home-changed"}else{"login-status-unverified"}))};Ok(())
    })
}
fn changed(before:&Res<Vec<Option<String>>>,after:&Res<Vec<Option<String>>>)->Option<bool> {match (before,after) {(Ok(a),Ok(b)) if a.len()==b.len() && !a.iter().zip(b).any(|(old,new)|old.is_some() && new.is_none())=>Some(a!=b),_=>None}}
fn read(mut pipe:impl Read,stream:usize,events:mpsc::Sender<Event>) {
    let mut bytes=[0;8192];loop {match pipe.read(&mut bytes) {Ok(0)=>{let _=events.send(Event::Closed(stream,true));break;},Ok(n)=>{if events.send(Event::Bytes(stream,bytes[..n].to_vec())).is_err() {break;}},Err(_)=>{let _=events.send(Event::Closed(stream,false));break;}}}
}
pub(super) fn file_fingerprint(path:&Path)->Res<Option<String>> {match std::fs::read(path) {Ok(bytes)=>Ok(Some(crate::sha256_hex(&bytes))),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),Err(_)=>Err("login-credential-unreadable".into())}}

// Separate native streams: never assemble half a URL or code from another pipe.
#[derive(Default)]
pub(super) struct Text([Vec<u8>;2]);
impl Text {
    pub fn append(&mut self,stream:usize,bytes:&[u8])->String {
        self.0[stream].extend_from_slice(bytes);let mut plain=String::new();let mut escape=0;
        for c in String::from_utf8_lossy(&self.0[stream]).chars() {
            match escape {0=>if c=='\u{1b}' {escape=1;}else{plain.push(c);},1=>{escape=if c=='[' {2}else{0};},_=>if ('@'..='~').contains(&c) {escape=0;}}
        }plain
    }
}
pub(super) fn tokens(text:&str)->impl Iterator<Item=&str> {text.split_inclusive(char::is_whitespace).filter(|s|s.ends_with(char::is_whitespace)).map(str::trim).filter(|s|!s.is_empty())}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn default_native_home_is_not_pinned_and_secrets_leave_no_debug_values() {
        let context=Context {home:std::env::temp_dir(),explicit:false,callback_authority:None,program:"native-client".into(),env:vec![],home_var:"CLAUDE_CONFIG_DIR",pin:None};
        assert!(!context.command().get_envs().any(|(key,_)|key=="CLAUDE_CONFIG_DIR"));
        let prompt=LoginPrompt {url:"https://auth.openai.com/oauth/authorize?state=private".into(),user_code:Some("secret-device-code".into()),needs_code:false};
        let debug=format!("{prompt:?}");assert!(!debug.contains("private"));assert!(!debug.contains("secret-device-code"));
    }
    #[test] fn disappeared_credential_is_unknown_and_streams_never_join() {
        assert_eq!(changed(&Ok(vec![Some("old".into())]),&Ok(vec![None])),None);
        let mut text=Text::default();text.append(0,b"https://claude.");assert!(!text.append(1,b"ai/oauth/authorize?code=true\n").contains("https://claude.ai"));
        assert!(tokens("https://auth.openai.com/oauth/authorize?incomplete").next().is_none());
        assert_eq!(text.append(0,b"ai\x1b[0m\n"),"https://claude.ai\n");
    }
}

fn env_key(key:&OsStr,name:&str)->bool {
    #[cfg(windows)] {
        use std::os::windows::ffi::OsStrExt;
        #[link(name="kernel32")] unsafe extern "system" {fn CompareStringOrdinal(a:*const u16,an:i32,b:*const u16,bn:i32,ignore_case:i32)->i32;}
        let a:Vec<_>=key.encode_wide().collect();let b:Vec<_>=name.encode_utf16().collect();
        let (Ok(an),Ok(bn))=(i32::try_from(a.len()),i32::try_from(b.len()))else{return false};
        unsafe {CompareStringOrdinal(a.as_ptr(),an,b.as_ptr(),bn,1)==2}
    }
    #[cfg(not(windows))] {key==name}
}

// A Windows command shim has a separate native child; its PID cannot authorize native cancellation.
pub(super) fn direct_executable(program:PathBuf)->Res<PathBuf> {
    #[cfg(windows)] if program.extension().and_then(OsStr::to_str).is_some_and(|e|["cmd","bat","ps1"].iter().any(|suffix|e.eq_ignore_ascii_case(suffix))) {return Err("login-native-executable-required".into());}
    Ok(program)
}
