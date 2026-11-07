//! Native Claude stream controls; SDK source 2209f2c134fee914c0f75733b631a5fc238ca846.
use super::*;
use crate::agent::{control::Requests,stdio::Stdio as JsonStdio};

fn text<'a>(v:&'a Value,key:&str)->Res<&'a str> {v[key].as_str().filter(|s|!s.is_empty()).ok_or_else(||format!("request-{key}-invalid"))}

fn request(v:&Value,policy:&Policy)->Res<Event> {
    let id=text(v,"request_id")?;let native=&v["request"];
    if native["subtype"]!="can_use_tool" {return Err("request-native-unsupported".into());}
    text(native,"tool_use_id")?;let tool=text(native,"tool_name")?;
    if !native["input"].is_object() {return Err("request-input-invalid".into());}
    let mut questions=vec![];let mut choices=vec![];
    let kind=if tool=="AskUserQuestion" {
        let mut prompts=std::collections::BTreeSet::new();
        for (index,q) in native["input"]["questions"].as_array().filter(|a|!a.is_empty()).ok_or("request-questions-invalid")?.iter().enumerate() {
            let prompt=text(q,"question")?.to_string();if !prompts.insert(prompt.clone()) {return Err("request-question-duplicate".into());}
            let mut options=vec![];
            for option in q["options"].as_array().ok_or("request-options-invalid")? {
                let label=text(option,"label")?.to_string();if options.iter().any(|c:&Choice|c.id==label) {return Err("request-choice-duplicate".into());}
                options.push(Choice{id:label.clone(),label,kind:"answer".into()});
            }
            questions.push(Question{id:format!("q{}",index+1),prompt,choices:options,multiple:match q.get("multiSelect") {None=>false,Some(Value::Bool(v))=>*v,_=>return Err("request-question-invalid".into())},free_text:true});
        }
        RequestKind::Question
    }else{
        if matches!(policy.approval,Approval::Native|Approval::Bypass) {choices.push(Choice{id:"allow".into(),label:"Allow once".into(),kind:"allow_once".into()});}
        choices.push(Choice{id:"deny".into(),label:"Deny".into(),kind:"deny".into()});RequestKind::Approval
    };
    Ok(Event::Request{id:json!(id),kind,prompt:Some(tool.into()),choices,questions})
}

fn reply(v:&Value,control:&Control)->Res<Value> {
    let native=&v["request"];
    let response=match control {
        Control::Reply{option,..} if option.as_deref()==Some("allow")=>json!({"behavior":"allow","updatedInput":native["input"]}),
        Control::Reply{..}=>json!({"behavior":"deny","message":"Caller declined the request"}),
        Control::Answers{answers,..}=>{
            let mut input=native["input"].clone();let mut values=serde_json::Map::new();
            for (index,q) in input["questions"].as_array().ok_or("request-questions-invalid")?.iter().enumerate() {
                let selected=answers.get(&format!("q{}",index+1)).ok_or("question-answers-invalid")?.iter().map(|a|match a {Answer::Choice(s)|Answer::Text(s)=>s.clone()}).collect::<Vec<_>>();
                values.insert(text(q,"question")?.into(),if q["multiSelect"]==true {json!(selected)}else{json!(selected.first().ok_or("question-answers-invalid")?)});
            }
            input["answers"]=Value::Object(values);json!({"behavior":"allow","updatedInput":input})
        },
        _=>return Err("request-response-unsupported".into()),
    };
    Ok(json!({"type":"control_response","response":{"subtype":"success","request_id":v["request_id"],"response":response}}))
}

pub(crate) struct Session {
    server:Option<JsonStdio>,
    native:NativeProcess,
    scope:String,
    resume:bool,
    read_pos:u64,
    pub path:PathBuf,
    pub boundary:u64,
    log:File,
    pub session:String,
    pub identified:bool,
    requests:Requests,
    owner:SessionOwner,
    policy:Policy,
    pending:Vec<Event>,
    messages:Vec<Value>,
    report:Option<Report>,
    active:bool,
    cancelling:bool,
    closed:Option<String>,
    deadline:Option<Instant>,
    started:Instant,
    started_ms:i64,
    sampler:Option<capture::Sampler>,
    cursor:events::Cursor,
}
impl Drop for Session {fn drop(&mut self) {if let Some(server)=&mut self.server {let _=self.requests.retire(&mut self.log,&self.session,"unknown");if self.owner.control().is_ok() {let _=server.close();}else{server.close_stream();let _=server.poll();}}}}

impl Session {
    pub fn open(base:Command,spec:&Spec,l:&Launch,owner:&SessionOwner)->Res<Self> {
        let mut command=command(base,spec);
        if !spec.args.iter().any(|s|s=="--input-format") {command.args(["--input-format","stream-json"]);}
        command.args(["--permission-prompt-tool","stdio"]);
        if let Some(parent)=l.log.parent() {std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(&l.log).map_err(|e|e.to_string())?;let boundary=log.metadata().map_err(|e|e.to_string())?.len();
        let server=JsonStdio::spawn_stream(command,&l.log.with_extension("stderr"),&l.process)?;let sampler=Some(capture::Sampler::start(server.id()));let native=NativeProcess::read(server.id())?.ok_or("control-native-identity-unknown")?;
        owner.hold_controls()?;
        let mut session=Self {server:Some(server),native,scope:native::scope(spec),resume:spec.resume,read_pos:boundary,path:l.log.clone(),boundary,log,session:spec.session.clone(),identified:false,requests:Requests::new(),owner:owner.clone(),policy:l.policy.clone(),pending:vec![],messages:vec![],report:None,active:false,cancelling:false,closed:None,deadline:l.deadline,started:Instant::now(),started_ms:unix_ms(),sampler,cursor:events::Cursor::new(l.deltas)};
        session.exchange(json!({"subtype":"initialize","hooks":null}))?;
        let parts=if l.parts.is_empty() {vec![Part::Text(l.input.clone())]}else{l.parts.clone()};session.input(&parts)?;Ok(session)
    }
    pub fn id(&self)->u32 {self.native.pid}
    pub fn recovered(&self)->bool {self.server.is_none()}
    pub fn checkpoint(&self)->Value {
        json!({"path":self.path,"boundary":self.boundary,"session":self.session,"native":self.native,"scope":self.scope,"resume":self.resume,"identified":self.identified,"started_ms":self.started_ms})
    }
    pub fn recover(spec:&Spec,l:&Launch,saved:&Value,owner:&SessionOwner)->Res<Self> {
        if saved["path"]!=json!(l.log) || saved["scope"]!=native::scope(spec) || saved["session"]!=spec.session || saved["resume"]!=spec.resume {return Err("Claude control recovery scope differs".into());}
        let identified=saved["identified"].as_bool().ok_or("Claude control recovery identification invalid")?;
        let boundary=saved["boundary"].as_u64().ok_or("Claude control recovery boundary invalid")?;
        let native:NativeProcess=serde_json::from_value(saved["native"].clone()).map_err(|_|"Claude control recovery identity invalid")?;
        if NativeProcess::read(native.pid)?.is_some_and(|current|current!=native) {return Err("Claude control recovery native identity changed".into());}
        let started_ms=saved["started_ms"].as_i64().filter(|s|*s>=0).ok_or("Claude control recovery time invalid")?;
        let log=File::open(&l.log).map_err(|e|e.to_string())?;if log.metadata().map_err(|e|e.to_string())?.len()<boundary {return Err("Claude control recovery log truncated".into());}
        Ok(Self {server:None,native,scope:native::scope(spec),resume:spec.resume,read_pos:boundary,path:l.log.clone(),boundary,log,session:spec.session.clone(),identified,requests:Requests::new(),owner:owner.clone(),policy:l.policy.clone(),pending:vec![],messages:vec![],report:None,active:true,cancelling:false,closed:None,deadline:None,started:Instant::now(),started_ms,sampler:None,cursor:events::Cursor::default()})
    }
    pub fn run_id(&self)->&str {&self.requests.run}
    fn writable(&self)->Res<()> {
        if self.server.is_none() {return Err("control-transport-unavailable".into());}
        self.owner.control()?;
        if self.closed.is_some() || self.requests.uncertain() {return Err("control-state-unknown".into());}Ok(())
    }
    fn write(&mut self,v:&Value)->Res<()> {
        self.writable()?;
        let result=(|| {writeln!(self.log,"{}",json!({"type":"agentq_control_outgoing","native":v})).and_then(|_|self.log.sync_data()).map_err(|e|format!("control-journal-unknown: {e}"))?;self.server.as_mut().ok_or("control-transport-unavailable")?.send_until(v,self.deadline)})();
        if let Err(error)=&result {self.closed=Some(error.clone());}result
    }
    fn exchange(&mut self,request:Value)->Res<()> {
        self.writable()?;let id=crate::incident::uuid();self.write(&json!({"type":"control_request","request_id":id,"request":request}))?;
        loop {
            let value=match self.server.as_mut().ok_or("control-transport-unavailable")?.wait(self.deadline) {Ok(v)=>v,Err(e)=>{self.closed=Some(e.clone());return Err(e);}};
            let replied=value["type"]=="control_response" && value["response"]["request_id"]==id;
            self.observe(value.clone())?;
            if replied {
                self.log.sync_data().map_err(|e|{self.closed=Some(e.to_string());e.to_string()})?;
                if value["response"]["subtype"]!="success" {self.closed=Some("native-control-refused-or-unknown".into());return Err("native-control-refused-or-unknown".into());}return Ok(());
            }
        }
    }
    pub fn input(&mut self,parts:&[Part])->Res<()> {
        self.writable()?;if self.active {return Err("session-input-in-flight".into());}
        if parts.is_empty() || parts.iter().all(|p|matches!(p,Part::Text(t) if t.is_empty())) {return Err("input-empty".into());}
        let mut message:Value=serde_json::from_str(&stream_message(parts)?).map_err(|e|e.to_string())?;
        message["uuid"]=json!(crate::incident::uuid());message["session_id"]=json!(self.session);message["parent_tool_use_id"]=Value::Null;
        if self.report.is_some() {self.owner.submit()?;}
        self.boundary=self.log.metadata().map_err(|e|e.to_string())?.len();
        self.active=true;self.cancelling=false;self.report=None;self.messages.clear();self.cursor=events::Cursor::new(self.cursor.deltas);self.started=Instant::now();self.started_ms=unix_ms();if let Some(sampler)=&mut self.sampler {sampler.mark();}self.write(&message)
    }
    fn observe(&mut self,v:Value)->Res<()> {
        let result=self.observe_inner(v);
        if let Err(error)=&result {self.closed=Some(error.clone());let _=self.requests.retire(&mut self.log,&self.session,"unknown");}result
    }
    fn observe_inner(&mut self,v:Value)->Res<()> {
        if !v.is_object() {return Err("control-native-record-invalid".into());}
        if self.server.is_some() {writeln!(self.log,"{v}").map_err(|e|format!("control-journal-unknown: {e}"))?;}
        if v.get("session_id").is_some_and(|s|s!=&json!(self.session)) {return Err("control-native-session-differs".into());}
        if v["type"]=="system" && v["subtype"]=="init" {
            if v["session_id"]!=self.session {return Err("control-native-session-unverified".into());}self.identified=true;
        }
        if v["type"]=="control_request" {
            if self.recovered() {return Ok(());}
            if !self.identified || !self.active || self.cancelling {return Err("control-native-turn-unavailable".into());}
            let event=request(&v,&self.policy)?;let event=self.requests.register(&mut self.log,&self.session,event,v,None)?;self.pending.push(event);return Ok(());
        }
        if v["type"]=="control_cancel_request" {if self.recovered() {return Ok(());}self.requests.retire_native(&mut self.log,&self.session,&v["request_id"],None,&v)?;return Ok(());}
        if v["type"]=="result" {
            if !v["is_error"].is_boolean() || v["is_error"]==false && v["subtype"]!="success" {return Err("control-native-result-unknown".into());}
            if !self.active || v["session_id"]!=self.session {return Err("control-native-result-unassociated".into());}
            if !self.recovered() {self.requests.retire(&mut self.log,&self.session,"cancelled")?;}self.messages.push(v.clone());
            let text=self.messages.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");let mut report=Report::read(Client::Claude,&text,&self.messages,None);
            if let Some(server)=&mut self.server {server.observe_exit(&mut report)?;}
            report.started_ms=self.started_ms;report.duration_us=if self.recovered() {0}else{self.started.elapsed().as_micros().try_into().unwrap_or(u64::MAX)};report.process=self.sampler.as_ref().map_or_else(||capture::ProcessUse::none("recovered"),capture::Sampler::usage);
            self.active=false;self.report=Some(report);
        }else{self.messages.push(v.clone());}
        self.pending.extend(self.cursor.read_owned(Client::Claude,vec![v],Some(&self.session),None));Ok(())
    }
    pub fn pump(&mut self)->Res<()> {
        if self.closed.is_some() {return Ok(());}
        if self.recovered() {
            let (rows,next)=activity_lines(&self.path,self.read_pos)?;
            for row in rows {if row["type"]!="agentq_control" && row["type"]!="agentq_control_outgoing" {self.observe(row)?;}}
            self.read_pos=next;return Ok(());
        }
        loop {match self.server.as_mut().ok_or("control-transport-unavailable")?.receive(Instant::now()) {Ok(Some(v))=>self.observe(v)?,Ok(None)=>return Ok(()),Err(error)=>{self.closed=Some(error);self.requests.retire(&mut self.log,&self.session,"unknown")?;return Ok(());}}}
    }
    pub fn poll(&mut self)->Res<State> {
        self.pump()?;
        if self.requests.uncertain() {return Ok(State::Unknown);}
        if let Some(report)=&self.report {return Ok(state(report));}
        if self.closed.is_some() || self.recovered() {return Ok(State::Unknown);}
        if self.active && self.deadline.is_some_and(|d|Instant::now()>=d) {if !self.owner.stop_scope()? {self.server.as_mut().ok_or("control-transport-unavailable")?.kill()?;}self.closed=Some("caller deadline; native result unknown".into());return Ok(State::Unknown);}
        Ok(self.report.as_ref().map_or(State::Running,|r|match r.outcome {"ok"=>State::Finished,"unknown"=>State::Unknown,_=>State::Failed}))
    }
    pub fn report(&self)->Res<Report> {self.report.clone().ok_or_else(||self.closed.clone().unwrap_or_else(||"running".into()))}
    pub fn snapshot(&self,ctx:&Context)->Res<crate::agent::Frozen> {
        let report=self.report()?;let mut value=report.observation(Client::Claude,ctx);
        if self.recovered() {value["duration_us"]=Value::Null;value["unknown"]["duration_us"]=json!("recovered");}
        let key=crate::sha256_hex(format!("claude\0{}\0{}\0{}",self.session,self.path.display(),self.boundary).as_bytes());
        capture::freeze("agent.turn",&key,&value,&report,Client::Claude,ctx)
    }
    pub fn progress(&mut self)->Res<(Vec<Event>,Value)> {self.pump()?;Ok((std::mem::take(&mut self.pending),json!(self.cursor)))}
    pub fn respond(&mut self,control:Control)->Res<()> {
        self.writable()?;if !self.active || self.cancelling {return Err("control-turn-unavailable".into());}
        let response=reply(self.requests.native(&control)?,&control)?;let request=self.requests.begin(&mut self.log,&self.session,&control,&response)?;
        if let Err(error)=self.write(&response) {self.requests.fail(&mut self.log,&self.session,&request);return Err(error);}
        self.requests.sent(&mut self.log,&self.session,&request,&response)
    }
    pub fn set_option(&mut self,control:Control)->Res<()> {
        let Control::Model(model)=control else {return Err("option-unsupported".into());};if !model_code(&model) {return Err("model-invalid".into());}
        self.exchange(json!({"subtype":"set_model","model":model}))
    }
    pub fn compact(&mut self)->Res<()> {if self.active {return Err("compact-turn-active".into());}self.input(&[Part::Text("/compact".into())])}
    pub fn cancel(&mut self)->Res<bool> {
        self.writable()?;if !self.active {return Ok(true);}
        self.cancelling=true;self.requests.retire(&mut self.log,&self.session,"cancelled")?;self.exchange(json!({"subtype":"interrupt"}))?;Ok(false)
    }
    pub fn stop_facts(&mut self,run:String)->Res<crate::agent::history::stop::Facts> {
        let exit=self.server.as_mut().map(JsonStdio::poll).transpose()?.flatten();
        Ok(crate::agent::history::stop::Facts {run,format:"claude-stream",path:self.path.clone(),start:self.boundary,exited:(!self.recovered()).then_some(exit.is_some()),native_end:self.report.is_some(),exit,lost_owner:self.recovered()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_claude_callbacks_never_become_answerable_requests() {
        let base=json!({"request_id":"native","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"tool","input":{"questions":[{"question":"Choose","options":[{"label":"A"}]}]}}});
        for bad in [Value::Null,json!(""),json!([]),json!({}),json!(1)] {
            let mut v=base.clone();v["request_id"]=bad;assert!(request(&v,&Policy::default()).is_err());
        }
        for bad in [Value::Null,json!("native"),json!([]),json!({})] {
            let mut v=base.clone();v["request"]["input"]=bad;assert!(request(&v,&Policy::default()).is_err());
        }
        for questions in [json!([]),json!([null]),json!([{"question":"Choose","options":null}]),json!([{"question":"Choose","options":[{"label":"A"},{"label":"A"}]}]),json!([{"question":"Choose","options":[],"multiSelect":"true"}])] {
            let mut v=base.clone();v["request"]["input"]["questions"]=questions;assert!(request(&v,&Policy::default()).is_err());
        }
    }
}
