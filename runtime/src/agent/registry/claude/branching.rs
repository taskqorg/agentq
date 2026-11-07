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
    #[cfg(windows)] {
        use std::os::windows::{fs::MetadataExt,io::AsRawHandle};
        if metadata.file_attributes()&0x400!=0 {return Err("fork-source-alias-unsupported".into());}
        #[repr(C)] struct Standard {allocation:i64,end:i64,links:u32,pending:u8,directory:u8}
        #[link(name="kernel32")] unsafe extern "system" {fn GetFileInformationByHandleEx(handle:*mut std::ffi::c_void,kind:i32,data:*mut std::ffi::c_void,size:u32)->i32;}
        let file=File::open(path).map_err(|e|e.to_string())?;
        let mut info=Standard {allocation:0,end:0,links:0,pending:0,directory:0};
        // SAFETY: FILE_STANDARD_INFO writes the live repr(C) buffer for this owned file handle.
        if unsafe {GetFileInformationByHandleEx(file.as_raw_handle(),1,(&mut info as *mut Standard).cast(),std::mem::size_of::<Standard>() as u32)}==0 {return Err("fork-source-alias-check-unavailable".into());}
        if info.links!=1 || info.pending!=0 || info.directory!=0 {return Err("fork-source-alias-unsupported".into());}Ok(())
    }
    #[cfg(not(any(unix,windows)))] {Err("fork-source-alias-check-unavailable".into())}
}

fn resolve(step:&Step,turn:&mut native::NativeTurn,generation:&Value)->Res<Boundary> {
    let path=turn.path().to_path_buf();source_unaliased(&path)?;let label=path.to_str().ok_or("fork-source-unknown")?;
    if step.source["format"]!="claude-native" || step.source["source"]!=label || step.turn.session!=turn.session() {return Err("fork-selection-unsupported".into());}
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
    let content=selected["message"]["content"].as_array();
    let selected_tail=match selected["type"].as_str() {
        Some("assistant")=>step.kind==StepKind::Message && content.is_some_and(|c|step.part+1==c.len() && c[step.part]["type"]=="text"),
        Some("user")=>content.is_some_and(|c|!c.is_empty() && step.part+1==c.len() && c.iter().all(|b|b["type"]=="tool_result")),
        Some("attachment")=>matches!(selected["attachment"]["type"].as_str(),Some("structured_output"|"inlined_image_paths"|"mcp_resource")),
        _=>false,
    };
    if !selected_tail {return Err("fork-selection-not-terminal-entry".into());}
    let mut chain=vec![];let mut ids=std::collections::BTreeSet::new();let mut previous:Option<String>=None;
    let mut tools=std::collections::BTreeSet::new();let mut seen_tools=std::collections::BTreeSet::new();
    let mut prompt=None::<String>;let mut group=None::<String>;let mut closed=true;let mut selected_prompt=None;let mut selected_chain=None;let mut retained_end=0u64;let mut offset=0u64;
    for (row,line) in rows.iter().zip(text.split_inclusive('\n')) {
        offset+=line.len() as u64;
        if row["subtype"]=="compact_boundary" || row["isCompactSummary"]==true {return Err("fork-compaction-cursor-unmapped".into());}
        if row["isSidechain"]==true {return Err("fork-sidechain-source-unmapped".into());}
        if row["type"]=="system" && row["subtype"]=="turn_duration" && row.get("uuid").is_none() {
            if row.get("sessionId").is_some_and(|s|s!=turn.session()) || closed || previous.is_none() || !tools.is_empty() {return Err("fork-turn-marker-unknown".into());}
            closed=true;continue;
        }
        if !matches!(row["type"].as_str(),Some("user"|"assistant"|"attachment")) || row["isSidechain"]!=false || row["sessionId"]!=turn.session() || row.get("parentSessionId").is_some_and(|v|!v.is_null()) {return Err("fork-chain-entry-unsupported".into());}
        let id=uuid(&row["uuid"])?;
        if !ids.insert(id.to_string()) {return Err("fork-native-uuid-duplicate".into());}
        match (previous.as_deref(),row.get("parentUuid")) {
            (None,Some(Value::Null))=>{},
            (Some(parent),Some(value)) if uuid(value)?==parent=>{},
            _=>return Err("fork-native-chain-unknown".into()),
        }
        let blocks=row["message"]["content"].as_array();
        if row["type"]=="user" && row["message"]["role"]!="user" {return Err("fork-user-role-unknown".into());}
        let carrier=row["type"]=="user" && blocks.is_some_and(|c|!c.is_empty() && c.iter().all(|b|b["type"]=="tool_result"));
        if row["type"]=="user" && !carrier {
            if !closed || row["isMeta"]==true || row["stackedExpansion"]==true || !(row["message"]["content"].is_string() || blocks.is_some_and(|c|!c.is_empty() && c.iter().all(|b|matches!(b["type"].as_str(),Some("text"|"image"|"document"))))) {return Err("fork-prompt-boundary-unknown".into());}
            prompt=Some(id.to_string());group=Some(uuid(&row["promptId"] )?.to_string());closed=false;seen_tools.clear();
        }else{
            if prompt.is_none() || row.get("promptId").is_some_and(|p|Some(p.as_str().unwrap_or_default())!=group.as_deref()) {return Err("fork-prompt-association-unknown".into());}
            if carrier {
                if closed {return Err("fork-tool-carrier-outside-turn".into());}
                for block in blocks.ok_or("fork-tool-carrier-invalid")? {
                    let tool=block["tool_use_id"].as_str().filter(|s|!s.is_empty()).ok_or("fork-tool-carrier-invalid")?;
                    if !tools.remove(tool) {return Err("fork-tool-carrier-unassociated".into());}
                }
            }else if row["type"]=="assistant" {
                if closed || row["message"]["role"]!="assistant" || row["isApiErrorMessage"]==true {return Err("fork-whole-turn-unknown".into());}
                for block in blocks.filter(|b|!b.is_empty()).ok_or("fork-assistant-content-unknown")? {
                    match block["type"].as_str() {
                        Some("text") if block["text"].is_string()=>{},
                        Some("thinking") if block["thinking"].is_string()=>{},
                        Some("redacted_thinking") if block["data"].is_string()=>{},
                        Some("tool_use") if block["name"].as_str().is_some_and(|s|!s.is_empty()) && block["input"].is_object()=>{
                            let tool=block["id"].as_str().filter(|s|!s.is_empty()).ok_or("fork-tool-id-unknown")?;
                            if !seen_tools.insert(tool.to_string()) || !tools.insert(tool.to_string()) {return Err("fork-tool-id-duplicate".into());}
                        },
                        _=>return Err("fork-native-block-unmapped".into()),
                    }
                }
            }else if row["type"]=="attachment" {
                if !matches!(row["attachment"]["type"].as_str(),Some("structured_output"|"inlined_image_paths"|"mcp_resource")) {return Err("fork-attachment-turn-unmapped".into());}
                if !tools.is_empty() {return Err("fork-attachment-tool-tail-unknown".into());}
            }
        }
        chain.push(json!({"uuid":id,"parent_uuid":row["parentUuid"]}));previous=Some(id.to_string());
        if id==target {selected_prompt=prompt.clone();selected_chain=Some(chain.clone());retained_end=offset;}
        else if selected_chain.is_some() && prompt==selected_prompt {return Err("fork-selected-not-last-chain-entry".into());}
    }
    if !closed || !tools.is_empty() {return Err("fork-whole-turn-unknown".into());}
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
                if !matches!(row["type"].as_str(),Some("user"|"assistant"|"attachment")) || row["sessionId"]!=session || row["isSidechain"]!=false {return Err("fork-prefix-entry-unsupported".into());}
                uuid(&row["uuid"])?;
                out.push(json!({"uuid":row["uuid"],"parentUuid":row["parentUuid"],"promptId":row["promptId"],"type":row["type"],"message":row["message"],"isMeta":row["isMeta"],"isCompactSummary":row["isCompactSummary"],"attachment":row["attachment"]}));
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

#[cfg(test)]
mod retained_tool_tail_tests {
    use super::*;
    #[cfg(any(unix,windows))]
    #[test]
    fn whole_turn_uses_actual_structured_attachment_after_thinking_and_tool_carrier() {
        let root=crate::scratch("claude-tool-tail");let path=root.join("native.jsonl");let session="00000000-0000-4000-8000-000000000099";
        let spec=Spec {client:Client::Claude,root:root.clone(),session:session.into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let fixture=include_str!("../../../../tests/fixtures/history/claude-fork-tool-chain.jsonl");
        let mut turn=native::NativeTurn::before(&spec,&path,"first").unwrap();std::fs::write(&path,fixture).unwrap();assert!(turn.poll().unwrap().is_some());let generation=turn.checkpoint();
        let mut history=History::new("claude-native",path.to_str().unwrap(),session).unwrap();let page=history.steps(&path,fixture.len() as u64).unwrap();
        let selected=page.steps.iter().find(|s|s.source["raw"].as_str().is_some_and(|raw|raw.contains("\"type\":\"attachment\""))).unwrap();
        let proof=resolve(selected,&mut turn,&generation).unwrap();
        assert_eq!(proof.descriptor["last_chain_uuid"],"00000000-0000-4000-8000-000000000004");assert_eq!(proof.descriptor["chain"].as_array().unwrap().len(),4);
        for mid in page.steps.iter().filter(|s|s.kind==StepKind::Thinking || s.source["raw"].as_str().is_some_and(|raw|raw.contains("\"type\":\"tool_result\""))) {assert!(resolve(mid,&mut turn,&generation).is_err());}
        let launch=Launch::new(&root,root.join("child.log"),"next","fixtureq");let child=child_spec(&launch,session,&proof.descriptor).unwrap();let child_path=claude_transcript(&child,&child.session).unwrap();std::fs::create_dir_all(child_path.parent().unwrap()).unwrap();
        let prefix=&fixture[..proof.descriptor["retained_end"].as_u64().unwrap() as usize];std::fs::write(&child_path,prefix.replace(session,&child.session)).unwrap();assert_eq!(proof.child_prefix(&child).unwrap()["session_id"],child.session);
        let carrier_fixture=fixture.lines().filter(|line|!line.contains("\"type\":\"attachment\"")).collect::<Vec<_>>().join("\n")+"\n";
        let carrier_path=root.join("carrier.jsonl");let mut carrier_turn=native::NativeTurn::before(&spec,&carrier_path,"first").unwrap();std::fs::write(&carrier_path,&carrier_fixture).unwrap();assert!(carrier_turn.poll().unwrap().is_some());let carrier_generation=carrier_turn.checkpoint();
        let mut carrier_history=History::new("claude-native",carrier_path.to_str().unwrap(),session).unwrap();let carrier_page=carrier_history.steps(&carrier_path,carrier_fixture.len() as u64).unwrap();
        let carrier=carrier_page.steps.iter().find(|s|s.source["raw"].as_str().is_some_and(|r|r.contains("\"type\":\"tool_result\""))).unwrap();assert_eq!(resolve(carrier,&mut carrier_turn,&carrier_generation).unwrap().descriptor["last_chain_uuid"],"00000000-0000-4000-8000-000000000003");
        drop(history);drop(carrier_history);std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn windows_link_count_is_measured_and_aliases_refuse() {
        let root=crate::scratch("claude-windows-alias");let path=root.join("native.jsonl");std::fs::write(&path,b"native\n").unwrap();source_unaliased(&path).unwrap();
        let alias=root.join("alias.jsonl");std::fs::hard_link(&path,&alias).unwrap();assert!(source_unaliased(&path).is_err());std::fs::remove_file(alias).unwrap();source_unaliased(&path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
