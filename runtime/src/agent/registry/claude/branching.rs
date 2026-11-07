//! Claude-private #300 selection. A source witness is never a child-loaded-prefix receipt.
use super::*;
use crate::agent::history::{History,Step,StepKind};

fn uuid(value:&Value)->Res<&str> {
    let id=value.as_str().ok_or("fork-native-uuid-unknown")?;
    if id.len()!=36 || !id.bytes().enumerate().all(|(i,b)|if matches!(i,8|13|18|23) {b==b'-'}else{b.is_ascii_hexdigit()}) {return Err("fork-native-uuid-unknown".into());}Ok(id)
}

/// Only measured identifiers, source generation and hashes may cross into later lineage projection.
struct Boundary { descriptor:Value, generation:Value, path:PathBuf }
struct PreparedFork { boundary:Boundary, intent:Value, child:Spec }
impl Boundary {
    // Existing private producer intent; never an applied receipt or a portable archive anchor.
    fn intent(&self,step:&Step,owner:Value,launch:&Launch)->Value {
        json!({"id":crate::incident::uuid(),"operation":"fork","parent":self.descriptor["parent_session"],
            "parent_owner":owner,"selected":step,"boundary":self.descriptor,
            "physical":{"path":self.path,"identity":self.generation["identity"]},
            "requested_conditions":{"model":launch.model,"effort":launch.effort,"policy":launch.policy}})
    }
    fn revalidate(&self)->Res<()> {
        source_unaliased(&self.path)?;
        let history=History::recover(&self.generation)?;
        history.read_bytes(&self.path,0,0)?;
        if std::fs::metadata(&self.path).map_err(|e|e.to_string())?.len()!=self.generation["offset"].as_u64().ok_or("fork-source-generation-unknown")? {return Err("fork-source-generation-changed".into());}Ok(())
    }
}

fn source_unaliased(path:&Path)->Res<()> {
    let metadata=std::fs::symlink_metadata(path).map_err(|e|e.to_string())?;
    if !metadata.file_type().is_file() {return Err("fork-source-alias-unsupported".into());}
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink()!=1 {return Err("fork-source-alias-unsupported".into());}
        Ok(())
    }
    #[cfg(not(unix))] {Err("fork-source-alias-check-unavailable".into())}
}

fn resolve(step:&Step,turn:&mut native::NativeTurn,generation:&Value)->Res<Boundary> {
    let path=turn.path().to_path_buf();source_unaliased(&path)?;let label=path.to_str().ok_or("fork-source-unknown")?;
    if step.source["format"]!="claude-native" || step.source["source"]!=label || step.turn.session!=turn.session() || step.kind!=StepKind::Message || step.author!="assistant" {return Err("fork-selection-unsupported".into());}
    // Reuse the native decoder's accepted-input/end marker and immutable pre-input physical guard.
    if turn.checkpoint()!=*generation || std::fs::metadata(&path).map_err(|e|e.to_string())?.len()!=generation["protected_until"].as_u64().ok_or("fork-source-generation-unknown")? {return Err("fork-source-generation-changed".into());}
    if turn.poll()?.is_none() || turn.aborted {return Err("fork-native-turn-unsettled".into());}
    let saved=turn.checkpoint();if saved!=*generation {return Err("fork-source-generation-changed".into());}let length=saved["protected_until"].as_u64().ok_or("fork-source-generation-unknown")?;
    let mut history=History::new("claude-native",label,turn.session())?;
    let page=history.steps(&path,length.max(1))?;
    if page.pending_bytes!=0 || page.more || page.checkpoint["offset"]!=length || page.checkpoint["identity"]!=saved["identity"] || page.checkpoint["prefix"]!=saved["protected_hash"] {return Err("fork-source-generation-changed".into());}
    if !page.steps.iter().any(|current|current==step) {return Err("fork-selection-stale".into());}
    let bytes=history.read_bytes(&path,0,length)?;
    let text=std::str::from_utf8(&bytes).map_err(|_|"fork-source-invalid")?;
    let rows=text.lines().map(|line|serde_json::from_str::<Value>(line).map_err(|_|"fork-source-invalid".to_string())).collect::<Res<Vec<_>>>()?;
    let selected:Value=serde_json::from_str(step.source["raw"].as_str().ok_or("fork-selection-source-unknown")?).map_err(|_|"fork-selection-source-unknown")?;
    let target=uuid(&selected["uuid"])?;
    let mut chain=vec![];let mut ids=std::collections::BTreeSet::new();let mut previous:Option<String>=None;
    let mut prompt=None::<String>;let mut group=None::<String>;let mut closed=true;let mut selected_prompt=None;let mut selected_chain=None;let mut retained_end=0u64;let mut offset=0u64;
    for (row,line) in rows.iter().zip(text.split_inclusive('\n')) {
        offset+=line.len() as u64;
        if row["type"]=="system" && row["subtype"]=="turn_duration" && row.get("uuid").is_none() {
            if row.get("sessionId").is_some_and(|s|s!=turn.session()) || closed || previous.is_none() {return Err("fork-turn-marker-unknown".into());}
            closed=true;continue;
        }
        // Attachments, compaction, tool carriers and side chains need their own proven whole-turn map.
        if !matches!(row["type"].as_str(),Some("user"|"assistant")) || row["isSidechain"]!=false || row["sessionId"]!=turn.session() || row.get("parentSessionId").is_some_and(|v|!v.is_null()) {return Err("fork-chain-entry-unsupported".into());}
        let id=uuid(&row["uuid"])?;
        if !ids.insert(id.to_string()) {return Err("fork-native-uuid-duplicate".into());}
        match (previous.as_deref(),row.get("parentUuid")) {
            (None,Some(Value::Null))=>{},
            (Some(parent),Some(value)) if uuid(value)?==parent=>{},
            _=>return Err("fork-native-chain-unknown".into()),
        }
        if row["type"]=="user" {
            if !closed || row["isMeta"]==true || !row["message"]["content"].is_string() {return Err("fork-prompt-boundary-unknown".into());}
            prompt=Some(id.to_string());group=Some(uuid(&row["promptId"] )?.to_string());closed=false;
        }else{
            if closed || group.as_deref()!=Some(uuid(&row["promptId"])? ) || row["message"]["role"]!="assistant" || row["isApiErrorMessage"]==true || !row["message"]["content"].as_array().is_some_and(|blocks|!blocks.is_empty() && blocks.iter().all(|b|b["type"]=="text" && b["text"].is_string())) {return Err("fork-whole-turn-unknown".into());}
            if row["message"]["stop_reason"]!="end_turn" {return Err("fork-whole-turn-unknown".into());}
        }
        chain.push(json!({"uuid":id,"parent_uuid":row["parentUuid"]}));previous=Some(id.to_string());
        if id==target {selected_prompt=prompt.clone();selected_chain=Some(chain.clone());retained_end=offset;}
        else if selected_chain.is_some() && prompt==selected_prompt {return Err("fork-selected-not-last-chain-entry".into());}
    }
    if !closed {return Err("fork-whole-turn-unknown".into());}
    let chain=selected_chain.ok_or("fork-selected-native-entry-unknown")?;
    let prefix=history.read_bytes(&path,0,retained_end)?;
    let boundary=Boundary {path,generation:page.checkpoint.clone(),descriptor:json!({"parent_session":turn.session(),"prompt_uuid":selected_prompt,"last_chain_uuid":target,"chain":chain,"retained_end":retained_end,"retained_sha256":crate::sha256_hex(&prefix),"generation":page.checkpoint,"selected":{"id":step.id,"revision":step.revision,"part":step.part,"start":step.source["start"],"end":step.source["end"],"sha256":step.source["sha256"]},"whole_turn":true})};
    boundary.revalidate()?;Ok(boundary)
}

// Use normal Claude launch mapping; the fresh ID is requested, not measured native identity.
fn child_spec(launch:&Launch,parent:&str,boundary:&Value)->Res<Spec> {
    uuid(&json!(parent))?;
    if boundary["parent_session"]!=parent || boundary["whole_turn"]!=true {return Err("fork-parent-boundary-differs".into());}
    let cursor=uuid(&boundary["last_chain_uuid"])?;
    let id=new_session();if id==parent {return Err("fork-child-identity-differs-required".into());}
    let mut child=ADAPTER.spec(launch,id,false)?;
    child.args.extend(["--resume".into(),parent.into(),"--fork-session".into(),format!("--resume-session-at={cursor}").into()]);
    Ok(child)
}

impl Boundary {
    fn child_prefix(&self,spec:&Spec)->Res<Value> {
        self.revalidate()?;
        let path=claude_transcript(spec,&spec.session)?;source_unaliased(&path).map_err(|e|format!("fork-child-native-persistence-unavailable: {e}"))?;
        let file=File::open(&path).map_err(|e|e.to_string())?;
        let length=file.metadata().map_err(|e|e.to_string())?.len();
        let mut history=History::new("claude-native",path.to_str().ok_or("fork-child-source-unknown")?,&spec.session)?;
        let page=history.steps(&path,length.max(1))?;
        if page.pending_bytes!=0 || page.more {return Err("fork-child-prefix-incomplete".into());}
        let bytes=history.read_bytes(&path,0,length)?;
        let original=History::recover(&self.generation)?.read_bytes(&self.path,0,self.descriptor["retained_end"].as_u64().ok_or("fork-boundary-unknown")?)?;
        // Compare native conversation values, not byte serialization or copied display Steps.
        let values=|bytes:&[u8],session:&str|->Res<Vec<Value>> {
            let mut out=vec![];
            for line in std::str::from_utf8(bytes).map_err(|_|"fork-prefix-invalid")?.lines() {
                let row:Value=serde_json::from_str(line).map_err(|_|"fork-prefix-invalid")?;
                if row["type"]=="system" && row["subtype"]=="turn_duration" {continue;}
                if !matches!(row["type"].as_str(),Some("user"|"assistant")) || row["sessionId"]!=session || row["isSidechain"]!=false {return Err("fork-prefix-entry-unsupported".into());}
                uuid(&row["uuid"])?;
                out.push(json!({"uuid":row["uuid"],"parentUuid":row["parentUuid"],"promptId":row["promptId"],"type":row["type"],"message":row["message"],"isMeta":row["isMeta"],"isCompactSummary":row["isCompactSummary"]}));
            }Ok(out)
        };
        if values(&bytes,&spec.session)?!=values(&original,self.descriptor["parent_session"].as_str().ok_or("fork-parent-unknown")?)? {return Err("fork-child-prefix-differs".into());}
        History::recover(&page.checkpoint)?.read_bytes(&path,0,0)?;
        if std::fs::metadata(&path).map_err(|e|e.to_string())?.len()!=length {return Err("fork-child-generation-changed".into());}
        let first=std::str::from_utf8(&bytes).map_err(|_|"fork-child-identity-unknown")?.lines().find_map(|line| {
            let row:Value=serde_json::from_str(line).ok()?;
            matches!(row["type"].as_str(),Some("user"|"assistant")).then(||row["sessionId"].clone())
        }).ok_or("fork-child-identity-unknown")?;
        Ok(json!({"session_id":first,"path":path,"history":page.checkpoint}))
    }
}

impl Session {
    pub fn fork(&mut self,through:Option<&Step>,launch:&Launch,archive:&crate::archive::Handle)->Result<Run,Failure> {
        let refuse=|cause|Failure::Refused(Cause(cause));
        if !ADAPTER.descriptor.has("fork") {return Err(refuse("fork-unsupported"));}
        let prepared=self.prepare_fork(through,launch)?;
        self.start_fork(prepared,launch,archive)
    }

    fn start_fork(&mut self,prepared:PreparedFork,launch:&Launch,archive:&crate::archive::Handle)->Result<Run,Failure> {
        let PreparedFork {boundary,mut intent,child:spec}=prepared;
        let canonical=ADAPTER.spec(launch,spec.session.clone(),false)?;
        let readiness=ADAPTER.preflight(launch,&Repair::default());readiness.permit(launch.readiness)?;
        if !readiness.checks.iter().any(|c|c.check=="client" && c.state=="ok" && version_parts(&c.detail)==Some(vec![2,1,284])) {return Err(Failure::Refused(Cause("fork-native-support-unknown")));}
        let native_path=claude_transcript(&spec,&spec.session)?;
        match std::fs::symlink_metadata(&native_path) {
            Ok(_)=>return Err(Failure::Refused(Cause("fork-child-native-source-exists"))),
            Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.to_string().into()),
        }
        intent["child_source"]=json!({"path":native_path,"before":"absent"});
        let parts=if launch.parts.is_empty() {vec![Part::Text(launch.input.clone())]}else{launch.parts.clone()};
        if parts.is_empty() || parts.iter().all(|p|matches!(p,Part::Text(text) if text.is_empty())) {return Err(Failure::Refused(Cause("input-empty")));}
        stream_message(&parts)?;
        let owner=launch.session_store.acquire(&spec,true,None,launch.lifetime)?;
        let mut child_launch=launch.clone();child_launch.process=owner.process_hooks(&launch.process,true)?;
        let command=preflight::launch_base(&ADAPTER,&child_launch)?;
        self.writable()?;boundary.revalidate()?;
        self.owner.branch_begin(&intent)?;
        let lineage=json!({"intent":intent,"parent_owner":self.owner.checkpoint()?,"requested_child":spec.session});
        // Existing process hooks track incarnation/lifetime before native control or explicit input.
        let mut child=Session::boot(command,&spec,&child_launch,&owner,Some(&lineage))?;
        self.owner.branch_authority(&intent)?;boundary.revalidate()?;
        child.pump()?;
        // The expected native path was absent before spawn. Read actual newly persisted session IDs.
        let persistence=boundary.child_prefix(&spec)?;
        let identity=child.fork_identity.clone();
        if persistence["session_id"]!=spec.session || identity.as_ref().is_some_and(|v|v["session_id"]!=persistence["session_id"]) || spec.session==self.session || child.initialization["pid"].as_u64()!=Some(child.native.pid as u64) {return Err("fork-child-identity-unknown".into());}
        if NativeProcess::read(child.native.pid)?.as_ref()!=Some(&child.native) {return Err("fork-child-process-unknown".into());}
        owner.bind(persistence["session_id"].as_str().ok_or("fork-child-identity-unknown")?)?;
        if child.initialization["session_state"]!="idle" {return Err("fork-child-native-idle-unknown".into());}
        let settings=child.exchange(json!({"subtype":"get_settings"}))?;
        let rules=child.exchange(json!({"subtype":"list_permission_rules"}))?;
        let model=settings["applied"]["model"].as_str().filter(|m|!m.is_empty()).ok_or("fork-child-model-unknown")?;
        if launch.model.as_ref().is_some_and(|m|m!=model) {return Err("fork-child-model-differs-or-alias-unresolved".into());}
        let permission=match launch.policy.approval {Approval::Native=>None,Approval::Plan=>Some("plan"),Approval::Edits=>Some("acceptEdits"),Approval::Auto=>Some("auto"),Approval::Deny=>Some("dontAsk"),Approval::Bypass=>Some("bypassPermissions")};
        if permission.is_some_and(|mode|child.initialization["current_permission_mode"]!=mode) {return Err("fork-child-permission-unknown".into());}
        if boundary.child_prefix(&spec)?!=persistence {return Err("fork-child-generation-changed".into());}
        self.owner.branch_authority(&intent)?;boundary.revalidate()?;
        child.pump()?;
        let native=json!({"claude":{"identity":identity,"persistence":persistence,"initialize":child.initialization,"settings":settings,"rules":rules,"process":child.native}});
        let observed=json!({"child":spec.session,"boundary":boundary.descriptor});
        self.owner.branch_observed(&intent,&native)?;owner.branch_observed(&intent,&native)?;
        self.owner.branch_finish(&intent,&observed)?;owner.branch_finish(&intent,&observed)?;
        child.fork_intent=None;child.identified=true;child.spec=canonical;child.scope=native::scope(&child.spec);
        // Durable lineage and measured native ownership precede the one explicit Launch input.
        archive.lineage(&intent,&native,&observed);
        child.input(&parts)?;
        let offset=child.boundary;let stop_id=child.run_id().to_string();
        let mut run=ADAPTER.checked_run(&readiness,Body::Claude(Box::new(child)),offset,&child_launch);
        run.stop_id=stop_id;run.owner=Some(owner);Ok(run)
    }

    fn prepare_fork(&mut self,through:Option<&Step>,launch:&Launch)->Result<PreparedFork,Failure> {
        let refuse=|cause|Failure::Refused(Cause(cause));
        self.pump()?;self.writable()?;
        if self.active || self.cancelling || !self.identified || self.report.is_none() || !self.requests.ids().is_empty() {return Err(refuse("fork-source-not-idle"));}
        if NativeProcess::read(self.native.pid)?.as_ref()!=Some(&self.native) {return Err(refuse("fork-source-process-unknown"));}
        if launch.mode!=Mode::Turn || !launch.controls {return Err(refuse("fork-child-mode-unsupported"));}
        if launch.title.is_some() || launch.mcp.is_some() || launch.instructions.is_some() || launch.effort.is_some() || !launch.settings_env.is_empty() || launch.compact_hook.is_some() {return Err(refuse("fork-condition-unsupported"));}
        let spec=ADAPTER.spec(launch,self.session.clone(),true)?;self.owner.branch_scope(&launch.session_store,&spec)?;
        let step=through.ok_or_else(||refuse("fork-empty-prefix-unsupported"))?;
        let turn=self.native_turn.as_mut().ok_or_else(||refuse("fork-native-generation-unavailable"))?;
        let generation=self.native_generation.as_ref().ok_or_else(||refuse("fork-native-generation-unavailable"))?;
        let boundary=resolve(step,turn,generation).map_err(Failure::Failed)?;
        let parent=File::open(&self.path).map_err(|e|e.to_string())?;
        match File::open(&launch.log) {
            Ok(child) if native::file_id(&parent)?==native::file_id(&child)?=>return Err(refuse("fork-child-log-differs-required")),
            Ok(_)=>{},Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},Err(error)=>return Err(error.to_string().into()),
        }
        // Source preparation creates no owner/child and grants no native acceptance.
        self.owner.control()?;boundary.revalidate()?;
        let child=child_spec(launch,&self.session,&boundary.descriptor)?;
        let intent=boundary.intent(step,self.owner.checkpoint()?,launch);
        Ok(PreparedFork {boundary,intent,child})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // #300: display completion/native_id never substitutes for exact bytes, chain and generation.
    #[cfg(unix)]
    #[test]
    fn original_native_boundary_is_exact_and_never_a_child_receipt() {
        let root=crate::scratch("claude-fork-boundary");let path=root.join("native.jsonl");
        let session="00000000-0000-4000-8000-000000000099";
        let spec=Spec {client:Client::Claude,root:root.clone(),session:session.into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let fixture=include_str!("../../../../tests/fixtures/history/claude-fork-chain.jsonl");
        let mut turn=native::NativeTurn::before(&spec,&path,"second").unwrap();
        std::fs::write(&path,fixture).unwrap();
        let label=path.to_str().unwrap();let mut history=History::new("claude-native",label,session).unwrap();
        let page=history.steps(&path,fixture.len() as u64).unwrap();
        let step=page.steps.iter().find(|s|s.author=="assistant").unwrap().clone();
        assert!(turn.poll().unwrap().is_some());let generation=turn.checkpoint();
        let proof=resolve(&step,&mut turn,&generation).unwrap();
        assert_eq!(proof.descriptor["last_chain_uuid"],"00000000-0000-4000-8000-000000000002");
        assert_eq!(proof.descriptor["prompt_uuid"],"00000000-0000-4000-8000-000000000001");
        assert_eq!(proof.descriptor["chain"].as_array().unwrap().len(),2);
        let launch=Launch::new(&root,root.join("child.log"),"next","fixtureq");
        let child=child_spec(&launch,session,&proof.descriptor).unwrap();
        assert_ne!(child.session,session);assert!(!child.resume);
        let args=native_args(&child);
        assert!(args.windows(2).any(|a|a==[OsString::from("--resume"),OsString::from(session)]));
        assert!(args.contains(&OsString::from("--fork-session")));
        assert!(args.contains(&OsString::from("--resume-session-at=00000000-0000-4000-8000-000000000002")));
        assert_eq!(args[args.len()-2],"--session-id");assert_eq!(args.last(),Some(&OsString::from(child.session.as_str())));
        let child_path=claude_transcript(&child,&child.session).unwrap();std::fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let retained=&fixture[..proof.descriptor["retained_end"].as_u64().unwrap() as usize];
        std::fs::write(&child_path,retained.replace(session,&child.session)).unwrap();
        assert_eq!(proof.child_prefix(&child).unwrap()["session_id"],child.session);
        std::fs::write(&child_path,retained).unwrap();assert!(proof.child_prefix(&child).is_err());
        let intent=proof.intent(&step,json!({"session":session}),&launch);
        assert_eq!(intent["parent"],session);assert_eq!(intent["selected"],json!(step));
        assert_eq!(intent["physical"]["identity"],proof.generation["identity"]);
        assert_eq!(intent["requested_conditions"]["model"],Value::Null);
        assert!(intent.get("observed").is_none());assert!(intent.get("child").is_none());
        assert!(proof.descriptor.get("child").is_none());assert!(!proof.descriptor.to_string().contains("first answer"));
        let mut forged=step.clone();forged.native_id=Some("forged".into());assert!(resolve(&forged,&mut turn,&generation).is_err());
        let mut forged=step.clone();forged.revision+=1;assert!(resolve(&forged,&mut turn,&generation).is_err());
        let mut forged=step.clone();forged.origins.clear();assert!(resolve(&forged,&mut turn,&generation).is_err());
        let before=std::fs::read(&path).unwrap();
        let mut append=File::options().append(true).open(&path).unwrap();writeln!(append,"{{\"type\":\"attachment\"}}").unwrap();
        assert!(proof.revalidate().is_err());assert!(resolve(&step,&mut turn,&generation).is_err());
        std::fs::write(&path,&before).unwrap();let mut bad=step.clone();bad.source["source"]=json!(root.join("copy.jsonl"));assert!(resolve(&bad,&mut turn,&generation).is_err());
        let alias=root.join("alias.jsonl");std::fs::hard_link(&path,&alias).unwrap();assert!(proof.revalidate().is_err());std::fs::remove_file(alias).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(),before);
        std::fs::remove_dir_all(root).unwrap();
    }
}
