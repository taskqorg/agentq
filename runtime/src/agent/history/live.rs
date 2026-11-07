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

/// Read-scoped accounting facts; raw rows remain the source of truth.
pub(crate) struct UsageRevision {pub row:usize,pub revised_at:usize,pub call:super::super::Call,pub superseded_at:Option<usize>}
pub(crate) fn usage_rows(read:fn(&[Value])->(Vec<super::super::Call>,Option<u64>,bool),rows:&[Value])->Vec<UsageRevision> {
    rows.iter().enumerate().flat_map(|(row,value)|read(std::slice::from_ref(value)).0.into_iter().map(move |call|UsageRevision {row,revised_at:row,call,superseded_at:None})).collect()
}
pub(super) fn usage_steps(format:&str,events:&[Value],rows:&[Value],offset:u64)->BTreeMap<usize,Vec<Step>> {
    let mut out=BTreeMap::<usize,Vec<Step>>::new();
    for fact in (super::super::registry::history_protocol(format).expect("native history protocol").history_usage)(rows) {
        let event=&events[fact.row];let call=&fact.call;
        let mut step=super::steps::base_step(event,&rows[fact.row],&json!({"id":format!("usage:{}",json!([call.native_turn,call.id.clone().unwrap_or_else(||event["id"].as_str().unwrap_or_default().to_string())]))}),StepKind::Usage,call_json(call),0);
        step.author="system".into();step.data["active"]=json!(true);
        if let Some(turn)=&call.native_turn {step.turn.id=Some(turn.clone());step.turn.native_id=Some(turn.clone());step.turn.unknown=None;}
        if fact.revised_at>fact.row {
            let changed=&events[fact.revised_at];step.revision=changed["start"].as_u64().unwrap_or(0);step.source=changed.clone();
            step.origins.push(json!({"source":changed["source"],"start":changed["start"],"end":changed["end"],"sha256":changed["sha256"]}));
        }
        if let Some(at)=fact.superseded_at {
            let replaced=&events[at];
            if event["start"].as_u64().unwrap_or(0)>=offset || replaced["start"].as_u64().unwrap_or(0)<offset {continue;}
            step.state="superseded".into();step.revision=replaced["start"].as_u64().unwrap_or(0);step.source=replaced.clone();
            step.origins.push(json!({"source":replaced["source"],"start":replaced["start"],"end":replaced["end"],"sha256":replaced["sha256"]}));
            for field in ["input","cache_read","cache_write","output","reasoning","model","time","cost_usd","cost_source"] {step.data[field]=Value::Null;step.data["unknown"][field]=json!("superseded by identified native requests in this turn");}
            step.data["active"]=json!(false);
        }
        out.entry(fact.row).or_default().push(step);
    }
    out
}

pub(crate) fn record_steps(format:&str,event:&Value,record:&Value)->Vec<Step> {
    let p=record.get("payload").or(record.get("params")).unwrap_or(record);
    let mut out=Vec::new();
    let runtime=super::super::registry::history_runtime(format).expect("native history runtime");
    let mut add=|kind,data:Value,tag:&str| {
        let mut s=super::steps::base_step(event,record,&json!({"id":format!("{}:{tag}",event["id"].as_str().unwrap_or_default())}),kind,data,0);
        s.unknown=None;out.push(s);
    };
    if let Some((used,window))=(super::super::registry::history_protocol(format).expect("native history protocol").history_context)(record) {
        let mut unknown=BTreeMap::new();if used.is_none(){unknown.insert("used","native field not reported");}if window.is_none(){unknown.insert("window","native field not reported");}
        add(StepKind::Context,json!({"used":used,"window":window,"source":"native-context","unknown":unknown}),"context");
    }
    if record["type"]=="agentq_statusline" {
        if let Ok((context,windows))=statusline(p) {add(StepKind::Context,serde_json::to_value(context).unwrap(),"context");add(StepKind::Windows,json!(windows),"windows");}
    } else {
        let windows=runtime.log_limits(&record.to_string(),None,None);
        if windows.iter().any(|w|w.used.is_some() || w.resets_at.is_some()) {add(StepKind::Windows,json!(windows),"windows");}
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
    if let Some(reason)=call.accounting_unknown {unknown.insert("accounting".into(),json!(reason));}
    let mut value=json!({"id":call.id,"native_turn":call.native_turn,"accounting_unknown":call.accounting_unknown,"countable":call.accounting_unknown.is_none(),"input":call.input,"cache_read":call.cache_read,"cache_write":call.cache_write,"output":call.output,"reasoning":call.reasoning,
        "model":call.model,"time":call.at_ms,"cost_usd":call.cost_usd,"cost_source":call.cost_usd.map(|_|"native-reported"),"aggregate":call.aggregate,"unknown":unknown});
    if let Some(reason)=call.accounting_unknown {
        for field in ["input","cache_read","cache_write","output","reasoning","cost_usd","cost_source"] {value[field]=Value::Null;value["unknown"][field]=json!(reason);}
    }
    value
}

impl super::sessions::Session {
    pub fn live_data(&self,feed:Option<&Path>)->Res<LiveData> {
        if super::sources::external(&self.history.format){return self.source_live(feed);}
        let (mut file,identity)=self.history.open(&self.path)?;
        file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
        let mut raw=String::new();file.read_to_string(&mut raw).map_err(|e|format!("history unavailable: {e}"))?;
        let rows:Vec<Value>=raw.split_inclusive('\n').filter(|s|s.ends_with('\n') && !s.trim().is_empty()).map(|s|serde_json::from_str(s).map_err(|_|"history invalid JSON".to_string())).collect::<Res<_>>()?;
        let rt=super::super::registry::history_runtime(&self.history.format).ok_or("live data runtime unsupported")?;
        let protocol=super::super::registry::history_protocol(&self.history.format).ok_or("live data runtime unsupported")?;
        let calls=(protocol.read_calls)(&rows).0;
        let mut context=Context::default();
        if let Some(call)=calls.last() {
            context.used=call.input.and_then(|v|v.checked_add(call.cache_read?)).and_then(|v|v.checked_add(call.cache_write?));
            context.model=call.model.clone();context.source="native-call-input-estimate".into();context.time=json!(call.at_ms);
            if context.used.is_some(){context.unknown.remove("used");}if context.model.is_some(){context.unknown.remove("model");}
            if call.at_ms.is_some(){context.unknown.remove("time");}
        }
        if let Some((record,(used,window)))=rows.iter().rev().find_map(|v|(protocol.history_context)(v).map(|facts|(v,facts))) {
            context.used=used;context.window=window;context.source="native-context".into();context.time=record["timestamp"].clone();
            for (name,known) in [("used",used.is_some()),("window",window.is_some()),("time",!context.time.is_null())] {
                if known{context.unknown.remove(name);}else{context.unknown.insert(name.into(),"native context field not reported".into());}
            }
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
        let cost=rows.iter().rev().find(|v|v["type"]=="result").and_then(native_cost);
        assemble(&self.history,steps,context,windows,calls.iter().map(call_json).collect(),cost)
    }
}

fn assemble(history:&History,steps:Vec<Step>,context:Context,windows:Vec<super::super::Window>,calls:Vec<Value>,cost:Option<f64>)->Res<LiveData> {
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
            let mut rollup=last.clone();rollup.id=crate::sha256_hex(json!([history.source,history.session,"file-rollup",turn]).to_string().as_bytes());
            rollup.native_id=None;rollup.kind=StepKind::FileRollup;rollup.data=data.clone();
            rollup.origins=steps.iter().filter(|s|s.kind==StepKind::FileChange && s.turn.id==turn).flat_map(|s|s.origins.clone()).collect();
            file_steps.push(rollup);files.push(data);
        }
        let mut unknown=BTreeMap::new();
        if let Some(reason)=calls.iter().find_map(|c|c["accounting_unknown"].as_str()){unknown.insert("usage".into(),reason.into());}
        if cost.is_none(){unknown.insert("cost_usd".into(),"native turn cost not reported".into());}
        if plan.is_null(){unknown.insert("plan".into(),"native plan not reported".into());}
        if todos.is_null(){unknown.insert("todos".into(),"native todos not reported".into());}
        if files.is_empty(){unknown.insert("files".into(),"no native file changes observed".into());}
        let mut events=vec![LiveEvent::Context(context.clone()),LiveEvent::Windows(windows.clone())];
        events.extend(calls.iter().cloned().map(LiveEvent::ModelCall));
        if !plan.is_null(){events.push(LiveEvent::Plan(plan.clone()));}if !todos.is_null(){events.push(LiveEvent::Todos(todos.clone()));}
        events.extend(files.iter().cloned().map(LiveEvent::FileRollup));
        Ok(LiveData {context,windows,calls,plan,todos,files,cost_usd:cost,cost_source:cost.map(|_|"native-reported".into()),unknown,events,file_steps})
}
impl super::sessions::Session {
    fn source_live(&self,feed:Option<&Path>)->Res<LiveData> {
        if feed.is_some(){return Err("statusline feed unsupported by this history source".into());}
        let mut history=self.history.clone();history.offset=0;
        let steps=history.steps(&self.path,u64::MAX)?.steps;
        let calls:Vec<_>=steps.iter().filter(|s|s.kind==StepKind::Usage && s.data["aggregate"]!=true && !matches!(s.data["scope"].as_str(),Some("turn"|"session"|"model"))).map(|s|s.data.clone()).collect();
        let cost=if calls.is_empty(){None}else{calls.iter().try_fold(0.0,|n,c|Some(n+native_cost(c)?)).filter(|n|n.is_finite())};
        let aggregate:Vec<_>=steps.iter().filter(|s|s.kind==StepKind::Usage && s.data["aggregate"]==true && matches!(s.data["scope"].as_str(),Some("session"|"model"))).collect();
        let cost=cost.or_else(||if aggregate.is_empty(){None}else{aggregate.iter().try_fold(0.0,|n,s|Some(n+native_cost(&s.data)?)).filter(|n|n.is_finite())});
        let mut context=Context::default();
        if let Some(last)=calls.last(){
            context.used=last["input"].as_u64().and_then(|n|n.checked_add(last["cache_read"].as_u64()?)).and_then(|n|n.checked_add(last["cache_write"].as_u64()?));
            context.model=last["model"].as_str().map(String::from);context.source="native-call-input-estimate".into();context.time=last["time"].clone();
            if context.used.is_some(){context.unknown.remove("used");}if context.model.is_some(){context.unknown.remove("model");}if !context.time.is_null(){context.unknown.remove("time");}
        }
        let windows=vec![super::super::Window {window:None,used:None,resets_at:None,event_id:None,exhausted:None,rejected:None,unknown:Some("native limit windows not reported by source".into())}];
        assemble(&self.history,steps,context,windows,calls,cost)
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
    let context=call.accounting_unknown.is_none().then_some(call.input).flatten().and_then(|n|n.checked_add(call.cache_read?)).and_then(|n|n.checked_add(call.cache_write?));
    value["context_tokens"]=json!(context);
    if context.is_none(){value["unknown"]["context_tokens"]=json!("not-reported");}
    cost_metadata(value,call.accounting_unknown.is_none().then_some(call.cost_usd).flatten());
}
pub(crate) fn turn_metadata(value:&mut Value,report:&super::super::Report) {cost_metadata(value,report.usage.cost_usd);}
