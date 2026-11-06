//! Explicit refusals preserve the single runtime trait for agents we only read.
use super::*;
use crate::agent::{self,Runtime,Descriptor,Run,Launch,Part,Failure,Cause,Account,Model,Window,Event,Accounts,State,Report,Observation,Check,Ready,Quota,NativeSpec,Process,BackgroundJob};
use crate::optimization::{Intent,Optimization};
use std::{ffi::{OsStr,OsString},process::{Command,Output},time::Duration};
struct Reader {spec:&'static SourceSpec,descriptor:Descriptor}
impl Reader {
    const fn new(spec:&'static SourceSpec)->Self {Self {spec,descriptor:Descriptor {name:spec.name,label:spec.name,brand:"",family:"history",models:&[],minimum:"",capabilities:&["history","history-only"]}}}
}
fn unavailable<T>()->Res<T> {Err("history-only runtime; execution operation unsupported".into())}
const CAUSE:Cause=Cause("history-only");
impl Runtime for Reader {
    fn descriptor(&self)->&Descriptor {&self.descriptor}
    fn stop_event(&self,_:&mut Run,_:u64)->Res<Option<super::super::stop::StopEvent>> {unavailable()}
    fn launch(&self,_:&Launch)->Res<Run> {unavailable()}
    fn resume(&self,_:&str,_:&Launch)->Res<Run> {unavailable()}
    fn input(&self,_:&mut Run,_:&[Part])->Result<(),Failure>{Err(Failure::Refused(CAUSE))}
    fn account(&self,_:&mut Run)->Result<Account,Failure>{Err(Failure::Refused(CAUSE))}
    fn catalogue_with_executable(&self,_:Option<&Path>,_:Option<&Path>)->Res<Vec<Model>>{unavailable()}
    fn log_limits(&self,_:&str,_:Option<&str>,_:Option<&Path>)->Vec<Window>{vec![]}
    fn account_limits(&self,home:Option<&Path>,now:i64)->Vec<Window>{self.account_limits_with_executable(home,None,now)}
    fn account_limits_with_executable(&self,_:Option<&Path>,_:Option<&Path>,_:i64)->Vec<Window>{crate::agent::limits::unknown("history-only runtime; account limits unsupported")}
    fn log_events(&self,_:&str)->Vec<Event>{vec![Event::Unsupported{reason:"history-only; use history steps"}]}
    fn log_records(&self,_:&str)->Vec<agent::LogRecord>{vec![]}
    fn account_identity(&self,_:Option<&Path>)->Option<String>{None}
    fn cancel(&self,_:&mut Run)->Res<bool>{unavailable()}
    fn poll(&self,_:&mut Run)->Res<State>{unavailable()}
    fn report(&self,_:&Run)->Res<Report>{unavailable()}
    fn progress(&self,_:&mut Run)->Res<Vec<Event>>{unavailable()}
    fn launcher_output(&self,_:&mut Run)->Result<Result<Output,crate::process::Failure>,Cause>{Err(CAUSE)}
    fn session_id(&self,_:&Run)->Option<String>{None}
    fn session_url(&self,_:&Run)->Result<String,Cause>{Err(CAUSE)}
    fn observe(&self,_:&mut Run)->Observation{Observation{state:State::Unknown,phase:"unknown",starting_ms:0,error:Some("history-only".into()),session:None,url:None,cause:Some(CAUSE),detail:None}}
    fn transcript(&self,_:&Run)->Result<PathBuf,Cause>{Err(CAUSE)}
    fn ready(&self,_:Option<&Path>,_:Option<&Path>)->Ready{let check=Check{state:"unknown",detail:"history-only; no execution readiness".into()};Ready{install:check.clone(),version:check.clone(),login:check}}
    fn quota(&self,_:&Accounts,_:i64)->Quota{Quota{account:None,free:false,off:true,hold:None}}
    fn running(&self)->usize{0}
    fn protected_paths(&self,home:Option<&Path>)->Vec<PathBuf>{roots(self.spec,home).unwrap_or_default()}
    fn home_env(&self,_:&str,_:&str)->Vec<(OsString,OsString)>{vec![]}
    fn checkpoint(&self,_:&Run)->Result<Value,Cause>{Err(CAUSE)}
    fn recover(&self,_:&Value,_:&Launch)->Res<Run>{unavailable()}
    fn classify(&self,_:&str)->Report{let mut r=Report::empty();r.outcome="unknown";r.error="history-only";r.usage.unknown=Some("history-only");r}
    fn executable(&self,_:Option<&OsStr>)->Res<PathBuf>{unavailable()}
    fn home(&self)->Option<PathBuf>{source_home(self.spec,None).ok()}
    fn session_transcript(&self,id:&str,home:Option<&Path>)->Option<PathBuf>{list(self.spec.name,home).ok()?.into_iter().find(|s|s.session==id || s.id==id).map(|s|s.path)}
    fn session_link(&self,_:&NativeSpec,_:Option<&str>)->Result<String,(Cause,Option<String>)>{Err((CAUSE,None))}
    fn prepare(&self,intent:Intent,_:&str,_:&mut NativeSpec,_:Option<&Path>)->Optimization{Optimization{intent,effective:"off",reason:"history-only"}}
    fn command(&self,_:Command,_:&NativeSpec)->Res<Command>{unavailable()}
    fn turn(&self,_:Command,_:&NativeSpec,_:&Path,_:&str,_:&Process)->Res<agent::Turn>{unavailable()}
    fn background_launch(&self,_:Command,_:&NativeSpec,_:&str,_:Duration)->Res<Result<Output,crate::process::Failure>>{unavailable()}
    fn jobs(&self,_:Command,_:&NativeSpec,_:bool,_:Duration)->Res<Vec<Value>>{unavailable()}
    fn job(&self,_:&Value,_:&NativeSpec,_:&str)->Res<BackgroundJob>{unavailable()}
    fn stop_job(&self,_:Command,_:&NativeSpec,_:&BackgroundJob,_:Duration)->Res<Output>{unavailable()}
    fn launch_refusal(&self,_:&str)->Option<Cause>{Some(CAUSE)}
    fn app_queue(&self,_:Command,_:&NativeSpec,_:&str,_:Duration)->Res<Output>{unavailable()}
    fn app_server_command(&self,_:Command,_:&NativeSpec)->Res<Command>{unavailable()}
}

static READERS:&[Reader]=&[Reader::new(&hermes::SPEC),Reader::new(&gemini::SPEC),Reader::new(&cursor::SPEC),Reader::new(&copilot::SPEC),Reader::new(&cline::SPEC),Reader::new(&codebuddy::SPEC),Reader::new(&devin::SPEC),Reader::new(&droid::SPEC),Reader::new(&grok::SPEC),Reader::new(&jcode::SPEC),Reader::new(&kimi::SPEC),Reader::new(&kiro::SPEC),Reader::new(&muse::SPEC),Reader::new(&omp::SPEC),Reader::new(&openclaw::SPEC),Reader::new(&opencode::SPEC),Reader::new(&opencode2::SPEC),Reader::new(&pi::SPEC),Reader::new(&qoder::SPEC),Reader::new(&zcode::SPEC),Reader::new(&rovo::SPEC)];
pub(super) fn get(name:&str)->Option<&'static dyn Runtime> {READERS.iter().find(|r|r.spec.name==name).map(|r|r as &dyn Runtime)}
