//! Native JSONL RPC session state; command acknowledgements are not completed model turns.
use super::*;
use super::stdio::Stdio;

pub(super) struct Config {pub args:&'static [&'static str],pub home_var:&'static str,pub resume_arg:&'static str}

pub(super) struct Session {
    server:Stdio,
    pub path:PathBuf,
    pub boundary:u64,
    log:File,
    pub session:String,
    serial:u64,
    pending:Vec<Event>,
    report:Option<Report>,
    messages:Vec<Value>,
    deadline:Option<Instant>,
    started:Instant,
    started_ms:i64,
    closed:Option<String>,
    sampler:Option<capture::Sampler>,
    active:bool,
    prompt:Option<Value>,
}

impl Drop for Session {fn drop(&mut self){let _=self.server.kill();}}

pub(super) fn validate_launch(l:&Launch)->Res<()> {
    if l.policy!=Policy::default() {return Err("policy unsupported by JSONL RPC; native provider permissions required".into());}
    if l.model.is_some() || l.effort.is_some() || l.compact_hook.is_some() || l.title.is_some() || l.mcp.is_some() || l.instructions.is_some()
        || !l.parts.is_empty() || !l.settings_env.is_empty() || !l.host_args.is_empty() {return Err("launch option unsupported by JSONL RPC".into());}
    if l.input.is_empty() {return Err("input-empty".into());}Ok(())
}

impl Session {
    pub fn open(mut command:Command,config:&Config,l:&Launch,resume:Option<&str>)->Res<Self> {
        validate_launch(l)?;
        command.args(config.args).current_dir(&l.root).envs(l.env.iter().cloned());
        if let Some(home)=&l.home {command.env(config.home_var,home);}
        if let Some(session)=resume {command.arg(config.resume_arg).arg(session);}
        capture::inherit_caller(&mut command);
        if let Some(parent)=l.log.parent(){std::fs::create_dir_all(parent).map_err(|e|format!("RPC log directory: {e}"))?;}
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(&l.log).map_err(|e|format!("RPC log: {e}"))?;
        let boundary=log.metadata().map_err(|e|e.to_string())?.len();
        let server=Stdio::spawn_stream(command,&l.log.with_extension("stderr"),&l.process)?;
        let sampler=Some(capture::Sampler::start(server.id()));
        let mut s=Self{server,path:l.log.clone(),boundary,log,session:String::new(),serial:0,pending:vec![],report:None,messages:vec![],deadline:l.deadline,started:Instant::now(),started_ms:unix_ms(),closed:None,sampler,active:false,prompt:None};
        let id=s.request("get_state",json!({}))?;
        loop {
            let v=s.server.wait(s.deadline)?;
            if v["id"]==id && v["type"]=="response" {
                s.log(&v)?;
                if v["success"]!=true {return Err(format!("RPC get_state refused: {}",v["error"]));}
                s.session=v["data"]["sessionFile"].as_str().filter(|s|!s.is_empty()).ok_or("RPC native session file missing")?.into();
                if resume.is_some_and(|old|old!=s.session) {return Err("RPC resumed session differs".into());}
                if v["data"]["isStreaming"]!=false || v["data"]["isCompacting"]!=false || v["data"]["pendingMessageCount"].as_u64().is_some_and(|n|n!=0) {
                    return Err("RPC session not proven idle; input not submitted".into());
                }
                s.pending.push(Event::Metadata{kind:"session".into(),data:v["data"].clone()});break;
            }
            s.log(&json!({"type":"rpc-replay","message":v,"replay":true}))?;
        }
        s.prompt=Some(s.request("prompt",json!({"message":l.input}))?);Ok(s)
    }

    fn log(&mut self,v:&Value)->Res<()> {writeln!(self.log,"{v}").map_err(|e|format!("RPC transcript: {e}"))}
    fn request(&mut self,command:&str,mut fields:Value)->Res<Value> {
        self.serial=self.serial.checked_add(1).ok_or("RPC request ID overflow")?;
        fields["id"]=json!(self.serial.to_string());fields["type"]=json!(command);
        self.server.send_until(&fields,self.deadline)?;Ok(fields["id"].clone())
    }
    fn observe(&mut self,v:Value)->Res<()> {
        if v["type"].as_str().is_none() {return Err("RPC record type missing".into());}
        self.log(&v)?;
        if v["type"]=="response" && v["success"]==false && self.prompt.as_ref()==v.get("id") {
            let mut r=Report::empty();r.session=Some(self.session.clone());r.outcome="failed";r.error="client-error";r.detail=v["error"].as_str().map(String::from);self.report=Some(r);
        }
        if v["type"]=="agent_start" {self.active=true;}
        if v["type"]=="message_end" && v["message"]["role"]=="assistant" {self.messages.push(v["message"].clone());}
        self.pending.extend(record_events(&v));
        // agent_end can precede automatic retry or compaction; only the small native settlement record is final.
        if v["type"]=="agent_settled" && self.active {
            let mut r=Report::empty();r.session=Some(self.session.clone());r.started_ms=self.started_ms;r.duration_us=self.started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
            for m in &self.messages {
                let u=&m["usage"];
                r.calls.push(Call{id:None,at_ms:m["timestamp"].as_u64(),input:u["input"].as_u64(),output:u["output"].as_u64(),cache_read:u["cacheRead"].as_u64(),cache_write:u["cacheWrite"].as_u64(),reasoning:None,
                    model:m["model"].as_str().map(String::from),outcome:match m["stopReason"].as_str(){Some("error")=>"failed",Some("aborted")=>"cancelled",_=>"ok"},error:if m["stopReason"]=="error"{"client-error"}else{"none"},aggregate:false});
            }
            let last=self.messages.last();
            (r.outcome,r.error)=match last.and_then(|m|m["stopReason"].as_str()) {Some("error")=>("failed","client-error"),Some("aborted")=>("cancelled","turn-cancelled"),Some("stop"|"length"|"toolUse")=>("ok","none"),_=>("unknown","native-stop-unknown")};
            r.detail=last.and_then(|m|m["errorMessage"].as_str()).map(String::from);
            r.text=last.map(|m|text(&m["content"]));r.usage=sum_usage(&r.calls,(!r.calls.is_empty()).then_some(r.calls.len() as u64),!r.calls.is_empty());
            self.pending.push(Event::TurnEnd{outcome:r.outcome});self.report=Some(r);
        }
        Ok(())
    }
    fn pump(&mut self)->Res<()> {
        if self.closed.is_some(){return Ok(());}
        loop {match self.server.receive(Instant::now()) {Ok(Some(v))=>self.observe(v)?,Ok(None)=>break,Err(e)=>{self.closed=Some(e);break;}}}
        if let Some(r)=&mut self.report {
            self.server.close_stream(self.deadline)?;r.exit=self.server.poll()?;
            if let Some(sampler)=self.sampler.take(){r.process=sampler.usage();}
            self.closed=Some("terminal".into());
        }Ok(())
    }
    pub fn poll(&mut self)->Res<State> {
        self.pump()?;
        if let Some(r)=&self.report{return Ok(match r.outcome{"ok"=>State::Finished,"unknown"=>State::Unknown,_=>State::Failed});}
        if self.deadline.is_some_and(|d|Instant::now()>=d){self.server.kill()?;self.closed=Some("caller deadline; native turn outcome unknown".into());}
        Ok(if self.closed.is_some(){State::Unknown}else{State::Running})
    }
    pub fn report(&self)->Res<Report>{self.report.clone().ok_or_else(||self.closed.clone().unwrap_or_else(||"running".into()))}
    pub fn progress(&mut self)->Res<Vec<Event>>{self.pump()?;Ok(std::mem::take(&mut self.pending))}
    pub fn id(&self)->u32{self.server.id()}
    pub fn cancel(&mut self)->Res<bool>{if self.report.is_some(){return Ok(true);}self.request("abort",json!({}))?;Ok(false)}
}

fn text(v:&Value)->String {v.as_str().map(String::from).unwrap_or_else(||v.as_array().into_iter().flatten().filter_map(|v|v["text"].as_str()).collect::<String>())}

pub(super) fn record_events(v:&Value)->Vec<Event> {
    let id=v["toolCallId"].as_str().map(String::from);
    match v["type"].as_str() {
        Some("agent_start")=>vec![Event::TurnStart],
        Some("message_update") if v["assistantMessageEvent"]["type"]=="text_delta" && v["assistantMessageEvent"]["delta"].is_string()=>vec![Event::Text{item:None,delta:v["assistantMessageEvent"]["delta"].as_str().unwrap().into()}],
        Some("message_end") if v["message"]["role"]=="assistant"=>vec![Event::Message{item:None,text:text(&v["message"]["content"])}],
        Some("tool_execution_start")=>vec![Event::ToolStart{id,kind:if v["toolName"]=="bash"{ToolKind::Shell}else{events::tool_kind(v["toolName"].as_str().unwrap_or_default())},name:v["toolName"].as_str().map(String::from),detail:None,timeout_ms:events::native_timeout(&v["args"])}],
        Some("tool_execution_end")=>vec![Event::ToolEnd{id,status:v["isError"].as_bool().map(|failed|if failed{"failed"}else{"completed"}.into())}],
        _=>vec![Event::Metadata{kind:v["type"].as_str().unwrap_or("unknown").into(),data:v.clone()}],
    }
}

pub(super) static PROTOCOL:registry::Protocol=registry::Protocol {
    prepare_command:|_,_,_|Err("JSONL RPC requires the structured launch path".into()),input_bytes:|_|vec![],native_record:|_,_|Err("native transcript polling unsupported".into()),native_metadata:None,
    finish_native_report:|_,_|{},session_report:|_,_|{},reported:|_,_|None,update_limits:|_,_,_|false,account_limits:|_,_|limits::unknown("account-limits unsupported"),identity:|_|None,
    credentials:|_,_|Err("oauth-unavailable".into()),oauth_token:|_,_|Err("oauth-unavailable".into()),oauth_usage:|_|Err("oauth-unavailable".into()),turn_ended:|ended,v|ended || v["type"]=="agent_settled",
    parse_event:|_,v,out,_|out.extend(record_events(v)),terminal:|_,_,_,_,_|None,assistant_failure:|_|None,permission_mode:|_|None,lifecycle:|_|None,advances:|_|false,
    session:|_|None,snapshot:|_,_|Snapshot{unknown:Some("not-reported"),..Snapshot::default()},report:|_,_,_|{let mut r=Report::empty();r.error="classify-unsupported";r},
};
