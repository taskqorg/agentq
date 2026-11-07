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
            questions.push(Question{id:format!("q{}",index+1),prompt,choices:options,multiple:q["multiSelect"].as_bool().ok_or("request-question-invalid")?,free_text:true});
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
    server:JsonStdio,
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
    sampler:capture::Sampler,
    cursor:events::Cursor,
}
impl Drop for Session {fn drop(&mut self) {let _=self.requests.retire(&mut self.log,&self.session,"unknown");let _=self.server.close();}}

impl Session {
    pub fn open(base:Command,spec:&Spec,l:&Launch,owner:&SessionOwner)->Res<Self> {
        let mut command=command(base,spec);
        if !spec.args.iter().any(|s|s=="--input-format") {command.args(["--input-format","stream-json"]);}
        command.args(["--permission-prompt-tool","stdio"]);
        if let Some(parent)=l.log.parent() {std::fs::create_dir_all(parent).map_err(|e|e.to_string())?;}
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(&l.log).map_err(|e|e.to_string())?;let boundary=log.metadata().map_err(|e|e.to_string())?.len();
        let server=JsonStdio::spawn_stream(command,&l.log.with_extension("stderr"),&l.process)?;let sampler=capture::Sampler::start(server.id());
        owner.hold_controls()?;
        let mut session=Self {server,path:l.log.clone(),boundary,log,session:spec.session.clone(),identified:false,requests:Requests::new(),owner:owner.clone(),policy:l.policy.clone(),pending:vec![],messages:vec![],report:None,active:false,cancelling:false,closed:None,deadline:l.deadline,started:Instant::now(),started_ms:unix_ms(),sampler,cursor:events::Cursor::default()};
        session.exchange(json!({"subtype":"initialize","hooks":null}))?;
        let parts=if l.parts.is_empty() {vec![Part::Text(l.input.clone())]}else{l.parts.clone()};session.input(&parts)?;Ok(session)
    }
    pub fn id(&self)->u32 {self.server.id()}
    pub fn run_id(&self)->&str {&self.requests.run}
    fn writable(&self)->Res<()> {
        self.owner.control()?;
        if self.closed.is_some() || self.requests.uncertain() {return Err("control-state-unknown".into());}Ok(())
    }
    fn write(&mut self,v:&Value)->Res<()> {
        let result=(|| {writeln!(self.log,"{}",json!({"type":"agentq_control_outgoing","native":v})).and_then(|_|self.log.sync_data()).map_err(|e|format!("control-journal-unknown: {e}"))?;self.server.send_until(v,self.deadline)})();
        if let Err(error)=&result {self.closed=Some(error.clone());}result
    }
    fn exchange(&mut self,request:Value)->Res<()> {
        self.writable()?;let id=crate::incident::uuid();self.write(&json!({"type":"control_request","request_id":id,"request":request}))?;
        loop {
            let value=match self.server.wait(self.deadline) {Ok(v)=>v,Err(e)=>{self.closed=Some(e.clone());return Err(e);}};
            if value["type"]=="control_response" && value["response"]["request_id"]==id {
                writeln!(self.log,"{value}").and_then(|_|self.log.sync_data()).map_err(|e|{self.closed=Some(e.to_string());e.to_string()})?;
                if value["response"]["subtype"]!="success" {self.closed=Some("native-control-refused-or-unknown".into());return Err("native-control-refused-or-unknown".into());}return Ok(());
            }
            self.observe(value)?;
        }
    }
    pub fn input(&mut self,parts:&[Part])->Res<()> {
        self.writable()?;if self.active {return Err("session-input-in-flight".into());}
        if parts.is_empty() || parts.iter().all(|p|matches!(p,Part::Text(t) if t.is_empty())) {return Err("input-empty".into());}
        let mut message:Value=serde_json::from_str(&stream_message(parts)?).map_err(|e|e.to_string())?;
        message["uuid"]=json!(crate::incident::uuid());message["session_id"]=json!(self.session);message["parent_tool_use_id"]=Value::Null;
        if self.report.is_some() {self.owner.submit()?;}
        self.active=true;self.cancelling=false;self.report=None;self.messages.clear();self.started=Instant::now();self.started_ms=unix_ms();self.sampler.mark();self.write(&message)
    }
    fn observe(&mut self,v:Value)->Res<()> {
        let result=self.observe_inner(v);
        if let Err(error)=&result {self.closed=Some(error.clone());let _=self.requests.retire(&mut self.log,&self.session,"unknown");}result
    }
    fn observe_inner(&mut self,v:Value)->Res<()> {
        writeln!(self.log,"{v}").map_err(|e|format!("control-journal-unknown: {e}"))?;
        if v.get("session_id").is_some_and(|s|s!=&json!(self.session)) {return Err("control-native-session-differs".into());}
        if v["type"]=="system" && v["subtype"]=="init" {
            if v["session_id"]!=self.session {return Err("control-native-session-unverified".into());}self.identified=true;
        }
        if v["type"]=="control_request" {
            let native_id=v["request_id"].clone();
            if !self.identified || !self.active || self.cancelling {return self.write(&json!({"type":"control_response","response":{"subtype":"error","request_id":native_id,"error":"No active caller-owned turn"}}));}
            let event=request(&v,&self.policy)?;let event=self.requests.register(&mut self.log,&self.session,event,v,None)?;self.pending.push(event);return Ok(());
        }
        if v["type"]=="control_cancel_request" {self.requests.retire_native(&mut self.log,&self.session,&v["request_id"],None,&v)?;return Ok(());}
        if v["type"]=="result" {
            if !self.active || v["session_id"]!=self.session {return Err("control-native-result-unassociated".into());}
            self.requests.retire(&mut self.log,&self.session,"cancelled")?;self.messages.push(v.clone());
            let text=self.messages.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");let mut report=Report::read(Client::Claude,&text,&self.messages,None);
            report.started_ms=self.started_ms;report.duration_us=self.started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);report.process=self.sampler.usage();
            self.active=false;self.report=Some(report);
        }else{self.messages.push(v.clone());}
        self.pending.extend(self.cursor.read_owned(Client::Claude,vec![v],Some(&self.session),None));Ok(())
    }
    pub fn pump(&mut self)->Res<()> {
        if self.closed.is_some() {return Ok(());}
        loop {match self.server.receive(Instant::now()) {Ok(Some(v))=>self.observe(v)?,Ok(None)=>return Ok(()),Err(error)=>{self.closed=Some(error);self.requests.retire(&mut self.log,&self.session,"unknown")?;return Ok(());}}}
    }
    pub fn poll(&mut self)->Res<State> {
        self.pump()?;
        if self.closed.is_some() || self.requests.uncertain() {return Ok(State::Unknown);}
        if self.active && self.deadline.is_some_and(|d|Instant::now()>=d) {self.owner.stop_scope()?;self.server.kill()?;self.closed=Some("caller deadline; native result unknown".into());return Ok(State::Unknown);}
        Ok(self.report.as_ref().map_or(State::Running,|r|match r.outcome {"ok"=>State::Finished,"unknown"=>State::Unknown,_=>State::Failed}))
    }
    pub fn report(&self)->Res<Report> {self.report.clone().ok_or_else(||self.closed.clone().unwrap_or_else(||"running".into()))}
    pub fn progress(&mut self)->Res<Vec<Event>> {self.pump()?;Ok(std::mem::take(&mut self.pending))}
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
        let exit=self.server.poll()?;
        Ok(crate::agent::history::stop::Facts {run,format:"claude-stream",path:self.path.clone(),start:self.boundary,exited:Some(exit.is_some()),native_end:self.report.is_some(),exit,lost_owner:false})
    }
}
