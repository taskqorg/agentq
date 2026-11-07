//! Private continuation descriptors grant no read outside the selected native profile inventory.
use super::*;
use crate::hook_spool::Delivery;
use agentq_runtime::agent::{archive_file_identity,history::sessions};
use std::fs::{self,File};

fn mark(state:&Path,path:&Path)->PathBuf {state.join(crate::session::event_id("transcript",&path.to_string_lossy())[..16].to_string())}
fn descriptor(state:&Path,path:&Path)->PathBuf {mark(state,path).with_extension("read.json")}
fn identity(path:&Path)->Result<String,String> {
    if !fs::symlink_metadata(path).map_err(|_|"continuation source unavailable")?.is_file() {return Err("continuation source not regular".into());}
    archive_file_identity(&File::open(path).map_err(|_|"continuation source unavailable")?)
}

pub(super) fn retain(delivery:&Delivery,runtime:&str,project:&str,session:Option<&str>,path:&Path,outcome:(&str,&str),model:Option<&str>)->Result<String,String> {
    delivery.scope(runtime,project)?;
    let path=path.canonicalize().map_err(|_|"continuation source unavailable")?;
    let identity=identity(&path)?;
    let row=json!({"schema":1,"runtime":runtime,"project":project,"session":session,"home":crate::onboard::root(runtime),"path":path,"identity":identity,"outcome":outcome,"model":model});
    let target=descriptor(&delivery.state,&path);
    match fs::read(&target) {
        Ok(bytes)=>{
            let saved:Value=serde_json::from_slice(&bytes).map_err(|_|"continuation descriptor corrupt")?;
            if ["schema","runtime","home","path","identity"].iter().any(|key|saved[key]!=row[key])
                || saved["session"]!=row["session"] && !saved["session"].is_null()
                || saved["project"]!=row["project"] && saved["project"]!="other" {return Err("continuation source binding changed".into());}
            if saved!=row {coreq::write_durable(&target,row.to_string())?;}
        },
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>coreq::write_durable(&target,row.to_string())?,
        Err(_)=>return Err("continuation descriptor unavailable".into()),
    }
    Ok(identity)
}
fn descriptors(delivery:&Delivery)->Result<Vec<PathBuf>,String> {
    match fs::read_dir(&delivery.state) {
        Ok(rows)=>rows.map(|row|row.map(|row|row.path()).map_err(|_|"continuation directory unavailable".into())).collect::<Result<Vec<_>,String>>().map(|rows|rows.into_iter().filter(|p|p.file_name().is_some_and(|n|n.to_string_lossy().ends_with(".read.json"))).collect()),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(vec![]),
        Err(_)=>Err("continuation directory unavailable".into()),
    }
}
fn read(delivery:&Delivery,file:&Path)->Result<Value,String> {
    if !fs::symlink_metadata(file).map_err(|_|"continuation descriptor unavailable")?.is_file() {return Err("continuation descriptor not regular".into());}
    let row:Value=serde_json::from_slice(&fs::read(file).map_err(|_|"continuation descriptor unavailable")?).map_err(|_|"continuation descriptor corrupt")?;
    if row.as_object().is_none_or(|o|o.len()!=9) || row["schema"]!=1 || !(row["session"].is_null() || row["session"].is_string()) {return Err("continuation descriptor invalid".into());}
    let runtime=row["runtime"].as_str().ok_or("continuation runtime unknown")?;
    let project=row["project"].as_str().ok_or("continuation project unknown")?;
    delivery.scope(runtime,project)?;
    let home=crate::onboard::root(runtime);
    if row["home"]!=json!(home) {return Err("continuation profile changed".into());}
    let path=Path::new(row["path"].as_str().ok_or("continuation source unknown")?);
    if !path.is_absolute() || descriptor(&delivery.state,path)!=file {return Err("continuation source changed".into());}
    Ok(row)
}
fn remaining(delivery:&Delivery,row:&Value)->Result<u64,String> {
    let path=Path::new(row["path"].as_str().ok_or("continuation path unknown")?);
    let saved=match fs::read_to_string(mark(&delivery.state,path)) {
        Ok(saved)=>saved.trim().parse::<u64>().map_err(|_|"continuation checkpoint corrupt")?,
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>0,
        Err(_)=>return Err("continuation checkpoint unavailable".into()),
    };
    let length=fs::metadata(path).map_err(|_|"continuation source unavailable")?.len();
    Ok(if saved>length {length}else{length-saved})
}
fn native_source(row:&Value)->Result<(),String> {
    let runtime=row["runtime"].as_str().ok_or("continuation runtime unknown")?;
    let home=Path::new(row["home"].as_str().ok_or("continuation profile unknown")?);
    let path=Path::new(row["path"].as_str().ok_or("continuation source unknown")?);
    let canonical_home=home.canonicalize().map_err(|_|"continuation profile unavailable")?;
    if !path.starts_with(&canonical_home) {return Err("continuation source outside native profile".into());}
    let sources=sessions::list(runtime,Some(home))?;
    let mut matching=sources.iter().filter(|source|source.path.canonicalize().ok().as_deref()==Some(path));
    let source=matching.next().ok_or("continuation source not in native inventory")?;
    if matching.next().is_some() || row["session"].as_str().is_some_and(|s|source.session!=s && source.parent.as_deref()!=Some(s)) {return Err("continuation native binding differs".into());}
    if row["identity"]!=identity(path)? {return Err("continuation source incarnation changed".into());}
    Ok(())
}

// Existing retained legacy backlog is an input to migration, never authority for an arbitrary source read.
fn retain_legacy(data:&Path)->Result<(),String> {
    let pending=crate::model::marks(data).into_iter().collect::<Result<Vec<_>,_>>()?;
    if !pending.iter().any(|(_,left)|*left>0) {return Ok(());}
    if crate::control::disabled(data,"claude")? {return Ok(());}
    let home=crate::onboard::root("claude");let sources=sessions::list("claude",Some(&home))?;
    for (path,left) in pending {
        if left==0 {continue;}
        let path=path.canonicalize().map_err(|_|"legacy continuation source unavailable")?;
        let mut matching=sources.iter().filter(|source|source.path.canonicalize().ok().as_ref()==Some(&path));
        let source=matching.next().ok_or("legacy continuation outside native inventory")?;
        if matching.next().is_some() {return Err("legacy continuation source ambiguous".into());}
        let session=source.parent.as_deref().unwrap_or(&source.session);
        // Project attribution was not retained by the old offset format; never infer it from a native path.
        let delivery=Delivery::open(data,"claude","other")?;
        if !descriptor(&delivery.state,&path).try_exists().map_err(|_|"continuation descriptor unavailable")? {
            retain(&delivery,"claude","other",Some(session),&path,("ok","none"),None)?;
        }
    }
    Ok(())
}

pub fn drain(data:&Path,deadline:Instant)->Result<(),String> {
    let mut error=retain_legacy(data).err();
    for delivery in crate::hook_spool::continuation_locations(data)? {
        for file in descriptors(&delivery)? {
            if Instant::now()>=deadline {return error.map_or(Ok(()),Err);}
            let result=(|| {
                let row=read(&delivery,&file)?;
                native_source(&row)?;
                if remaining(&delivery,&row)?==0 {return Ok(());}
                let runtime=row["runtime"].as_str().ok_or("continuation runtime unknown")?;
                let input=json!({"session_id":row["session"],"transcript_path":row["path"],"modelName":row["model"]});
                let outcome=match (row["outcome"][0].as_str(),row["outcome"][1].as_str()) {
                    (Some("ok"),Some("none"))=>("ok","none"),
                    (Some("failed"),Some("turn-failed"))=>("failed","turn-failed"),
                    (Some("failed"),Some("step-limit"))=>("failed","step-limit"),
                    _=>return Err("continuation outcome invalid".into()),
                };
                capture(data,runtime,&input,deadline,row["project"].as_str().ok_or("continuation project unknown")?,&delivery,Some(row["identity"].as_str().ok_or("continuation identity unknown")?),false,Some(outcome))
            })();
            if let Err(e)=result {error.get_or_insert(e);}
        }
    }
    error.map_or(Ok(()),Err)
}

pub fn backlog(data:&Path)->Value {
    let result=(||->Result<(u64,u64),String> {
        let (mut count,mut bytes)=(0u64,0u64);
        for entry in crate::model::marks(data) {
            let (path,left)=entry?;
            if left>0 {
                let path=path.canonicalize().map_err(|_|"legacy continuation source unavailable")?;
                if !descriptor(&data.join("observe"),&path).try_exists().map_err(|_|"continuation descriptor unavailable")? {return Err("legacy continuation migration pending".into());}
            }
        }
        for delivery in crate::hook_spool::continuation_locations(data)? {
            for file in descriptors(&delivery)? {
                let row=read(&delivery,&file)?;native_source(&row)?;let left=remaining(&delivery,&row)?;
                if left>0 {count+=1;bytes=bytes.checked_add(left).ok_or("continuation size overflow")?;}
            }
        }
        Ok((count,bytes))
    })();
    match result {Ok((count,bytes))=>json!({"state":if count==0 {"none"}else{"pending"},"transcripts":count,"bytes":bytes}),Err(_)=>json!({"state":"unknown","transcripts":null,"bytes":null,"reason":"continuation-unavailable"})}
}

#[cfg(test)]
mod tests {
    use super::*;
    // #301: the final Stop can leave native bytes; service continuation retains IDs and cannot read foreign paths.
    #[test]
    fn native_pending_bytes_survive_reconstruction_and_refuse_foreign_descriptors() {
        let root=std::env::temp_dir().join(format!("agentq-canonical-backlog-{}",std::process::id()));
        let _=fs::remove_dir_all(&root);let data=root.join("data");let home=root.join("home");
        let path=home.join("projects/fixture/native-session.jsonl");fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old=std::env::var_os("CLAUDE_CONFIG_DIR");unsafe {std::env::set_var("CLAUDE_CONFIG_DIR",&home);}
        struct Restore(Option<std::ffi::OsString>);impl Drop for Restore {fn drop(&mut self){unsafe {match &self.0 {Some(v)=>std::env::set_var("CLAUDE_CONFIG_DIR",v),None=>std::env::remove_var("CLAUDE_CONFIG_DIR")}}}}
        let _restore=Restore(old);
        let source=|id:&str|json!({"type":"assistant","sessionId":"native-session","uuid":id,"message":{"id":id,"model":"fixture-model","usage":{"input_tokens":1,"output_tokens":2}}}).to_string()+"\n";
        fs::write(&path,source("first")).unwrap();
        let input=json!({"session_id":"native-session","transcript_path":path});
        stop(&data,"claude",&input,false,Instant::now()+STOP_PASS).unwrap();
        fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(source("second").as_bytes()).unwrap();
        assert_eq!(backlog(&data)["state"],"pending");
        drain(&data,Instant::now()+Duration::from_secs(10)).unwrap();assert_eq!(backlog(&data)["state"],"none");
        let count=|| {
            let reader=coreq::telemetry::Reader::open(&data.join("observations.sqlite3")).unwrap();
            reader.export_kinds(&["agent.call"],0,0,100).unwrap()["events"].as_array().unwrap().len()
        };
        assert_eq!(count(),2);drain(&data,Instant::now()+Duration::from_secs(10)).unwrap();assert_eq!(count(),2);
        let delivery=Delivery::open(&data,"claude","other").unwrap();let descriptor=descriptor(&delivery.state,&path.canonicalize().unwrap());
        let mut row:Value=serde_json::from_slice(&fs::read(&descriptor).unwrap()).unwrap();
        row["path"]=json!(root.join("foreign.jsonl"));fs::write(&descriptor,row.to_string()).unwrap();
        assert_eq!(backlog(&data)["state"],"unknown");assert!(drain(&data,Instant::now()+Duration::from_secs(10)).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
