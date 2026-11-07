//! Explicit protected-receipt stop; native observations never supply caller grants.
use super::*;

pub struct BackgroundExecution<'a> {
    pub owner_receipt:&'a Value,
    pub project:&'a Path,
    pub job:&'a str,
    pub submitted_session:Option<&'a str>,
    pub native_session:&'a str,
    pub process:&'a NativeProcess,
    pub previous_process:Option<&'a NativeProcess>,
    pub input:&'a str,
    pub before:usize,
}

pub(super) fn stop(adapter:&Adapter,command:&dyn Fn()->Command,launch:&Launch,expected:&BackgroundExecution<'_>,source:&history::sessions::Session,budget:Duration)->Res<Output> {
    adapter.needs("background")?;
    let started=Instant::now();let left=||budget.checked_sub(started.elapsed()).filter(|d|!d.is_zero()).ok_or_else(||"background control budget exhausted".to_string());
    let mut native=preflight::spec(adapter,launch);native.session=expected.native_session.into();native.resume=true;
    let spec=NativeSpec {root:native.root.clone(),session:native.session.clone(),resume:true,args:native.args.clone(),home:native.home.clone(),env:native.env.clone(),owner:None};
    spec.validate_session()?;
    let project=expected.project.canonicalize().map_err(|_|"background control project unavailable")?;
    let cwd=spec.root.canonicalize().map_err(|_|"background control cwd unavailable")?;
    if project!=expected.project || cwd!=spec.root || !cwd.starts_with(&project) || !native::cli_id(expected.job) || !native::cli_id(expected.native_session) || expected.input.is_empty() {return Err("background control expected binding invalid".into());}
    if expected.submitted_session.is_some_and(|s|s!=expected.native_session) && expected.previous_process.is_none() {return Err("background copied process transition missing".into());}
    launch.session_store.background_stop(&native,expected.native_session,expected.process,expected.previous_process,expected.owner_receipt,launch.lifetime,|| {
    left()?;
    let candidates=adapter.background_candidates(command(),&spec,false,left()?)?;
    let mut selected=None;
    for candidate in candidates {
        if candidate.session()!=expected.native_session {continue;}
        if candidate.kind()!=BackgroundKind::Background || candidate.id()!=Some(expected.job) || selected.is_some() {return Err("background control native writers ambiguous".into());}
        selected=Some(candidate);
    }
    let candidate=selected.ok_or("background control job unavailable")?;
    if candidate.cwd()!=cwd || candidate.project()!=cwd || candidate.pid()!=BackgroundPid::Present(std::num::NonZeroU32::new(expected.process.pid).ok_or("background control process invalid")?) {return Err("background control native binding differs".into());}
    let check_process=|| -> Res<()> {
        if NativeProcess::read(expected.process.pid)?.as_ref()!=Some(expected.process) {return Err("background control process incarnation changed".into());}
        if let Some(old)=expected.previous_process {if old.pid==0 || old.started.is_empty() || old==expected.process || NativeProcess::read(old.pid)?.as_ref()==Some(old) {return Err("background control previous incarnation not gone".into());}}
        Ok(())
    };
    check_process()?;candidate.control_locator()?;
    let file=candidate.control_source(&source.source().path)?;
    let proof=source.control_proof(&file,expected.native_session,expected.submitted_session,expected.input,expected.before,expected.previous_process.is_some(),&left)?;
    let receipt=json!({"schema":"agentq-background-binding/1","project":project,"cwd":cwd,"home":candidate.home(),"job":expected.job,"session":expected.native_session,"submitted_session":expected.submitted_session,"process":expected.process,"previous_process":expected.previous_process,"input_hash":crate::sha256_hex(expected.input.as_bytes()),"history":proof});
    let row=json!({"kind":"background","id":expected.job,"sessionId":expected.native_session,"cwd":cwd,"state":candidate.state().ok_or("background control state unknown")?.as_str(),"pid":expected.process.pid});
    let job=(adapter.ops.background_job)(&row,&native,expected.job)?;
    Ok((receipt,job))
    },|job| (adapter.ops.background_stop)(command(),&native,&job,left()?))
}
