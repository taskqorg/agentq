//! Grant-checked auxiliary construction and private physical-to-accepted mapping.
use super::*;
use crate::agent::history::{Step, lineage::{self as wire, *}};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Auxiliary { path:PathBuf, operation:String, identity:String, parent:Capture, intent:Value, native:Value, observed:Value }

fn condition<T:Serialize+serde::de::DeserializeOwned>(value:&Value)->Option<T> {
    let bytes=serde_json::to_vec(value).ok()?;
    let (scrubbed,_)=crate::archive::scrub::bytes(&bytes);
    if bytes!=scrubbed{return None;}
    serde_json::from_value(value.clone()).ok()
}
fn conditions(intent:&Value,native:&Value)->Conditions {
    let mut c=Conditions::default();
    // Parent effective/requested settings are not recoverable from mixed legacy manifests.
    c.model.requested=Pair::new(None,condition::<String>(&intent["requested_conditions"]["model"]).filter(|s|safe_label(s)));
    c.effort.requested=Pair::new(None,condition::<String>(&intent["requested_conditions"]["effort"]).filter(|s|safe_label(s)));
    c.policy.requested=Pair::new(None,condition::<PortablePolicy>(&intent["requested_conditions"]["policy"]));
    // Only the exact verified fork acknowledgement supplies an effective child model.
    if intent["operation"]=="fork" {let model=if native["claude"].is_object() {&native["claude"]["settings"]["applied"]["model"]}else{&native["model"]};c.model.observed=Pair::new(None,condition::<String>(model).filter(|s|safe_label(s)));}
    // Invalid/unsafe policy disclosure withholds the whole cell, not selected fields.
    let probe=Record {record_type:"agentq_lineage".into(),version:1,operation_id:"disclosure".into(),revision:0,kind:Kind::Rewind,effect:Effect::Noop,session:"disclosure".into(),parent:Parent {session:"disclosure".into(),point:Point::Empty},predecessor:None,activation:Activation::Pending {reason:Pending::NoNativeBytes},conditions:c.clone()};
    if probe.validate("disclosure").is_err(){c.policy.requested=Pair::default();}
    c
}

/// Accepted chunks from one immutable capture generation, verified against their durable receipts.
fn accepted_prefix(st:&Checkpoint,frame:&crate::archive::lineage::Frame,grant:&Value,c:&Capture)->Res<Vec<u8>> {
    if c.grant!=identity(grant) || excluded(&c.root,&c.root)? {return Err("archive parent grant unavailable".into());}
    let mut chunks:Vec<_>=st.accepted.iter().filter(|a|a.body["source"]==frame.source).collect();
    chunks.sort_by_key(|a|a.body["start"].as_u64());
    let mut bytes=Vec::new();
    for a in chunks {
        let b=&a.body;let r=&a.receipt;
        if a.authority.grant!=identity(grant) || a.authority.root!=c.root || a.authority.runtime!=c.runtime || a.authority.home!=c.home || c.root.canonicalize().ok().as_ref()!=Some(&a.authority.canonical_root) || excluded(&a.authority.canonical_root,&a.authority.canonical_root)? || !allowed(grant,&a.authority.capture) || a.authority.evidence["identity"]!=frame.identity || a.authority.evidence["path"].as_str()!=frame.path.to_str() || a.authority.evidence["start"]!=frame.base || b["format"]!=frame.format || b["manifest"]["session"]!=c.manifest["session"] || b["media"]==true || b["format"]==wire::FORMAT || r["state"]!="accepted" || r["complete"]!=true {return Err("archive accepted frame unavailable".into());}
        for k in ["source","start","end","sha256","manifest_sha256"] {if b[k]!=r[k]{return Err("archive accepted coverage changed".into());}}
        if frame.format==agent::history::CODEX_APP_SERVER_SELECTED && (b["history"]!=frame.history.clone().unwrap_or(Value::Null) || a.authority.evidence["history"]!=b["history"]) {return Err("archive selected caller binding changed".into());}
        if enroll::digest(&b["manifest"])!=b["manifest_sha256"] || crate::sha256_hex(&a.bytes)!=b["sha256"] || agent::encode_base64(&a.bytes)!=b["bytes"] || b["end"].as_u64().and_then(|end|end.checked_sub(b["start"].as_u64()?))!=Some(a.bytes.len() as u64) {return Err("archive accepted bytes changed".into());}
        for p in &a.authority.paths {if excluded(&a.authority.root,p)? || excluded(&a.authority.canonical_root,p)?{return Err("archive parent excluded".into());}}
        if b["start"]!=bytes.len() as u64 {return Err("archive accepted prefix incomplete".into());}
        bytes.extend_from_slice(&a.bytes);
    }
    Ok(bytes)
}

fn point(data:&Path,grant:&Value,parent:&Capture,intent:&Value)->Point {
    if intent["selected"].is_null() {return Point::Empty;}
    let Ok(step)=serde_json::from_value::<Step>(intent["selected"].clone()) else {return Point::Unmapped {reason:Unmapped::MappingUnavailable};};
    let Some(identity)=intent["physical"]["identity"].as_str() else {return Point::Unmapped {reason:Unmapped::MappingUnavailable};};
    if intent["physical"]["path"]!=step.source["source"] {return Point::Unmapped {reason:Unmapped::MappingUnavailable};}
    if step.source["format"].as_str().is_none_or(|f|f.starts_with("history-") || f==wire::FORMAT || f=="raw") {return crate::archive::lineage::unsupported();}
    let Ok(paths)=files(&root(data).join("sources")) else {return Point::Unmapped {reason:Unmapped::MappingUnavailable};};
    let mut candidates=Vec::new();let mut archived=false;
    for path in paths {
        let Ok(st)=read::<Checkpoint>(&path) else {continue;};
        if st.grant!=identity_of(grant){continue;}
        for frame in &st.frames {
            let mapped=if frame.identity==identity && frame.path.to_str()==step.source["source"].as_str(){Some(step.clone())}else{crate::archive::selected::map(&step,identity,frame,parent)};
            let Some(mapped)=mapped else{continue;};
            archived=true;
            let Ok(bytes)=accepted_prefix(&st,frame,grant,parent) else {continue;};
            if let Some(mut anchor)=frame.anchor(&mapped,&frame.identity,&bytes){
                let projection=frame.projection(&mapped,&frame.identity,&bytes,&parent.runtime,parent.manifest["session"].as_str().unwrap_or_default(),&anchor);
                if let Point::Record {projection:p,..}=&mut anchor {*p=projection;}
                candidates.push(anchor);
            }
        }
    }
    if candidates.len()==1 {candidates.remove(0)}else{Point::Unmapped {reason:if archived {Unmapped::MappingUnavailable}else{Unmapped::NotArchived}}}
}

pub(super) fn publish(handle:&Handle,intent:&Value,native:&Value,observed:&Value)->Res<()> {
    let data=&handle.data;
    let Some(grant)=active(data)? else {return Ok(());};
    let mut c=handle.capture.clone();
    if !allowed(&grant,&c.capture) || !enabled(data)? || excluded(&c.root,&c.root)? {return Ok(());}
    // Only an existing scoped capture can provide an authorized portable session label.
    let prior=root(data).join("captures").join(format!("{}.json",crate::sha256_hex(c.id.as_bytes())));
    let registered:Registration=read(&prior).map_err(|_|"archive lineage parent grant unavailable")?;
    if registered.capture.grant!=identity(&grant) || registered.capture.root!=c.root || registered.capture.runtime!=c.runtime || registered.capture.home!=c.home || registered.capture.manifest["session"]!=c.manifest["session"] {return Err("archive lineage parent grant changed".into());}
    c=registered.capture;
    // Never republish an old private operation into a replacement destination/grant scope.
    if !c.grant.is_null() && c.grant!=identity(&grant){return Err("archive lineage grant changed".into());}
    let parent=intent["parent"].as_str().filter(|s|safe_label(s)).ok_or("archive lineage unsafe parent session")?;
    // The pre-existing capture must already authorize the exact parent label; no invented aliases.
    if c.manifest["session"]!=parent{return Err("archive lineage parent label unavailable".into());}
    let kind=match intent["operation"].as_str(){Some("fork")=>Kind::Fork,Some("rewind")=>Kind::Rewind,_=>return Err("archive lineage operation unsupported".into())};
    let session=if kind==Kind::Fork {observed["child"].as_str().filter(|s|safe_label(s)).ok_or("archive lineage child identity unavailable")?}else{parent};
    if kind==Kind::Fork {
        let matches=if c.runtime=="claude" {
            let identity=&native["claude"]["identity"];
            let persisted=&native["claude"]["persistence"];let history=&persisted["history"];
            let observed=identity.is_null() || identity["type"]=="system" && matches!(identity["subtype"].as_str(),Some("session_metadata"|"session_state_changed")) && identity["session_id"]==session;
            observed && persisted["session_id"]==session && history["session"]==session && history["native_session"]==session && history["format"]=="claude-native"
                && history["source"]==persisted["path"] && history["identity"].as_str().is_some_and(|s|!s.is_empty()) && history["offset"].as_u64().is_some_and(|n|n>0)
                && history["prefix"].as_str().is_some_and(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())) && history["native_session_alias"].is_null()
                && intent["child_source"]["before"]=="absent" && intent["child_source"]["path"]==persisted["path"]
                && native["claude"]["initialize"]["pid"].as_u64().is_some_and(|pid|Some(pid)==native["claude"]["process"]["pid"].as_u64())
        }else{native["thread"]["id"]==session};
        if !matches || session==parent {return Err("archive lineage child result unknown".into());}
    }
    if kind==Kind::Rewind && native["thread"]["id"]!=session {return Err("archive lineage rewind result unknown".into());}
    let effect=if intent["effect"]=="noop" && observed["noop"]==true {Effect::Noop}else{Effect::Applied};
    let operation=intent["id"].as_str().filter(|s|safe_label(s)).ok_or("archive lineage operation identity unavailable")?;
    c.grant=identity(&grant);
    let anchor=point(data,&grant,&c,intent);
    let mut affected=c.clone();affected.manifest["session"]=json!(session);
    let record=Record {record_type:"agentq_lineage".into(),version:1,operation_id:operation.into(),revision:0,kind,effect,session:session.into(),parent:Parent {session:parent.into(),point:anchor},predecessor:if kind==Kind::Rewind {predecessor(data,&grant,&c,intent)}else{None},activation:activation(data,&grant,&affected,operation),conditions:conditions(intent,native)};
    let parent_capture=c.clone();
    c.id=operation.into();c.turn=None;c.manifest["session"]=json!(session);
    if kind==Kind::Fork {c.manifest["parent_session"]=json!(parent);}
    for k in ["cli_version","model","effort","policy","account_label","instructions_hash"]{c.manifest[k]=Value::Null;}
    let dir=root(data).join("lineage");private(&dir)?;
    let key=crate::sha256_hex(operation.as_bytes());
    let path=dir.join(format!("{key}.jsonl"));
    append_record(&path,record)?;
    c.sources=vec![Source {path:path.clone(),format:wire::FORMAT.into(),start:0,end:None}];
    let auxiliary=Auxiliary {identity:agent::archive_file_identity(&fs::File::open(&path).map_err(|_|"archive lineage unavailable")?)?,path,operation:operation.into(),parent:parent_capture,intent:intent.clone(),native:native.clone(),observed:observed.clone()};
    let captures=root(data).join("captures");private(&captures)?;
    let _registration=coreq::file_lock::lock(&captures.join(format!("{key}.lock"))).map_err(|_|"archive lineage registration locked")?;
    write(&captures.join(format!("{key}.json")),&Registration {capture:c,native:None,auxiliary:Some(auxiliary)})
}

pub(super) fn upload_auxiliary(data:&Path,grant:&Value,c:&Capture,aux:&Auxiliary,wire:&mut dyn Wire)->Res<()> {
    if c.grant!=identity(grant) || !allowed(grant,&c.capture) || !enabled(data)? || aux.parent.grant!=identity(grant) || c.turn.is_some() || c.sources.len()!=1 || c.id!=aux.operation {return Err("archive auxiliary authority invalid".into());}
    let src=&c.sources[0];
    if src.path!=aux.path || src.format!=FORMAT || src.start!=0 || src.end.is_some() || !src.path.starts_with(root(data).join("lineage")) || excluded(&c.root,&src.path)? {return Err("archive auxiliary source invalid".into());}
    let file=fs::File::open(&src.path).map_err(|_|"archive auxiliary unavailable")?;
    if agent::archive_file_identity(&file)?!=aux.identity {return Err("archive auxiliary incarnation changed".into());}
    // Existing shipper cadence may refine a previously unknown accepted mapping, never native work.
    let session=c.manifest["session"].as_str().ok_or("archive auxiliary session unavailable")?;
    let existing=fs::read(&aux.path).map_err(|_|"archive auxiliary unavailable")?;
    let mut retained=Chain::default();
    for row in existing.split_inclusive(|b|*b==b'\n'){retained.append(decode(row,session)?)?;}
    if let Some(latest)=retained.latest()? {
        let mut next=latest.clone();
        if matches!(next.parent.point,Point::Unmapped {..}) {
            let mapped=point(data,grant,&aux.parent,&aux.intent);
            if !matches!(mapped,Point::Unmapped {..}){next.parent.point=mapped;}
        }else if let Point::Record {projection:None,..}=&next.parent.point {
            let mapped=point(data,grant,&aux.parent,&aux.intent);
            if let Point::Record {projection:Some(_),..}=&mapped {next.parent.point=mapped;}
        }
        if next.kind==Kind::Rewind && next.predecessor.is_none(){next.predecessor=predecessor(data,grant,&aux.parent,&aux.intent);}
        if matches!(next.activation,Activation::Pending {..}) {next.activation=activation(data,grant,c,&aux.operation);}
        append_record(&aux.path,next)?;
    }
    let bytes=fs::read(&src.path).map_err(|_|"archive auxiliary unavailable")?;
    let session=c.manifest["session"].as_str().ok_or("archive auxiliary session unavailable")?;
    let mut chain=Chain::default();
    for row in bytes.split_inclusive(|b|*b==b'\n') {let r=decode(row,session)?;if r.operation_id!=aux.operation{return Err("archive auxiliary operation changed".into());}chain.append(r)?;}
    if chain.latest()?.is_none() || crate::archive::scrub::bytes(&bytes).0!=bytes {return Err("archive auxiliary empty or scrubbed".into());}
    let mut manifest=c.manifest.clone();
    for k in ["cli_version","model","effort","policy","account_label","instructions_hash"] {if !manifest[k].is_null(){return Err("archive auxiliary condition manifest invalid".into());}}
    upload(data,grant,c,src,&mut manifest,None,None,wire)
}

fn append_record(path:&Path,mut next:Record)->Res<()> {
    let _lock=coreq::file_lock::lock(&path.with_extension("lock")).map_err(|_|"archive lineage locked")?;
    let mut chain=Chain::default();
    if path.exists() {
        let old=fs::read(path).map_err(|_|"archive lineage unreadable")?;
        for row in old.split_inclusive(|b|*b==b'\n'){chain.append(decode(row,&next.session)?)?;}
        if let Some(latest)=chain.latest()? {
            next.revision=latest.revision;
            if latest==&next{return Ok(());}
            next.revision=latest.revision.checked_add(1).ok_or("lineage revision overflow")?;
        }
    }
    chain.append(next.clone())?;
    let bytes=next.line()?;
    if crate::archive::scrub::bytes(&bytes).0!=bytes{return Err("archive lineage structural scrub refused".into());}
    let mut options=fs::OpenOptions::new();options.create(true).append(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(path).map_err(|_|"archive lineage unavailable")?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"archive lineage unavailable".into())
}

fn activation(data:&Path,grant:&Value,parent:&Capture,operation:&str)->Activation {
    let pending=||Activation::Pending {reason:Pending::MappingUnavailable};
    let Ok(paths)=files(&root(data).join("sources")) else{return pending();};
    let mut candidates=Vec::new();
    for path in paths {
        let Ok(st)=read::<Checkpoint>(&path) else{continue;};if st.grant!=identity_of(grant){continue;}
        for frame in &st.frames {
            if frame.format!=agent::history::CODEX_APP_SERVER_SELECTED {continue;}
            let Ok(bindings)=crate::archive::selected::bindings(&frame.path) else{continue;};
            for binding in bindings.iter().filter(|b|b.operation==operation && b.source==frame.source && b.captured.identity==frame.identity) {
                let Ok(bytes)=accepted_prefix(&st,frame,grant,parent) else{continue;};
                let Some(end)=binding.captured.end.checked_sub(frame.base) else{continue;};
                if end>bytes.len() as u64 || binding.start>=end {continue;}
                // The manifest at that actual accepted native byte must be the one bound before queueing.
                if !st.accepted.iter().any(|a|a.body["source"]==binding.source && a.body["manifest_sha256"]==binding.manifest_sha256 && a.body["start"].as_u64().is_some_and(|s|s<=binding.start) && a.body["end"].as_u64().is_some_and(|e|e>binding.start)){continue;}
                let value=Activation::Byte {source:binding.source.clone(),start:binding.start,manifest_sha256:binding.manifest_sha256.clone()};
                if !candidates.contains(&value){candidates.push(value);}
            }
        }
    }
    if candidates.len()==1{candidates.remove(0)}else{pending()}
}

fn predecessor(data:&Path,grant:&Value,parent:&Capture,intent:&Value)->Option<SourceRef> {
    let primary=&intent["archive_primary"];
    if primary["session"]!=parent.manifest["session"] || primary["format"]!=agent::history::CODEX_APP_SERVER_SELECTED{return None;}
    let mut candidates=Vec::new();
    for path in files(&root(data).join("sources")).ok()? {
        let st:Checkpoint=read(&path).ok()?;
        if st.owner!=parent.id || st.grant!=identity_of(grant){continue;}
        if parent.turn.as_ref().is_none_or(|turn|!st.bound.contains(turn)){continue;}
        for frame in &st.frames {
            if frame.source!=st.source || primary["identity"]!=frame.identity || primary["path"].as_str()!=frame.path.to_str(){continue;}
            let accepted=accepted_prefix(&st,frame,grant,parent).ok()?;
            if accepted.is_empty() || frame.base.checked_add(accepted.len() as u64)? > primary["end"].as_u64()? {continue;}
            let source=SourceRef {session:parent.manifest["session"].as_str()?.into(),source:frame.source.clone()};
            if !candidates.contains(&source){candidates.push(source);}
        }
    }
    if candidates.len()==1 {candidates.pop()}else{None}
}
