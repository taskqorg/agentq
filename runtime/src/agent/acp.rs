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

pub(super) fn check_version(config:&Config,client:&Check)->Res<()> {
    if config.rejected_versions.is_empty() {return Ok(());}
    if client.state!="ok" {return Err("ACP version unknown; server not started".into());}
    let have=adapter::version_parts(&client.detail).ok_or("ACP version unknown; server not started")?;
    if config.rejected_versions.iter().any(|v|adapter::version_parts(v).as_ref()==Some(&have)) {return Err("ACP server version refused: workspace write defect".into());}Ok(())
}

// These are the protocol fields consumed below; extension fields stay in the raw value.
#[allow(dead_code)]
mod wire {
    use serde::Deserialize;
    #[derive(Deserialize)] #[serde(rename_all="camelCase")]
    pub struct Initialize {pub protocol_version:u64,pub agent_info:Option<Info>,#[serde(default)] pub agent_capabilities:Capabilities}
    #[derive(Deserialize)] pub struct Info {pub version:Option<String>,pub name:Option<String>}
    #[derive(Default,Deserialize)] #[serde(rename_all="camelCase")]
    pub struct Capabilities {#[serde(default)] pub load_session:bool,#[serde(default,rename="load_session")] pub native_load_session:bool,#[serde(default)] pub prompt_capabilities:PromptCapabilities}
    #[derive(Default,Deserialize)] pub struct PromptCapabilities {#[serde(default)] pub image:bool}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Session {pub session_id:Option<String>}
    #[derive(Deserialize)] pub struct Error {pub code:i64,pub message:String}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Complete {pub stop_reason:String,pub usage:Option<Usage>}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")]
    pub struct Usage {pub input_tokens:Option<u64>,pub output_tokens:Option<u64>,pub cached_read_tokens:Option<u64>,pub cached_write_tokens:Option<u64>,pub thought_tokens:Option<u64>}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Permission {pub session_id:String,pub tool_call:Tool,pub options:Vec<Choice>}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Tool {pub tool_call_id:String,pub title:Option<String>,pub kind:Option<String>,pub status:Option<String>}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Choice {pub option_id:String,pub name:String,pub kind:String}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Update {pub session_id:String,pub update:Change}
    #[derive(Deserialize)] #[serde(rename_all="camelCase")] pub struct Change {pub session_update:String,pub tool_call_id:Option<String>,pub kind:Option<String>,pub title:Option<String>,pub status:Option<String>,pub used:Option<u64>,pub size:Option<u64>}
    #[derive(Deserialize)] pub struct Content {#[serde(rename="type")] pub kind:String,pub text:Option<String>}
}
fn checked<T:serde::de::DeserializeOwned>(value:&Value)->Res<T> {serde_json::from_value(value.clone()).map_err(|e|format!("ACP payload invalid: {e}"))}
fn result_fields(method:&str,value:&Value)->Res<()> {
    if !value.is_object(){return Err("ACP result must be an object".into());}
    match method {
        "initialize"=>{checked::<wire::Initialize>(value)?;},
        "session/new"|"session/load"=>{checked::<wire::Session>(value)?;},
        _=>{},
    }Ok(())
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
    } else if (m.id.is_none() && !(v.get("id")==Some(&Value::Null) && v.get("error").is_some())) || (v.get("result").is_some()==v.get("error").is_some()) {
        return Err("ACP response envelope invalid".into());
    }
    if let Some(error)=v.get("error") {checked::<wire::Error>(error)?;}
    Ok(())
}

// One checked interpretation for live input and retained readback; correlation is never a log marker.
fn incoming(v:&Value,session:Option<&str>,prompt:Option<&Value>)->Res<bool> {
    validate(v)?;
    match v["method"].as_str() {
        Some("session/update"|"session/request_permission")=>{
            if session.is_some_and(|id|v["params"]["sessionId"].as_str()!=Some(id)) {return Err("ACP message session differs".into());}
            if v["method"]=="session/request_permission" {
                let permission=checked::<wire::Permission>(&v["params"])?;
                if permission.options.is_empty() || permission.options.iter().any(|o|o.option_id.is_empty()) {return Err("ACP permission choices invalid".into());}
            } else {
                let params=checked::<wire::Update>(&v["params"])?;
                let update=&v["params"]["update"];
                match params.update.session_update.as_str() {
                    "agent_message_chunk"|"agent_thought_chunk"|"user_message_chunk"=>{checked::<wire::Content>(&update["content"])?;},
                    "tool_call"|"tool_call_update"=>{checked::<wire::Tool>(update)?;},
                    _=>{},
                }
            }
        },
        None if prompt.is_some() && prompt==v.get("id")=>{
            if v.get("error").is_none() {checked::<wire::Complete>(&v["result"])?;}
            return Ok(true);
        },
        _=>{},
    }Ok(false)
}

pub(super) struct Session {
    server: Stdio,
    pub path: PathBuf,
    pub boundary: u64,
    log: File,
    pub session: String,
    pending: Vec<Event>,
    requests: super::control::Requests,
    owner: SessionOwner,
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
    cancelling: bool,
}

impl Drop for Session {fn drop(&mut self) {let _=self.requests.retire(&mut self.log,&self.session,"unknown");let _=self.server.kill();}}

impl Session {
    pub fn open(mut command: Command, config: &Config, l: &Launch, resume: Option<&str>,owner:&SessionOwner) -> Res<Self> {
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
        let mut s=Self {server,path:l.log.clone(),boundary,log,session:resume.unwrap_or("").into(),pending:vec![],requests:super::control::Requests::new(),owner:owner.clone(),serial:0,prompt:None,
            report:None,text:String::new(),started:Instant::now(),started_ms:unix_ms(),closed:None,deadline:l.deadline,sampler,question_prefixes:config.question_prefixes,cancelling:false};
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
        owner.bind(&s.session)?;
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

    fn write(&mut self, v: &Value) -> Res<()> {
        self.server.send_until(v,self.deadline)?;
        writeln!(self.log,"{}",json!({"type":"acp","message":v,"outgoing":true,"replay":false}).to_string()).map_err(|e|format!("ACP transcript: {e}"))
    }
    fn request(&mut self, method:&str, params:Value)->Res<Value> {
        self.serial=self.serial.checked_add(1).ok_or("ACP request ID overflow")?;
        let request=json!({"jsonrpc":"2.0","id":self.serial,"method":method,"params":params});
        self.write(&request)?;
        Ok(request["id"].clone())
    }
    fn log(&mut self,v:&Value,replay:bool,terminal:bool)->Res<()> {
        writeln!(self.log,"{}",json!({"type":"acp","message":v,"replay":replay,"terminal":terminal}).to_string()).map_err(|e|format!("ACP transcript: {e}"))
    }
    fn exchange(&mut self,method:&str,params:Value,replay:bool)->Res<Value> {
        let id=self.request(method,params)?;
        loop {
            let v=self.server.wait(self.deadline)?;
            if let Err(error)=validate(&v) {self.log(&v,replay,false)?;return Err(error);}
            if v.get("id")==Some(&Value::Null) {self.log(&v,replay,false)?;self.pending.push(Event::Metadata{kind:"protocol-error".into(),data:v});continue;}
            if v["id"]==id && v.get("method").is_none() {
                self.log(&v,replay,false)?;
                if let Some(error)=v.get("error") {return Err(format!("ACP {method}: {error}"));}
                result_fields(method,&v["result"])?;
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
        self.cancelling=false;self.report=None;self.text.clear();self.started=Instant::now();self.started_ms=unix_ms();
        self.prompt=Some(self.request("session/prompt",json!({"sessionId":self.session,"prompt":prompt}))?);
        self.pending.push(Event::TurnStart);Ok(())
    }

    fn observe(&mut self,v:Value,replay:bool)->Res<()> {
        let interpretation=incoming(&v,Some(&self.session),self.prompt.as_ref());
        self.log(&v,replay,!replay && interpretation.as_ref()==Ok(&true))?;
        let terminal=interpretation?;
        if v.get("id")==Some(&Value::Null) {self.pending.push(Event::Metadata{kind:"protocol-error".into(),data:v});return Ok(());}
        if replay {
            if v.get("id").is_some() && v["method"].is_string() {
                self.write(&json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Replay cannot request client actions"}}))?;
            }
            return Ok(());
        }
        if let Some(method)=v["method"].as_str() {
            if v.get("id").is_some() {
                if method=="session/request_permission" {
                    let params=&v["params"];
                    let choices=params["options"].as_array().ok_or("ACP permission choices missing")?.iter().map(|o|Ok(Choice {
                        id:o["optionId"].as_str().ok_or("ACP choice ID missing")?.into(),label:o["name"].as_str().ok_or("ACP choice name missing")?.into(),kind:o["kind"].as_str().ok_or("ACP choice kind missing")?.into(),
                    })).collect::<Res<Vec<_>>>()?;
                    let question=params["toolCall"]["toolCallId"].as_str().is_some_and(|id|self.question_prefixes.iter().any(|prefix|id.starts_with(prefix)));
                    let event=Event::Request{id:v["id"].clone(),kind:if question {RequestKind::Question}else{RequestKind::Approval},prompt:params["toolCall"]["title"].as_str().map(String::from),choices,questions:vec![]};
                    let event=self.requests.register(&mut self.log,&self.session,event,v.clone(),self.prompt.as_ref().map(Value::to_string))?;
                    if self.cancelling {self.cancel_requests()?;return Ok(());}
                    self.pending.push(event);
                } else {self.write(&json!({"jsonrpc":"2.0","id":v["id"],"error":{"code":-32601,"message":"Client method not supported"}}))?;}
                return Ok(());
            }
            if method=="session/update" {
                let update=&v["params"]["update"];
                if update["sessionUpdate"]=="agent_message_chunk" {
                    if let Some(text)=update["content"]["text"].as_str() {self.text.push_str(text);}
                }
                self.pending.extend(update_events(update));
            } else {self.pending.push(Event::Metadata{kind:method.into(),data:v});}
        } else if terminal {
            let mut r=Report::empty();r.session=Some(self.session.clone());r.started_ms=self.started_ms;r.duration_us=self.started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
            r.text=(!self.text.is_empty()).then(||self.text.clone());
            if let Some(error)=v.get("error") {r.outcome="failed";r.error="client-error";r.detail=Some(error.to_string());}
            else {
                (r.outcome,r.error)=outcome(&v);
                let u=&v["result"]["usage"];
                r.usage=read_usage(u);
            }
            self.pending.push(Event::Message{item:None,text:self.text.clone()});self.pending.push(Event::TurnEnd{outcome:r.outcome});
            self.requests.retire(&mut self.log,&self.session,"cancelled")?;self.prompt=None;self.report=Some(r);
        } else {self.pending.push(Event::Metadata{kind:"response".into(),data:v});}
        Ok(())
    }

    pub fn pump(&mut self)->Res<()> {
        if self.closed.is_some() {
            if let Some(report)=&mut self.report {self.server.observe_exit(report)?;}
            return Ok(());
        }
        loop {
            match self.server.receive(Instant::now()) {
                Ok(Some(v))=>if let Err(error)=self.observe(v,false) {self.closed=Some(error.clone());let _=self.requests.retire(&mut self.log,&self.session,"unknown");return Err(error);},Ok(None)=>break,
                Err(e)=>{self.closed=Some(e);self.requests.retire(&mut self.log,&self.session,"unknown")?;break;}
            }
        }
        if self.report.is_some() {
            self.server.close_stream();self.closed=Some("terminal".into());
            if let Some(report)=&mut self.report {
                if let Some(sampler)=self.sampler.take() {report.process=sampler.usage();}
                self.server.observe_exit(report)?;
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
    pub fn stop_facts(&mut self,run:String)->Res<super::history::stop::Facts> {
        let exit=self.server.poll()?;
        Ok(super::history::stop::Facts {run,format:"acp",path:self.path.clone(),start:self.boundary,exited:Some(exit.is_some()),native_end:self.report.is_some(),exit,lost_owner:false})
    }
    pub fn progress(&mut self)->Res<Vec<Event>> {self.pump()?;Ok(std::mem::take(&mut self.pending))}
    pub fn id(&self)->u32 {self.server.id()}
    pub fn run_id(&self)->&str {&self.requests.run}
    pub fn cancel(&mut self)->Res<bool> {
        if let Some(report)=&mut self.report {
            self.server.kill()?;self.server.observe_exit(report)?;
            return Ok(matches!(report.termination,ProcessTermination::Observed));
        }
        self.cancelling=true;
        self.write(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.session}}))?;
        self.cancel_requests()?;Ok(false)
    }
    fn cancel_requests(&mut self)->Res<()> {
        for id in self.requests.ids() {
            self.control(Control::Reply{id,option:None})?;
        }
        Ok(())
    }
    pub fn control(&mut self,control:Control)->Res<()> {
        self.owner.control()?;
        if self.closed.is_some() || self.report.is_some() {return Err("control-run-ended".into());}
        match control {
            Control::Model(model)=>{self.exchange("session/set_model",json!({"sessionId":self.session,"modelId":model}),false)?;},
            Control::Mode(mode)=>{self.exchange("session/set_mode",json!({"sessionId":self.session,"modeId":mode}),false)?;},
            Control::Answers{..}=>return Err("question-answers-unsupported".into()),
            Control::Reply{id,option}=>{
                let control=Control::Reply{id,option:option.clone()};
                let outcome=option.map_or_else(||json!({"outcome":"cancelled"}),|id|json!({"outcome":"selected","optionId":id}));
                let response=json!({"jsonrpc":"2.0","id":self.requests.native(&control)?["id"],"result":{"outcome":outcome}});
                let request=self.requests.begin(&mut self.log,&self.session,&control,&response)?;
                if let Err(error)=self.write(&response) {self.requests.fail(&mut self.log,&self.session,&request);return Err(error);}
                self.requests.sent(&mut self.log,&self.session,&request,&response)?;
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

#[derive(Default,serde::Serialize,serde::Deserialize)]
pub(crate) struct ReadState {prompt:Option<Value>,session:Option<String>}
impl ReadState {
    pub(super) fn record_events(&mut self,record:&Value)->Vec<Event> {
        let m=&record["message"];
        if record["outgoing"]==true {
            if record["replay"]!=true && m["method"]=="session/prompt" && validate(m).is_ok() {
                self.prompt=m.get("id").filter(|id|!id.is_null()).cloned();
                self.session=m["params"]["sessionId"].as_str().map(String::from);
                return vec![Event::TurnStart];
            }return vec![];
        }
        if record["replay"]==true {return vec![];}
        match incoming(m,self.session.as_deref(),self.prompt.as_ref()) {
            Err(_)=>return vec![Event::Metadata{kind:"invalid".into(),data:m.clone()}],
            Ok(true)=>{self.prompt=None;return vec![Event::TurnEnd{outcome:outcome(m).0}];},
            Ok(false)=>{},
        }
        if m["method"]=="session/update" {return update_events(&m["params"]["update"]);}
        vec![Event::Metadata{kind:if m.get("id")==Some(&Value::Null) {"protocol-error"}else{m["method"].as_str().unwrap_or("response")}.into(),data:m.clone()}]
    }
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
    let mut state=ReadState::default();
    let mut text=String::new();let mut ended=false;
    for record in events {
        for event in state.record_events(record) {
            match event {
                Event::TurnStart=>{text.clear();r=Report{session:state.session.clone(),..Report::empty()};ended=false;},
                Event::Text{delta,..} if !ended=>text.push_str(&delta),
                Event::TurnEnd{..} if !ended=>{
                    let m=&record["message"];(r.outcome,r.error)=outcome(m);r.usage=read_usage(&m["result"]["usage"]);
                    r.detail=m.get("error").map(Value::to_string);ended=true;
                },
                _=>{},
            }
        }
    }
    r.text=(!text.is_empty()).then_some(text);r
}

pub(super) fn normalize(v:&Value)->super::history::Normalized {
    if v["replay"]==true {return ("system","replay",Value::Null,vec![],"none");}
    let m=&v["message"];let u=&m["params"]["update"];let item=u["toolCallId"].clone();
    match u["sessionUpdate"].as_str() {
        Some("agent_message_chunk"|"agent_thought_chunk"|"user_message_chunk")=>(if u["sessionUpdate"]=="user_message_chunk" {"user"}else{"assistant"},if u["sessionUpdate"]=="agent_thought_chunk" {"thinking"}else{"message"},item,vec![u["content"].clone()],"none"),
        Some("tool_call"|"tool_call_update")=>("tool","tool",item,vec![u.clone()],"none"),
        Some("usage_update")=>("system","usage",item,vec![u.clone()],"none"),
        _ if m["method"]=="session/request_permission"=>("system","approval",m["id"].clone(),vec![m["params"].clone()],"none"),
        _=>("unknown","unknown",item,vec![],"unsupported-source-shape"),
    }
}

#[cfg(test)] mod wire_tests {
    use super::*;
    #[test] fn retained_terminal_requires_checked_prompt_correlation() {
        let prompt=json!({"type":"acp","outgoing":true,"message":{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"s"}}});
        let complete=json!({"type":"acp","terminal":true,"message":{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn","usage":{"inputTokens":17}}}});
        for (field,value) in [("usage",json!({"inputTokens":"bad"})),("usage",json!({"thoughtTokens":-1})),("stopReason",json!(true))] {
            let mut rejected=complete.clone();rejected["message"]["result"][field]=value;
            let mut state=ReadState::default();state.record_events(&prompt);
            assert!(!state.record_events(&rejected).iter().any(|e|matches!(e,Event::TurnEnd{..})));
            let report=read_report("",&[prompt.clone(),rejected],None);assert_ne!(report.outcome,"ok");assert_eq!(report.usage.input,None);
        }
        for rejected in [complete.clone(),{let mut v=complete.clone();v["message"]["id"]=json!(99);v},{let mut v=complete.clone();v["replay"]=json!(true);v}] {
            let mut state=ReadState::default();
            if rejected!=complete {state.record_events(&prompt);}
            assert!(!state.record_events(&rejected).iter().any(|e|matches!(e,Event::TurnEnd{..})),"a marker is not correlation");
        }
        let mut no_marker=complete.clone();no_marker["terminal"]=json!(false);
        assert_eq!(read_report("",&[prompt.clone(),no_marker],None).outcome,"ok","checked correlation supplies authority");
        let mut later=complete.clone();later["message"]["result"]["stopReason"]=json!("cancelled");
        assert_eq!(read_report("",&[prompt,complete,later],None).outcome,"ok","the first checked settlement stays frozen");
    }
    #[test] fn consumed_protocol_fields_are_checked() {
        assert!(validate(&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}})).is_ok());
        assert!(validate(&json!({"jsonrpc":"2.0","id":null,"result":{}})).is_err());
        assert!(validate(&json!({"jsonrpc":"2.0","id":null,"error":{"code":"bad","message":"x"}})).is_err());
        for malformed in [json!({"protocolVersion":"1"}),json!({"protocolVersion":1,"agentCapabilities":{"loadSession":"yes"}}),json!({"protocolVersion":1,"agentInfo":{"version":4}}),json!({"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":1}}})] {assert!(result_fields("initialize",&malformed).is_err());}
        assert!(result_fields("session/new",&json!({"sessionId":3})).is_err());
        assert!(checked::<wire::Complete>(&json!({"stopReason":true})).is_err());
        assert!(checked::<wire::Complete>(&json!({"stopReason":"end_turn","usage":{"inputTokens":"17"}})).is_err());
        assert!(checked::<wire::Update>(&json!({"sessionId":"s","update":{"sessionUpdate":"usage_update","used":"1"}})).is_err());
        assert!(checked::<wire::Content>(&json!({"type":"text","text":false})).is_err());
        assert!(checked::<wire::Permission>(&json!({"sessionId":"s","toolCall":{"toolCallId":5},"options":[]})).is_err());
        assert!(checked::<wire::Initialize>(&json!({"protocolVersion":1,"future":{"preserved":true}})).is_ok());
        assert!(checked::<wire::Initialize>(&json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"load_session":true}})).is_ok());
    }
}
pub(super) static PROTOCOL: registry::Protocol=registry::Protocol {
    stream_history:"acp",native_history:"acp",
    ends:|_|false,start_limit:|_|None,read_calls:|_|(vec![],None,false),history_usage:|_|vec![],history_context:|_|None,final_text:|events|read_report("",events,None).text,
    history_alias:false,history_provenance:|_,_,_|false,history_parent:|_|None,rollout_report:|_,_|{},history_formats:&["acp"],
    history_session:|_,v,_|v["message"]["params"]["sessionId"].as_str().or(v["message"]["result"]["sessionId"].as_str()).map(String::from),history_normalize:|_,v|normalize(v),history_validity:|v|json!({"status":if validate(&v["message"]).is_ok(){"valid"}else{"invalid"}}),
    prepare_command:|_,_,_|Err("ACP requires the structured launch path".into()),input_bytes:|_|vec![],
    native_record:|_,_|Err("native transcript polling unsupported".into()),native_metadata:None,finish_native_report:|_,_|{},session_report:|_,_|{},
    reported:|_,_|None,update_limits:|_,_,_|false,account_limits:|_,_,_|limits::unknown("account-limits unsupported"),identity:|_|None,
    credentials:|_,_|Err("oauth-unavailable".into()),oauth_token:|_,_|Err("oauth-unavailable".into()),oauth_usage:|_|Err("oauth-unavailable".into()),
    turn_ended:|ended,_|ended,parse_event:|cursor,v,out,_|out.extend(cursor.acp.record_events(v)),
    terminal:|_,_,_,_,_|None,assistant_failure:|_|None,permission_mode:|_|None,lifecycle:|_|None,advances:|_|false,
    session:|text|text.lines().filter_map(|l|serde_json::from_str::<Value>(l).ok()).find_map(|v|v["message"]["params"]["sessionId"].as_str().or(v["message"]["result"]["sessionId"].as_str()).map(String::from)),
    snapshot:|_,_|Snapshot{unknown:Some("not-reported"),..Snapshot::default()},report:read_report,
};
