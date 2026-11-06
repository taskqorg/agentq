//! M12.8: independent protected native host; existing operator authority and caller-owned permits only.
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use std::{fs,io::Read,path::{Path,PathBuf}};
type Res<T> = Result<T,String>;
const SCHEMA:&str="agentq.app-server/1";
#[path="host/store.rs"]
mod resources;
#[path="host/permits.rs"]
mod permits;
#[cfg(any(test,target_os="macos"))]
#[path="host/environment.rs"]
mod environment;
#[cfg(target_os="macos")]
use environment::{environment,plist};
#[derive(Clone,Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
struct Request {schema:String,permit:Value,policy_revision:u64,account:String,#[serde(default)] preparation:String}
impl Request {
    fn parse(text:&str)->Res<Self> {
        if text.len()>16384 {return Err("protected request exceeds bound".into());}
        let r:Self=serde_json::from_str(text).map_err(|_|"protected request invalid")?;
        if r.schema!=SCHEMA||r.policy_revision==0||r.account.is_empty()||r.permit["binding"]["runtime"]!="codex" {return Err("protected request identity invalid".into());}Ok(r)
    }
    fn root(&self)->Res<PathBuf> {
        let root=PathBuf::from(self.permit["binding"]["root"].as_str().ok_or("project unknown")?);
        if !root.is_absolute()||canonical(&root)?!=root {return Err("canonical project required".into());}Ok(root)
    }
}
/// Execute the independent host protocol; native containment remains macOS-only.
pub fn dispatch(action:&str,text:&str)->Res<()> {
    #[cfg(target_os="macos")] { if matches!(action,"retained"|"retained-cancel") {return mac::retained(action,text);} mac::dispatch(action,text) }
    #[cfg(not(target_os="macos"))] {let _=(action,text);Err("protected native launch requires qualified macOS containment".into())}
}
/// Verify a domain-separated Ed25519 signature without an OS certificate authority.
pub fn verify_signature(key:&str,signature:&str,domain:&str,payload:&[u8])->Res<()> {
    let key=hex(key,32)?;let signature=hex(signature,64)?;
    let mut message=domain.as_bytes().to_vec();message.push(0);message.extend_from_slice(payload);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519,key).verify(&message,&signature)
        .map_err(|_|"protected signature invalid; authority unknown".into())
}
pub(super) fn hex(text:&str,bytes:usize)->Res<Vec<u8>> {
    if text.len()!=bytes*2||!text.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {return Err("protected signature encoding invalid".into());}
    (0..text.len()).step_by(2).map(|i|u8::from_str_radix(&text[i..i+2],16).map_err(|_|"protected signature encoding invalid".into())).collect()
}
fn random_id()->Res<String> {
    use ring::rand::SecureRandom;
    let mut bytes=[0;32];ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_|"host generation unavailable")?;
    Ok(bytes.iter().map(|v|format!("{v:02x}")).collect())
}
fn read(path:&Path)->Res<Value> {
    let f=fs::File::open(path).map_err(|_|"protected state unreadable")?;
    let mut bytes=Vec::new();f.take(16*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"protected state read unknown")?;
    if bytes.len()>16*1024*1024 {return Err("protected state exceeds bound".into());}
    serde_json::from_slice(&bytes).map_err(|_|"protected state corrupt".into())
}
fn verify_artifact(install:&Path)->Res<Value> {
    let trust=read(&install.join("trust.json"))?;
    let digest=file_digest(&std::env::current_exe().map_err(|_|"adapter identity unknown")?)?;
    if trust["schema"]!="agentq.protected-adapter/1"||trust["sha256"]!=digest {return Err("adapter not pinned by host operator".into());}
    verify_signature(trust["public_key"].as_str().ok_or("operator key unknown")?,trust["signature"].as_str().ok_or("adapter signature unknown")?,"agentq.protected-adapter/1",digest.as_bytes())?;Ok(trust)
}
fn envelope(install:&Path,path:&Path,domain:&str)->Res<Value> {
    let trust=verify_artifact(install)?;let doc=read(path)?;
    let payload=doc["payload"].as_str().ok_or("signed policy payload absent")?;
    verify_signature(trust["public_key"].as_str().ok_or("operator key unknown")?,doc["signature"].as_str().ok_or("operator signature unknown")?,domain,payload.as_bytes())?;
    serde_json::from_str(payload).map_err(|_|"signed policy payload invalid".into())
}
fn policy_path(install:&Path,root:&Path)->PathBuf {install.join("policies").join(format!("{}.json",digest(root.as_os_str().as_encoded_bytes())))}
fn policy(install:&Path,root:&Path)->Res<Value> {
    let p=envelope(install,&policy_path(install,root),"agentq.protected-policy/1")?;
    if p["schema"]!="agentq.protected-policy/1"||p["authority"]["project"]!=json!(root) {return Err("existing policy identity mismatched".into());}Ok(p)
}
fn canonical(p:&Path)->Res<PathBuf> {p.canonicalize().map_err(|_|"protected launch path identity unknown".into())}
pub(super) fn digest(bytes:&[u8])->String {ring::digest::digest(&ring::digest::SHA256,bytes).as_ref().iter().map(|b|format!("{b:02x}")).collect()}
fn operation_id(r:&Request)->Res<String> {
    let b=&r.permit["binding"];
    Ok(format!("op-{}",digest(format!("{}\0{}",b["caller"].as_str().ok_or("permit caller unknown")?,b["op_id"].as_str().ok_or("permit operation unknown")?).as_bytes())))
}
fn scope_directory(install:&Path,r:&Request)->Res<PathBuf> {
    // A short socket name under the existing protected install; collisions refuse creation, never replace a scope.
    Ok(install.join("native-scopes").join(&operation_id(r)?[3..35]))
}
fn preparation(r:&Request,authority:&Value,instance:&Value)->Value {
    json!({"schema":"agentq.app-server-preparation/1","binding":r.permit["binding"],"authority":authority,"host_instance":instance})
}
fn prepared(s:&resources::State,key:&str,r:&Request,authority:&Value)->Res<()> {
    let op=s.ops.get(key).ok_or("permit absent; preparation unknown")?;
    let expected=preparation(r,authority,&op.receipt["instance"]);
    if r.preparation.is_empty()||op.bound["local_permit"]["prepared"]!=expected
        ||r.preparation!=digest(expected.to_string().as_bytes()) {
        return Err("independent preparation missing, stale or changed; no native dispatch/input".into());
    }Ok(())
}

fn certified(launch:&Value)->Res<()> {
    let scope=&launch["scope"];let cert=&scope["certificate"];
    if launch["input_closed"]!=true||scope["dispatch_ack"]!=true
        ||cert["schema"]!="agentq.native-completion/1"||cert["scope"]!=scope["binding"]
        ||cert["native"]!=launch["native"]||cert["supervisor"]!=launch["supervisor"]
        ||cert["counts"][0].as_u64().zip(cert["counts"][1].as_u64()).is_none_or(|(a,b)|a.checked_sub(b)!=Some(1))
        ||native(&cert["supervisor"]).is_err()||native(&cert["native"]).is_err()
        ||cert["supervisor"]==cert["native"]||cert["counts"].as_array().is_none_or(|v|v.len()!=2)
        ||scope["binding"]["coalition"].as_u64().is_none_or(|v|v==0)||scope["binding"]["boot"].as_i64().is_none_or(|v|v<=0) {
        return Err("protected descendant completion unproved; retain original permit".into());
    }Ok(())
}
/// M12.8: an authenticated consumer still cannot release unproved protected descendants.
fn release_ready(op:&resources::Op)->Res<()> {
    let launch=&op.bound["local_permit"]["launch"];
    if launch.is_null() {return Ok(());}
    certified(launch)?;
    if op.bound["local_permit"]["binding"]["boot"].as_i64().zip(launch["scope"]["binding"]["boot"].as_i64()).is_none_or(|(a,b)|a<=0||a.abs_diff(b)>2) {
        return Err("protected permit/scope boot binding unknown; retain original permit".into());
    }
    if launch["scope"]["cleanup_intent"]!=true||launch["scope"]["bootout_ack"]!=true||launch["scope"]["cleanup_verified"]!=true {
        return Err("protected scope cleanup unproved; retain original permit".into());
    }Ok(())
}
fn certify(s:&mut resources::State,key:&str,r:&Request,supervisor:&coreq::process::Identity,binding:&Value,counts:[u64;2])->Res<bool> {
    launch_record(s,key,r)?;
    let launch=&mut s.ops.get_mut(key).unwrap().bound["local_permit"]["launch"];
    let mine=json!({"pid":supervisor.pid,"started":supervisor.started});
    if launch["supervisor"]!=mine||launch["scope"]["binding"]!=*binding||launch["input_closed"]!=true
        ||launch["scope"]["dispatch_ack"]!=true||counts[0].checked_sub(counts[1]).is_none() {
        return Err("scope identity/input seal unknown; completion refused".into());
    }
    let old=&launch["scope"]["last_counts"];
    if !old.is_null()&&(old[0].as_u64().is_none_or(|n|n>counts[0])||old[1].as_u64().is_none_or(|n|n>counts[1])) {
        return Err("scope counters regressed; completion unknown".into());
    }
    if !launch["scope"]["certificate"].is_null() {
        if launch["scope"]["certificate"]["counts"]!=json!(counts) {return Err("sealed scope changed after certification; retain original receipt".into());}
        return Ok(true);
    }
    if counts[0]-counts[1]==1 {native(&launch["native"])?;}
    launch["scope"]["last_counts"]=json!(counts);
    if counts[0]-counts[1]!=1 {return Ok(false);}
    launch["scope"]["certificate"]=json!({"schema":"agentq.native-completion/1","scope":binding,"supervisor":mine,"native":launch["native"],"counts":counts});
    Ok(true)
}
fn file_digest(path:&Path)->Res<String> {
    let mut file=fs::File::open(path).map_err(|_|"protected executable unreadable")?;
    let mut hash=ring::digest::Context::new(&ring::digest::SHA256);let mut bytes=[0;65536];
    loop {let n=file.read(&mut bytes).map_err(|_|"protected executable read unknown")?;if n==0 {break;}hash.update(&bytes[..n]);}
    Ok(hash.finish().as_ref().iter().map(|b|format!("{b:02x}")).collect())
}
fn stamp(path:&Path)->Res<Value> {
    let m=fs::metadata(path).map_err(|_|"protected executable identity unknown")?;
    if !m.is_file() {return Err("protected executable is not a file".into());}
    let modified=m.modified().map_err(|_|"protected executable clock unknown")?;
    #[cfg(unix)] {use std::os::unix::fs::MetadataExt;return Ok(json!([m.dev(),m.ino(),m.ctime(),m.ctime_nsec(),m.len(),format!("{modified:?}")]));}
    #[cfg(not(unix))] {Ok(json!([m.len(),format!("{modified:?}"),format!("{:?}",m.created().map_err(|_|"protected executable creation unknown")?)]))}
}

struct Plan {authority:Value,access:String,args:Vec<String>,root:PathBuf,home:PathBuf,bin:PathBuf,queries:Vec<SandboxQuery>}
fn query_paths(install:&Path,p:&Plan)->[(&'static str,PathBuf,bool);6] {
    [("workspace-probe",p.root.clone(),false),
        ("policy-store",policy_path(install,&p.root),true),("host-trust",install.join("trust.json"),true),
        ("native-executable",p.bin.clone(),true),("account-config",Path::new(p.authority["account_home"].as_str().unwrap()).join("config.toml"),true),
        ("outside-workspace",p.home.clone(),true)]
}
#[derive(serde::Serialize)]
struct SandboxQuery {category:&'static str,path:PathBuf,denied:bool,identity:Value}
fn query_identity(path:&Path)->Res<Value> {
    let real=canonical(path)?;let m=fs::metadata(path).map_err(|_|"sandbox path identity unknown")?;
    if !m.is_file()&&!m.is_dir() {return Err("sandbox path is not a file or directory".into());}
    #[cfg(unix)] let object={use std::os::unix::fs::MetadataExt;json!([m.dev(),m.ino()])};
    // Windows is refused by resolve; this branch only supports portable fixtures.
    #[cfg(not(unix))] let object=json!(format!("{:?}",m.created().map_err(|_|"sandbox path creation unknown")?));
    Ok(json!({"canonical":real,"directory":m.is_dir(),"object":object,"file_stamp":if m.is_file(){stamp(path)?}else{Value::Null}}))
}
impl SandboxQuery {
    fn path_error(&self,operation:&str)->String {
        format!("native sandbox path readback: operation={operation} category={} observed=unknown-or-changed; input refused",self.category)
    }
    fn unchanged(&self,operation:&str)->Res<()> {
        if query_identity(&self.path).ok().as_ref()!=Some(&self.identity) {return Err(self.path_error(operation));}Ok(())
    }
    #[cfg(any(test,target_os="macos"))]
    fn check(&self,operation:&str,query:impl FnOnce(&Path)->Res<(i32,Option<i32>)>)->Res<()> {
        self.unchanged(operation)?;
        let (result,errno)=query(&self.path)?;
        self.unchanged(operation)?;
        sandbox_result(operation,self.category,result,errno,self.denied)
    }
}
// Only an operator-authenticated snapshot can carry existing grant/profile/account authority.
fn resolve(install:&Path,home:&Path,r:&Request,os:&str,bin:&Path)->Res<Plan> {
    if os!="macos" {return Err("protected native launch requires qualified macOS containment".into());}
    let root=r.root()?;let install=canonical(install)?;let home=canonical(home)?;let bin=canonical(bin)?;
    let policy=policy(install.as_path(),&root)?;let a=&policy["authority"];let grant=&a["saved"]["grant"];
    if a["project"]!=json!(root)||a["runtime"]!="codex"||a["account"]!=r.account||a["saved"]["revision"]!=r.policy_revision
        ||r.policy_revision==0||grant["mode"]!="workspace"||grant["runtime"]!="codex"||grant["project"]!=json!(root)
        ||grant["by"].as_str().is_none_or(|v|v.trim().is_empty()||v=="unknown")||grant["at"].as_u64().is_none_or(|v|v==0)
        ||a["executable"]!=json!(bin)||a["executable_sha256"]!=file_digest(&bin)?
        ||a["resolved"]["effective"]!="workspace"||!a["requested"].is_string() {
        return Err("existing Workspace authority missing, changed or mismatched".into());
    }
    let account=canonical(Path::new(a["account_home"].as_str().ok_or("account home unknown")?))?;
    if !account.starts_with(home.join(".codex")) {return Err("account outside existing native homes".into());}
    let mut profile=policy["profile"].as_str().filter(|v|!v.is_empty()).ok_or("existing profile unknown")?.to_string();
    if a["profile_sha256"]!=digest(profile.as_bytes()) {return Err("existing profile digest mismatched".into());}
    let adapter=canonical(&std::env::current_exe().map_err(|_|"adapter image unknown")?)?;
    for path in [&install,adapter.parent().ok_or("adapter parent unknown")?,bin.parent().ok_or("native parent unknown")?] {
        if root.starts_with(path) {return Err("protected control directory overlaps writable project".into());}
        profile.push_str(&format!("(deny file-write* (subpath {}))\n",json!(path)));
    }
    let control=scope_directory(&install,r)?;
    profile.push_str(&format!("(deny file* (subpath {}))\n",json!(control)));
    let args:Vec<String>=policy["args"].as_array().ok_or("native arguments unknown")?.iter()
        .map(|v|v.as_str().map(String::from).ok_or("native arguments invalid".into())).collect::<Res<_>>()?;
    if args.len()%2!=0||args.chunks(2).any(|v|v[0]!="-c"&&v[0]!="--config")
        ||!args.windows(2).any(|v|v==["-c","sandbox_mode=\"danger-full-access\""])
        ||!args.windows(2).any(|v|v==["-c","approval_policy=\"never\""]) {
        return Err("existing native configuration unknown; no fallback".into());
    }
    let mut authority=a.clone();authority["profile_sha256"]=json!(digest(profile.as_bytes()));
    authority["adapter"]=json!({"executable":adapter,"identity":stamp(&adapter)?,"sha256":file_digest(&adapter)?});
    authority["executable_identity"]=stamp(&bin)?;authority["scope_directory"]=json!(control);
    authority["policy_sha256"]=json!(digest(policy.to_string().as_bytes()));
    let mut plan=Plan{authority,access:profile,args,root,home,bin,queries:vec![]};
    for (category,path,denied) in query_paths(&install,&plan) {
        let mut q=SandboxQuery{category,path,denied,identity:Value::Null};
        q.identity=query_identity(&q.path).map_err(|_|q.path_error("bind"))?;
        if q.identity["directory"]!=json!(category=="workspace-probe"||category=="outside-workspace") {return Err(q.path_error("bind"));}
        plan.queries.push(q);
    }
    plan.authority["sandbox_paths"]=json!(plan.queries);Ok(plan)
}

fn launch_record<'a>(s:&'a resources::State,key:&str,r:&Request)->Res<&'a Value> {
    let launch=&s.ops.get(key).ok_or("permit absent; execution unknown")?.bound["local_permit"]["launch"];
    if launch["schema"]!=SCHEMA||launch["policy_revision"]!=r.policy_revision||launch["account"]!=r.account {
        return Err("launch identity absent, corrupt or mismatched; never retry launch".into());
    }Ok(launch)
}
fn consume(s:&mut resources::State,key:&str,r:&Request,authority:&Value,supervisor:&coreq::process::Identity)->Res<()> {
    prepared(s,key,r,authority)?;
    let op=s.ops.get_mut(key).ok_or("permit absent; no launch")?;
    if !op.bound["local_permit"]["launch"].is_null() {return Err("permit launch already consumed; observe the same operation, never replay".into());}
    op.bound["local_permit"]["launch"]=json!({"schema":SCHEMA,"policy_revision":r.policy_revision,"account":r.account,
        "authority":authority,"supervisor":{"pid":supervisor.pid,"started":supervisor.started},"state":"unknown",
        "host_instance":op.receipt["instance"],"cancel_requested":false,"input_closed":false,
        "reason":"launch intent consumed; native execution not yet verified"});
    Ok(())
}
fn input_gate(s:&resources::State,key:&str,r:&Request)->Res<()> {
    let launch=launch_record(s,key,r)?;
    if launch["cancel_requested"]!=false||launch["input_closed"]!=false||launch["host_instance"].as_str().filter(|v|!v.is_empty()).is_none()
        ||launch["host_instance"]!=s.ops[key].receipt["instance"] {
        return Err("launch cancelled or host generation changed; input refused, retain permit".into());
    }Ok(())
}
fn bound_gate(s:&resources::State,key:&str,r:&Request,authority:&Value,id:&coreq::process::Identity,supervisor:&coreq::process::Identity,effective:Option<&Value>)->Res<()> {
    input_gate(s,key,r)?;let record=launch_record(s,key,r)?;
    if record["authority"]!=*authority||native(&record["native"])?!=*id||native(&record["supervisor"])?!=*supervisor
        ||effective.is_some_and(|e|record["state"]!="verified-effective"||record["verified_effective"]!=*e) {
        return Err("durable launch binding changed or uncommitted; input refused".into());
    }Ok(())
}
fn cancel(s:&mut resources::State,key:&str,r:&Request)->Res<Value> {
    launch_record(s,key,r)?;
    let launch=&mut s.ops.get_mut(key).unwrap().bound["local_permit"]["launch"];
    launch["cancel_requested"]=json!(true);Ok(launch.clone())
}
fn native(value:&Value)->Res<coreq::process::Identity> {
    Ok(coreq::process::Identity{pid:value["pid"].as_u64().and_then(|v|u32::try_from(v).ok()).filter(|v|*v>0&&*v<=i32::MAX as u32).ok_or("native identity unknown")?,
        started:value["started"].as_str().filter(|s|!s.is_empty()).ok_or("native identity unknown")?.into()})
}
fn exact(expected:&coreq::process::Identity,seen:Res<Option<coreq::process::Identity>>)->Res<()> {
    if seen?.as_ref()!=Some(expected) {return Err("native incarnation absent or changed; retain permit and reconcile".into());}Ok(())
}

#[cfg(any(test,target_os="macos"))]
fn sandbox_result(operation:&str,category:&str,result:i32,errno:Option<i32>,denied:bool)->Res<()> {
    if result<0||(result!=0)!=denied {
        let expected=if denied {"denied"}else{"allowed"};
        let observed=if result<0 {"unknown"}else if result==0 {"allowed"}else{"denied"};
        let errno=errno.map_or("none".into(),|v|v.to_string());
        return Err(format!("native sandbox readback: operation={operation} category={category} expected={expected} observed={observed} return={result} errno={errno}; input refused"));
    }Ok(())
}

#[cfg(any(target_os="macos",all(test,unix)))]
fn native_output(mut socket:std::os::unix::net::UnixStream)->Res<(std::os::fd::OwnedFd,std::thread::JoinHandle<std::io::Result<u64>>)> {
    let (mut reader,writer)=std::io::pipe().map_err(|_|"native output pipe unavailable; retain scope")?;
    // Native code must never inherit the duplex socket that carries input before validation.
    let pump=std::thread::Builder::new().spawn(move||std::io::copy(&mut reader,&mut socket))
        .map_err(|_|"native output forwarding unavailable; retain scope")?;
    Ok((writer.into(),pump))
}

#[cfg(target_os="macos")]
#[path="host/scope.rs"]
mod scope;

#[cfg(target_os="macos")]
mod mac {
    use super::*;
    use super::permits as external_permits;
    use std::{ffi::{CStr,CString},io::{self,Write},os::{fd::AsRawFd,unix::ffi::OsStrExt},process::Stdio,time::{Duration,Instant}};

    // Sandbox SPI declarations match WebKit's Source/WTF/wtf/spi/darwin/SandboxSPI.h.
    #[link(name="sandbox")]
    unsafe extern "C" {fn sandbox_check(pid:i32,operation:*const libc::c_char,filter:i32,...)->i32;static SANDBOX_CHECK_NO_REPORT:i32;}

    pub(super) fn retained(action:&str,text:&str)->Res<()> {
        let (install,_home)=host_paths()?;
        let input:Value=serde_json::from_str(text).map_err(|_|"retained request invalid")?;
        let (operation,original)=retained_original(&install,&input)?;
        let cleanup=if action=="retained-cancel" {
            match retained_incarnation(&original) {
                Ok(incarnation)=>retained_cancel(&install,&operation,&incarnation,||{
                    let child=native(&original["native"])?;let supervisor=native(&original["supervisor"])?;
                    child_of(&child,&supervisor)?;scope::membership(&child,&original["scope"]["binding"])?;
                    let program=Path::new(original["authority"]["executable"].as_str().ok_or("retained native image unknown")?);
                    image(&child,program)?;
                    if file_digest(program)?!=original["authority"]["executable_sha256"] {return Err("retained native image changed".into());}
                    Ok(())
                },||coreq::process::terminate(&native(&original["native"])?).map_err(|_|"retained native stop unknown".into())),
                Err(reason)=>json!({"state":"unknown","reason":reason}),
            }
        }else{scope::cleanup_retained(&install,&original,&operation)?};
        println!("{}",json!({"schema":"agentq.retained-operation/1","checkpoint":input["checkpoint"],"receipt":input["receipt"],
            "state":if cleanup["cleanup_verified"]==true {"completed"}else{"unknown"},"cleanup":cleanup}));Ok(())
    }
    pub(super) fn host_paths()->Res<(PathBuf,PathBuf)> {
        let mut pwd=std::mem::MaybeUninit::<libc::passwd>::uninit();let mut result=std::ptr::null_mut();let mut buf=vec![0u8;65536];
        if unsafe{libc::getpwuid_r(libc::geteuid(),pwd.as_mut_ptr(),buf.as_mut_ptr().cast(),buf.len(),&mut result)}!=0||result.is_null() {return Err("host account unknown".into());}
        let home=canonical(Path::new(std::ffi::OsStr::from_bytes(unsafe{CStr::from_ptr((*result).pw_dir)}.to_bytes())))?;
        let install=canonical(&home.join(".codex/agentq-host"))?;
        verify_artifact(&install)?;
        Ok((install,home))
    }
    pub(super) fn executable(install:&Path,r:&Request)->Res<PathBuf> {
        let p=policy(install,&r.root()?)?;
        canonical(Path::new(p["authority"]["executable"].as_str().ok_or("native image unknown")?))
    }
    pub(super) fn live(id:&coreq::process::Identity)->Res<()> {exact(id,coreq::process::identity(id.pid).map_err(|_|"native liveness unknown".into()))}
    fn child_of(id:&coreq::process::Identity,supervisor:&coreq::process::Identity)->Res<()> {
        live(id)?;live(supervisor)?;
        let mut info=std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();let size=std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let n=unsafe{libc::proc_pidinfo(id.pid as i32,libc::PROC_PIDTBSDINFO,0,info.as_mut_ptr().cast(),size)};
        if n!=size||unsafe{info.assume_init()}.pbi_ppid!=supervisor.pid {return Err("native child ownership unknown or mismatched; permit retained".into());}
        live(id)?;live(supervisor)
    }
    pub(super) fn image(id:&coreq::process::Identity,path:&Path)->Res<()> {
        live(id)?;let mut buf=[0u8;4096];
        let n=unsafe{libc::proc_pidpath(id.pid as i32,buf.as_mut_ptr().cast(),buf.len() as u32)};
        if n<=0 {return Err("native executable readback unknown".into());}
        let end=buf.iter().position(|b|*b==0).ok_or("native executable path incomplete")?;
        if canonical(Path::new(std::ffi::OsStr::from_bytes(&buf[..end])))?!=path {return Err("native executable has not matched the selected image".into());}
        live(id)
    }
    fn check(pid:u32,operation:&str,q:&SandboxQuery)->Res<()> {
        q.check(operation,|path| {
            let op=CString::new(operation).map_err(|_|"invalid sandbox operation")?;
            let path=CString::new(path.as_os_str().as_bytes()).map_err(|_|"invalid sandbox path")?;
            let result=unsafe{*libc::__error()=0;sandbox_check(pid as i32,op.as_ptr(),1|SANDBOX_CHECK_NO_REPORT,path.as_ptr())};
            let errno=if result<0 {io::Error::last_os_error().raw_os_error()}else{None};
            Ok((result,errno))
        })
    }
    fn verified(id:&coreq::process::Identity,p:&Plan,scope:&Value)->Res<Value> {
        image(id,&p.bin)?;
        for q in &p.queries {for &op in if q.denied {&["file-write-data","file-write-create","file-write-unlink"][..]}else{&["file-write-data"][..]} {check(id.pid,op,q)?;}}
        let control=SandboxQuery{category:"scope-control",path:PathBuf::from(p.authority["scope_directory"].as_str().ok_or("scope control unknown")?),denied:true,identity:scope["directory_identity"].clone()};
        for op in ["file-read-data","file-write-data","file-write-create","file-write-unlink"] {check(id.pid,op,&control)?;}
        scope::membership(id,&scope["binding"])?;
        image(id,&p.bin)?;
        let protected:Vec<_>=p.queries.iter().filter(|q|q.denied).map(|q|&q.path).collect();
        Ok(json!({"state":"verified-effective","scope":"live native image and named effective sandbox queries; native workload qualification separate",
            "workspace_write":true,"protected_write_create_unlink_denied":protected,"reads":"existing profile","network":"existing profile"}))
    }
    pub(super) fn refresh(install:&Path,home:&Path,r:&Request,original:&Value)->Res<Plan> {
        let bin=executable(install,r)?;let p=resolve(install,home,r,"macos",&bin)?;
        if p.authority!=*original {return Err("bound project/profile/account/executable authority changed; native input refused".into());}Ok(p)
    }
    fn key(install:&Path,r:&Request)->Res<String> {
        resources::within(install,resources::WAIT,|s|external_permits::reserved(install,s,&r.permit,"codex"))
    }
    pub(super) fn dispatch(action:&str,text:&str)->Res<()> {
        if action=="scope-run" {return scope::run(text);}
        if action=="prepare" {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Prepare {schema:String,permit:Value}
            if text.len()>16384 {return Err("preparation request exceeds 16384 bytes".into());}
            let input:Prepare=serde_json::from_str(text).map_err(|_|"invalid preparation request")?;
            if input.schema!=SCHEMA {return Err("unsupported preparation schema".into());}
            let (install,home)=host_paths()?;
            let result=resources::within(&install,resources::WAIT,|s| {
                let account=policy(&install,Path::new(input.permit["binding"]["root"].as_str().ok_or("project unknown")?))?["authority"]["account"].as_str().ok_or("account unknown")?.to_string();
                let r=Request{schema:input.schema.clone(),permit:input.permit.clone(),policy_revision:policy(&install,Path::new(input.permit["binding"]["root"].as_str().ok_or("project unknown")?))?["authority"]["saved"]["revision"].as_u64().ok_or("revision unknown")?,account,preparation:String::new()};
                let key=external_permits::prepare(&install,s,&r.permit)?;
                let p=resolve(&install,&home,&r,"macos",&executable(&install,&r)?)?;
                let op=s.ops.get_mut(&key).ok_or("permit absent")?;
                if !op.bound["local_permit"]["launch"].is_null() {return Err("launch already consumed; never derive expected authority from a launch receipt".into());}
                let value=preparation(&r,&p.authority,&op.receipt["instance"]);
                let old=&op.bound["local_permit"]["prepared"];
                if !old.is_null()&&old!=&value {return Err("original preparation changed; retain operation, no native launch".into());}
                op.bound["local_permit"]["prepared"]=value.clone();
                Ok(json!({"schema":SCHEMA,"state":"prepared","preparation":digest(value.to_string().as_bytes()),"policy_revision":r.policy_revision,"account":r.account,"prepared":value}))
            })?;println!("{result}");return Ok(());
        }
        command(action,&Request::parse(text)?)
    }
    pub(super) fn command(action:&str,r:&Request)->Res<()> {
        if !["launch","observe","cancel","reconcile"].contains(&action) {return Err("app-server action must be prepare, launch, observe, cancel or reconcile".into());}
        let (install,home)=host_paths()?;
        // Existing permit authentication, including exact payload/key, occurs before any native action.
        let observed=external_permits::exchange(&install,"observe",&r.permit)?;
        let permit_key=operation_id(r)?;let _=observed;
        if action=="launch" {return scope::launch(&install,&home,r,&permit_key);}
        if action=="reconcile" {let result=scope::reconcile(&install,r,&permit_key)?;println!("{result}");return Ok(());}
        let (record,completed)=resources::within(&install,resources::WAIT,|s| {
            let record=if action=="cancel" {cancel(s,&permit_key,r)?}else{launch_record(s,&permit_key,r)?.clone()};
            Ok((record,release_ready(s.ops.get(&permit_key).ok_or("permit absent")?).is_ok()))
        })?;
        let mut receipt=json!({"schema":SCHEMA,"operation":r.permit["binding"]["op_id"],"caller":r.permit["binding"]["caller"],"launch":record,"state":"unknown"});
        if completed {receipt["state"]=json!("completed");println!("{receipt}");return Ok(());}
        let id=match native(&record["native"]) {
            Ok(id)=>id,Err(reason)=>{receipt["reason"]=json!(if action=="cancel" {"cancellation saved before native identity; input remains closed".into()}else{reason});println!("{receipt}");return Ok(());}
        };
        if action=="cancel" {
            // Revocation closes input, but cannot remove the original owner's exact-process stop authority.
            child_of(&id,&native(&record["supervisor"])?)?;
            image(&id,Path::new(record["authority"]["executable"].as_str().ok_or("bound executable unknown")?))?;
            coreq::process::terminate(&id).map_err(|_|"native cancellation unconfirmed; retain permit and reconcile")?;
            receipt["reason"]=json!("exact native termination requested; turn/descendant completion unproved; permit retained");
        } else {
            let result=(||->Res<()> {
                key(&install,r)?;let p=refresh(&install,&home,r,&record["authority"])?;
                let supervisor=native(&record["supervisor"])?;live(&supervisor)?;
                child_of(&id,&supervisor)?;
                let effective=verified(&id,&p,&record["scope"])?;
                resources::within(&install,resources::WAIT,|s|bound_gate(s,&permit_key,r,&p.authority,&id,&supervisor,Some(&effective)))?;
                receipt["verified_effective"]=effective;
                if record["state"]!="verified-effective" {return Err("launch receipt uncommitted; execution unknown".into());}Ok(())})();
            match result {Ok(())=>receipt["state"]=json!("verified-effective"),Err(reason)=>receipt["reason"]=json!(reason)}
        }
        println!("{receipt}");Ok(())
    }
    pub(super) fn launch_native(install:&Path,home:&Path,r:&Request,key:&str,mut transport:std::io::BufReader<std::os::unix::net::UnixStream>,stdout:Stdio,stderr:Stdio)->Res<()> {
        let original=resources::within(install,resources::WAIT,|s|Ok(launch_record(s,key,r)?.clone()))?;
        let p=refresh(install,home,r,&original["authority"])?;
        let supervisor=coreq::process::identity(std::process::id()).map_err(|_|"supervisor identity unknown")?.ok_or("supervisor absent")?;
        resources::within(install,resources::WAIT,|s| {prepared(s,key,r,&p.authority)?;input_gate(s,key,r)})?;
        let mut cmd=std::process::Command::new("/usr/bin/sandbox-exec");
        cmd.args(["-p",&p.access]).arg(&p.bin);
        cmd.env_clear();for (name,value) in environment(home).as_object().ok_or("native environment unknown")? {
            cmd.env(name,value.as_str().ok_or("native environment invalid")?);
        }
        let mut spec=super::super::Spec{client:super::super::Client::Codex,root:p.root.clone(),session:"-".into(),resume:false,
            args:p.args.iter().map(Into::into).collect(),home:Some(PathBuf::from(p.authority["account_home"].as_str().ok_or("account home unknown")?)),env:vec![]};
        let intent=crate::optimization::Intent::parse(&std::env::var("AGENTQ_OPTIMIZATION").unwrap_or_else(|_|"inherit".into())).ok_or("optimization intent unknown")?;
        crate::optimization::prepare(intent,"protected-host",&mut spec,None);
        let mut cmd=super::super::app::app_server_command(cmd,&spec)?;
        cmd.stdin(Stdio::piped()).stdout(stdout).stderr(stderr);
        resources::within(install,resources::WAIT,|s|{external_permits::reserved(install,s,&r.permit,"codex")?;input_gate(s,key,r)})?;
        let mut child=cmd.spawn().map_err(|e|{eprintln!("agentq: `sandbox-exec`: {}",e.kind());"native spawn outcome unknown; intent retained, never replay"})?;
        let id=coreq::process::identity(child.id()).map_err(|_|"spawned native identity unknown; retain intent")?.ok_or("spawned native absent; retain intent")?;
        resources::within(install,resources::WAIT,|s| {
            launch_record(s,key,r)?;scope::membership(&id,&original["scope"]["binding"])?;
            let launch=&mut s.ops.get_mut(key).unwrap().bound["local_permit"]["launch"];
            launch["native"]=json!({"pid":id.pid,"started":id.started});Ok(())
        })?;
        let mut input=child.stdin.take().ok_or("native stdin unavailable; intent retained")?;
        let gate=(||->Res<()> {
            let until=Instant::now()+Duration::from_secs(5);
            loop {
                match image(&id,&p.bin) {Ok(())=>break,Err(e)=>{if Instant::now()>=until||child.try_wait().map_err(|_|"native liveness unknown")?.is_some() {return Err(e);}std::thread::sleep(Duration::from_millis(10));}}
            }
            resources::within(install,resources::WAIT,|s| {
                external_permits::reserved(install,s,&r.permit,"codex")?;refresh(install,home,r,&p.authority)?;
                bound_gate(s,key,r,&p.authority,&id,&supervisor,None)?;child_of(&id,&supervisor)?;
                let effective=verified(&id,&p,&original["scope"])?;
                let launch=&mut s.ops.get_mut(key).ok_or("permit disappeared")?.bound["local_permit"]["launch"];
                launch["verified_effective"]=effective;launch["state"]=json!("verified-effective");launch["reason"]=Value::Null;Ok(())
            })
        })();
        if let Err(e)=gate {drop(input);let _=coreq::process::terminate(&id);return Err(e);}
        let flags=unsafe{libc::fcntl(input.as_raw_fd(),libc::F_GETFL)};
        if flags<0||unsafe{libc::fcntl(input.as_raw_fd(),libc::F_SETFL,flags|libc::O_NONBLOCK)}<0 {return Err("bounded native input unavailable; intent retained".into());}
        transport.get_ref().set_read_timeout(Some(Duration::from_millis(250))).map_err(|_|"bounded scope input unavailable")?;
        loop {
            if let Some(status)=child.try_wait().map_err(|_|"native liveness unknown; permit retained")? {
                return if status.success(){Ok(())}else{Err("native app-server exited; turn outcome unproved, permit retained".into())};
            }
            resources::within(install,resources::WAIT,|s| {external_permits::reserved(install,s,&r.permit,"codex")?;input_gate(s,key,r)})?;
            let mut bytes=vec![0;65536];
            let count=match transport.read(&mut bytes) {
                Ok(n)=>n,Err(e) if matches!(e.kind(),io::ErrorKind::WouldBlock|io::ErrorKind::TimedOut|io::ErrorKind::Interrupted)=>continue,
                Err(_)=>return Err("consumer input interrupted; retain native scope".into()),
            };
            if count==0 {drop(input);return Err("consumer input ended; retain native scope until descendants finish".into());}
            bytes.truncate(count);
            resources::within(install,resources::WAIT,|s| {
                external_permits::reserved(install,s,&r.permit,"codex")?;refresh(install,home,r,&p.authority)?;let effective=verified(&id,&p,&original["scope"])?;
                bound_gate(s,key,r,&p.authority,&id,&supervisor,Some(&effective))?;child_of(&id,&supervisor)?;scope::membership(&id,&original["scope"]["binding"])?;
                let until=Instant::now()+Duration::from_secs(1);let mut left=bytes.as_slice();
                while !left.is_empty() {match input.write(left) {
                    Ok(0)=>return Err("native input disconnected; delivery unknown, never replay".into()),Ok(n)=>left=&left[n..],
                    Err(e) if e.kind()==io::ErrorKind::Interrupted=>{},
                    Err(e) if e.kind()==io::ErrorKind::WouldBlock&&Instant::now()<until=>{std::thread::sleep(Duration::from_millis(5));},
                    Err(_)=>return Err("native input deadline/error; delivery unknown, never replay".into()),
                }}Ok(())
            })?;
        }
    }
}

// A retained stop is fenced by operation and native incarnation, never by request or receipt bytes.
#[cfg(any(test,target_os="macos"))]
fn retained_incarnation(original:&Value)->Res<String> {
    let (supervisor,child)=(native(&original["supervisor"])?,native(&original["native"])?);
    let scope=&original["scope"];
    Ok(digest(json!({"supervisor":[supervisor.pid,supervisor.started],"native":[child.pid,child.started],"boot":scope["binding"]["boot"],"coalition":scope["binding"]["coalition"],"label":scope["label"],"domain":scope["domain"]}).to_string().as_bytes()))
}
#[cfg(any(test,target_os="macos"))]
fn retained_fence(s:&resources::State,operation:&str,incarnation:&str)->Res<Value> {
    let key=format!("retained-{operation}");
    // Both existing action records bind the operation; keep their receipts and consumed intents intact.
    for key in [&key,&format!("retained-cancel-{operation}")] {
        if s.ops.get(key).is_some_and(|op|op.bound["incarnation"]!=incarnation) {
            return Err("retained cleanup binding changed".into());
        }
    }
    Ok(s.ops.get(&key).map_or(Value::Null,|op|op.receipt.clone()))
}
#[cfg(any(test,target_os="macos"))]
fn retained_inherit(install:&Path,operation:&str,incarnation:&str,acknowledged:bool)->Res<()> {
    let key=format!("retained-{operation}");
    resources::within(install,resources::WAIT,|s| {
        retained_fence(s,operation,incarnation)?;
        let op=s.ops.entry(key.clone()).or_insert_with(||resources::Op{bound:json!({"incarnation":incarnation}),receipt:json!({"cleanup_intent":true})});
        if acknowledged {op.receipt["bootout_ack"]=json!(true);}Ok(())
    })
}
/// Durable intent precedes the single bootout; a lost acknowledgment stays unknown and is never retried.
#[cfg(any(test,target_os="macos"))]
fn retained_cleanup(install:&Path,operation:&str,incarnation:&str,verify:impl FnOnce()->Res<()>,bootout:impl FnOnce()->Res<()>,exited:impl FnOnce()->Res<()>)->Res<Value> {
    let key=format!("retained-{operation}");
    let saved=resources::within(install,resources::WAIT,|s|retained_fence(s,operation,incarnation))?;
    if saved["cleanup_verified"]==true {
        if saved["cleanup_intent"]!=true||saved["bootout_ack"]!=true {return Err("retained cleanup proof incomplete".into());}
        return Ok(saved);
    }
    let result=(||->Res<Value> {
        if saved.is_null() {
            verify()?;
            resources::within(install,resources::WAIT,|s| {
                retained_fence(s,operation,incarnation)?;
                if s.ops.contains_key(&key) {return Err("retained cleanup intent consumed; never retry".into());}
                s.ops.insert(key.clone(),resources::Op{bound:json!({"incarnation":incarnation}),receipt:json!({"cleanup_intent":true})});Ok(())
            })?;
            bootout()?;
            resources::within(install,resources::WAIT,|s| {s.ops.get_mut(&key).ok_or("cleanup record absent")?.receipt["bootout_ack"]=json!(true);Ok(())})?;
        }
        let acknowledged=resources::within(install,resources::WAIT,|s|retained_fence(s,operation,incarnation))?;
        if acknowledged["bootout_ack"]!=true {return Err("retained cleanup unacknowledged; never repeat".into());}
        exited()?;
        resources::within(install,resources::WAIT,|s| {
            retained_fence(s,operation,incarnation)?;
            let op=s.ops.get_mut(&key).ok_or("cleanup record absent")?;
            op.receipt["cleanup_verified"]=json!(true);Ok(op.receipt.clone())
        })
    })();
    Ok(result.unwrap_or_else(|reason|json!({"state":"unknown","reason":reason})))
}
/// Exact native stop at most once per operation; a request never proves completion.
#[cfg(any(test,target_os="macos"))]
fn retained_cancel(install:&Path,operation:&str,incarnation:&str,verify:impl FnOnce()->Res<()>,terminate:impl FnOnce()->Res<()>)->Value {
    let key=format!("retained-cancel-{operation}");
    let result=(||->Res<()> {
        resources::within(install,resources::WAIT,|s|retained_fence(s,operation,incarnation))?;
        verify()?;
        resources::within(install,resources::WAIT,|s| {
            retained_fence(s,operation,incarnation)?;
            if s.ops.contains_key(&key) {return Err("retained cancellation already requested; observe without retry".into());}
            s.ops.insert(key.clone(),resources::Op{bound:json!({"incarnation":incarnation}),receipt:json!({"cancel_intent":true})});Ok(())
        })?;
        terminate()
    })();
    json!({"state":"unknown","reason":result.err().unwrap_or_else(||"exact native cancellation requested; completion unproved".into())})
}

fn retained_original(install:&Path,input:&Value)->Res<(String,Value)> {
    let checkpoint=input["checkpoint"].as_str().filter(|s|s.len()<=1024*1024).ok_or("retained checkpoint absent or oversized")?;
    let receipt=input["receipt"].as_str().filter(|s|s.len()<=1024*1024).ok_or("retained receipt absent or oversized")?;
    let checkpoint:Value=serde_json::from_str(checkpoint).map_err(|_|"retained checkpoint corrupt")?;
    let seen:Value=serde_json::from_str(receipt).map_err(|_|"retained receipt corrupt")?;
    let permit=&checkpoint["slot"];super::protected::validate_slot(permit)?;
    let r=Request{schema:SCHEMA.into(),permit:permit.clone(),policy_revision:1,account:"recovery".into(),preparation:String::new()};
    let key=operation_id(&r)?;
    let archived=envelope(install,&install.join("retained").join(&key).join(format!("{}.json",digest(receipt.as_bytes()))),"agentq.retained-operation/1")?;
    let prepared=&checkpoint["receipt"]["prepared"];
    if archived!=*input||prepared["schema"]!="taskq.app-server-preparation/1"||prepared["binding"]!=permit["binding"]
        ||checkpoint["receipt"]["preparation"]!=digest(prepared.to_string().as_bytes())
        ||prepared["authority"]["saved"]["grant"]["mode"]!="workspace"||prepared["authority"]["runtime"]!="codex"
        ||prepared["authority"]["project"]!=permit["binding"]["root"]
        ||checkpoint["receipt"]["schema"]!="taskq.app-server/1"||checkpoint["receipt"]["state"]!="prepared"
        ||seen["schema"]!="taskq.app-server/1"||seen["caller"]!=permit["binding"]["caller"]||seen["operation"]!=permit["binding"]["op_id"]
        ||seen["launch"]["authority"]!=checkpoint["receipt"]["prepared"]["authority"]
        ||seen["launch"]["host_instance"]!=checkpoint["receipt"]["prepared"]["host_instance"]
        ||seen["launch"]["policy_revision"]!=checkpoint["receipt"]["policy_revision"]||seen["launch"]["account"]!=checkpoint["receipt"]["account"] {
        return Err("retained authority/operation proof changed; retain occupancy".into());
    }
    Ok((key,seen["launch"].clone()))
}

#[cfg(test)]
#[path="host/tests.rs"]
mod tests;
