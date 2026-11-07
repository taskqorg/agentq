//! Native sources are immutable input; SQLite is queried only in a private temporary copy.
use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead,BufReader};
use std::sync::atomic::{AtomicU64,Ordering};
struct Part {path:PathBuf,bytes:Vec<u8>,hash:String,identity:String}
struct Row {value:Value,file:usize,start:u64,end:u64,pointer:Value,raw:Option<String>}
struct Snapshot {files:Vec<Part>,rows:Vec<Row>,pending_bytes:u64}
impl Snapshot {
    fn file(&mut self,path:&Path)->Res<usize> {
        let mut file=File::open(path).map_err(|e|format!("history source unavailable: {e}"))?;
        let identity=native::file_id(&file)?;let mut bytes=Vec::new();file.read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        let index=self.files.len();self.files.push(Part {path:path.into(),hash:crate::sha256_hex(&bytes),bytes,identity});Ok(index)
    }
    fn json(&mut self,path:&Path)->Res<()> {
        let file=self.file(path)?;let bytes=&self.files[file].bytes;
        if path.extension().is_some_and(|s|s=="jsonl") {
            let mut offset=0u64;
            for line in bytes.split_inclusive(|b|*b==b'\n') {
                if !line.ends_with(b"\n"){self.pending_bytes+=line.len() as u64;break;}
                let text=String::from_utf8(line.to_vec()).map_err(|_|"history invalid UTF-8")?;
                if text.trim().is_empty(){offset+=line.len() as u64;continue;}
                let value=serde_json::from_str(&text).map_err(|_|"history invalid JSON")?;
                self.rows.push(Row {value,file,start:offset,end:offset+line.len() as u64,pointer:Value::Null,raw:Some(text)});offset+=line.len() as u64;
            }
        }else {
            let value:Value=serde_json::from_slice(bytes).map_err(|e|format!("history invalid JSON: {e}"))?;
            let arrays:Vec<_>=if value.is_array(){vec![("",&value)]}else{["messages","steps","history","events","message_history"].iter().filter_map(|key|value.get(key).filter(|v|v.is_array()).map(|v|(*key,v))).collect()};
            if !arrays.is_empty() {
                if let Some(object)=value.as_object(){let mut metadata=object.clone();for (key,_) in &arrays{metadata.remove(*key);}metadata.insert("_session".into(),json!(true));self.rows.push(Row {value:Value::Object(metadata),file,start:0,end:bytes.len() as u64,pointer:json!(""),raw:None});}
                for (key,array) in arrays {
                for (index,value) in array.as_array().unwrap().iter().enumerate(){self.rows.push(Row {value:value.clone(),file,start:0,end:bytes.len() as u64,pointer:json!(if key.is_empty(){format!("/{index}")}else{format!("/{key}/{index}")}),raw:None});}
                }
            }else{self.rows.push(Row {value,file,start:0,end:bytes.len() as u64,pointer:json!(""),raw:None});}
        }
        Ok(())
    }
    fn verify(&self)->Res<()> {
        for part in &self.files {
            let mut file=File::open(&part.path).map_err(|e|e.to_string())?;
            if native::file_id(&file)?!=part.identity{return Err("history source replaced during snapshot; read again".into());}
            let mut current=Vec::new();file.read_to_end(&mut current).map_err(|e|e.to_string())?;
            if crate::sha256_hex(&current)!=part.hash{return Err("history source changed during snapshot; read again".into());}
            if native::file_id(&File::open(&part.path).map_err(|e|e.to_string())?)?!=part.identity{return Err("history source replaced during snapshot; read again".into());}
        }
        Ok(())
    }
    fn manifest(&self)->Value {json!(self.files.iter().map(|f|json!({"key":f.hash,"path":f.path,"sha256":f.hash,"identity":f.identity,"size":f.bytes.len()})).collect::<Vec<_>>())}
}
fn database(path:&Path)->bool {path.extension().is_some_and(|e|e=="db" || e=="sqlite")}
static NEXT:AtomicU64=AtomicU64::new(0);
struct Temp(PathBuf);
impl Drop for Temp {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
fn copy_database(path:&Path)->Res<(Snapshot,Temp,rusqlite::Connection)> {
    let mut snapshot=Snapshot {files:Vec::new(),rows:Vec::new(),pending_bytes:0};snapshot.file(path)?;
    let wal=PathBuf::from(format!("{}-wal",path.display()));let wal_part=if wal.exists(){Some(snapshot.file(&wal)?)}else{None};
    let shm=PathBuf::from(format!("{}-shm",path.display()));let shm_part=if shm.exists(){Some(snapshot.file(&shm)?)}else{None};
    snapshot.verify()?;
    let committed=super::wal::committed(wal_part.map(|i|snapshot.files[i].bytes.as_slice()).unwrap_or(&[]),shm_part.map(|i|snapshot.files[i].bytes.as_slice()))?;
    let mut random=[0;16];ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(),&mut random).map_err(|_|"history snapshot randomness unavailable")?;
    let temp=Temp(std::env::temp_dir().join(format!("agentq-history-{}-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed),crate::sha256_hex(&random))));
    let mut builder=std::fs::DirBuilder::new();#[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;builder.mode(0o700);}
    builder.create(&temp.0).map_err(|e|e.to_string())?;
    let copy=temp.0.join("source.db");
    for (index,part) in snapshot.files.iter().enumerate().filter(|(index,_)|*index==0 || Some(*index)==wal_part) {
        let target=if index==0{copy.clone()}else{temp.0.join("source.db-wal")};
        let bytes=if index==0{part.bytes.as_slice()}else{&part.bytes[..committed]};
        let mut opts=File::options();opts.create_new(true).write(true);#[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;opts.mode(0o600);}
        use std::io::Write;opts.open(target).and_then(|mut f|f.write_all(bytes)).map_err(|e|e.to_string())?;
    }
    snapshot.verify()?;
    if wal.exists()!=wal_part.is_some() || shm.exists()!=shm_part.is_some(){return Err("history WAL/index changed during snapshot; read again".into());}
    let connection=rusqlite::Connection::open(&copy).map_err(|e|format!("history snapshot database: {e}"))?;
    connection.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;").map_err(|e|e.to_string())?;
    Ok((snapshot,temp,connection))
}
fn table(spec:&SourceSpec)->&'static str {match spec.name {"opencode2"=>"session_v2","hermes"=>"sessions",_=>"session"}}
fn fallback_id(path:&Path)->String {
    let file=path.file_name().unwrap_or_default().to_string_lossy();
    if file=="wire.jsonl" {if let Some(part)=path.ancestors().filter_map(Path::file_name).find(|n|n.to_string_lossy().starts_with("session_")){return part.to_string_lossy().into_owned();}}
    if matches!(file.as_ref(),"events.jsonl"|"chat_history.jsonl"|"summary.json"|"session_context.json"|"metadata.json"|"state.json"|"session.jsonl") {return path.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy().into_owned();}
    path.file_stem().unwrap_or_default().to_string_lossy().trim_end_matches(".messages").to_string()
}
pub(super) fn identities(spec:&SourceSpec,path:&Path)->Res<Vec<(String,Option<String>)>> {
    if database(path) {
        let (_snapshot,_temp,db)=copy_database(path)?;let table=table(spec);
        let present:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",[table],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !present{return Ok(Vec::new());}
        let mut statement=db.prepare(&format!("SELECT * FROM {table} ORDER BY id")).map_err(|e|e.to_string())?;
        let names=statement.column_names();let id=names.iter().position(|n|*n=="id").ok_or("history session id column missing")?;
        let parent=names.iter().position(|n|matches!(*n,"parent_session_id"|"parent_id"|"parentSessionId"));
        return statement.query_map([],|r|Ok((r.get::<_,String>(id)?,parent.map(|p|r.get::<_,Option<String>>(p)).transpose()?.flatten()))).map_err(|e|e.to_string())?.map(|v|v.map_err(|e|e.to_string())).collect();
    }
    if spec.name=="kimi" {
        if let Some(file)=super::kimi::companion(path).filter(|p|p.is_file()) {
            let state:Value=serde_json::from_slice(&std::fs::read(file).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if let Some(identity)=super::kimi::identity(path,&state)?{return Ok(vec![identity]);}
        }else if let Some(identity)=super::kimi::identity(path,&Value::Null)?{return Ok(vec![identity]);}
    }
    let metadata=companion(spec,path).filter(|p|p.is_file());
    let identity_path=metadata.as_deref().unwrap_or(path);
    let value=if identity_path.extension().is_some_and(|e|e=="jsonl") {
        let mut text=String::new();let mut reader=BufReader::new(File::open(identity_path).map_err(|e|e.to_string())?);
        loop {text.clear();if reader.read_line(&mut text).map_err(|e|e.to_string())?==0 || !text.ends_with('\n'){break Value::Null;}if text.trim().is_empty(){continue;}break serde_json::from_str::<Value>(&text).map_err(|_|"history invalid JSON")?;}
    }else{serde_json::from_slice::<Value>(&std::fs::read(identity_path).map_err(|e|e.to_string())?).unwrap_or(Value::Null)};
    let id=value["data"]["sessionId"].as_str().or(value["sessionId"].as_str()).or(value["session_id"].as_str()).or(value["sessionID"].as_str()).or_else(||{
        (path.extension().is_some_and(|e|e=="json") || matches!(value["type"].as_str(),Some("session"|"session_start"|"session.start"))).then(||value["id"].as_str()).flatten()
    }).map(String::from).unwrap_or_else(||fallback_id(path));
    let parent=value["parent_session_id"].as_str().or(value["parentSessionId"].as_str()).or(value["parent_id"].as_str()).map(String::from);
    Ok(vec![(id,parent)])
}
fn sql_rows(snapshot:&mut Snapshot,db:&rusqlite::Connection,query:&str,session:&str,table:&str)->Res<Vec<Value>> {
    let mut statement=db.prepare(query).map_err(|e|format!("history database schema unavailable: {e}"))?;
    let columns:Vec<_>=statement.column_names().iter().map(|n|n.to_string()).collect();let mut values=Vec::new();
    let rows=statement.query_map([session],|row| {
        let mut object=serde_json::Map::new();
        for (index,column) in columns.iter().enumerate() {
            use rusqlite::types::ValueRef;let value=match row.get_ref(index)? {ValueRef::Null=>Value::Null,ValueRef::Integer(n)=>json!(n),ValueRef::Real(n)=>json!(n),ValueRef::Text(t)=>json!(String::from_utf8_lossy(t)),ValueRef::Blob(b)=>json!(b)};
            object.insert(column.clone(),value);
        }
        Ok(Value::Object(object))
    }).map_err(|e|e.to_string())?;
    for row in rows {
        let row=row.map_err(|e|e.to_string())?;values.push(row.clone());
        snapshot.rows.push(Row {value:row.clone(),file:0,start:0,end:snapshot.files[0].bytes.len() as u64,pointer:json!({"table":table,"id":row["id"],"key":if row["id"].is_null(){json!([row["session_id"],row["model"],row["billing_provider"],row["billing_base_url"],row["billing_mode"],row["task"]])}else{Value::Null}}),raw:None});
    }
    Ok(values)
}
fn read(spec:&SourceSpec,path:&Path,session:&str)->Res<Snapshot> {
    if database(path) {
        let (mut snapshot,_temp,db)=copy_database(path)?;
        let session_rows=sql_rows(&mut snapshot,&db,&format!("SELECT * FROM {} WHERE id=?",table(spec)),session,table(spec))?;
        if session_rows.len()!=1{return Err("history native session differs or missing".into());}
        snapshot.rows[0].value["_session"]=json!(true);
        if spec.name=="hermes" {
            sql_rows(&mut snapshot,&db,"SELECT * FROM messages WHERE session_id=? ORDER BY id",session,"messages")?;
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_model_usage')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
            if exists {let begin=snapshot.rows.len();sql_rows(&mut snapshot,&db,"SELECT * FROM session_model_usage WHERE session_id=? ORDER BY model",session,"session_model_usage")?;for row in &mut snapshot.rows[begin..]{row.value["type"]=json!("usage");row.value["scope"]=json!("model");row.value["_usage_aggregate"]=json!(true);}}
        }
        else if spec.name=="opencode2" {
            let mut stmt=db.prepare("SELECT name FROM pragma_table_info('session_message')").map_err(|e|e.to_string())?;
            let cols:Vec<String>=stmt.query_map([],|r|r.get(0)).map_err(|e|e.to_string())?.collect::<Result<_,_>>().map_err(|e|e.to_string())?;
            let order=if cols.iter().any(|n|n=="seq"){"seq"}else{"time_created,id"};
            sql_rows(&mut snapshot,&db,&format!("SELECT * FROM session_message WHERE session_id=? ORDER BY {order}"),session,"session_message")?;
        }
        else {
            let messages=sql_rows(&mut snapshot,&db,"SELECT * FROM message WHERE session_id=? ORDER BY time_created,id",session,"message")?;
            sql_rows(&mut snapshot,&db,"SELECT * FROM part WHERE session_id=? ORDER BY time_created,id",session,"part")?;
            let roles:BTreeMap<_,_>=messages.iter().filter_map(|m|Some((m["id"].as_str()?.to_string(),serde_json::from_str::<Value>(m["data"].as_str()?).ok()?["role"].clone()))).collect();
            let finished:std::collections::BTreeSet<_>=snapshot.rows.iter().filter(|r|r.value["data"].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).is_some_and(|v|v["type"]=="step-finish")).filter_map(|r|r.value["message_id"].as_str().map(String::from)).collect();
            for row in &mut snapshot.rows {if let Some(role)=row.value["message_id"].as_str().and_then(|id|roles.get(id)){row.value["role"]=role.clone();}
                if row.pointer["table"]=="message" && row.value["id"].as_str().is_some_and(|id|finished.contains(id)){row.value["_usage_aggregate"]=json!(true);}
            }
        }
        validate_records(spec,path,session,&snapshot)?;snapshot.verify()?;
        return Ok(snapshot);
    }
    let mut snapshot=Snapshot {files:Vec::new(),rows:Vec::new(),pending_bytes:0};snapshot.json(path)?;
    let metadata=companion(spec,path).filter(|p|p.is_file());
    if let Some(metadata)=&metadata {
        let begin=snapshot.rows.len();snapshot.json(metadata)?;
        for row in &mut snapshot.rows[begin..]{row.value["_session"]=json!(true);}
        let count=snapshot.rows.len()-begin;snapshot.rows.rotate_right(count);
    }
    // Legacy OpenCode JSON separates the session, messages and parts into native directories.
    if spec.name=="opencode" && has(path,"session") {
        if let Some(storage)=path.ancestors().find(|p|p.file_name().is_some_and(|n|n=="storage")) {
            let mut files=Vec::new();walk(&storage.join("message").join(session),&SourceSpec{admit:|p|p.extension().is_some_and(|e|e=="json"),..*spec},&mut files)?;files.sort();
            for file in files {let value:Value=serde_json::from_slice(&std::fs::read(&file).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;snapshot.json(&file)?;
                if let Some(id)=value["id"].as_str(){let mut parts=Vec::new();walk(&storage.join("part").join(id),&SourceSpec{admit:|p|p.extension().is_some_and(|e|e=="json"),..*spec},&mut parts)?;parts.sort();for part in parts {let begin=snapshot.rows.len();snapshot.json(&part)?;for row in &mut snapshot.rows[begin..]{if row.value["role"].is_null(){row.value["role"]=value["role"].clone();}}}}
            }
        }
    }
    validate_records(spec,path,session,&snapshot)?;snapshot.verify()?;
    if companion(spec,path).filter(|p|p.is_file())!=metadata{return Err("history companion changed during snapshot; read again".into());}
    Ok(snapshot)
}

fn companion(spec:&SourceSpec,path:&Path)->Option<PathBuf> {
    match spec.name {"cline"=>super::cline::companion(path),"kiro"=>super::kiro::companion(path),"kimi"=>super::kimi::companion(path),_=>None}
}
fn validate_record(value:&Value,session:&str)->Res<()> {
    for key in ["sessionId","session_id","sessionID"] {
        if let Some(id)=value[key].as_str(){if id!=session{return Err("history native session differs".into());}}
    }
    if value["_session"]==true || matches!(value["type"].as_str(),Some("session"|"session_start"|"session.start")) {
        if value["id"].as_str().is_some_and(|id|id!=session){return Err("history native session differs".into());}
    }
    for key in ["data","message","payload","event"] {
        if value[key].is_object(){validate_record(&value[key],session)?;}
        else if key=="data" {if let Some(data)=value[key].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).filter(Value::is_object){validate_record(&data,session)?;}}
    }
    Ok(())
}
fn validate_records(spec:&SourceSpec,path:&Path,session:&str,snapshot:&Snapshot)->Res<()> {
    let expected=if spec.name=="kimi" {super::kimi::session(path).unwrap_or_else(||session.to_string())}else{session.to_string()};
    if spec.name=="kimi" {
        let state=snapshot.rows.iter().find(|r|r.value["agents"].is_object()).map(|r|&r.value).unwrap_or(&Value::Null);
        if super::kimi::identity(path,state)?.is_some_and(|(id,_)|id!=session){return Err("history native agent differs".into());}
    }
    if path.extension().is_some_and(|e|e=="json") {
        let root:Value=serde_json::from_slice(&snapshot.files[0].bytes).map_err(|e|e.to_string())?;
        if root["id"].as_str().is_some_and(|id|id!=expected){return Err("history native session differs".into());}
    }
    for row in &snapshot.rows {validate_record(&row.value,&expected)?;}
    Ok(())
}

pub(in crate::agent::history) fn steps(history:&mut History,path:&Path,bytes:u64)->Res<StepPage> {
    let mut next=history.clone();let page=read_steps(&mut next,path,bytes,None)?;*history=next;Ok(page)
}
fn read_steps(history:&mut History,path:&Path,bytes:u64,mut physical:Option<&mut Vec<(Step,Value)>>)->Res<StepPage> {
    if bytes==0{return Err("history bytes must be positive".into());}
    let spec=SPECS.iter().find(|s|s.format==history.format).ok_or("history source unsupported")?;
    let snapshot=read(spec,path,&history.native_session)?;let manifest=snapshot.manifest();let hash=crate::sha256_hex(manifest.to_string().as_bytes());
    let reset=history.snapshot_digest.as_deref()!=Some(&hash);
    if reset {
        history.offset=0;history.snapshot_revision=history.snapshot_next.checked_add(1).ok_or("history revision overflow")?;
        history.snapshot_next=history.snapshot_revision.checked_add(snapshot.rows.len() as u64).ok_or("history revision overflow")?;history.snapshot_digest=Some(hash);
    }
    let begin=history.offset;let mut consumed=0u64;let mut out=BTreeMap::<String,Step>::new();let mut changed=BTreeMap::<String,Step>::new();let mut end=begin;
    let mut turn=None;let mut presentation=json!({});
    for (index,row) in snapshot.rows.iter().enumerate() {
        if index as u64>=begin && consumed>=bytes{break;}
        let part=&snapshot.files[row.file];let record=&row.value;
        let event=json!({"id":crate::sha256_hex(json!([history.source,row.file,row.pointer,row.start]).to_string().as_bytes()),"format":history.format,"session":history.session,"source":history.source,"part":part.hash,"files":manifest,
            "start":row.start,"end":row.end,"sha256":if row.raw.is_some(){crate::sha256_hex(row.raw.as_ref().unwrap().as_bytes())}else{part.hash.clone()},"raw":row.raw,"record":record,"pointer":row.pointer,"encoding":if row.raw.is_some(){"native-jsonl"}else{"projected-row"},"author":"unknown","time":record["timestamp"]});
        let mut part_base=0;
        for (normalized_index,normalized) in decode::records(spec,record).into_iter().enumerate() {
            let mut event=event.clone();event["record_id"]=event["id"].clone();event["id"]=json!(format!("{}:{normalized_index}",event["id"].as_str().unwrap()));event["author"]=json!(normalized["type"].as_str().unwrap_or("unknown"));
            let derived=super::super::steps::contextual_steps("claude-native",&event,&normalized,&mut turn,&mut presentation);let count=derived.len();
            for mut step in derived {
                if let Some(physical)=physical.as_deref_mut(){physical.push((step.clone(),normalized.clone()));}
                step.revision=history.snapshot_revision+index as u64;step.part+=part_base;
                if normalized["type"]=="usage" && matches!(normalized["scope"].as_str(),Some("session"|"model")){step.turn.id=None;step.turn.native_id=None;step.turn.unknown=Some("native session/model aggregate; no turn attribution".into());}
                if let Some(old)=out.get(&step.id){step=super::super::steps::revise(old,step);}
                out.insert(step.id.clone(),step.clone());if index as u64>=begin{changed.insert(step.id.clone(),step);}
            }
            part_base+=count;
        }
        if index as u64>=begin{consumed=consumed.saturating_add(row.raw.as_ref().map_or_else(||row.value.to_string().len(),String::len) as u64);end=index as u64+1;}
    }
    history.identity=snapshot.files.first().map(|p|p.identity.clone());
    history.offset=end;let mut steps:Vec<_>=changed.into_values().collect();steps.sort_by_key(|s|(s.revision,s.part));
    Ok(StepPage {steps,generation:Some(history.snapshot_revision),reset,more:end<snapshot.rows.len() as u64 || snapshot.pending_bytes>0,pending_bytes:snapshot.pending_bytes,checkpoint:history.checkpoint()})
}

/// Every physical native row contributes media before logical revisions coalesce.
pub(in crate::agent::history) fn media_steps(history:&History,path:&Path)->Res<Vec<(Step,Value)>> {
    let mut history=history.clone();history.offset=0;let mut physical=Vec::new();
    read_steps(&mut history,path,u64::MAX,Some(&mut physical))?;Ok(physical)
}

pub(in crate::agent::history) fn source_bytes(history:&History,path:&Path,key:&str,start:u64,length:u64)->Res<Vec<u8>> {
    let spec=SPECS.iter().find(|s|s.format==history.format).ok_or("history source unsupported")?;
    let snapshot=read(spec,path,&history.native_session)?;
    let part=snapshot.files.iter().find(|p|p.hash==key).ok_or("history source part unavailable or changed")?;
    let end=start.checked_add(length).ok_or("history source range overflow")?;
    if end>part.bytes.len() as u64{return Err("history source range unavailable".into());}Ok(part.bytes[start as usize..end as usize].to_vec())
}
