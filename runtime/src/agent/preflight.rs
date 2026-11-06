//! M12.15–M12.17: ordered launch checks; uncertainty never turns into a false success.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadinessMode { #[default] Continue, Strict }

/// Explicit preparation authority, separate from launch. Journal uses the installation's owned.json format.
#[derive(Clone, Debug, Default)]
pub struct Repair { pub workspace: bool, pub hooks: bool, pub journal: Option<PathBuf> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaunchCheck {
    pub check: String,
    pub state: String,
    pub cause: Option<String>,
    pub detail: String,
    pub next: String,
}
impl LaunchCheck {
    pub(super) fn new(check: &str, state: &str, cause: Option<&str>, detail: impl Into<String>, next: &str) -> Self {
        Self { check: check.into(), state: state.into(), cause: cause.map(String::from), detail: detail.into(), next: next.into() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Readiness { pub checks: Vec<LaunchCheck> }
impl Readiness {
    pub(super) fn unsupported() -> Self { Self { checks: vec![LaunchCheck::new("client", "unknown", Some("preflight-unsupported"), "No native preflight", "use a registered runtime")] } }
    pub(in crate::agent) fn metadata(&self) -> Value {
        let counts=self.checks.iter().map(|c| {
            let mut key=format!("preflight.{}.{}",c.check,c.state);
            if let Some(cause)=&c.cause {key.push('.');key.push_str(cause);}
            (key,json!(1))
        }).collect::<serde_json::Map<_,_>>();
        json!(counts)
    }

    pub fn permit(&self, mode: ReadinessMode) -> Res<()> {
        let refused=self.checks.iter().find(|c|c.state=="blocked").or_else(|| {
            (mode==ReadinessMode::Strict).then(||self.checks.iter().find(|c|c.state=="unknown")).flatten()
        });
        if let Some(c)=refused {
            if c.cause.as_deref()==Some("quota-exhausted") {return Err("quota-exhausted".into());}
            if c.check=="permissions" && c.state=="blocked" {return Err(format!("{}; next: {}",c.detail,c.next));}
            return Err(format!("{}: {}; next: {}",c.cause.as_deref().unwrap_or("readiness-unknown"),c.detail,c.next));}
        Ok(())
    }
}

pub(super) fn spec(adapter: &Adapter,l: &Launch)->Spec {
    let mut spec=Spec {client:adapter.client,root:l.root.clone(),session:"-".into(),resume:false,args:vec![],home:l.home.clone(),env:l.env.clone()};
    (adapter.ops.environment)(l,&mut spec);spec
}

pub(super) fn command(adapter: &Adapter, l: &Launch) -> Res<Command> {
    (adapter.ops.validate)(&l.command.iter().skip(1).chain(l.host_args.iter()).cloned().collect::<Vec<_>>(),&l.settings_env,&l.policy)?;
    let spec=spec(adapter,l);
    if let Some(home)=explicit_home(adapter,&spec) {if !home.is_dir() {return Err("account-home-missing".into());}}
    let mut command = launch_base(adapter,l)?;
    super::super::clean_environment(&mut command, explicit_home(adapter,&spec).is_some());
    command.envs(spec.env);
    if let Some(home) = &l.home { command.env(adapter.client.home_var(), home); }
    command.current_dir(&l.root);
    Ok(command)
}

pub(super) fn launch_base(adapter: &Adapter,l: &Launch)->Res<Command> {
    if l.command.is_empty() {
        if let Some((_,path))=spec(adapter,l).env.iter().rev().find(|(k,_)|k=="PATH") {return Ok(Command::new(adapter.executable(Some(path))?));}
    }
    base(&l.command,adapter.client)
}

fn explicit_home(adapter:&Adapter,spec:&Spec)->Option<PathBuf> {
    spec.home.clone().or_else(||spec.env.iter().rev().find(|(k,_)|k==adapter.client.home_var()).map(|(_,v)|PathBuf::from(v)))
}
pub(super) fn home(adapter: &Adapter, l: &Launch) -> Option<PathBuf> {
    explicit_home(adapter,&spec(adapter,l)).or_else(||base_home(adapter.client))
}

pub(super) fn ready(adapter:&Adapter,l:&Launch)->Ready {
    let check=|state,detail:String|Check {state,detail};
    let command=match launch_base(adapter,l) {Ok(c)=>c,Err(e)=>return Ready {install:check("unknown",e),version:check("unknown","executable unknown".into()),login:check("unknown","executable unknown".into())}};
    let program=Path::new(command.get_program());
    let path=spec(adapter,l).env.into_iter().rev().find(|(k,_)|k=="PATH").map(|(_,v)|v).unwrap_or_else(||std::env::var_os("PATH").unwrap_or_default());
    let exe=if program.components().count()>1 {program.to_path_buf()} else {which_in(&program.to_string_lossy(),&path)};
    if !exe.is_file() {return Ready {install:check("missing",format!("{} not found",exe.display())),version:check("unknown","not installed".into()),login:check("unknown","not installed".into())};}
    let install=check("ok",exe.to_string_lossy().into());
    if explicit_home(adapter,&spec(adapter,l)).is_some_and(|h|!h.is_dir()) {return Ready {install,version:check("unknown","account home missing; no probe was run".into()),login:check("missing","account home missing".into())};}
    let version=match self::command(adapter,l).and_then(|mut c| {c.arg("--version");crate::quiet(&mut c);crate::process::output("agent.ready",&mut c,Duration::from_secs(5)).map_err(|e|e.to_string())}) {
        Ok(o)=>match String::from_utf8_lossy(&o.stdout).lines().next().map(str::trim).filter(|s|!s.is_empty()) {
            None=>check("unknown","version not reported".into()),
            Some(v) if adapter.descriptor.minimum.is_empty()=>check("ok",v.into()),
            Some(v)=>match (version_parts(v),version_parts(adapter.descriptor.minimum)) {
                (Some(have),Some(need)) if have<need=>check("outdated",format!("{v} is below {}",adapter.descriptor.minimum)),
                (Some(_),_)=>check("ok",v.into()),_=>check("unknown",format!("{v} is not a version")),
            }
        },Err(e)=>check("unknown",e),
    };
    let login=self::command(adapter,l).map(|c|(adapter.ops.login)(c,l.policy.subscription)).unwrap_or_else(|e|check("unknown",e));
    Ready {install,version,login}
}

pub(super) fn catalogue(adapter:&Adapter,l:&Launch)->Res<Vec<Model>> {
    adapter.needs("catalogue")?;
    let home=home(adapter,l).ok_or("home-unavailable")?;
    if !home.is_dir() {return Err("account-home-missing".into());}
    let path=home.join("auth.json");let before=registry::credential_snapshot(&path)?;
    let result=rpc(adapter,l,|rpc| {
        let mut models=vec![];let mut cursor=Value::Null;let mut seen=std::collections::BTreeSet::new();
        loop {
            let reply=rpc("model/list",json!({"cursor":cursor,"includeHidden":false}))?;
            for model in reply["data"].as_array().ok_or("model catalogue unreadable")? {
                let id=model["model"].as_str().filter(|s|model_code(s)).ok_or("model catalogue invalid")?;
                models.push(Model {id:id.into(),name:model["displayName"].as_str().unwrap_or(id).into()});
            }
            match &reply["nextCursor"] {Value::Null=>return Ok(models),Value::String(next) if seen.insert(next.clone())=>cursor=json!(next),_=>return Err("model catalogue cursor invalid".into())}
        }
    });
    if registry::credential_snapshot(&path)?!=before {return Err("credential-changed".into());}
    result
}

pub(super) fn check(adapter: &Adapter, l: &Launch, repair: &Repair) -> Readiness {
    let mut checks = vec![];
    let selected_home=home(adapter,l);
    let explicit=explicit_home(adapter,&spec(adapter,l));
    let row = |check: &str, state: &str, cause: Option<&str>, detail: &str, next: &str| LaunchCheck::new(check,state,cause,detail,next);
    let native=self::ready(adapter,l);
    let ready=if native.install.state=="missing" {None} else {Some(native.version.state=="ok")};
    let (mut client_state,mut client_cause)=match (native.install.state,native.version.state) {
        ("missing",_) => ("blocked",Some("client-missing")),(_,"outdated") => ("blocked",Some("client-outdated")),("ok","ok") => ("ok",None),_ => ("unknown",Some("client-probe-failed")),
    };
    let mut client_detail=if native.install.state=="missing" {native.install.detail.clone()}else{native.version.detail.clone()};
    if native.install.state=="ok" && native.version.state!="outdated" && explicit.as_ref().is_none_or(|h|h.is_dir()) {
        if let Some(error)=adapter.ops.acp.and_then(|config|super::super::acp::check_version(config,&native.version).err()) {
            client_state="blocked";client_cause=Some(if native.version.state=="ok" {"client-version-refused"}else{"client-version-unverified"});
            client_detail=format!("{error}: {}",native.version.detail);
        }
    }
    checks.push(row("client",client_state,client_cause,&client_detail,"install or update the requested CLI"));
    if selected_home.as_ref().is_some_and(|h|!h.is_dir()) && explicit.is_some() {
        checks.push(row("account","blocked",Some("account-home-missing"),"Requested account home is missing","create or select that account home"));
    } else if l.home.as_ref().zip(spec(adapter,l).env.into_iter().rev().find(|(k,_)|k==adapter.client.home_var())).is_some_and(|(h,(_,v))| h!=Path::new(&v)) {
        checks.push(row("account","blocked",Some("account-home-conflict"),"Launch home and environment select different accounts","select one account home"));
    } else {
        let login=native.login;
        let state=match login.state {"ok"=>"ok","missing"=>"blocked","api-key" if l.policy.subscription=>"blocked","api-key"=>"ok",_=>"unknown"};
        checks.push(row("account",state,match state {"blocked"=>Some("login-required"),"unknown"=>Some("login-unknown"),_=>None},&login.detail,"sign in explicitly to the requested account"));
    }
    let quota=l.accounts.as_ref().map(|a|adapter.quota(a,(unix_ms()/1000).max(0)));
    checks.push(if quota.as_ref().is_some_and(|q|q.off || !q.free) {row("capacity","blocked",Some("quota-exhausted"),&format!("hold: {:?}",quota.and_then(|q|q.hold)),"wait for the account quota or explicitly select another account")}
        else {row("capacity","ok",None,if quota.is_some() {"Account is available"} else {"No account pool requested"},"")});
    let (workspace,evidence)=if !l.root.is_dir() {
        (row("workspace","blocked",Some("workspace-missing"),"Workspace is not a directory","select an existing workspace"),row("evidence","degraded",Some("evidence-unchecked"),"Workspace unavailable","check AgentQ hooks"))
    } else if !matches!(l.policy.write,Scope::Native|Scope::None) && writable(&l.root).is_err() {
        (row("workspace","blocked",Some("workspace-unwritable"),"Workspace is not writable","fix workspace access"),row("evidence","degraded",Some("evidence-unchecked"),"Workspace unavailable","check AgentQ hooks"))
    } else if ready==Some(true) { (adapter.ops.preparation)(l,repair) }
    else {(row("workspace","unknown",Some("workspace-trust-unknown"),"Native version could not be verified","check native trust for this exact path"),row("evidence","degraded",Some("hooks-unknown"),"Native hooks could not be checked","check AgentQ hooks"))};
    checks.push(workspace);
    checks.push(match adapter.spec(l,"-".into(),false).and_then(|_|if adapter.descriptor.has("app-server") && automatic(l) {turn_thread(l).map(|_|())} else {Ok(())}) {
        Ok(_) if l.policy.unelevated || matches!(l.policy.write,Scope::None|Scope::Project)=> {
            if ready==Some(true) {adapter.ops.sandbox.map(|probe|probe(l)).unwrap_or_else(||row("permissions","unknown",Some("sandbox-unverified"),"No native sandbox start probe","qualify native sandbox availability"))}
            else {row("permissions","unknown",Some("sandbox-unverified"),"Native version unavailable for sandbox probe","check native sandbox availability")}
        },
        Ok(_)=>row("permissions","ok",None,"Policy mapped; no explicit sandbox requested",""),
        Err(e)=>row("permissions","blocked",Some("policy-unsupported"),&e,"select a supported policy"),
    });
    let inputs=(|| {
        if let Some(file)=&l.mcp { mcp_servers(file)?; }
        if let Some(file)=&l.instructions {std::fs::read(file).map_err(|_|"instructions unreadable")?;}
        for part in &l.parts {if let Part::Image(file)=part {if !file.is_absolute() || std::fs::read(file).is_err() {return Err("image unreadable".into());}}}
        Ok::<_,String>(())
    })();
    let model=if l.model.is_some() && adapter.descriptor.has("catalogue") && ready==Some(true) {
        catalogue(adapter,l).map(|models|Some(models.iter().any(|model|Some(model.id.as_str())==l.model.as_deref())))
    } else {Ok(None)};
    checks.push(match (inputs,model) {
        (Err(e),_)=>row("inputs","blocked",Some("input-invalid"),&e,"provide readable input files"),
        (_,Ok(Some(false)))=>row("inputs","blocked",Some("model-unavailable"),"Requested model is absent from the native catalogue","select an available model"),
        (_,Err(e))=>row("inputs","unknown",Some("model-catalogue-unreadable"),&e,"check the native model catalogue"),
        _=>row("inputs","ok",None,"Input files checked; model checked when the native catalogue is available",""),
    });
    let storage=std::iter::once(&l.log).chain(l.store.iter()).all(|p| {
        if p.exists() {File::options().append(true).open(p).is_ok()} else {p.ancestors().skip(1).find(|p|p.exists()).is_some_and(|p|writable(p).is_ok())}
    });
    checks.push(row("storage",if storage {"ok"} else {"blocked"},(!storage).then_some("storage-unavailable"),"Caller log and requested store paths checked","provide writable caller storage"));
    checks.push(evidence);
    Readiness { checks }
}

fn writable(directory: &Path) -> Res<()> {
    let path=directory.join(format!(".agentq-probe-{}",crate::incident::uuid()));
    let file=File::options().write(true).create_new(true).open(&path).map_err(|e|e.to_string())?;
    drop(file); std::fs::remove_file(path).map_err(|e|e.to_string())
}

/// Persist ownership before a native mutation; the journal stores only the changed trust value.
pub(super) fn journal(repair: &Repair, entry: Value) -> Res<()> {
    let journal=repair.journal.as_ref().ok_or("repair-journal-required")?;
    if let Some(parent)=journal.parent() {std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
    let _lock=coreq::file_lock::try_lock(&journal.with_extension("lock")).map_err(|_|"repair-journal-locked")?;
    let mut entries:Vec<Value>=match std::fs::read(journal) {Ok(bytes)=>serde_json::from_slice(&bytes).map_err(|_|"repair-journal-invalid")?,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>vec![],Err(_)=>return Err("repair-journal-unreadable".into())};
    entries.push(entry);
    coreq::write_atomic(journal,&serde_json::to_string(&entries).map_err(|e|e.to_string())?)?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(journal,std::fs::Permissions::from_mode(0o600)).map_err(|e|e.to_string())?;}
    Ok(())
}

pub(super) fn safe_root(root: &Path, account: Option<&Path>) -> Res<PathBuf> {
    let root=root.canonicalize().map_err(|_|"workspace-missing")?;
    let home=std::env::var_os(if cfg!(windows) {"USERPROFILE"} else {"HOME"}).map(PathBuf::from).and_then(|p|p.canonicalize().ok());
    if root.parent().is_none() || home.as_ref().is_some_and(|h|h.starts_with(&root)) || account.and_then(|p|p.canonicalize().ok()).is_some_and(|h|h.starts_with(&root)) {return Err("workspace-trust-forbidden".into());}
    Ok(root)
}

/// Native config/hook reads reuse the app-server read protocol and its existing deadline, without a thread or turn.
pub(super) fn rpc<T>(adapter: &Adapter, l: &Launch, read: impl FnOnce(&mut dyn FnMut(&'static str,Value)->Res<Value>)->Res<T>) -> Res<T> {
    let mut spec=spec(adapter,l);spec.home=home(adapter,l);
    registry::codex_read_command(&spec,launch_base(adapter,l)?,|server,session,until| {
        read(&mut |method,params| {
            let request=session.request(method,params)?;
            registry::exchange(server,session,request,until).map(|reply|reply["result"].clone())
        })
    })
}

pub(super) fn native_journal(repair: &Repair, file: &Path, root: &Path, key: &[&str], before: &Value, after: &Value) -> Res<()> {
    journal(repair,json!({"file":file,"root":root,"key":key,"kind":"native-config","before":before,"after":after,"agent":"codex"}))
}


#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repair_refuses_the_account_home_and_every_ancestor() {
        let root=crate::scratch("trust-home-ancestors");let home=root.join("account/home");let project=root.join("project");std::fs::create_dir_all(&home).unwrap();std::fs::create_dir_all(&project).unwrap();
        assert_eq!(safe_root(&root,Some(&home)).unwrap_err(),"workspace-trust-forbidden");
        assert_eq!(safe_root(home.parent().unwrap(),Some(&home)).unwrap_err(),"workspace-trust-forbidden");
        assert_eq!(safe_root(&home,Some(&home)).unwrap_err(),"workspace-trust-forbidden");assert!(safe_root(&project,Some(&home)).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }
}
