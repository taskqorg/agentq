//! Claude-private #300 selection. A source witness is never a child-loaded-prefix receipt.
use super::*;
use crate::agent::history::{History,Step,StepKind};

fn uuid(value:&Value)->Res<&str> {
    let id=value.as_str().ok_or("fork-native-uuid-unknown")?;
    if id.len()!=36 || !id.bytes().enumerate().all(|(i,b)|if matches!(i,8|13|18|23) {b==b'-'}else{b.is_ascii_hexdigit()}) {return Err("fork-native-uuid-unknown".into());}Ok(id)
}

/// Only measured identifiers, source generation and hashes may cross into later lineage projection.
struct Boundary { descriptor:Value, generation:Value, path:PathBuf }
struct PreparedFork { boundary:Boundary, intent:Value }
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

impl Session {
    pub fn fork(&mut self,through:Option<&Step>,launch:&Launch)->Result<Run,Failure> {
        let refuse=|cause|Failure::Refused(Cause(cause));
        if !ADAPTER.descriptor.has("fork") {return Err(refuse("fork-unsupported"));}
        let prepared=self.prepare_fork(through,launch)?;
        // No guessed control/receipt: this backend has no established held local-fork admission.
        self.owner.control()?;prepared.boundary.revalidate()?;
        let _intent=prepared.intent;
        Err(refuse("fork-native-held-receipt-unavailable"))
    }

    fn prepare_fork(&mut self,through:Option<&Step>,launch:&Launch)->Result<PreparedFork,Failure> {
        let refuse=|cause|Failure::Refused(Cause(cause));
        self.writable()?;
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
        // No admission, spawn, initialize or model input until the backend can hold all queue producers.
        self.owner.control()?;boundary.revalidate()?;
        let intent=boundary.intent(step,self.owner.checkpoint()?,launch);
        Ok(PreparedFork {boundary,intent})
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
