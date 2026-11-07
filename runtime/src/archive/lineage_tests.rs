// F300-L2 SOURCE fixtures, deliberately not executed before the global functional freeze.
#[test]
fn lineage_auxiliary_spool_uses_existing_grants_and_fifteen_key_wire() {
    let (data,mut c)=setup("lineage-auxiliary");c.runtime="codex".into();c.manifest["runtime"]=json!("codex");
    register(&data,c.clone()).unwrap();
    let handle=Handle {data:data.clone(),capture:c.clone(),native:None,native_run:None};
    let intent=json!({"id":"fixture-lineage","operation":"fork","parent":"s-1","selected":null,"requested_conditions":{"model":"fixture-model"}});
    let native=json!({"thread":{"id":"fixture-child"},"model":"fixture-model"});let observed=json!({"child":"fixture-child"});
    portable::publish(&handle,&intent,&native,&observed).unwrap();
    let mut sink=Sink::default();pass(&data,&mut sink).unwrap();
    let request=sink.requests.iter().find(|r|r["format"]==agent::history::lineage::FORMAT).unwrap();
    assert_eq!(request["manifest"].as_object().unwrap().len(),15);
    assert!(request.get("turns").is_none() && request.get("media").is_none() && request.get("history").is_none());
    for key in ["model","effort","policy","account_label","cli_version","instructions_hash"]{assert!(request["manifest"][key].is_null());}
    let path=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"fixture-lineage")));
    let record=agent::history::lineage::decode(&fs::read(&path).unwrap(),"fixture-child").unwrap();
    assert!(matches!(record.parent.point,agent::history::lineage::Point::Empty));
    assert!(matches!(record.activation,agent::history::lineage::Activation::Pending {..}));
    assert_eq!(record.conditions.model.requested.after.as_deref(),Some("fixture-model"));assert_eq!(record.conditions.model.observed.after.as_deref(),Some("fixture-model"));
    assert!(record.conditions.model.observed.before.is_none());assert!(record.conditions.model.observed.changed.is_none());
    let count=sink.requests.len();pass(&data,&mut sink).unwrap();assert_eq!(sink.requests.len(),count);
    fs::remove_file(connection(&data).join("connection.json")).unwrap();pass(&data,&mut sink).unwrap();assert_eq!(sink.requests.len(),count);
    let _=fs::remove_dir_all(data.parent().unwrap());
}

#[test]
fn lineage_same_physical_jsonl_maps_across_accepted_chunks_and_keeps_hashes_private() {
    use agent::history::lineage::{native_prefix,Point};
    let (data,mut c)=setup("lineage-physical-mapping");
    c.sources[0].format="claude-native".into();
    // An invented complete message larger than one wire chunk exercises retained coverage assembly.
    let raw=format!("{}\n",json!({"type":"assistant","sessionId":"s-1","uuid":"fixture-message","message":{"content":[{"type":"text","text":"x".repeat(enroll::MAX_BODY)}]}}));
    fs::write(&c.sources[0].path,&raw).unwrap();
    register(&data,c.clone()).unwrap();
    let step=native_prefix("claude","claude-native",c.sources[0].path.to_str().unwrap(),"s-1",Some("s-1"),raw.as_bytes()).unwrap().remove(0);
    let physical=agent::archive_file_identity(&fs::File::open(&c.sources[0].path).unwrap()).unwrap();
    let handle=Handle {data:data.clone(),capture:c.clone(),native:None,native_run:None};
    let child_path=c.root.join("fixture-child.jsonl");assert!(!child_path.exists());
    let intent=json!({"id":"fixture-physical","operation":"fork","parent":"s-1","selected":step,"physical":{"path":c.sources[0].path,"identity":physical},"child_source":{"before":"absent","path":child_path}});
    let child_raw=raw.replace("s-1","fixture-child");fs::write(&child_path,&child_raw).unwrap();
    let mut child_history=agent::history::History::new("claude-native",child_path.to_str().unwrap(),"fixture-child").unwrap();
    let page=child_history.steps(&child_path,child_raw.len() as u64).unwrap();assert!(!page.more);assert_eq!(page.pending_bytes,0);
    let pid=std::process::id();
    let native=json!({"claude":{"persistence":{"session_id":"fixture-child","path":child_path,"history":child_history.checkpoint()},"initialize":{"pid":pid},"process":{"pid":pid}}});
    let observed=json!({"child":"fixture-child"});
    let mut missing=native.clone();missing["claude"]["persistence"]["history"]["prefix"]=Value::Null;
    assert!(portable::publish(&handle,&intent,&missing,&observed).is_err(),"incomplete child witness must refuse");
    portable::publish(&handle,&intent,&native,&observed).unwrap();
    let path=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"fixture-physical")));
    let first=agent::history::lineage::decode(&fs::read(&path).unwrap(),"fixture-child").unwrap();assert!(matches!(first.parent.point,Point::Unmapped {..}));
    let mut sink=Sink::default();pass(&data,&mut sink).unwrap();pass(&data,&mut sink).unwrap();
    let bytes=fs::read(&path).unwrap();let mut chain=agent::history::lineage::Chain::default();
    for row in bytes.split_inclusive(|b|*b==b'\n'){chain.append(agent::history::lineage::decode(row,"fixture-child").unwrap()).unwrap();}
    let latest=chain.latest().unwrap().unwrap();assert_eq!(latest.revision,1);
    let Point::Record {source,start,end,sha256,projection}=&latest.parent.point else{panic!("accepted physical record must map");};
    assert_eq!((*start,*end),(0,raw.len() as u64));assert_eq!(sha256,&crate::sha256_hex(raw.as_bytes()));assert!(projection.is_some());
    assert!(sink.requests.iter().filter(|r|r["source"]==*source).count()>1);
    let wire=String::from_utf8(bytes).unwrap();assert!(!wire.contains(c.sources[0].path.to_str().unwrap()));assert!(!wire.contains(&physical));
    // A foreign physical contributor cannot become a projection or source-label rewrite.
    let mut altered=intent.clone();altered["id"]=json!("fixture-unmapped");altered["physical"]["identity"]=json!("another-incarnation");
    portable::publish(&handle,&altered,&native,&observed).unwrap();
    let unknown=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"fixture-unmapped")));
    assert!(matches!(agent::history::lineage::decode(&fs::read(unknown).unwrap(),"fixture-child").unwrap().parent.point,Point::Unmapped {..}));
    let _=fs::remove_dir_all(data.parent().unwrap());
}

#[test]
fn lineage_missing_parent_grant_manual_optin_and_unsafe_session_do_not_publish() {
    let (data,mut c)=setup("lineage-grant-boundary");
    let intent=json!({"id":"fixture-denied","operation":"fork","parent":"s-1","selected":null});let native=json!({"thread":{"id":"fixture-child"}});let observed=json!({"child":"fixture-child"});
    let mut handle=Handle {data:data.clone(),capture:c.clone(),native:None,native_run:None};
    assert!(portable::publish(&handle,&intent,&native,&observed).is_err());
    c.capture="manual".into();handle.capture=c.clone();portable::publish(&handle,&intent,&native,&observed).unwrap();assert!(!root(&data).join("lineage").exists());
    c.capture="launched".into();c.manifest["session"]=json!("/private/native-session");register(&data,c.clone()).unwrap();handle.capture=c;
    let mut unsafe_intent=intent;unsafe_intent["parent"]=json!("/private/native-session");assert!(portable::publish(&handle,&unsafe_intent,&native,&observed).is_err());assert!(!root(&data).join("lineage").exists());
    let _=fs::remove_dir_all(data.parent().unwrap());
}

#[test]
fn selected_bridge_uses_measured_journal_and_exact_caller_decoder_domain() {
    use agent::history::lineage::{self as lineage,Point,Activation};
    let (data,mut parent)=setup("lineage-selected-bridge");parent.runtime="codex".into();parent.manifest["runtime"]=json!("codex");
    let value=json!({"method":"item/completed","params":{"threadId":"s-1","turnId":"turn-parent","item":{"id":"fixture-item","type":"agentMessage","text":"fixture retained"}}});
    let journal=format!("{value}\n");let selected=format!(" {} \r\n",value);
    let journal_path=parent.sources[0].path.clone();fs::write(&journal_path,&journal).unwrap();
    let identity=agent::archive_file_identity(&fs::File::open(&journal_path).unwrap()).unwrap();
    let origin=JournalRecord {root:parent.root.clone(),path:journal_path.clone(),identity:identity.clone(),start:0,end:journal.len() as u64,sha256:crate::sha256_hex(journal.as_bytes())};
    let selected_path=root(&data).join("streams").join("parent-selected.jsonl");
    super::super::selected::write_selected(&selected_path,selected.as_bytes(),Some(&origin),"s-1",Some("turn-parent"),None,false).unwrap();
    parent.sources=vec![Source {path:selected_path.clone(),format:agent::history::CODEX_APP_SERVER_SELECTED.into(),start:0,end:None}];parent.turn=Some("fixture-old-turn".into());register(&data,parent.clone()).unwrap();
    let mut sink=Sink::default();pass(&data,&mut sink).unwrap();
    let step=lineage::native_prefix("codex","codex-app-server",journal_path.to_str().unwrap(),"s-1",Some("s-1"),journal.as_bytes()).unwrap().remove(0);
    let handle=Handle {data:data.clone(),capture:parent.clone(),native:None,native_run:None};
    let intent=json!({"id":"fixture-selected-fork","operation":"fork","parent":"s-1","selected":step,"physical":{"path":journal_path,"identity":identity}});
    portable::publish(&handle,&intent,&json!({"thread":{"id":"child"}}),&json!({"child":"child"})).unwrap();
    let sidecar=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"fixture-selected-fork")));
    let record=lineage::decode(&fs::read(&sidecar).unwrap(),"child").unwrap();
    let Point::Record {source,start,end,sha256,projection}=&record.parent.point else{panic!("measured selected physical bridge must map");};
    assert_eq!((*start,*end),(0,selected.len() as u64));assert_eq!(sha256,&crate::sha256_hex(selected.as_bytes()));assert_ne!(sha256,&origin.sha256);assert!(projection.is_some());
    let parent_request=sink.requests.iter().find(|r|r["source"]==*source).unwrap();let history=&parent_request["history"];
    let canonical=lineage::native_prefix(history["runtime"].as_str().unwrap(),history["format"].as_str().unwrap(),history["source"].as_str().unwrap(),history["binding"].as_str().unwrap(),history["native_session"].as_str(),selected.as_bytes()).unwrap();
    assert_eq!(projection.as_ref().unwrap().part,canonical[0].part as u64);
    assert!(matches!(record.activation,Activation::Pending {..}));
    let mut rewind=intent.clone();rewind["id"]=json!("fixture-selected-rewind");rewind["operation"]=json!("rewind");rewind["archive_primary"]=json!({"path":selected_path,"identity":agent::archive_file_identity(&fs::File::open(&selected_path).unwrap()).unwrap(),"end":selected.len(),"format":agent::history::CODEX_APP_SERVER_SELECTED,"session":"s-1"});
    portable::publish(&handle,&rewind,&json!({"thread":{"id":"s-1"}}),&json!({"thread":{"id":"s-1"}})).unwrap();
    let rewind_path=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"fixture-selected-rewind")));
    let rewind_record=lineage::decode(&fs::read(rewind_path).unwrap(),"s-1").unwrap();assert_eq!(rewind_record.predecessor.as_ref().unwrap().source,*source);
    // A distinct actual child input supplies the operation's first selected byte, not copied parent work.
    let mut child=parent.clone();child.id="fixture-child-run".into();child.manifest["session"]=json!("child");child.manifest["parent_session"]=json!("s-1");
    let child_journal=parent.root.join("child-control.jsonl");
    let child_value=json!({"method":"turn/started","params":{"threadId":"child","turn":{"id":"child-turn"}}});let child_raw=format!("{child_value}\n");fs::write(&child_journal,&child_raw).unwrap();
    let child_origin=JournalRecord {root:parent.root.clone(),path:child_journal.clone(),identity:agent::archive_file_identity(&fs::File::open(&child_journal).unwrap()).unwrap(),start:0,end:child_raw.len() as u64,sha256:crate::sha256_hex(child_raw.as_bytes())};
    let child_selected=root(&data).join("streams").join("child-selected.jsonl");
    super::super::selected::write_selected(&child_selected,child_raw.as_bytes(),Some(&child_origin),"child",Some("child-turn"),Some("fixture-selected-fork"),true).unwrap();
    child.sources=vec![Source {path:child_selected,format:agent::history::CODEX_APP_SERVER_SELECTED.into(),start:0,end:None}];register(&data,child).unwrap();
    pass(&data,&mut sink).unwrap();pass(&data,&mut sink).unwrap();
    let bytes=fs::read(&sidecar).unwrap();let mut chain=lineage::Chain::default();for row in bytes.split_inclusive(|b|*b==b'\n'){chain.append(lineage::decode(row,"child").unwrap()).unwrap();}
    let Activation::Byte {source,start,manifest_sha256}=&chain.latest().unwrap().unwrap().activation else{panic!("exact accepted first child byte must activate");};
    assert_eq!(*start,0);let affected=sink.requests.iter().find(|r|r["source"]==*source).unwrap();assert_eq!(&affected["manifest_sha256"],manifest_sha256);assert_eq!(affected["manifest"]["session"],"child");
    let _=fs::remove_dir_all(data.parent().unwrap());
}

#[test]
fn missed_first_native_frame_never_activates_a_later_captured_frame() {
    let (data,mut capture)=setup("lineage-selected-first-fence");capture.runtime="codex".into();capture.manifest["runtime"]=json!("codex");
    let journal=capture.sources[0].path.clone();let raw=format!("{}\n",json!({"method":"turn/started","params":{"threadId":"s-1","turn":{"id":"turn"}}}));fs::write(&journal,&raw).unwrap();
    let origin=JournalRecord {root:capture.root.clone(),path:journal.clone(),identity:agent::archive_file_identity(&fs::File::open(&journal).unwrap()).unwrap(),start:0,end:raw.len() as u64,sha256:crate::sha256_hex(raw.as_bytes())};
    let selected=root(&data).join("streams").join("late-selected.jsonl");let connection_path=connection(&data).join("connection.json");let saved=fs::read(&connection_path).unwrap();fs::remove_file(&connection_path).unwrap();
    super::super::selected::write_selected(&selected,raw.as_bytes(),Some(&origin),"s-1",Some("turn"),Some("missed-first"),true).unwrap();assert!(!selected.exists());fs::write(connection_path,saved).unwrap();
    // The runtime already consumed its first-frame flag while ungranted; no storage inference may reset it.
    super::super::selected::write_selected(&selected,raw.as_bytes(),Some(&origin),"s-1",Some("turn"),Some("missed-first"),false).unwrap();
    capture.sources=vec![Source {path:selected.clone(),format:agent::history::CODEX_APP_SERVER_SELECTED.into(),start:0,end:None}];register(&data,capture.clone()).unwrap();
    let handle=Handle {data:data.clone(),capture,native:None,native_run:None};let intent=json!({"id":"missed-first","operation":"rewind","parent":"s-1","selected":null});let evidence=json!({"thread":{"id":"s-1"}});portable::publish(&handle,&intent,&evidence,&evidence).unwrap();
    let mut sink=Sink::default();pass(&data,&mut sink).unwrap();pass(&data,&mut sink).unwrap();assert!(super::super::selected::bindings(&selected).unwrap().is_empty());
    let sidecar=root(&data).join("lineage").join(format!("{}.jsonl",crate::sha256_hex(b"missed-first")));
    let record=agent::history::lineage::decode(&fs::read(sidecar).unwrap(),"s-1").unwrap();assert!(matches!(record.activation,agent::history::lineage::Activation::Pending {..}));
    let _=fs::remove_dir_all(data.parent().unwrap());
}
