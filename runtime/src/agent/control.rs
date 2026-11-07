//! Live reply authority; retained journal rows are observations only.
use super::*;

pub(super) struct Pending {
    pub native_id: Value,
    generation: u64,
    event: Event,
    turn: Option<String>,
    native: Value,
}

pub(super) struct Requests {
    pub run: String,
    generation: u64,
    pending: BTreeMap<String, Pending>,
    uncertain: bool,
}

impl Requests {
    pub fn new() -> Self {Self {run:crate::incident::uuid(),generation:0,pending:BTreeMap::new(),uncertain:false}}

    fn journal(&self, log:&mut File, session:&str, request:&Pending, state:&str, native:&Value) -> Res<()> {
        let row=json!({"type":"agentq_control","session_id":session,"run_id":self.run,"generation":request.generation,
            "native_id":request.native_id,"turn_id":request.turn,"event":request.event,"state":state,"native":native});
        let mut bytes=serde_json::to_vec(&row).map_err(|e|e.to_string())?;bytes.push(b'\n');
        log.write_all(&bytes).and_then(|_|log.sync_data()).map_err(|e|format!("control-journal-unknown: {e}"))
    }

    pub fn register(&mut self,log:&mut File,session:&str,mut event:Event,native:Value,turn:Option<String>) -> Res<Event> {
        if self.uncertain {return Err("control-state-unknown".into());}
        let Event::Request{id,..}=&mut event else {return Err("request-invalid".into());};
        if self.pending.values().any(|p|p.native_id==*id) {return Err("request-native-id-in-flight".into());}
        let native_id=id.clone();let opaque=crate::incident::uuid();*id=json!(opaque);
        self.generation=self.generation.checked_add(1).ok_or("request-generation-exhausted")?;
        let request=Pending {native_id,generation:self.generation,event:event.clone(),turn,native};
        if let Err(error)=self.journal(log,session,&request,"pending",&request.native) {self.uncertain=true;return Err(error);}
        self.pending.insert(opaque,request);Ok(event)
    }

    pub fn native(&self,control:&Control)->Res<&Value> {
        Ok(&self.pending(control)?.native)
    }

    fn pending(&self,control:&Control)->Res<&Pending> {
        if self.uncertain {return Err("control-state-unknown".into());}
        let id=match control {Control::Reply{id,..}|Control::Answers{id,..}=>id,_=>return Err("request-invalid".into())};
        let key=id.as_str().ok_or("request-unknown")?;
        let request=self.pending.get(key).ok_or("request-unknown")?;
        validate(&request.event,control)?;Ok(request)
    }

    /// Remove authority before durable intent; any ambiguous failure is never retryable.
    pub fn begin(&mut self,log:&mut File,session:&str,control:&Control,response:&Value) -> Res<Pending> {
        self.pending(control)?;
        let key=match control {Control::Reply{id,..}|Control::Answers{id,..}=>id.as_str().expect("validated request ID"),_=>unreachable!()};
        let request=self.pending.remove(key).expect("validated pending request");
        if let Err(error)=self.journal(log,session,&request,"reply-pending",response) {self.uncertain=true;return Err(error);}
        Ok(request)
    }

    pub fn sent(&mut self,log:&mut File,session:&str,request:&Pending,response:&Value) -> Res<()> {
        if let Err(error)=self.journal(log,session,request,"reply-sent",response) {self.uncertain=true;return Err(error);}Ok(())
    }

    pub fn fail(&mut self,log:&mut File,session:&str,request:&Pending) {
        self.uncertain=true;let _=self.journal(log,session,request,"unknown",&request.native);
    }

    pub fn retire_native(&mut self,log:&mut File,session:&str,native_id:&Value,turn:Option<&str>,native:&Value)->Res<()> {
        let key=self.pending.iter().find(|(_,p)|p.native_id==*native_id && p.turn.as_deref()==turn).map(|(id,_)|id.clone());
        if let Some(key)=key {
            let request=self.pending.remove(&key).expect("matched native request");
            if let Err(error)=self.journal(log,session,&request,"cancelled",native) {self.uncertain=true;return Err(error);}
        }Ok(())
    }

    pub fn retire(&mut self,log:&mut File,session:&str,state:&str) -> Res<()> {
        let pending=std::mem::take(&mut self.pending);let mut error=None;
        for request in pending.values() {if let Err(e)=self.journal(log,session,request,state,&request.native) {self.uncertain=true;error.get_or_insert(e);}}
        error.map_or(Ok(()),Err)
    }

    pub fn ids(&self)->Vec<Value> {self.pending.keys().map(|id|json!(id)).collect()}
    pub fn uncertain(&self)->bool {self.uncertain}
}

fn validate(event:&Event,control:&Control)->Res<()> {
    let Event::Request{choices,questions,..}=event else {return Err("request-invalid".into());};
    match control {
        Control::Reply{option,..}=>{
            if !questions.is_empty() && option.is_some() {return Err("question-answers-required".into());}
            if option.as_ref().is_some_and(|id|!choices.iter().any(|c|c.id==*id)) {return Err("request-option-invalid".into());}
        },
        Control::Answers{answers,..}=>{
            if questions.is_empty() || answers.len()!=questions.len() {return Err("question-answers-invalid".into());}
            for question in questions {
                let values=answers.get(&question.id).ok_or("question-answers-invalid")?;
                if values.is_empty() || !question.multiple && values.len()!=1 {return Err("question-answers-invalid".into());}
                for (index,answer) in values.iter().enumerate() {
                    if values[..index].contains(answer) {return Err("question-answer-duplicate".into());}
                    match answer {
                        Answer::Choice(id) if question.choices.iter().any(|c|c.id==*id)=>{},
                        Answer::Text(_) if question.free_text=>{},
                        _=>return Err("question-answer-invalid".into()),
                    }
                }
            }
        },
        _=>return Err("request-invalid".into()),
    }Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request()->Event {Event::Request{id:json!("native"),kind:RequestKind::Approval,prompt:None,choices:vec![Choice{id:"once".into(),label:"Allow".into(),kind:"allow_once".into()}],questions:vec![]}}
    fn id(event:&Event)->Value {let Event::Request{id,..}=event else {panic!()};id.clone()}
    #[test]
    fn authority_is_occurrence_local_and_intent_failure_fences_retry() {
        let root=crate::scratch("control-authority");std::fs::create_dir_all(&root).unwrap();let path=root.join("journal");
        let mut log=File::options().create(true).append(true).open(&path).unwrap();let mut requests=Requests::new();
        let first=requests.register(&mut log,"session",request(),json!({"id":"native"}),None).unwrap();
        let reply=Control::Reply{id:id(&first),option:Some("once".into())};let response=json!({"id":"native","result":{"optionId":"once"}});
        assert!(Requests::new().begin(&mut log,"session",&reply,&response).is_err(),"another Run has no authority");
        assert!(requests.begin(&mut log,"session",&Control::Reply{id:id(&first),option:Some("invented".into())},&response).is_err());
        let original=requests.begin(&mut log,"session",&reply,&response).unwrap();assert_eq!(original.native_id,"native");
        requests.sent(&mut log,"session",&original,&response).unwrap();assert!(requests.begin(&mut log,"session",&reply,&response).is_err());
        let reused=requests.register(&mut log,"session",request(),Value::Null,None).unwrap();assert_ne!(id(&first),id(&reused));
        assert!(requests.begin(&mut log,"session",&reply,&response).is_err(),"same native ID cannot revive a stale reply");
        let mut readonly=File::open(&path).unwrap();let second=Control::Reply{id:id(&reused),option:None};
        assert!(requests.begin(&mut readonly,"session",&second,&response).is_err());
        assert!(requests.begin(&mut log,"session",&second,&response).is_err(),"durable intent failure grants no retry");
        let rows:Vec<Value>=std::fs::read_to_string(path).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect();
        assert_eq!(rows.iter().map(|r|r["state"].as_str().unwrap()).collect::<Vec<_>>(),["pending","reply-pending","reply-sent","pending"]);
        assert_eq!(rows[1]["native"],response);assert_eq!(rows[2]["native"],response);
        assert_eq!(rows[0]["generation"],1);assert_eq!(rows[3]["generation"],2);assert_eq!(rows[0]["run_id"],requests.run);
        drop((readonly,log));std::fs::remove_dir_all(root).unwrap();
    }
}
