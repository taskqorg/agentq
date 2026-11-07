//! Runtime-authored request observations share live types and grant no reply authority.
use super::*;
use super::super::{Choice,Event,Question,RequestKind};

pub(super) fn session(record:&Value)->Option<String> {
    (record["type"]=="agentq_control").then(||record["session_id"].as_str().map(String::from)).flatten()
}

fn request(record:&Value)->Res<Event> {
    let event=record["event"].as_object().filter(|e|e.len()==1).ok_or("invalid request event")?;
    let body=event.get("Request").filter(|v|v.is_object()).ok_or("invalid request event")?;
    let id=body.get("id").filter(|id|!id.is_null()).cloned().ok_or("request ID not reported")?;
    let prompt=match &body["prompt"] {Value::Null=>None,Value::String(s)=>Some(s.clone()),_=>return Err("invalid request prompt".into())};
    let kind:RequestKind=serde_json::from_value(body["kind"].clone()).map_err(|_|"invalid request kind")?;
    let choices:Vec<Choice>=serde_json::from_value(body["choices"].clone()).map_err(|_|"invalid request choices")?;
    let questions:Vec<Question>=serde_json::from_value(body["questions"].clone()).map_err(|_|"invalid request questions")?;
    Ok(Event::Request {id,kind,prompt,choices,questions})
}

fn binding(record:&Value)->Option<(&str,u64)> {
    let run=record["run_id"].as_str().filter(|s|native::plain_id(s))?;
    let generation=record["generation"].as_u64()?;
    record["session_id"].as_str().filter(|s|!s.is_empty())?;
    if !record["turn_id"].is_null() && record["turn_id"].as_str().is_none_or(str::is_empty) {return None;}
    record["event"]["Request"].get("id").filter(|id|!id.is_null())?;
    Some((run,generation))
}

pub(super) fn decode(event:&Value,record:&Value)->Option<Step> {
    if record["type"]!="agentq_control" {return None;}
    let parsed=request(record);
    let kind=match &parsed {Ok(Event::Request {kind:RequestKind::Approval,..})=>StepKind::Approval,Ok(Event::Request {kind:RequestKind::Question,..})=>StepKind::Question,_=>StepKind::Unknown};
    let state=record["state"].as_str().filter(|s|matches!(*s,"pending"|"reply-pending"|"reply-sent"|"cancelled"|"unknown"));
    let scope=binding(record);
    let fault=parsed.as_ref().err().map(String::as_str).or_else(||scope.is_none().then_some("request scope missing or invalid"))
        .or_else(||state.is_none().then_some("unsupported request state"))
        .or_else(||(state==Some("unknown")).then_some("request delivery or state unknown"));
    let mut data=record.clone();
    data["unknown"]=json!({"native_origin":"native physical association not recorded; control row is authoritative"});
    if let Some(reason)=fault {data["unknown"]["request"]=json!(reason);}
    if record["native_id"].is_null(){data["unknown"]["native_id"]=json!("native request ID not reported");}
    if record["native"].is_null(){data["unknown"]["native"]=json!("native record not reported");}
    if record["turn_id"].as_str().is_none(){data["unknown"]["turn_id"]=json!("native turn ID not reported");}
    let mut step=super::steps::base_step(event,record,&json!({}),if parsed.is_err() || state.is_none(){StepKind::Unknown}else{kind},data,0);
    // A missing binding is one physical observation, never an inferred request occurrence.
    step.id=if let Some((run,generation))=scope {
        crate::sha256_hex(json!([event["format"],event["source"],event["session"],"request",run,generation,record["event"]["Request"]["id"]]).to_string().as_bytes())
    }else{format!("{}:request",event["id"].as_str().unwrap_or_default())};
    step.native_id=record["native_id"].as_str().map(String::from);
    step.author="system".into();step.state=if fault.is_some(){"unknown"}else{state.unwrap_or("unknown")}.into();
    step.unknown=fault.map(String::from);
    Some(step)
}

pub(super) fn unscoped(step:&mut Step,event:&Value,record:&Value) {
    let server_request=event["format"]=="codex-app-server" && record.get("id").is_some() && record["method"].is_string();
    if !matches!(step.kind,StepKind::Approval|StepKind::Question) && !server_request {return;}
    step.id=format!("{}:{}:request",event["id"].as_str().unwrap_or_default(),step.part);
    if !step.data.is_object(){step.data=json!({});}
    step.data["native"]=record.clone();step.data["unknown"]=json!({"request_scope":"native-only request has no verified run/generation scope"});
    step.state="unknown".into();step.unknown=Some("native-only request scope not recorded; observation only".into());
}
