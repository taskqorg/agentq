//! Stop observations explain missing terminal evidence without becoming completion authority.
use super::*;
use serde::{Serialize,Deserialize};

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="snake_case")]
pub enum StopCause { ExitWithoutTurnEnd, Signal, LostOwner, TurnEndProcessLive }

#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
pub struct StopEvent {
    pub id:String, pub cause:StopCause, pub session:Option<String>, pub exit_code:Option<i32>,
    pub outcome:String, pub unknown:Option<String>, pub last_activity:Option<Step>, pub activity_unknown:Option<String>,
}

/// Private adapter evidence; absent observations never imply a clean exit.
#[derive(Debug)]
pub(crate) struct Facts { pub run:String, pub format:&'static str, pub path:std::path::PathBuf, pub start:u64,
    pub exited:Option<bool>, pub native_end:bool, pub exit:Option<i32>, pub lost_owner:bool }

impl StopEvent {
    pub fn step(&self)->Step {
        let source=self.last_activity.as_ref().map(|s|s.source.clone()).unwrap_or(Value::Null);
        Step {id:self.id.clone(),native_id:None,revision:source["end"].as_u64().unwrap_or(0),kind:StepKind::Stop,
            turn:Turn {id:self.last_activity.as_ref().and_then(|s|s.turn.id.clone()),session:self.session.clone().unwrap_or_default(),unknown:Some("observation only; poll/report retain authority".into())},
            parent:self.last_activity.as_ref().map(|s|s.id.clone()),parent_session:self.session.clone(),author:"system".into(),state:"observed".into(),
            data:serde_json::to_value(self).expect("serializable stop observation"),unknown:self.unknown.clone(),
            origins:self.last_activity.as_ref().map(|s|s.origins.clone()).unwrap_or_default(),source}
    }
}

fn last(facts:&Facts, session:&str, bytes:u64)->Res<Option<Step>> {
    if bytes==0 {return Err("activity byte window must be positive".into());}
    let mut file=File::open(&facts.path).map_err(|e|format!("activity unavailable: {e}"))?;
    let end=file.metadata().map_err(|e|e.to_string())?.len();
    let start=end.saturating_sub(bytes).max(facts.start);
    let mut h=History::new(facts.format,facts.path.to_str().ok_or("activity path invalid UTF-8")?,session)?;
    h.offset=facts.start;h.identity=Some(native::file_id(&file)?);h.prefix=native::digest_prefix(&mut file,facts.start)?;
    Ok(h.steps_from(&facts.path,u64::MAX,facts.start)?.steps.into_iter().rev().find(|s|s.revision>=start))
}

pub(crate) fn observe(runtime:&dyn super::super::Runtime, run:&mut super::super::Run, bytes:u64)->Res<Option<StopEvent>> {
    if run.runtime()!=runtime.descriptor().name {return Err("foreign run".into());}
    let polled=runtime.poll(run);
    let facts=run.stop_facts()?;
    let cause=if facts.lost_owner || polled.is_err() {StopCause::LostOwner}
        else if cfg!(unix) && facts.exited==Some(true) && facts.exit==Some(-1) {StopCause::Signal}
        else if facts.exited==Some(true) && !facts.native_end {StopCause::ExitWithoutTurnEnd}
        else if facts.exited==Some(false) && facts.native_end {StopCause::TurnEndProcessLive}
        else{return Ok(None)};
    let session=runtime.session_id(run);
    let activity=last(&facts,session.as_deref().unwrap_or("unreported"),bytes);
    let (last_activity,activity_unknown)=match activity {Ok(Some(s))=>(Some(s),None),Ok(None)=>(None,Some("no complete activity in caller byte window".into())),Err(e)=>(None,Some(e))};
    let exit_code=if facts.lost_owner || cfg!(unix) && facts.exit==Some(-1){None}else{facts.exit};
    let unknown=if let Err(e)=polled {Some(e)}else if facts.lost_owner {Some("exit not observed after owner recovery".into())}
        else if cause==StopCause::Signal {Some("native process was signalled; signal number not retained".into())}else{None};
    let id=crate::sha256_hex(json!([runtime.descriptor().name,facts.run]).to_string().as_bytes());
    let outcome=if facts.lost_owner || unknown.is_some(){"unknown"}else if facts.exited==Some(true) && facts.exit.is_some_and(|c|c!=0){"failed"}else{"unknown"};
    Ok(Some(StopEvent {id,cause,session,exit_code,outcome:outcome.into(),unknown,last_activity,activity_unknown}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_activity_keeps_its_command_and_current_turn_boundary() {
        let path=std::env::temp_dir().join(format!("agentq-stop-activity-{}.jsonl",std::process::id()));
        let older="{\"type\":\"system\",\"session_id\":\"older-session\"}\n";
        let call=json!({"type":"assistant","sessionId":"current","cwd":"/fixture","message":{"content":[{"type":"tool_use","name":"Bash","id":"shell","input":{"command":"printf proof"}}]}});
        let result=json!({"type":"user","sessionId":"current","message":{"content":[{"type":"tool_result","tool_use_id":"shell","content":"proof"}]}});
        std::fs::write(&path,format!("{older}\n \t\r\n{call}\n\n{result}\n")).unwrap();
        let facts=Facts {run:"fixture-run".into(),format:"claude-stream",path:path.clone(),start:older.len() as u64,exited:Some(true),native_end:false,exit:Some(0),lost_owner:false};
        let step=last(&facts,"current",u64::MAX).unwrap().unwrap();
        assert_eq!(step.kind,StepKind::Shell);assert_eq!(step.data["command"],"printf proof");assert_eq!(step.data["output"],"proof");
        assert_eq!(step.origins.len(),2);assert!(last(&facts,"current",1).unwrap().is_none());
        use std::io::Write;
        let mut saved=History::new("claude-stream",path.to_str().unwrap(),"current").unwrap();saved.steps_from(&path,u64::MAX,older.len() as u64).unwrap();
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"malformed\n").unwrap();
        let before=saved.checkpoint();assert_eq!(saved.steps_from(&path,u64::MAX,older.len() as u64).unwrap_err(),"history invalid JSON");assert_eq!(saved.checkpoint(),before);
        std::fs::remove_file(path).unwrap();
    }
}
