//! M12.25: private owned Codex mutation; no archive identity or reply authority crosses this boundary.
use super::*;
use super::super::history::{History,Step,StepKind};
use std::io::Seek;

fn refuse(cause:&'static str)->Failure {Failure::Refused(Cause(cause))}

fn source<'a>(adapter:&Adapter,run:&'a mut Run,operation:&'static str)->Result<&'a mut App,Failure> {
    if !adapter.descriptor.has(operation) {return Err(refuse(if operation=="fork" {"fork-unsupported"}else{"rewind-unsupported"}));}
    let run=adapter.own_mut(run)?;
    let known_protocol=run.readiness.checks.iter().any(|c|c.check=="client" && c.state=="ok" && c.detail=="codex-cli 0.162.0");
    let Body::App(app)=&mut run.body else {return Err(refuse("branch-transport-unsupported"));};
    if !app.owned || app.control.is_none() || app.transport.is_none() {return Err(refuse("branch-transport-unsupported"));}
    if !known_protocol {return Err(refuse("branch-native-support-unknown"));}
    app.branch_source_intact()?;app.pump()?;app.writable()?;
    app.writer.as_ref().ok_or("control-owner-required")?.control()?;
    if !app.session.idle() || !app.requests.is_empty() || app.cancelling || app.control.as_ref().is_some_and(|(r,_)|!r.ids().is_empty()) {return Err(refuse("branch-source-not-idle"));}
    Ok(app)
}

impl App {
    fn branch_source_intact(&mut self)->Res<()> {
        let path=self.journal.as_ref().ok_or("branch-source-unknown")?;
        let (_,log)=self.control.as_mut().ok_or("branch-source-unknown")?;
        let current=File::open(path).map_err(|_|"branch-source-changed")?;
        if native::file_id(log)?!=native::file_id(&current)? || current.metadata().map_err(|e|e.to_string())?.len()!=log.stream_position().map_err(|e|e.to_string())? {return Err("branch-source-changed".into());}Ok(())
    }
    // Existing transport and request IDs, including while the durable intent fences other callers.
    fn branch_query(&mut self,method:&'static str,params:Value,intent:Option<&Value>,until:Option<Instant>)->Res<Value> {
        self.branch_source_intact()?;
        let request=self.session.request(method,params)?;
        let owner=self.writer.as_ref().ok_or("control-owner-required")?;
        if let Some(intent)=intent {owner.branch_authority(intent)?;}else{owner.control()?;}
        let result:Res<Value>=(|| {
            self.transport.as_mut().ok_or("branch-transport-ended")?.send_until(&request,until)?;
            loop {
                let message=match self.transport.as_mut() {Some(Transport::Plain(s))=>s.wait(until)?,_=>return Err("branch-transport-unsupported".into())};
                let replied=message["id"]==request["id"] && message.get("method").is_none();
                self.observe(message.clone(),until)?;
                if replied {return message.get("result").filter(|r|r.is_object()).cloned().ok_or("branch-reply-unknown".into());}
            }
        })();
        if let Err(error)=&result {self.closed=Some(error.clone());}
        result
    }

    fn branch_idle(&mut self,thread:&str,intent:Option<&Value>,until:Option<Instant>)->Res<()> {
        let read=self.branch_query("thread/read",json!({"threadId":thread,"includeTurns":false}),intent,until)?;
        if read["thread"]["id"]!=thread || read["thread"]["status"]["type"]!="idle" {return Err("branch-native-idle-unknown".into());}Ok(())
    }

    fn branch_turns(&mut self,thread:&str,intent:Option<&Value>,until:Option<Instant>)->Res<Vec<Value>> {
        self.branch_idle(thread,intent,until)?;
        let mut cursor=Value::Null;let mut cursors=std::collections::BTreeSet::new();let mut ids=std::collections::BTreeSet::new();let mut turns=vec![];
        loop {
            let page=self.branch_query("thread/turns/list",json!({"threadId":thread,"sortDirection":"asc","itemsView":"full","cursor":cursor}),intent,until)?;
            let data=page["data"].as_array().ok_or("branch-turns-unknown")?;
            for turn in data {
                let id=turn["id"].as_str().filter(|s|native::plain_id(s)).ok_or("branch-turn-id-unknown")?;
                if !ids.insert(id.to_string()) || turn["itemsView"]!="full" || !turn["items"].is_array() || !matches!(turn["status"].as_str(),Some("completed"|"failed"|"interrupted")) {return Err("branch-turn-boundary-unknown".into());}
                turns.push(turn.clone());
            }
            match page.get("nextCursor") {
                Some(Value::Null)=>break,
                Some(Value::String(next)) if !next.is_empty() && !data.is_empty() && cursors.insert(next.clone())=>cursor=json!(next),
                _=>return Err("branch-turn-cursor-unknown".into()),
            }
        }
        self.branch_idle(thread,intent,until)?;Ok(turns)
    }

    fn branch_goal_absent(&mut self,thread:&str,until:Option<Instant>)->Res<()> {
        let result=self.branch_query("thread/goal/get",json!({"threadId":thread}),None,until)?;
        if result.get("goal")!=Some(&Value::Null) {return Err("branch-goal-suppression-unknown".into());}Ok(())
    }

    fn branch_boundary(&self,step:Option<&Step>,turns:&[Value])->Result<usize,Failure> {
        let Some(step)=step else {return Ok(0);};
        let session=self.session.session().ok_or("branch-session-unknown")?;
        let path=self.journal.as_ref().ok_or("branch-source-unknown")?;
        let label=path.to_str().ok_or("branch-source-unknown")?;
        if step.source["source"]!=label || step.source["format"]!="codex-app-server" || step.turn.session!=session || step.kind!=StepKind::Message || step.author!="assistant" || step.state!="completed" {return Err(refuse("branch-selection-unsupported"));}
        let length=std::fs::metadata(path).map_err(|e|e.to_string())?.len();
        let mut history=History::new("codex-app-server",label,session)?;
        let page=history.steps(path,length.max(1))?;
        if page.pending_bytes!=0 || !page.steps.iter().any(|current|current==step) {return Err(refuse("branch-selection-stale"));}
        let raw:Value=serde_json::from_str(step.source["raw"].as_str().ok_or("branch-native-record-unknown")?).map_err(|_|"branch-native-record-unknown")?;
        let p=&raw["params"];
        if raw["method"]!="item/completed" || p["threadId"]!=session || p["item"]["type"]!="agentMessage" {return Err(refuse("branch-whole-turn-unknown"));}
        let at=turns.iter().position(|turn|turn["id"]==p["turnId"]).ok_or_else(||refuse("branch-native-turn-unknown"))?;
        let turn=&turns[at];
        let root=turn["rootTurnId"].as_str().filter(|s|native::plain_id(s)).ok_or_else(||refuse("branch-chain-unknown"))?;
        if turn["status"]!="completed" || turn["items"].as_array().and_then(|items|items.last())!=Some(&p["item"]) || turns[at+1..].iter().any(|next|next["rootTurnId"].as_str().filter(|s|native::plain_id(s)).is_none_or(|next_root|next_root==root)) {return Err(refuse("branch-whole-turn-unknown"));}
        Ok(at+1)
    }
}

pub(super) fn fork(adapter:&Adapter,run:&mut Run,through:Option<&Step>,launch:&Launch)->Result<Run,Failure> {
    let until=run.deadline;
    let app=source(adapter,run,"fork")?;
    if launch.mode!=Mode::Turn || !launch.controls {return Err(refuse("fork-child-mode-unsupported"));}
    if launch.mcp.is_some() || launch.instructions.is_some() || launch.effort.is_some() {return Err(refuse("fork-condition-unsupported"));}
    let parent=app.session.session().ok_or("branch-session-unknown")?.to_string();
    let me=table(adapter.client);
    let (spec,_)=me.spec(launch,parent.clone(),true)?;
    let owner=app.writer.as_ref().ok_or("control-owner-required")?.clone();
    owner.branch_scope(&launch.session_store,&spec)?;
    app.session.branch_scope(&spec)?;
    // Check every ordinary launch condition before creating the native child.
    let readiness=me.preflight(launch,&Repair::default());readiness.permit(launch.readiness)?;
    if !readiness.checks.iter().any(|c|c.check=="client" && c.state=="ok" && c.detail=="codex-cli 0.162.0") {return Err(refuse("branch-native-support-unknown"));}
    let mut params=turn_thread(launch)?;
    if !launch.parts.is_empty() {input_items(&me.descriptor,&launch.parts)?;}
    let parent_log=File::open(app.journal.as_ref().ok_or("branch-source-unknown")?).map_err(|e|e.to_string())?;
    match File::open(&launch.log) {
        Ok(log) if native::file_id(&log)?==native::file_id(&parent_log)?=>return Err(refuse("fork-child-log-differs-required")),
        Ok(_)=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.to_string().into()),
    }
    app.branch_goal_absent(&parent,until)?;
    let turns=app.branch_turns(&parent,None,until)?;
    let keep=app.branch_boundary(through,&turns)?;
    // beforeTurnId is experimental and has no verified support advertisement on this channel.
    if keep==0 {return Err(refuse("fork-empty-prefix-unsupported"));}
    params["threadId"]=json!(parent);params["lastTurnId"]=turns[keep-1]["id"].clone();
    let intent=json!({"id":crate::incident::uuid(),"operation":"fork","parent":parent,"selected":through,"retained":&turns[..keep],"requested":params});
    owner.branch_begin(&intent)?;
    let lineage=json!({"operation":"fork","parent_owner":owner.checkpoint()?,"intent":intent,"fork_params":params});
    let mut verify=|child_app:&mut App|->Res<()> {
        let child=child_app.session.session().ok_or("fork-child-identity-unknown")?.to_string();
        let result=child_app.pending.iter().rev().find(|(m,_)|m["result"]["thread"]["id"]==child).ok_or("fork-reply-unknown")?.0["result"].clone();
        owner.branch_observed(&intent,&result)?;
        if app.branch_turns(&parent,Some(&intent),until)?!=turns || child_app.branch_turns(&child,None,until)?!=turns[..keep] {return Err("fork-prefix-readback-unknown".into());}Ok(())
    };
    let child_run=me.start_lineage(launch,None,Some(&lineage),Some(&mut verify))?;
    let child=me.session_id(&child_run).ok_or("fork-child-identity-unknown")?;
    owner.branch_finish(&intent,&json!({"child":child,"retained":&turns[..keep]}))?;
    Ok(child_run)
}

pub(super) fn rewind(adapter:&Adapter,run:&mut Run,through:Option<&Step>)->Result<(),Failure> {
    let until=run.deadline;
    let app=source(adapter,run,"rewind")?;
    let thread=app.session.session().ok_or("branch-session-unknown")?.to_string();
    app.branch_goal_absent(&thread,until)?;
    let turns=app.branch_turns(&thread,None,until)?;
    let keep=app.branch_boundary(through,&turns)?;
    if keep==turns.len() {return Ok(());}
    let intent=json!({"id":crate::incident::uuid(),"operation":"rewind","parent":thread,"selected":through,"retained":&turns[..keep],"before":turns[keep]["id"]});
    let owner=app.writer.as_ref().ok_or("control-owner-required")?.clone();owner.branch_begin(&intent)?;
    let result=app.branch_query("thread/revert",json!({"threadId":thread,"beforeTurnId":turns[keep]["id"]}),Some(&intent),until)?;
    owner.branch_observed(&intent,&result)?;
    if result["thread"]["id"]!=thread || result["thread"]["turns"]!=json!([]) || !result.get("turnsBackwardsCursor").is_some_and(|v|v.is_null() || v.is_string()) || !result.get("itemsBackwardsCursor").is_some_and(|v|v.is_null() || v.is_string()) {return Err("rewind-reply-unknown".into());}
    if app.branch_turns(&thread,Some(&intent),until)?!=turns[..keep] {return Err("rewind-prefix-readback-unknown".into());}
    owner.branch_finish(&intent,&result)?;Ok(())
}

// Effective native conditions are observed separately from the request, before child input.
pub(super) fn child_conditions(result:&Value,launch:&Launch)->Res<()> {
    let observed=result["cwd"].as_str().ok_or("fork-child-cwd-unknown")?;
    if Path::new(observed).canonicalize().map_err(|_|"fork-child-cwd-unknown")?!=launch.root.canonicalize().map_err(|_|"fork-child-cwd-unknown")? || launch.model.as_ref().is_some_and(|m|result["model"]!=*m) {return Err("fork-child-conditions-unknown".into());}
    let sandbox=match launch.policy.write {Scope::Native=>None,Scope::None=>Some("readOnly"),Scope::Project=>Some("workspaceWrite"),Scope::Any=>Some("dangerFullAccess")};
    if sandbox.is_some_and(|s|result["sandbox"]["type"]!=s) || matches!(launch.policy.approval,Approval::Deny|Approval::Bypass) && result["approvalPolicy"]!="never" {return Err("fork-child-policy-unknown".into());}Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_registration_refuses_unimplemented_branch_paths_before_a_transport() {
        let root=crate::scratch("branch-capabilities");let launch=Launch::new(&root,root.join("log"),"input","fixtureq");
        for entry in registry::entries() {
            let Some(descriptor)=entry.descriptor() else {continue;};
            let adapter=registry::builtin(Client(descriptor.name));
            let spec=Spec {client:Client::Codex,root:root.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
            let app=App {transport:None,session:AppSession::new(&spec).unwrap(),pending:vec![],model:None,closed:None,owned:false,requests:vec![],effort:None,control:None,journal:None,writer:None,cancelling:false,approval:Approval::Native};
            let mut run=adapter.run(Body::App(Box::new(app)),0,Optimization {intent:Intent::Off,effective:"off",reason:"optimization_retired"},&launch);
            let supported=adapter.client==Client::Codex;
            assert_eq!(adapter.descriptor.has("fork"),supported);assert_eq!(adapter.descriptor.has("rewind"),supported);
            assert!(matches!(adapter.fork(&mut run,None,&launch),Err(Failure::Refused(Cause(reason))) if reason==if supported {"branch-transport-unsupported"}else{"fork-unsupported"}));
            assert!(matches!(adapter.rewind(&mut run,None),Err(Failure::Refused(Cause(reason))) if reason==if supported {"branch-transport-unsupported"}else{"rewind-unsupported"}));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
