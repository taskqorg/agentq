//! ACP version 1 over shared stdio. Runtime differences are registration data.
use super::*;
use super::stdio::Stdio;

pub(super) struct Config {
    pub args: &'static [&'static str],
    pub env: &'static [(&'static str, &'static str)],
    pub home_var: &'static str,
    pub home_arg: Option<&'static str>,
    pub modes: &'static [(Approval, &'static str)],
    pub rejected_versions: &'static [&'static str],
    pub bypass_args: &'static [&'static str],
    pub home: fn()->Option<PathBuf>,
    pub question_prefixes: &'static [&'static str],
}

/// Checked envelope; unknown fields and updates remain in the original JSON log.
#[derive(serde::Deserialize)]
struct Envelope {
    jsonrpc: String,
    #[serde(default)] id: Option<Value>,
    #[serde(default)] method: Option<String>,
}

fn validate(v: &Value) -> Res<()> {
    let m: Envelope=serde_json::from_value(v.clone()).map_err(|_|"ACP envelope invalid")?;
    if m.jsonrpc!="2.0" || m.id.as_ref().is_some_and(|id|!id.is_string() && !id.is_i64() && !id.is_u64()) {
        return Err("ACP version or request ID invalid".into());
    }
    if m.method.is_some() {
        if v.get("result").is_some() || v.get("error").is_some() {return Err("ACP request contains a response".into());}
    } else if m.id.is_none() || (v.get("result").is_some()==v.get("error").is_some()) {
        return Err("ACP response envelope invalid".into());
    }
    Ok(())
}

pub(super) struct Session {
    server: Stdio,
    pub path: PathBuf,
    pub boundary: u64,
    log: File,
    pub session: String,
    pending: Vec<Event>,
    requests: BTreeMap<String,Value>,
    serial: u64,
    prompt: Option<Value>,
    report: Option<Report>,
    text: String,
    started: Instant,
    started_ms: i64,
    closed: Option<String>,
    deadline: Option<Instant>,
    sampler: Option<capture::Sampler>,
    question_prefixes: &'static [&'static str],
}

impl Drop for Session {fn drop(&mut self) {let _=self.server.kill();}}

impl Session {
    pub fn open(mut command: Command, config: &Config, l: &Launch, resume: Option<&str>) -> Res<Self> {
        validate_launch(config,l)?;
        command.args(config.args).envs(config.env.iter().copied()).envs(l.env.iter().cloned()).current_dir(&l.root);
        if l.policy.approval==Approval::Bypass {command.args(config.bypass_args);}
        if let Some(home)=&l.home {
            command.env(config.home_var,home);
            if let Some(arg)=config.home_arg {command.arg(arg).arg(home);}
        }
        capture::inherit_caller(&mut command);
        if let Some(parent)=l.log.parent() {std::fs::create_dir_all(parent).map_err(|e|format!("ACP log directory: {e}"))?;}
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(&l.log).map_err(|e|format!("ACP log: {e}"))?;
        let boundary=log.metadata().map_err(|e|format!("ACP log metadata: {e}"))?.len();
        let server=Stdio::spawn_stream(command,&l.log.with_extension("stderr"),&l.process)?;
        let sampler=Some(capture::Sampler::start(server.id()));
        let mut s=Self {server,path:l.log.clone(),boundary,log,session:resume.unwrap_or("").into(),pending:vec![],requests:BTreeMap::new(),serial:0,prompt:None,
            report:None,text:String::new(),started:Instant::now(),started_ms:unix_ms(),closed:None,deadline:l.deadline,sampler,question_prefixes:config.question_prefixes};
        let init=s.exchange("initialize",json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}),false)?;
        if init["protocolVersion"]!=1 {return Err("ACP protocol version unsupported".into());}
        if init["agentInfo"]["version"].as_str().is_some_and(|v|config.rejected_versions.contains(&v)) {return Err("ACP server version refused: workspace write defect".into());}
        s.pending.push(Event::Metadata{kind:"capabilities".into(),data:init.clone()});
        let method=if resume.is_some() {"session/load"}else{"session/new"};
        if resume.is_some() && init["agentCapabilities"]["loadSession"]!=true && init["agentCapabilities"]["load_session"]!=true {
            return Err("ACP resume unsupported by server".into());
        }
        let mut params=json!({"cwd":l.root,"mcpServers":[]});
        if let Some(id)=resume {params["sessionId"]=json!(id);}
        if let Some(path)=&l.mcp {
            let v:Value=serde_json::from_str(&std::fs::read_to_string(path).map_err(|_|"mcp config unreadable")?).map_err(|_|"mcp config invalid")?;
            let servers=v["mcpServers"].as_object().ok_or("mcp config names no servers")?;
            params["mcpServers"]=Value::Array(servers.iter().map(|(name,server)|json!({"name":name,"command":server["command"],"args":server.get("args").cloned().unwrap_or(json!([])),
                "env":server["env"].as_object().map(|env|env.iter().map(|(name,value)|json!({"name":name,"value":value})).collect::<Vec<_>>()).unwrap_or_default()})).collect());
        }
        let created=s.exchange(method,params,resume.is_some())?;
        if let Some(id)=created["sessionId"].as_str().filter(|s|!s.is_empty()) {
            if resume.is_some_and(|old|old!=id) {return Err("ACP loaded session differs".into());}
            s.session=id.into();
        }
        if s.session.is_empty() {return Err("ACP session ID missing".into());}
        s.pending.push(Event::Metadata{kind:"session".into(),data:created});
        if let Some(model)=&l.model {s.control(Control::Model(model.clone()))?;}
        if !matches!(l.policy.approval,Approval::Native|Approval::Bypass) {
            let mode=config.modes.iter().find(|(approval,_)|*approval==l.policy.approval).ok_or("ACP approval unsupported")?.1;
            s.control(Control::Mode(mode.into()))?;
        }
        let parts=if l.parts.is_empty() {vec![Part::Text(l.input.clone())]}else{l.parts.clone()};
        s.input(&parts,init["agentCapabilities"]["promptCapabilities"]["image"]==true)?;
        Ok(s)
    }

    fn write(&mut self, v: &Value) -> Res<()> {self.server.send_until(v,self.deadline)}
    fn request(&mut self, method:&str, params:Value)->Res<Value> {
        self.serial=self.serial.checked_add(1).ok_or("ACP request ID overflow")?;
        let request=json!({"jsonrpc":"2.0","id":self.serial,"method":method,"params":params});
        self.write(&request)?;
        writeln!(self.log,"{}",json!({"type":"acp","message":request,"outgoing":true,"replay":false}).to_string()).map_err(|e|format!("ACP transcript: {e}"))?;
        Ok(request["id"].clone())
    }
    fn log(&mut self,v:&Value,replay:bool)->Res<()> {
        let terminal=self.prompt.as_ref()==v.get("id") && v.get("method").is_none();
        writeln!(self.log,"{}",json!({"type":"acp","message":v,"replay":replay,"terminal":terminal}).to_string()).map_err(|e|format!("ACP transcript: {e}"))
    }
    fn exchange(&mut self,method:&str,params:Value,replay:bool)->Res<Value> {
        let id=self.request(method,params)?;
        loop {
            let v=self.server.wait(self.deadline)?;validate(&v)?;
            if v["id"]==id && v.get("method").is_none() {
                self.log(&v,replay)?;
                if let Some(error)=v.get("error") {return Err(format!("ACP {method}: {error}"));}
                return Ok(v["result"].clone());
            }
            self.observe(v,replay)?;
        }
    }

    pub fn input(&mut self,parts:&[Part],images:bool)->Res<()> {
        if self.prompt.is_some() {return Err("ACP turn already running".into());}
        let mut prompt=vec![];
        for part in parts {prompt.push(match part {
            Part::Text(text)=>json!({"type":"text","text":text}),
            Part::Image(path)=>{
                if !images {return Err("image-unsupported".into());}
                if !path.is_absolute() {return Err("image-invalid".into());}
                let bytes=std::fs::read(path).map_err(|_|"image-invalid")?;
                let mime=match path.extension().and_then(|s|s.to_str()) {Some("png")=>"image/png",Some("gif")=>"image/gif",Some("webp")=>"image/webp",_=>"image/jpeg"};
                json!({"type":"image","mimeType":mime,"data":adapter::b64(&bytes)})
            }
        });}
        if prompt.is_empty() || parts.iter().all(|p|matches!(p,Part::Text(t) if t.is_empty())) {return Err("input-empty".into());}
        self.report=None;self.text.clear();self.started=Instant::now();self.started_ms=unix_ms();
        self.prompt=Some(self.request("session/prompt",json!({"sessionId":self.session,"prompt":prompt}))?);
        self.pending.push(Event::TurnStart);Ok(())
    }

    fn observe(&mut self,v:Value,replay:bool)->Res<()> {
        validate(&v)?;self.log(&v,replay)?;
        if replay {
            if v.get("id").is_some() && v["method"].is_string() {
                self.write(&json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Replay cannot request client actions"}}))?;
            }
            return Ok(());
        }
        if let Some(method)=v["method"].as_str() {
            if v.get("id").is_some() {
                if method=="session/request_permission" {
                    if v["params"]["sessionId"].as_str()!=Some(self.session.as_str()) {return Err("ACP request session differs".into());}
                    let params=&v["params"];
                    let choices=params["options"].as_array().ok_or("ACP permission choices missing")?.iter().map(|o|Ok(Choice {
                        id:o["optionId"].as_str().ok_or("ACP choice ID missing")?.into(),label:o["name"].as_str().ok_or("ACP choice name missing")?.into(),kind:o["kind"].as_str().ok_or("ACP choice kind missing")?.into(),
                    })).collect::<Res<Vec<_>>>()?;
                    let question=params["toolCall"]["toolCallId"].as_str().is_some_and(|id|self.question_prefixes.iter().any(|prefix|id.starts_with(prefix)));
                    self.requests.insert(v["id"].to_string(),v["params"].clone());
                    self.pending.push(Event::Request{id:v["id"].clone(),kind:if question {RequestKind::Question}else{RequestKind::Approval},prompt:params["toolCall"]["title"].as_str().map(String::from),choices});
                } else {self.write(&json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Client method not supported"}}))?;}
                return Ok(());
            }
            if method=="session/update" {
                if v["params"]["sessionId"].as_str()!=Some(self.session.as_str()) {return Err("ACP update session differs".into());}
                let update=&v["params"]["update"];
                if update["sessionUpdate"]=="agent_message_chunk" {
                    if let Some(text)=update["content"]["text"].as_str() {self.text.push_str(text);}
                }
                self.pending.extend(update_events(update));
            } else {self.pending.push(Event::Metadata{kind:method.into(),data:v});}
        } else if self.prompt.as_ref()==v.get("id") {
            let mut r=Report::empty();r.session=Some(self.session.clone());r.started_ms=self.started_ms;r.duration_us=self.started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
            r.text=(!self.text.is_empty()).then(||self.text.clone());
            if let Some(error)=v.get("error") {r.outcome="failed";r.error="client-error";r.detail=Some(error.to_string());}
            else {
                (r.outcome,r.error)=outcome(&v);
                let u=&v["result"]["usage"];
                r.usage=read_usage(u);
            }
            self.pending.push(Event::Message{item:None,text:self.text.clone()});self.pending.push(Event::TurnEnd{outcome:r.outcome});
            self.prompt=None;self.requests.clear();self.report=Some(r);
        } else {self.pending.push(Event::Metadata{kind:"response".into(),data:v});}
        Ok(())
    }

    pub fn pump(&mut self)->Res<()> {
        if self.closed.is_some() {return Ok(());}
        loop {
            match self.server.receive(Instant::now()) {
                Ok(Some(v))=>self.observe(v,false)?,Ok(None)=>break,
                Err(e)=>{self.closed=Some(e);break;}
            }
        }
        if self.report.is_some() {
            self.server.close_stream(self.deadline)?;self.closed=Some("terminal".into());
            if let Some(report)=&mut self.report {
                report.exit=self.server.poll()?;
                if let Some(sampler)=self.sampler.take() {report.process=sampler.usage();}
            }
        }
        Ok(())
    }
    pub fn poll(&mut self)->Res<State> {
        self.pump()?;
        if let Some(r)=&self.report {return Ok(match r.outcome {"ok"=>State::Finished,"unknown"=>State::Unknown,_=>State::Failed});}
        if self.deadline.is_some_and(|d|Instant::now()>=d) {self.server.kill()?;self.closed=Some("caller deadline; native turn outcome unknown".into());}
        Ok(if self.closed.is_some(){State::Unknown}else{State::Running})
    }
    pub fn report(&self)->Res<Report> {self.report.clone().ok_or_else(||self.closed.clone().unwrap_or_else(||"running".into()))}
    pub fn progress(&mut self)->Res<Vec<Event>> {self.pump()?;Ok(std::mem::take(&mut self.pending))}
    pub fn id(&self)->u32 {self.server.id()}
    pub fn cancel(&mut self)->Res<bool> {
        if self.report.is_some() {return Ok(true);}
        self.write(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.session}}))?;Ok(false)
    }
    pub fn control(&mut self,control:Control)->Res<()> {
        match control {
            Control::Model(model)=>{self.exchange("session/set_model",json!({"sessionId":self.session,"modelId":model}),false)?;},
            Control::Mode(mode)=>{self.exchange("session/set_mode",json!({"sessionId":self.session,"modeId":mode}),false)?;},
            Control::Reply{id,option}=>{
                let request=self.requests.get(&id.to_string()).ok_or("request-unknown")?;
                if option.as_ref().is_some_and(|id|!request["options"].as_array().into_iter().flatten().any(|o|o["optionId"].as_str()==Some(id))) {return Err("request-option-invalid".into());}
                let outcome=option.map_or_else(||json!({"outcome":"cancelled"}),|id|json!({"outcome":"selected","optionId":id}));
                self.write(&json!({"jsonrpc":"2.0","id":id,"result":{"outcome":outcome}}))?;self.requests.remove(&id.to_string());
            }
        }Ok(())
    }
}

fn outcome(v:&Value)->(&'static str,&'static str) {
    if v.get("error").is_some() {return ("failed","client-error");}
    match v["result"]["stopReason"].as_str() {
        Some("end_turn")=>("ok","none"),Some("cancelled")=>("cancelled","turn-cancelled"),
        Some("max_tokens"|"max_turn_requests"|"refusal")=>("refused","native-stop"),_=>("unknown","native-stop-unknown"),
    }
}

pub(super) fn record_events(record:&Value)->Vec<Event> {
    if record["replay"]==true {return vec![];}
    let m=&record["message"];
    if record["outgoing"]==true {return if m["method"]=="session/prompt" {vec![Event::TurnStart]}else{vec![]};}
    if record["terminal"]==true {return vec![Event::TurnEnd{outcome:outcome(m).0}];}
    if m["method"]=="session/update" {return update_events(&m["params"]["update"]);}
    vec![Event::Metadata{kind:m["method"].as_str().unwrap_or("response").into(),data:m.clone()}]
}

pub(super) fn validate_launch(config:&Config,l:&Launch)->Res<()> {
    let p=&l.policy;
    for (part,unsupported) in [("read",p.read!=Scope::Native),("write",p.write!=Scope::Native),("network",p.network!=Network::Native),
        ("tools",p.tools.is_some()),("roots",p.roots!=Roots::Native),("unelevated",p.unelevated),("builtin",p.builtin!=Builtin::Native),
        ("isolated",p.isolated),("subscription",p.subscription)] {if unsupported {return Err(format!("policy {part} unsupported by ACP"));}}
    if p.approval!=Approval::Native && !(p.approval==Approval::Bypass && !config.bypass_args.is_empty()) && !config.modes.iter().any(|(a,_)|*a==p.approval) {return Err("policy approval unsupported by ACP".into());}
    if l.effort.is_some() || l.title.is_some() || l.compact_hook.is_some() || l.instructions.is_some() || !l.host_args.is_empty() || !l.settings_env.is_empty() {
        return Err("launch option unsupported by ACP".into());
    }
    Ok(())
}

pub(super) fn update_events(u:&Value)->Vec<Event> {
    let text=u["content"]["text"].as_str();
    let id=u["toolCallId"].as_str().map(String::from);
    match u["sessionUpdate"].as_str() {
        Some("agent_message_chunk") if text.is_some()=>vec![Event::Text{item:None,delta:text.unwrap().into()}],
        Some("tool_call")=>vec![Event::ToolStart{id,kind:match u["kind"].as_str(){Some("execute")=>ToolKind::Shell,Some("fetch"|"search")=>ToolKind::Web,_=>ToolKind::Other},name:u["title"].as_str().map(String::from),detail:None,timeout_ms:None}],
        Some("tool_call_update") if matches!(u["status"].as_str(),Some("completed"|"failed"))=>vec![Event::ToolEnd{id,status:u["status"].as_str().map(String::from)}],
        Some("usage_update")=>vec![Event::Context{used:u["used"].as_u64(),window:u["size"].as_u64()}],
        _=>vec![Event::Metadata{kind:u["sessionUpdate"].as_str().unwrap_or("unknown").into(),data:u.clone()}],
    }
}


fn read_usage(u:&Value)->Usage {
    let mut usage=Usage{input:u["inputTokens"].as_u64(),output:u["outputTokens"].as_u64(),cache_read:u["cachedReadTokens"].as_u64(),cache_write:u["cachedWriteTokens"].as_u64(),reasoning:u["thoughtTokens"].as_u64(),..Usage::default()};
    usage.unknown=(usage.input.is_none() && usage.output.is_none() && usage.cache_read.is_none() && usage.cache_write.is_none() && usage.reasoning.is_none()).then_some("not-reported");usage
}

pub(super) fn read_report(_: &str,events:&[Value],_:Option<&Path>)->Report {
    let mut r=Report::empty();
    r.session=events.iter().rev().find_map(|v|v["message"]["params"]["sessionId"].as_str().or(v["message"]["result"]["sessionId"].as_str()).map(String::from));
    let start=events.iter().rposition(|v|v["outgoing"]==true && v["message"]["method"]=="session/prompt").unwrap_or(0);
    let current=&events[start..];
    let chunks:Vec<_>=current.iter().filter(|v|v["replay"]!=true && v["outgoing"]!=true && v["message"]["params"]["update"]["sessionUpdate"]=="agent_message_chunk")
        .filter_map(|v|v["message"]["params"]["update"]["content"]["text"].as_str()).collect();
    r.text=(!chunks.is_empty()).then(||chunks.concat());
    if let Some(end)=current.iter().rev().find(|v|v["terminal"]==true && v["replay"]!=true) {
        let m=&end["message"];(r.outcome,r.error)=outcome(m);r.usage=read_usage(&m["result"]["usage"]);
        r.detail=m.get("error").map(Value::to_string);
    }r
}
