//! Live facts share native call accounting; content never crosses the telemetry boundary.
use super::*;
use serde::Serialize;
use std::{collections::BTreeMap,io::{BufRead,BufReader,Write}};

#[derive(Clone,Debug,PartialEq,Serialize)]
pub struct Context { pub used:Option<u64>, pub window:Option<u64>, pub model:Option<String>, pub time:Value, pub observed_at_ms:Option<u64>, pub source:String, pub unknown:BTreeMap<String,String> }
impl Default for Context {
    fn default()->Self {Self {used:None,window:None,model:None,time:Value::Null,observed_at_ms:None,source:"unreported".into(),unknown:[("used".into(),"not reported".into()),("window".into(),"not reported".into()),("model".into(),"not reported".into()),("time".into(),"native time not reported".into())].into()}}
}

#[derive(Clone,Debug,Serialize)]
#[serde(tag="type",content="data",rename_all="snake_case")]
pub enum LiveEvent { Context(Context), Windows(Vec<super::super::Window>), ModelCall(Value), Plan(Value), Todos(Value), FileRollup(Value) }

#[derive(Clone,Debug,Serialize)]
pub struct LiveData {
    pub context:Context, pub windows:Vec<super::super::Window>, pub calls:Vec<Value>,
    pub plan:Value, pub todos:Value, pub files:Vec<Value>, pub cost_usd:Option<f64>, pub cost_source:Option<String>, pub unknown:BTreeMap<String,String>,
    pub events:Vec<LiveEvent>, pub file_steps:Vec<Step>,
}

pub(crate) fn native_cost(v:&Value)->Option<f64> {
    [v.get("cost_usd"),v.get("costUSD"),v.get("total_cost_usd")].into_iter().flatten().find_map(Value::as_f64).filter(|n|n.is_finite() && *n>=0.0)
}

pub fn statusline(v:&Value)->Res<(Context,Vec<super::super::Window>)> {
    if !v.is_object() {return Err("statusline payload must be an object".into());}
    let c=&v["context_window"];let u=&c["current_usage"];
    let used=u["input_tokens"].as_u64().and_then(|i|i.checked_add(u["cache_read_input_tokens"].as_u64()?))
        .and_then(|i|i.checked_add(u["cache_creation_input_tokens"].as_u64()?));
    let mut context=Context {used,window:c["context_window_size"].as_u64(),model:v["model"]["id"].as_str().map(String::from),time:v["timestamp"].clone(),source:"claude-statusline".into(),..Context::default()};
    context.unknown.clear();
    if context.used.is_none(){context.unknown.insert("used".into(),"native current usage not reported".into());}
    if context.window.is_none(){context.unknown.insert("window".into(),"native context window not reported".into());}
    if context.model.is_none(){context.unknown.insert("model".into(),"native model not reported".into());}
    if context.time.is_null(){context.unknown.insert("time".into(),"native time not reported".into());}
    let mut windows=Vec::new();
    for name in ["five_hour","seven_day"] {
        let w=&v["rate_limits"][name];
        let used=w["used_percentage"].as_f64().or(w["utilization"].as_f64()).filter(|n|n.is_finite() && *n>=0.0).map(|n|n/100.0);
        let reset=w["resets_at"].as_u64().or_else(||w["resets_at"].as_str().and_then(super::super::capture::time_ms).map(|n|n/1000));
        windows.push(super::super::Window {window:Some(name.into()),used,resets_at:reset,event_id:None,exhausted:used.map(|u|u>=1.0),rejected:None,unknown:if used.is_none() || reset.is_none(){Some("native-statusline-field-unreported".into())}else{None}});
    }
    Ok((context,windows))
}

/// Explicit caller-owned feed only. No user setting, HTTP call or installation write.
pub fn write_statusline(path:&Path,raw:&str)->Res<()> {
    if !path.is_absolute(){return Err("statusline output requires an absolute path".into());}
    let value:Value=serde_json::from_str(raw).map_err(|e|format!("statusline invalid JSON: {e}"))?;
    statusline(&value)?;
    if value["session_id"].as_str().is_none_or(str::is_empty){return Err("statusline session_id required".into());}
    let mut options=File::options();options.create(true).append(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(path).map_err(|e|format!("statusline feed: {e}"))?;
    let row=format!("{}\n",json!({"type":"agentq_statusline","payload":value,"raw":raw,"observed_at_ms":super::super::unix_ms()}));
    file.write_all(row.as_bytes()).map_err(|e|format!("statusline feed: {e}"))
}

pub(crate) fn record_steps(format:&str,event:&Value,record:&Value)->Vec<Step> {
    let p=record.get("payload").or(record.get("params")).unwrap_or(record);
    let mut out=Vec::new();
    let runtime=super::super::registry::history_runtime(format).expect("native history runtime");
    let mut add=|kind,data:Value,tag:&str| {
        let mut s=super::steps::base_step(event,record,&json!({"id":format!("{}:{tag}",event["id"].as_str().unwrap_or_default())}),kind,data,0);
        s.unknown=None;out.push(s);
    };
    if p["type"]=="token_count" {
        let info=&p["info"];add(StepKind::Context,json!({"used":info["last_token_usage"]["total_tokens"],"window":info["model_context_window"],"source":"native-token-count"}),"context");
    }
    if record["type"]=="agentq_statusline" {
        if let Ok((context,windows))=statusline(p) {add(StepKind::Context,serde_json::to_value(context).unwrap(),"context");add(StepKind::Windows,json!(windows),"windows");}
    } else {
        let windows=runtime.log_limits(&record.to_string(),None,None);
        if windows.iter().any(|w|w.used.is_some() || w.resets_at.is_some()) {add(StepKind::Windows,json!(windows),"windows");}
    }
    let calls=(super::super::registry::history_protocol(format).expect("native history protocol").read_calls)(std::slice::from_ref(record)).0;
    let mut usage_steps=Vec::new();
    for (index,call) in calls.iter().enumerate() {
        let mut usage=super::steps::base_step(event,record,&json!({"id":format!("usage:{}",call.id.clone().unwrap_or_else(||format!("{}:{index}",event["id"]))) }),StepKind::Usage,call_json(call),index);
        usage.author="system".into();usage_steps.push(usage);
    }
    let item=p.get("item").unwrap_or(p);
    if matches!(item["type"].as_str(),Some("plan"|"Plan")) || record["method"]=="turn/plan/updated" {
        add(StepKind::Plan,json!({"items":item.get("plan").or(p.get("plan")),"explanation":item.get("explanation").or(p.get("explanation"))}),"plan");
    }
    if let Some(blocks)=record["message"]["content"].as_array().filter(|_|matches!(record["type"].as_str(),Some("user"|"assistant"))) {
        for b in blocks {
            if b["type"]!="tool_use" {continue;}
            if b["name"]=="TodoWrite" {add(StepKind::Todo,json!({"items":b["input"]["todos"]}),"todos");}
            if b["name"]=="ExitPlanMode" {add(StepKind::Plan,json!({"text":b["input"]["plan"],"source":"native-plan-tool"}),"plan");}
        }
    }
    if item["name"]=="update_plan" {
        let input:Value=item["arguments"].as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or_else(||item["input"].clone());
        add(StepKind::Plan,json!({"items":input["plan"],"explanation":input["explanation"]}),"plan");
    }
    out.extend(usage_steps);
    let content=record["message"].get("content").unwrap_or(&item["content"]);
    for (index,step) in out.iter_mut().enumerate() {step.part=content.as_array().map_or(0,Vec::len)+index;}
    out
}

fn call_json(call:&super::super::Call)->Value {
    let mut unknown=serde_json::Map::new();
    for (name,value) in [("input",call.input),("cache_read",call.cache_read),("cache_write",call.cache_write),("output",call.output),("reasoning",call.reasoning)] {
        if value.is_none(){unknown.insert(name.into(),json!("native field not reported"));}
    }
    if call.cost_usd.is_none(){unknown.insert("cost_usd".into(),json!("native cost not reported; no estimate"));}
    if call.id.is_none(){unknown.insert("id".into(),json!("native request id not reported"));}
    if call.model.is_none(){unknown.insert("model".into(),json!("native model not reported"));}
    if call.at_ms.is_none(){unknown.insert("time".into(),json!("native time not reported"));}
    json!({"id":call.id,"input":call.input,"cache_read":call.cache_read,"cache_write":call.cache_write,"output":call.output,"reasoning":call.reasoning,
        "model":call.model,"time":call.at_ms,"cost_usd":call.cost_usd,"cost_source":call.cost_usd.map(|_|"native-reported"),"aggregate":call.aggregate,"unknown":unknown})
}

impl super::sessions::Session {
    pub fn live_data(&self,feed:Option<&Path>)->Res<LiveData> {
        let (mut file,identity)=self.history.open(&self.path)?;
        file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
        let mut raw=String::new();file.read_to_string(&mut raw).map_err(|e|format!("history unavailable: {e}"))?;
        let rows:Vec<Value>=raw.split_inclusive('\n').filter(|s|s.ends_with('\n') && !s.trim().is_empty()).map(|s|serde_json::from_str(s).map_err(|_|"history invalid JSON".to_string())).collect::<Res<_>>()?;
        let rt=super::super::registry::history_runtime(&self.history.format).ok_or("live data runtime unsupported")?;
        let calls=(super::super::registry::history_protocol(&self.history.format).ok_or("live data runtime unsupported")?.read_calls)(&rows).0;
        let mut context=Context::default();
        if let Some(call)=calls.last() {
            context.used=call.input.and_then(|v|v.checked_add(call.cache_read?)).and_then(|v|v.checked_add(call.cache_write?));
            context.model=call.model.clone();context.source="native-call-input-estimate".into();context.time=json!(call.at_ms);
            if context.used.is_some(){context.unknown.remove("used");}if context.model.is_some(){context.unknown.remove("model");}
            if call.at_ms.is_some(){context.unknown.remove("time");}
        }
        if let Some(v)=rows.iter().rev().find(|v|v["payload"]["info"]["model_context_window"].is_u64()) {
            context.window=v["payload"]["info"]["model_context_window"].as_u64();context.unknown.remove("window");
        }
        let mut windows=rt.log_limits(&raw,Some(&self.history.session),None);
        if let Some(feed)=feed {
            let mut reader=BufReader::new(File::open(feed).map_err(|e|format!("statusline feed unavailable: {e}"))?);
            let mut row=String::new();
            loop {
                row.clear();if reader.read_line(&mut row).map_err(|e|e.to_string())?==0 || !row.ends_with('\n'){break;}
                let v:Value=serde_json::from_str(&row).map_err(|_|"statusline feed invalid JSON")?;
                let p=v.get("payload").unwrap_or(&v);
                if p["session_id"]!=self.history.native_session || self.history.native_parent.is_some() {return Err("statusline native session differs".into());}
                (context,windows)=statusline(p)?;
                context.observed_at_ms=v["observed_at_ms"].as_u64();
            }
        }
        let mut h=self.history.clone();h.offset=0;h.prefix=crate::sha256_hex(b"");h.identity=Some(identity);
        let steps=h.steps(&self.path,u64::MAX)?.steps;
        if let Some(compaction)=steps.iter().rev().find(|s|s.kind==StepKind::Compaction) {
            let after_feed=context.observed_at_ms.zip(compaction.source["time"].as_str().and_then(super::super::capture::time_ms)).is_some_and(|(feed,compaction)|feed>=compaction);
            if !after_feed && steps.iter().rev().find(|s|s.kind==StepKind::Usage).is_none_or(|usage|usage.revision<compaction.revision) {
                context=Context::default();context.source="native-compaction".into();
            }
        }
        let plan=steps.iter().rev().find(|s|s.kind==StepKind::Plan).map(|s|s.data.clone()).unwrap_or(Value::Null);
        let todos=steps.iter().rev().find(|s|s.kind==StepKind::Todo).map(|s|s.data.clone()).unwrap_or(Value::Null);
        let mut files=BTreeMap::<(Option<String>,String),Vec<Value>>::new();
        for step in steps.iter().filter(|s|s.kind==StepKind::FileChange && s.state=="completed" && s.data["is_error"]!=true) {
            let changes=if let Some(changes)=step.data["changes"].as_object() {
                changes.iter().map(|(path,change)|json!({"path":path,"change":change})).collect::<Vec<_>>()
            }else{step.data["changes"].as_array().cloned().unwrap_or_default()};
            {
                for (index,change) in changes.iter().enumerate() {
                    let key=change["path"].as_str().map(String::from).unwrap_or_else(||format!("{}:{index}",step.id));
                    files.entry((step.turn.id.clone(),key)).or_default().push(json!({"change":change,"step":step.id,"revision":step.revision}));
                }
            }
        }
        let mut rollups=BTreeMap::<Option<String>,Vec<Value>>::new();
        for ((turn,path),changes) in files {rollups.entry(turn).or_default().push(json!({"path":path,"changes":changes}));}
        let mut file_steps=Vec::new();let mut files=Vec::new();
        for (turn,changes) in rollups {
            let Some(last)=steps.iter().rev().find(|s|s.kind==StepKind::FileChange && s.turn.id==turn) else{continue};
            let data=json!({"turn":last.turn,"changes":changes});
            let mut rollup=last.clone();rollup.id=crate::sha256_hex(json!([self.history.source,self.history.session,"file-rollup",turn]).to_string().as_bytes());
            rollup.native_id=None;rollup.kind=StepKind::FileRollup;rollup.data=data.clone();
            rollup.origins=steps.iter().filter(|s|s.kind==StepKind::FileChange && s.turn.id==turn).flat_map(|s|s.origins.clone()).collect();
            file_steps.push(rollup);files.push(data);
        }
        let cost=rows.iter().rev().find_map(|v|if v["type"]=="result" {native_cost(v)}else{None});
        let mut unknown=BTreeMap::new();
        if cost.is_none(){unknown.insert("cost_usd".into(),"native turn cost not reported".into());}
        if plan.is_null(){unknown.insert("plan".into(),"native plan not reported".into());}
        if todos.is_null(){unknown.insert("todos".into(),"native todos not reported".into());}
        if files.is_empty(){unknown.insert("files".into(),"no native file changes observed".into());}
        let calls:Vec<_>=calls.iter().map(call_json).collect();
        let mut events=vec![LiveEvent::Context(context.clone()),LiveEvent::Windows(windows.clone())];
        events.extend(calls.iter().cloned().map(LiveEvent::ModelCall));
        if !plan.is_null(){events.push(LiveEvent::Plan(plan.clone()));}if !todos.is_null(){events.push(LiveEvent::Todos(todos.clone()));}
        events.extend(files.iter().cloned().map(LiveEvent::FileRollup));
        Ok(LiveData {context,windows,calls,plan,todos,files,cost_usd:cost,cost_source:cost.map(|_|"native-reported".into()),unknown,events,file_steps})
    }
}

fn cost_metadata(value:&mut Value,cost:Option<f64>) {
    if !value["counts"].is_object(){value["counts"]=json!({});}
    if let Some(cost)=cost.and_then(|n|{let n=n*1_000_000_000.0;(n.is_finite() && n>=0.0 && n<u64::MAX as f64).then_some(n.round() as u64)}) {
        value["counts"]["cost_native_nanousd"]=json!(cost);value["counts"]["cost_source_native"]=json!(1);
    }else{value["counts"]["cost_source_unknown"]=json!(1);}
}

pub(crate) fn call_metadata(value:&mut Value,call:&super::super::Call,parent:&str) {
    value["parent"]=json!(crate::sha256_hex(parent.as_bytes()));
    let context=call.input.and_then(|n|n.checked_add(call.cache_read?)).and_then(|n|n.checked_add(call.cache_write?));
    value["context_tokens"]=json!(context);
    if context.is_none(){value["unknown"]["context_tokens"]=json!("not-reported");}
    cost_metadata(value,call.cost_usd);
}
pub(crate) fn turn_metadata(value:&mut Value,report:&super::super::Report) {cost_metadata(value,report.usage.cost_usd);}
