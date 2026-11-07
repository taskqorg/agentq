//! App-server protocol state over caller-owned transport. UI, tool execution and approvals stay with the caller.
use super::*;
pub(crate) use crate::agent::stdio::Stdio as AppServer;
use crate::agent::stdio::NativeFrame;

pub(crate) struct SessionLine {pub(crate) session:String,pub(crate) turn:Option<String>,pub(crate) raw:Vec<u8>}

/// Caller-owned server transport: preserve all policy arguments and the account, without CLI `exec` flags.
pub(crate) fn app_server_command(mut base: Command, spec: &Spec) -> Res<Command> {
    if spec.client != Client::Codex {return Err("app-server requires Codex".into());}
    base.arg("app-server").args(&spec.args);
    Ok(environment(base,spec))
}

/// One app-server session. Feed every native response/notification to `observe`; forward unhandled server requests
/// to the caller's existing tool/approval policy. This state machine never approves a server request.
pub(crate) struct AppSession {
    spec: Spec,
    next: u64,
    pending: Option<(u64, &'static str)>,
    initialized: bool,
    ready: bool,
    thread: Option<String>,
    turn: Option<String>,
    active: bool,
    terminal: Option<String>,
    model: Option<String>,
    text: Option<String>,
    usage: Vec<Value>,
    limits: Option<Value>,
    latest_limits: Option<Value>,
    windows: Vec<Window>,
    latest_windows: Vec<Window>,
    window_groups: BTreeMap<String, Vec<Window>>,
    started_ms: i64,
    /// Samples the plain server process; None for a protected or recovered session.
    sampler: Option<super::capture::Sampler>,
    /// The sampler's reading frozen at the terminal event, so a repeated report reads the same record.
    process: Option<super::capture::ProcessUse>,
    early: Vec<NativeFrame>,
    fault: Option<String>,
    scope: String,
    worked: bool,
    failure: Option<Value>,
}

impl AppSession {
    pub(crate) fn history_home(&self)->Option<PathBuf> {spec_home(&self.spec)}
    pub fn new(spec: &Spec) -> Res<Self> {
        if spec.client != Client::Codex || spec.resume && (!native::plain_id(&spec.session) || spec.session == "-") {
            return Err("app-server session invalid".into());
        }
        Ok(Self { spec: spec.clone(), next: 1, pending: None, initialized: false, ready: false,
            thread: None, turn: None, active: false, terminal: None, model: None, text: None, usage: vec![], limits: None, latest_limits:None, started_ms: 0, sampler: None, process: None,
            early:vec![],fault:None,scope:native::scope(spec),worked:false,failure:None, windows:limits::unknown("not-reported"), latest_windows:limits::unknown("not-reported"), window_groups:BTreeMap::new() })
    }

    /// Native catalogue/account methods share the same transport; caller handlers consume their raw responses.
    pub fn request(&mut self, method: &'static str, params: Value) -> Res<Value> {
        if self.pending.is_some() || self.fault.is_some() { return Err("app-server request pending or unknown; do not resubmit".into()); }
        if !params.is_object() { return Err("app-server params must be an object".into()); }
        let id = self.next; self.next=self.next.checked_add(1).ok_or("app-server request identity exhausted")?; self.pending = Some((id, method));
        Ok(json!({"id":id,"method":method,"params":params}))
    }

    pub fn initialize(&mut self, params: Value) -> Res<Value> {
        if self.initialized { return Err("app-server already initialized".into()); }
        self.request("initialize", params)
    }

    pub fn initialized(&self) -> Res<Value> {
        if !self.initialized { return Err("app-server initialization unconfirmed".into()); }
        Ok(json!({"method":"initialized"}))
    }

    /// Caller params, including model/provider/permissions/instructions/tools, pass through unchanged.
    pub fn open(&mut self, mut params: Value) -> Res<Value> {
        if !self.initialized || self.thread.is_some() { return Err("app-server thread not ready to open".into()); }
        if !params.is_object() { return Err("app-server params must be an object".into()); }
        if self.spec.resume {
            if params.get("threadId").is_some_and(|v| *v != self.spec.session) { return Err("app-server resume identity differs".into()); }
            params["threadId"] = json!(self.spec.session);
        }
        self.request(if self.spec.resume { "thread/resume" } else { "thread/start" }, params)
    }

    pub fn ready(&self) -> bool { self.ready && self.fault.is_none() }
    pub fn session(&self) -> Option<&str> { self.thread.as_deref() }
    pub(crate) fn turn_id(&self) -> Option<&str> { self.turn.as_deref() }
    pub(crate) fn control_pending(&self)->bool {self.pending.is_some_and(|(_,method)|matches!(method,"turn/steer"|"thread/compact/start"))}

    pub(crate) fn control_scope(&self,value:&Value)->Res<()> {
        if !self.active || self.terminal.is_some() || self.fault.is_some() {return Err("control-turn-unavailable".into());}
        if self.thread.is_none() || self.turn.is_none() || value["params"]["threadId"].as_str()!=self.thread.as_deref() || value["params"]["turnId"].as_str()!=self.turn.as_deref() {return Err("control-native-request-scope-differs".into());}Ok(())
    }
    pub(crate) fn control_request(&self,value:&Value)->Res<Option<Event>> {
        self.control_scope(value)?;
        super::control::request(value,self.thread.as_deref().ok_or("control-session-unknown")?,self.turn.as_deref().ok_or("control-turn-unknown")?)
    }
    pub(crate) fn control_reply(&self,native:&Value,control:&Control)->Res<Value> {super::control::reply(native,control)}
    pub(crate) fn steer(&mut self,input:Value)->Res<Value> {
        if !self.active || self.terminal.is_some() {return Err("steer-turn-unavailable".into());}
        self.request("turn/steer",json!({"threadId":self.thread.as_deref().ok_or("steer-session-unknown")?,"expectedTurnId":self.turn.as_deref().ok_or("steer-turn-unknown")?,"input":input}))
    }
    pub(crate) fn compact(&mut self)->Res<Value> {
        if !self.ready() || self.active && self.terminal.is_none() {return Err("compact-turn-active".into());}
        self.request("thread/compact/start",json!({"threadId":self.thread.as_deref().ok_or("compact-session-unknown")?}))
    }

    /// Save the returned state before writing this request to the transport. A lost reply remains pending.
    pub fn start_turn(&mut self, mut params: Value) -> Res<Value> {
        if !self.ready() || self.active { return Err("app-server turn not ready; original submission retained".into()); }
        if !params.is_object() { return Err("app-server params must be an object".into()); }
        let thread = self.thread.as_deref().ok_or("app-server thread missing")?;
        if params.get("threadId").is_some_and(|v| v != thread) { return Err("app-server turn thread differs".into()); }
        params["threadId"] = json!(thread);
        let changed_model = params.get("model").is_some_and(|m| m.as_str() != self.model.as_deref());
        let request = self.request("turn/start", params)?;
        self.active = true; self.turn = None; self.terminal = None; self.text = None; self.usage.clear(); self.limits = self.latest_limits.clone(); self.started_ms = unix_ms();
        self.windows = self.latest_windows.clone();
        self.process = None;
        if let Some(sampler) = &self.sampler { sampler.mark(); }
        self.worked=false;self.failure=None;
        if changed_model {self.model=None;}
        Ok(request)
    }

    /// A request only. Matching terminal evidence is still required; no server process is killed.
    pub fn interrupt(&mut self) -> Res<Value> {
        if !self.active || self.terminal.is_some() { return Err("app-server turn is not active".into()); }
        let params = json!({"threadId":self.thread, "turnId":self.turn.as_ref().ok_or("app-server turn acceptance unknown")?});
        self.request("turn/interrupt", params)
    }

    pub fn observe(&mut self, message: &Value) -> Res<()> {self.observe_frame(NativeFrame::parsed(message.clone())).map(|_|())}

    pub(crate) fn observe_frame(&mut self, frame:NativeFrame)->Res<Vec<SessionLine>> {
        if let Some(error)=&self.fault {return Err(error.clone());}
        let mut selected=vec![];
        if let Err(error)=self.observe_inner(frame,&mut selected) {self.fault=Some(error.clone());return Err(error);}
        Ok(selected)
    }

    fn selected(&self,frame:NativeFrame,selected:&mut Vec<SessionLine>) {
        let p=&frame.value["params"];
        if p.get("thread").is_some_and(|v|v["id"].as_str()!=self.thread.as_deref())
            || p.get("turn").is_some_and(|v|v["id"].as_str()!=self.turn.as_deref())
            || p.get("turnId").is_some_and(|v|v.as_str()!=self.turn.as_deref()) {return;}
        if let (Some(raw),Some(session))=(frame.raw,self.thread.clone()) {selected.push(SessionLine {session,turn:self.turn.clone(),raw});}
    }

    fn observe_inner(&mut self, frame:NativeFrame,selected:&mut Vec<SessionLine>) -> Res<()> {
        let message=&frame.value;
        // Server requests belong to the caller, even when their id happens to equal a client request id.
        if message["method"].is_string() && message.get("id").is_some() {
            if self.control_scope(message).is_ok() {
                let p=&message["params"];
                let known=if message["method"]=="item/tool/call" {p["tool"].as_str().is_some_and(|s|!s.is_empty()) && p["arguments"].is_object() && (message["id"].is_string() || message["id"].is_i64() || message["id"].is_u64())}
                    else {matches!(self.control_request(message),Ok(Some(_)))};
                if known {self.selected(frame,selected);}
            }
            return Ok(());
        }
        if limits::update(Client::Codex, message, (unix_ms()/1000).max(0) as u64, &mut self.window_groups) {
            self.latest_windows = self.window_groups.values().flatten().cloned().collect();
            if self.active && self.terminal.is_none() { self.windows = self.latest_windows.clone(); }
        }
        if let Some((id, method)) = self.pending {
            if message["id"].as_u64() == Some(id) {
                if !message["error"].is_null() {
                    if matches!(method,"model/list"|"account/read"|"account/rateLimits/read") {
                        if method=="account/rateLimits/read" {
                            self.latest_limits=None;self.window_groups.clear();self.latest_windows=limits::unknown("native-read-refused");
                            if self.active && self.terminal.is_none() {self.limits=None;self.windows=self.latest_windows.clone();}
                        }
                        self.pending=None;return Ok(());
                    }
                    return Err(format!("app-server {method} refused; original session retained"));
                }
                let result = &message["result"];
                match method {
                    "initialize" => self.initialized = true,
                    "thread/start" | "thread/resume" => {
                        let thread = result["thread"]["id"].as_str().filter(|s| native::plain_id(s)).ok_or("app-server returned no thread")?;
                        if self.spec.resume && thread != self.spec.session { return Err("app-server resumed another session".into()); }
                        self.thread = Some(thread.into()); self.ready = true;
                        self.model = result["model"].as_str().filter(|s| model_code(s)).map(String::from);
                    }
                    "turn/start" => {
                        let turn = result["turn"]["id"].as_str().filter(|s| native::plain_id(s)).ok_or("app-server returned no turn")?;
                        if self.turn.as_deref().is_some_and(|old| old != turn) { return Err("app-server turn identity differs".into()); }
                        self.turn = Some(turn.into());
                    }
                    "turn/steer"=>{if result["turnId"].as_str()!=self.turn.as_deref() {return Err("app-server steer turn differs".into());}},
                    "thread/compact/start"=>{if !result.is_object() {return Err("app-server compact reply invalid".into());}},
                    "account/rateLimits/read" => {
                        let limits=if result["rateLimitsByLimitId"].is_object() {&result["rateLimitsByLimitId"]["codex"]} else {&result["rateLimits"]};
                        self.latest_limits=limits.is_object().then(||limits.clone());
                        if self.active && self.terminal.is_none() {self.limits=self.latest_limits.clone();}
                    }
                    _ => {}
                }
                self.pending = None;
                if method == "turn/start" {
                    for event in std::mem::take(&mut self.early) {self.observe_inner(event,selected)?;}
                }
                return Ok(());
            }
        }
        let p = &message["params"];
        let method = message["method"].as_str().unwrap_or("");
        if method == "account/rateLimits/updated" {
            self.latest_limits=p["rateLimits"].is_object().then(||p["rateLimits"].clone());
            if self.active && self.terminal.is_none() {self.limits=self.latest_limits.clone();}
            return Ok(());
        }
        if !self.active { return Ok(()); }
        if p["threadId"].as_str() != self.thread.as_deref() { return Ok(()); }
        let id = p["turnId"].as_str().or(p["turn"]["id"].as_str());
        if self.turn.is_none() && self.pending.is_some_and(|(_, m)| m == "turn/start") {
            if self.early.len()>=256 || self.early.iter().map(|e|e.value.to_string().len()).sum::<usize>()+message.to_string().len()>8*1024*1024 {
                return Err("app-server early notifications exceed bound; reconcile original submission".into());
            }
            self.early.push(frame);return Ok(());
        }
        if self.turn.is_none() || id != self.turn.as_deref() { return Ok(()); }
        let capturable=self.terminal.is_none() && match method {
            "item/started"|"item/completed"=>p["item"].is_object() && super::history::normalize("codex-app-server",message).4=="none",
            "item/agentMessage/delta"=>p["itemId"].as_str().is_some_and(|s|!s.is_empty()) && p["delta"].is_string(),
            "thread/tokenUsage/updated"=>p["tokenUsage"].is_object() && ["last","total"].iter().all(|k|p["tokenUsage"].get(*k).is_none_or(Value::is_object)),
            "turn/started"|"turn/completed"=>p["turn"].is_object(),
            "turn/plan/updated"=>p["plan"].is_array(),
            "model/rerouted"=>p["toModel"].as_str().is_some_and(model_code),
            "error"=>p["error"].is_object(),
            _=>false,
        };
        if self.terminal.is_none() && matches!(method,"item/started"|"item/completed")
            && p["item"]["type"].as_str().is_some_and(|s|s!="userMessage") {self.worked=true;}
        if method == "turn/completed" {
            let status = p["turn"]["status"].as_str().filter(|s| matches!(*s,"completed"|"failed"|"interrupted")).ok_or("app-server terminal status unknown")?;
            if self.terminal.as_deref().is_some_and(|old| old != status) { return Err("app-server terminal outcomes conflict".into()); }
            if self.terminal.is_none() && p["turn"]["error"].is_object() {self.failure=Some(p["turn"]["error"].clone());}
            if self.terminal.is_none() { self.process = self.sampler.as_ref().map(super::capture::Sampler::usage); }
            self.terminal = Some(status.into());
        } else if self.terminal.is_none() {
            match method {
                "thread/tokenUsage/updated" => {
                    let last=&p["tokenUsage"]["last"];
                    self.worked|=["inputTokens","outputTokens"].iter().any(|k|last[k].as_u64().is_some_and(|n|n>0));
                    super::append_app_usage(&mut self.usage,message);
                }
                "item/completed" if p["item"]["type"] == "agentMessage" => self.text = p["item"]["text"].as_str().map(String::from),
                "model/rerouted" => self.model = p["toModel"].as_str().filter(|s| model_code(s)).map(String::from),
                "error" => self.failure=p["error"].is_object().then(||p["error"].clone()),
                _ => {}
            }
        }
        if capturable {self.selected(frame,selected);}
        Ok(())
    }

    /// Sample the plain server process this session owns.
    pub(crate) fn sample(&mut self, pid: u32) { self.sampler = Some(super::capture::Sampler::start(pid)); }

    /// Native terminal report only. Transport EOF or process exit cannot manufacture a report.
    pub fn report(&self) -> Option<Report> {
        if self.fault.is_some() || self.turn.is_none() || self.pending.is_some_and(|(_,method)|method=="turn/start") {return None;}
        let status = self.terminal.as_deref()?;
        let mut events = self.usage.clone();
        if let Some(l) = self.limits.as_ref().filter(|l|l.is_object()) {
            let mut limits = l.clone();
            for key in ["primary", "secondary"] {
                if let Some(w) = l.get(key).filter(|w| w.is_object()) {
                    limits[key] = json!({"used_percent":w["usedPercent"],"resets_at":w["resetsAt"],"window_minutes":w["windowDurationMins"]});
                }
            }
            events.push(json!({"rate_limits":limits}));
        }
        let mut r = Report::read(Client::Codex, "", &events, None);
        r.limits = self.windows.clone();
        r.session = self.thread.clone(); r.usage.model = self.model.clone(); r.text = self.text.clone(); r.started_ms = self.started_ms;
        r.process = super::capture::ProcessUse { exit_unknown: "session-process", ..self.process.clone().unwrap_or_default() };
        for call in &mut r.calls { if call.model.is_none() { call.model = self.model.clone(); } }
        (r.outcome, r.error) = match status { "completed" => ("ok","none"), "interrupted" => ("cancelled","interrupted"), _ => ("failed","turn-failed") };
        if status=="failed" {
            if let Some(error)=&self.failure {
                r.detail=error["message"].as_str().map(String::from);
                let event=json!({"type":"turn.failed","error":error});
                let evidence=[event.clone()];
                if !self.worked && limit_reached(Client::Codex,&evidence).is_some() {
                    r.reached=Some(error["resetsAt"].as_i64().or_else(||r.limit.resets_at.and_then(|n|i64::try_from(n).ok())));
                    (r.outcome,r.error)=("refused","usage-limit");
                } else if let Some(reason)=auth_rejected(&event.to_string()) {(r.outcome,r.error)=("failed","auth-rejected");r.reason=Some(reason.into());}
                else if transient_failure(&evidence).is_some() {(r.outcome,r.error)=("failed","model-capacity");}
            }
        }
        Some(r)
    }

    pub fn record(&self, ctx: &Context, store: &crate::telemetry::Store) -> Res<Option<Report>> {
        let Some(mut r) = self.report() else { return Ok(None) };
        r.recorded = self.snapshot(ctx).and_then(|f| f.ok_or("running")?.append(store));
        Ok(Some(r))
    }

    /// The terminal turn's immutable records, without a store or native call.
    pub(crate) fn snapshot(&self, ctx: &Context) -> Res<Option<super::Frozen>> {
        let Some(r) = self.report() else { return Ok(None) };
        let mut value = r.observation(Client::Codex, ctx);
        value["duration_us"] = Value::Null; value["unknown"]["duration_us"] = json!("not-reported");
        let key = crate::sha256_hex(format!("{}\0{}\0{}\0{}", self.spec.root.display(),self.scope, self.thread.as_deref().unwrap_or(""), self.turn.as_deref().unwrap_or("")).as_bytes());
        super::capture::freeze("agent.app", &key, &value, &r, Client::Codex, ctx).map(Some)
    }

    /// Retire the terminal turn after the caller durably saved its outcome; the same server/thread can take another.
    pub fn finish_turn(&mut self) -> Res<()> {
        if self.fault.is_some() || self.terminal.is_none() || self.pending.is_some() { return Err("app-server terminal acknowledgment pending or unknown".into()); }
        self.active = false;
        Ok(())
    }

    /// Private restart state, never telemetry. Recovery performs no native request or permission change.
    pub fn checkpoint(&self) -> Value {
        json!({"schema":"coreq-app-session/1", "root":self.spec.root, "home":self.spec.home, "session":self.spec.session,
            "next":self.next, "pending":self.pending.map(|(id, method)| json!([id,method])), "initialized":self.initialized, "ready":self.ready,
            "thread":self.thread,"turn":self.turn,"active":self.active,"terminal":self.terminal,"model":self.model,"text":self.text,
            "usage":self.usage,"limits":self.limits,"latest_limits":self.latest_limits,"windows":self.windows,"latest_windows":self.latest_windows,"window_groups":self.window_groups,"started_ms":self.started_ms,"early":self.early.iter().map(|e|&e.value).collect::<Vec<_>>(),"fault":self.fault,"scope":self.scope,"worked":self.worked,"failure":self.failure})
    }

    pub fn recover(spec: &Spec, v: &Value) -> Res<Self> {
        if v["schema"] != "coreq-app-session/1" || v["root"] != json!(spec.root) || v["home"] != json!(spec.home) || v["session"] != spec.session || v["scope"] != native::scope(spec) {
            return Err("app-server recovery scope differs".into());
        }
        let mut s = Self::new(spec)?;
        s.next = v["next"].as_u64().filter(|n| *n > 0).ok_or("app-server recovery request invalid")?;
        if !v["pending"].is_null() {
            let method = match v["pending"][1].as_str() { Some("initialize") => "initialize", Some("thread/start") => "thread/start", Some("thread/resume") => "thread/resume", Some("turn/start") => "turn/start", Some("turn/interrupt") => "turn/interrupt", Some("turn/steer")=>"turn/steer",Some("thread/compact/start")=>"thread/compact/start",_ => return Err("app-server recovery method invalid".into()) };
            s.pending = Some((v["pending"][0].as_u64().ok_or("app-server recovery id invalid")?,method));
        }
        s.initialized = v["initialized"].as_bool().ok_or("app-server recovery state invalid")?;
        s.ready = v["ready"].as_bool().ok_or("app-server recovery state invalid")?;
        s.active = v["active"].as_bool().ok_or("app-server recovery state invalid")?;
        for (field, target) in [("thread", &mut s.thread),("turn", &mut s.turn),("terminal", &mut s.terminal),("model", &mut s.model),("text", &mut s.text)] {
            *target = if v[field].is_null() { None } else { Some(v[field].as_str().ok_or("app-server recovery field invalid")?.into()) };
        }
        s.usage = v["usage"].as_array().ok_or("app-server recovery usage invalid")?.clone(); s.limits = (!v["limits"].is_null()).then(|| v["limits"].clone());
        s.latest_limits=(!v["latest_limits"].is_null()).then(||v["latest_limits"].clone());
        if let Some(w) = v.get("windows") { s.windows = serde_json::from_value(w.clone()).map_err(|_| "app-server recovery windows invalid")?; }
        if let Some(w) = v.get("latest_windows") { s.latest_windows = serde_json::from_value(w.clone()).map_err(|_| "app-server recovery windows invalid")?; }
        if let Some(groups) = v.get("window_groups") { s.window_groups = serde_json::from_value(groups.clone()).map_err(|_| "app-server recovery window groups invalid")?; }
        s.started_ms = v["started_ms"].as_i64().filter(|n| *n >= 0).ok_or("app-server recovery time invalid")?;
        s.early=v["early"].as_array().ok_or("app-server recovery early events missing")?.iter().cloned().map(NativeFrame::parsed).collect();
        s.fault=if v["fault"].is_null() {None} else {Some(v["fault"].as_str().ok_or("app-server recovery fault invalid")?.into())};
        s.worked=v["worked"].as_bool().ok_or("app-server recovery work boundary invalid")?;
        s.failure=if v["failure"].is_null() {None} else {Some(v["failure"].as_object().ok_or("app-server recovery failure invalid")?.clone().into())};
        if s.early.len()>256 || s.early.iter().map(|e|e.value.to_string().len()).sum::<usize>()>8*1024*1024
            || s.thread.as_deref().is_some_and(|s|!native::plain_id(s)) || s.turn.as_deref().is_some_and(|s|!native::plain_id(s))
            || s.terminal.as_deref().is_some_and(|s|!matches!(s,"completed"|"failed"|"interrupted"))
            || s.ready && s.thread.is_none() || s.terminal.is_some() && s.turn.is_none()
            || s.usage.iter().any(|v|v["type"]!="turn.completed" || !v["usage"].is_object()) || s.limits.as_ref().is_some_and(|v|!v.is_object())
            || s.latest_limits.as_ref().is_some_and(|v|!v.is_object()) {
            return Err("app-server recovery state inconsistent".into());
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // S283: exact inbound lines need positive native correlation; Value/checkpoint evidence cannot mint bytes.
    #[test]
    fn app_server_frame_selection_keeps_exact_bytes_scope_and_checkpoints() {
        let spec=Spec {client:Client::Codex,root:std::env::temp_dir(),session:"-".into(),resume:false,args:vec![],env:vec![],home:None};
        let mut app=AppSession::new(&spec).unwrap();
        let init=app.initialize(json!({})).unwrap();app.observe(&json!({"id":init["id"],"result":{}})).unwrap();
        let open=app.open(json!({})).unwrap();app.observe(&json!({"id":open["id"],"result":{"thread":{"id":"thread"}}})).unwrap();
        let start=app.start_turn(json!({"input":[]})).unwrap();
        let frame=|raw:&str|NativeFrame {value:serde_json::from_str(raw).unwrap(),raw:Some(raw.as_bytes().to_vec())};
        let first=concat!(r#" { "params": {"threadId":"thread","turnId":"turn","itemId":"one","delta":"\u4e2d"}, "method":"item/agentMessage/delta" }"#, "\r\n");
        let second="{\"method\":\"item/agentMessage/delta\",\"params\":{\"threadId\":\"thread\",\"turnId\":\"turn\",\"itemId\":\"two\",\"delta\":\"é\"}}\n";
        assert!(app.observe_frame(frame(first)).unwrap().is_empty());
        let old=second.replace("\"turn\"","\"old\"");assert!(app.observe_frame(frame(&old)).unwrap().is_empty());
        assert!(app.observe_frame(frame(second)).unwrap().is_empty());
        let saved=app.checkpoint();assert!(saved["early"][0].get("raw").is_none());
        let mut recovered=AppSession::recover(&spec,&saved).unwrap();
        let receipt=json!({"id":start["id"],"result":{"turn":{"id":"turn"}}});
        let selected=app.observe_frame(NativeFrame::parsed(receipt.clone())).unwrap();
        assert_eq!(selected.iter().map(|s|s.raw.as_slice()).collect::<Vec<_>>(),[first.as_bytes(),second.as_bytes()]);
        assert!(selected.iter().all(|s|s.session=="thread" && s.turn.as_deref()==Some("turn")));
        assert!(recovered.observe_frame(NativeFrame::parsed(receipt)).unwrap().is_empty());assert_eq!(app.checkpoint(),recovered.checkpoint());
        let row=json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"x","delta":"owned"}});
        let mut rejected=vec![row.clone(),row.clone(),row.clone(),row.clone(),row.clone(),row.clone(),row.clone()];
        rejected[0]["params"]["threadId"]=json!("foreign");rejected[1]["params"]["turnId"]=json!("old");rejected[2]["params"]["turn"]=json!({"id":"foreign"});rejected[3]["params"]["turnId"]=Value::Null;rejected[3]["params"]["turn"]=json!({"id":"turn"});rejected[4]["method"]=json!("future/unknown");rejected[5]["params"]["delta"]=json!([]);rejected[6]["params"].as_object_mut().unwrap().remove("threadId");
        rejected.extend([json!({"method":"account/rateLimits/updated","params":{"rateLimits":{}}}),json!({"id":9999,"result":{"account":{"email":"private"}}}),json!({"type":"agentq_control","session_id":"thread"})]);
        for value in rejected {let raw=format!("{value}\n");assert!(app.observe_frame(frame(&raw)).unwrap().is_empty(),"{value}");}
        let request=json!({"id":99,"method":"item/tool/call","params":{"threadId":"thread","turnId":"turn","tool":"owned","arguments":{}}});
        let raw=format!("{request}\n");assert_eq!(app.observe_frame(frame(&raw)).unwrap()[0].raw,raw.as_bytes());
        let terminal=json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed"}}});let raw=format!("{terminal}\n");
        assert_eq!(app.observe_frame(frame(&raw)).unwrap()[0].raw,raw.as_bytes());assert_eq!(app.report().unwrap().outcome,"ok");
        assert!(app.observe_frame(frame(second)).unwrap().is_empty(),"late output is private");
    }

    // #246 A4/P2: app-server groups survive updates/recovery; terminal reports stay frozen.
    #[test]
    fn app_server_keeps_independent_quota_groups() {
        let spec=Spec {client:Client::Codex,root:std::env::temp_dir(),session:"-".into(),resume:false,args:vec![],env:vec![],home:None};
        let mut app=AppSession::new(&spec).unwrap();
        let init=app.initialize(json!({})).unwrap();app.observe(&json!({"id":init["id"],"result":{}})).unwrap();
        let open=app.open(json!({})).unwrap();app.observe(&json!({"id":open["id"],"result":{"thread":{"id":"thread"}}})).unwrap();
        let start=app.start_turn(json!({"input":[]})).unwrap();app.observe(&json!({"id":start["id"],"result":{"turn":{"id":"turn"}}})).unwrap();
        let row=|group,used|json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"limitId":group,"primary":{"usedPercent":used,"windowDurationMins":300,"resetsAt":60},"secondary":{"usedPercent":90,"windowDurationMins":10080,"resetsAt":90}}}});
        app.observe(&row("codex",20)).unwrap();app.observe(&row("other",30)).unwrap();
        let read=app.request("account/rateLimits/read",json!({})).unwrap();
        app.observe(&json!({"id":read["id"],"result":{"rateLimitsByLimitId":{"codex":row("codex",20)["params"]["rateLimits"],"other":row("other",30)["params"]["rateLimits"]}}})).unwrap();
        let mut cold=AppSession::recover(&spec,&app.checkpoint()).unwrap();
        for current in [&mut app,&mut cold] {
            current.observe(&row("codex",40)).unwrap();
            current.observe(&json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed"}}})).unwrap();
            let windows=current.report().unwrap().limits;
            assert_eq!(windows.len(),4,"{windows:?}");
            assert_eq!(windows.iter().map(|w|w.used).collect::<Vec<_>>(),vec![Some(0.4),Some(0.9),Some(0.3),Some(0.9)]);
            current.observe(&row("other",99)).unwrap();assert_eq!(current.report().unwrap().limits,windows);
            current.finish_turn().unwrap();let start=current.start_turn(json!({"input":[]})).unwrap();
            current.observe(&json!({"id":start["id"],"result":{"turn":{"id":"next"}}})).unwrap();
            current.observe(&json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"limitId":"codex","primary":{"windowDurationMins":300},"secondary":null}}})).unwrap();
            current.observe(&json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"next","status":"completed"}}})).unwrap();
            let windows=current.report().unwrap().limits;assert_eq!(windows.len(),3);
            assert_eq!(windows[0].used,None);assert!(windows[0].unknown.is_some());assert_eq!(windows[1].used,Some(0.99));
            current.finish_turn().unwrap();let start=current.start_turn(json!({"input":[]})).unwrap();
            current.observe(&json!({"id":start["id"],"result":{"turn":{"id":"unread"}}})).unwrap();
            let read=current.request("account/rateLimits/read",json!({})).unwrap();
            current.observe(&json!({"id":read["id"],"error":{"code":-32601,"message":"invented refusal"}})).unwrap();
            current.observe(&json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"unread","status":"completed"}}})).unwrap();
            assert_eq!(current.report().unwrap().limits,limits::unknown("native-read-refused"));
        }
    }

    fn observations(store: &crate::telemetry::Store) -> Vec<Value> {
        store.export(0, 0, 100).unwrap()["events"].as_array().unwrap().iter().map(|e| e["observation"].clone()).filter(|o| o["kind"] == "agent.turn").collect()
    }

    // R47-R50: app-server protocol shares session/turn ownership while caller policy and server requests stay intact.
    // Risk: wrong-thread completion, restored cumulative usage, policy rewriting, or recovery charges another turn.
    #[test]
    fn app_server_preserves_session_policy_and_counts_turns() {
        let root=crate::scratch("app-server");
        let mut spec=Spec {client:Client::Codex,root:root.clone(),session:"native-thread".into(),resume:true,args:vec![],home:Some(root.join("account")),env:vec![]};
        let mut app=AppSession::new(&spec).unwrap();
        let init=app.initialize(json!({"clientInfo":{"name":"fixtureq","version":"1"},"capabilities":{"experimentalApi":true}})).unwrap();
        assert!(app.open(json!({})).is_err());
        app.observe(&json!({"id":init["id"],"result":{}})).unwrap();assert_eq!(app.initialized().unwrap(),json!({"method":"initialized"}));
        let policy=json!({"model":"fixture-model","modelProvider":"fixture-provider","effort":"high","sandbox":"read-only","approvalPolicy":"never","developerInstructions":"private rules"});
        let open=app.open(policy.clone()).unwrap();assert_eq!(open["method"],"thread/resume");
        for (k,v) in policy.as_object().unwrap() {assert_eq!(&open["params"][k],v);}
        let saved=app.checkpoint();
        assert!(AppSession::recover(&Spec {resume:false,..spec.clone()},&saved).is_err(),"recovery cannot turn resume into start");
        app.observe(&json!({"id":open["id"],"result":{"thread":{"id":"foreign"}}})).unwrap_err();
        assert!(!app.ready());assert!(app.report().is_none());
        app=AppSession::recover(&spec,&saved).unwrap();
        app.observe(&json!({"id":open["id"],"result":{"thread":{"id":"native-thread"},"model":"fixture-model"}})).unwrap();assert!(app.ready());
        for method in ["model/list","account/rateLimits/read"] {
            let optional=app.request(method,json!({})).unwrap();
            app.observe(&json!({"id":optional["id"],"error":{"code":-32601,"message":"invented unsupported optional read"}})).unwrap();assert!(app.ready());
        }
        let request=app.start_turn(json!({"input":[{"type":"text","text":"private input"}],"model":"fixture-model","effort":"high"})).unwrap();
        assert!(app.start_turn(json!({})).is_err(),"uncertain start is never replayed");
        app.observe(&json!({"method":"thread/tokenUsage/updated","params":{"threadId":"native-thread","turnId":"old","tokenUsage":{"total":{"inputTokens":999},"last":{"inputTokens":999}}}})).unwrap();
        for method in ["turn/started","turn/completed"] {
            app.observe(&json!({"method":method,"params":{"threadId":"native-thread","turn":{"id":"unrelated","status":"completed"}}})).unwrap();
        }
        assert!(app.report().is_none(),"same-thread notifications cannot bind a pending turn request");
        app.observe(&json!({"id":request["id"],"result":{"turn":{"id":"owned"}}})).unwrap();
        let before=app.checkpoint();let mut cold=AppSession::recover(&spec,&before).unwrap();
        for current in [&mut app,&mut cold] {
            current.observe(&json!({"method":"turn/completed","params":{"threadId":"foreign","turn":{"id":"owned","status":"completed"}}})).unwrap();
            assert!(current.report().is_none());
            current.observe(&json!({"method":"item/tool/call","id":999,"params":{"tool":"caller-owned"}})).unwrap();
            let first=json!({"method":"thread/tokenUsage/updated","params":{"threadId":"native-thread","turnId":"owned","tokenUsage":{"total":{"inputTokens":130,"cachedInputTokens":100,"outputTokens":14},"last":{"inputTokens":30,"cachedInputTokens":20,"outputTokens":4}}}});
            current.observe(&first).unwrap();current.observe(&first).unwrap();
            current.observe(&json!({"method":"thread/tokenUsage/updated","params":{"threadId":"native-thread","turnId":"owned","tokenUsage":{"total":{"inputTokens":145,"cachedInputTokens":110,"outputTokens":16},"last":{"inputTokens":15,"cachedInputTokens":10,"outputTokens":2}}}})).unwrap();
            current.observe(&json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"usedPercent":25,"resetsAt":1793700000u64,"windowDurationMins":300}}}})).unwrap();
            current.observe(&json!({"method":"item/completed","params":{"threadId":"native-thread","turnId":"owned","item":{"type":"agentMessage","text":"private answer"}}})).unwrap();
            current.observe(&json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"owned","status":"completed"}}})).unwrap();
        }
        let report=app.report().unwrap(); assert_eq!(report.usage.input,Some(15)); assert_eq!(report.usage.output,Some(6));assert_eq!(report.text.as_deref(),Some("private answer"));
        assert_eq!(report.limit.used_percent,Some(25));
        let ctx=Context {product:"fixtureq",version:"1.0.0",project:"other",task:None,route:Some("manager"),account:"work"};
        let store=crate::telemetry::Store::open(&root.join("telemetry.db")).unwrap();
        app.record(&ctx,&store).unwrap().unwrap().recorded.unwrap(); cold.record(&ctx,&store).unwrap().unwrap().recorded.unwrap();
        app.observe(&json!({"method":"account/rateLimits/updated","params":{"rateLimits":{"primary":{"usedPercent":99}}}})).unwrap();
        app.record(&ctx,&store).unwrap().unwrap().recorded.unwrap();
        assert_eq!(observations(&store).len(),1);
        let mut conflict=AppSession::recover(&spec,&app.checkpoint()).unwrap();
        assert!(conflict.observe(&json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"owned","status":"interrupted"}}})).is_err());
        assert!(conflict.report().is_none());assert!(conflict.record(&ctx,&store).unwrap().is_none());
        assert!(AppSession::recover(&spec,&conflict.checkpoint()).unwrap().report().is_none(),"conflict survives recovery");
        app.finish_turn().unwrap();let next=app.start_turn(json!({"input":[],"model":"another-model"})).unwrap();
        app.observe(&json!({"id":next["id"],"result":{"turn":{"id":"second"}}})).unwrap();
        let stop=app.interrupt().unwrap();assert_eq!(stop["params"],json!({"threadId":"native-thread","turnId":"second"}));
        assert!(app.report().is_none(),"interrupt request does not prove terminal");
        app.observe(&json!({"id":stop["id"],"result":{}})).unwrap();
        app.observe(&json!({"method":"account/rateLimits/updated","params":{"rateLimits":[]}})).unwrap();
        app.observe(&json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"second","status":"interrupted"}}})).unwrap();
        assert_eq!(app.report().unwrap().outcome,"cancelled");
        assert_eq!(app.report().unwrap().usage.model,None,"an override invalidates previous model attribution");
        for phase in [None,Some("item/started"),Some("item/completed")] {
            let worked=phase.is_some();
            app.finish_turn().unwrap();let req=app.start_turn(json!({"input":[]})).unwrap();
            let id=if worked {"limit-after-work"} else {"limit-before-work"};
            app.observe(&json!({"id":req["id"],"result":{"turn":{"id":id}}})).unwrap();
            if let Some(phase)=phase {app.observe(&json!({"method":phase,"params":{"threadId":"native-thread","turnId":id,"item":{"type":"commandExecution"}}})).unwrap();}
            app.observe(&json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":id,"status":"failed","error":{"message":"usage limit reached","resetsAt":1793700000u64}}}})).unwrap();
            let report=app.report().unwrap();
            assert_eq!(report.detail.as_deref(),Some("usage limit reached"));
            if worked {assert_eq!(report.reached,None);assert_eq!(report.outcome,"failed");}
            else {assert_eq!(report.reached,Some(Some(1793700000)));assert_eq!(report.outcome,"refused");}
        }
        // Auth classification never replaces or reparses the native failure detail.
        for (message,reason) in [("unexpected status 401 Unauthorized: token expired, auth error: 401","401 Unauthorized"),
            ("Your refresh token was already used; sign in again","refresh token rejected")] {
            app.finish_turn().unwrap();let start=app.start_turn(json!({"input":[]})).unwrap();
            app.observe(&json!({"id":start["id"],"result":{"turn":{"id":"auth"}}})).unwrap();
            app.observe(&json!({"method":"turn/completed","params":{"threadId":"native-thread","turn":{"id":"auth","status":"failed","error":{"message":message}}}})).unwrap();
            let report=app.report().unwrap();let fields=crate::cli::report_json(&report);
            assert_eq!((report.error,fields["reason"].as_str(),fields["detail"].as_str()),("auth-rejected",Some(reason),Some(message)));
            let cold=AppSession::recover(&spec,&app.checkpoint()).unwrap();
            assert_eq!(crate::cli::report_json(&cold.report().unwrap())["reason"],reason);
        }
        spec.resume=false;spec.session="-".into();let mut new=AppSession::new(&spec).unwrap();
        let req=new.initialize(json!({})).unwrap();new.observe(&json!({"id":req["id"],"result":{}})).unwrap();
        assert_eq!(new.open(json!({"dynamicTools":[{"name":"caller-tool"}]})).unwrap()["method"],"thread/start");
        drop(store);std::fs::remove_dir_all(root).unwrap();
    }
}
