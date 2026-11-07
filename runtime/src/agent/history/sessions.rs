//! Native locations stay behind the runtime boundary; discovery never launches a client.
use super::*;
use std::{path::PathBuf, io::{BufRead, BufReader}};
use serde::{Serialize,Deserialize};

#[derive(Clone, Debug, Serialize,Deserialize)]
pub struct Source { pub id:String, pub native_session:String, pub session: String, pub path: PathBuf, pub format: String, pub parent: Option<String>, pub parent_step: Option<String> }

pub struct Session { resolved:Source,pub(super) history: History, pub(super) path: PathBuf, pub(super) runtime:String, parent: Option<String>, parent_step: Option<String>, children:std::collections::BTreeMap<String,String> }
impl Session {
    /// Physical records through the already resolved native source and cursor.
    pub fn poll(&mut self)->Res<Page> {self.history.poll(&self.path)}
    pub fn poll_before(&mut self)->Res<Page> {self.history.poll_before(&self.path)}
    pub fn steps(&mut self, bytes: u64) -> Res<StepPage> {
        let mut page=self.history.steps(&self.path,bytes)?;
        for step in &mut page.steps {
            if step.parent.is_none(){step.parent=self.parent_step.clone();step.parent_session=self.parent.clone();}
            if let Some(child)=step.data["session"].as_str() {
                if let Some(id)=self.children.get(child) {step.data["session"]=json!(id);}
                else if self.runtime=="claude" && step.kind==StepKind::Subagent {
                    step.data["session"]=json!(format!("{}/agent/{child}",self.history.native_parent.as_deref().unwrap_or(&self.history.native_session)));
                }
            }
        }
        page.checkpoint=self.checkpoint();
        Ok(page)
    }
    pub fn checkpoint(&self) -> Value {
        let mut checkpoint=self.history.checkpoint();
        checkpoint["resolved"]=json!({"runtime":self.runtime,"source":self.resolved,"parent":self.parent,"parent_step":self.parent_step,"children":self.children});
        checkpoint
    }
    pub fn source_bytes(&self,key:&str,start:u64,length:u64)->Res<Vec<u8>> {super::sources::source_bytes(&self.history,&self.path,key,start,length)}
    pub fn read_bytes(&self, start:u64, length:u64)->Res<Vec<u8>> {self.history.read_bytes(&self.path,start,length)}
    /// Explicitly adopt and acquire terminal resume authority; source IDs resolve inside the adapter.
    pub fn resume_command(&self, launch:&super::super::Launch, native:&[super::super::NativeProcess])->Res<super::super::ResumeCommand> {
        if self.history.native_parent.is_some() || self.runtime=="claude" && self.history.session!=self.history.native_session {return Err("native subagent terminal resume unsupported".into());}
        super::super::runtime(&self.runtime).ok_or("history runtime unsupported")?.descriptor().history_resume(&self.history.native_session,launch,native)
    }
}

fn client(runtime: &str) -> Res<super::super::Client> {
    let entry=super::super::registry::entry(runtime).filter(|e|e.descriptor().is_some()).ok_or("history runtime unsupported")?;
    Ok(super::super::Client(entry.name))
}

fn walk(root:&Path, out:&mut Vec<PathBuf>) -> Res<()> {
    if !root.exists() {return Ok(());}
    for entry in std::fs::read_dir(root).map_err(|e|format!("history discovery: {e}"))? {
        let entry=entry.map_err(|e|e.to_string())?;
        let ty=entry.file_type().map_err(|e|e.to_string())?;
        if ty.is_dir() {walk(&entry.path(),out)?;}
        else if ty.is_file() && entry.path().extension().is_some_and(|e|e=="jsonl") {out.push(entry.path());}
    }
    Ok(())
}

pub(super) fn source_format(runtime:&str,format:&str)->Res<()> {
    let allowed=if let Some(spec)=super::sources::get(runtime).filter(|s|super::sources::external(s.format)) {
        spec.format==format
    }else{super::super::registry::protocol(client(runtime)?).history_formats.contains(&format)};
    if allowed {Ok(())}else{Err("history source format differs from runtime".into())}
}

fn identify(runtime:&str, path:&Path) -> Res<Option<Source>> {
    let mut format=super::super::registry::protocol(client(runtime)?).native_history;
    let mut reader=BufReader::new(File::open(path).map_err(|e|format!("history discovery: {e}"))?);
    let mut row=String::new();
    loop {
        row.clear();
        if reader.read_line(&mut row).map_err(|e|e.to_string())?==0 {return Ok(None);}
        if !row.ends_with('\n') {return Ok(None);}
        if row.trim().is_empty() {continue;}
        let v:Value=serde_json::from_str(&row).map_err(|e|format!("history discovery invalid JSON: {e}"))?;
        let acp=v["type"]=="acp";
        let rpc=v["type"]=="response" && v["command"]=="get_state" && v["data"]["sessionFile"].is_string();
        if acp {format="acp";}else if rpc {format="jsonl-rpc";}
        let control=super::requests::session(&v);
        if control.is_some() && format=="codex-rollout" {format="codex-app-server";}
        source_format(runtime,format)?;
        let id=if let Some(id)=control.as_deref() {Some(id)}else if acp {v["message"]["params"]["sessionId"].as_str().or(v["message"]["result"]["sessionId"].as_str())}else if rpc {v["data"]["sessionFile"].as_str()}else{match runtime {
            "codex" if v["type"]=="session_meta"=>v["payload"]["id"].as_str(),
            "claude"=>v["agentId"].as_str().or(v["sessionId"].as_str()).or(v["session_id"].as_str()),
            "antigravity"=>path.parent().and_then(Path::parent).and_then(Path::parent).and_then(Path::file_name).and_then(|s|s.to_str()),
            _=>None,
        }};
        if let Some(id)=id {
            return Ok(Some(Source {id:crate::sha256_hex(path.to_string_lossy().as_bytes()),native_session:id.into(),session:if runtime=="claude" && v["agentId"].is_string() {format!("{}/agent/{id}",v["sessionId"].as_str().unwrap_or("unreported"))}else{id.into()},path:path.into(),format:format.into(),parent:if runtime=="claude" && v["agentId"].is_string() {v["sessionId"].as_str().map(String::from)}else{v["payload"]["forked_from_id"].as_str().or(v.pointer("/payload/source/subagent/thread_spawn/parent_thread_id").and_then(Value::as_str)).map(String::from)},parent_step:None}));
        }
    }
}

fn discover(runtime:&str, home:Option<&Path>) -> Res<Vec<Source>> {
    if let Some(spec)=super::sources::get(runtime).filter(|s|super::sources::external(s.format)){return super::sources::list(spec.name,home);}
    let client=client(runtime)?;
    if !matches!(runtime,"codex"|"claude"|"antigravity") {return Err("history-discovery-unsupported; pass an explicit source".into());}
    let home=home.map(Path::to_path_buf).or_else(||super::super::base_home(client)).ok_or("history home unknown")?;
    let root=home.join(match runtime {"codex"=>"sessions","claude"=>"projects",_=>"antigravity-cli/brain"});
    let mut paths=Vec::new();walk(&root,&mut paths)?;paths.sort();
    let mut sources=Vec::new();
    for path in paths {
        if runtime=="antigravity" && path.file_name().is_none_or(|n|n!="transcript.jsonl") {continue;}
        if let Some(source)=identify(runtime,&path)? {sources.push(source);}
    }
    Ok(sources)
}

pub fn list(runtime:&str,home:Option<&Path>)->Res<Vec<Source>> {
    discover(runtime,home)
}

fn parent_source<'a>(runtime:&str,child:&Source,sources:&'a [Source])->Res<Option<&'a Source>> {
    let Some(parent)=&child.parent else{return Ok(None)};let mut found=None;
    for source in sources.iter().filter(|s|&s.session==parent) {
        if runtime=="claude" && !child.path.starts_with(source.path.with_extension("")) {continue;}
        if !super::sources::same_family(runtime,&source.path,&child.path)? {continue;}
        if found.is_some(){return Err("history parent has multiple sources; native family ambiguous".into());}
        found=Some(source);
    }
    Ok(found)
}

fn link_parents(runtime:&str,sources:&mut [Source])->Res<()> {
    if super::sources::get(runtime).is_some_and(|s|super::sources::external(s.format)){return Ok(());}
    let mut parents=std::collections::BTreeMap::<PathBuf,Vec<usize>>::new();
    for (index,child) in sources.iter().enumerate() {
        if let Some(source)=parent_source(runtime,child,sources)? {
            parents.entry(source.path.clone()).or_default().push(index);
        }
    }
    for (path,children) in parents {
        for row in BufReader::new(File::open(path).map_err(|e|e.to_string())?).split(b'\n') {
            let row=row.map_err(|e|e.to_string())?;
            let Ok(record)=serde_json::from_slice::<Value>(&row) else{continue};
            let p=record.get("payload").unwrap_or(&record);let item=p.get("item").unwrap_or(p);
            let result=super::steps::structured(&item["output"]);
            let agent=record["toolUseResult"]["agentId"].as_str().or(item["agent_thread_id"].as_str()).or(result["agent_id"].as_str());
            let Some(agent)=agent else{continue};
            let call=record["message"]["content"].as_array().and_then(|b|b.iter().find(|b|b["type"]=="tool_result")).and_then(|b|b["tool_use_id"].as_str())
                .or(item["call_id"].as_str()).or_else(||(item["type"]=="SubAgentActivity").then(||item["id"].as_str()).flatten());
            if let Some(call)=call {for index in &children {if sources[*index].native_session==agent {sources[*index].parent_step=Some(call.into());}}}
        }
    }
    Ok(())
}

pub fn open(runtime:&str, session:&str, home:Option<&Path>, source:Option<&Path>, saved:Option<&Value>) -> Res<Session> {
    if super::sources::get(runtime).is_none(){return Err("history runtime unsupported".into());}
    if let Some(saved)=saved.filter(|s|s["resolved"].is_object()) {
        let requested_source=source;
        let resolved=&saved["resolved"];
        let source:Source=serde_json::from_value(resolved["source"].clone()).map_err(|_|"history checkpoint source invalid")?;
        source_format(runtime,&source.format)?;
        if resolved["runtime"]!=runtime || ![source.id.as_str(),source.session.as_str()].contains(&session) || source.id!=super::sources::source_id(runtime,&source.path,&source.session) {
            return Err("history checkpoint binding differs".into());
        }
        if requested_source.is_some_and(|p|p!=source.path) || !source.path.is_absolute() {return Err("history checkpoint source invalid".into());}
        let history=History::recover(saved)?;
        if history.source!=source.path.to_str().ok_or("history source path invalid UTF-8")? || history.session!=source.session || history.native_session!=source.native_session || history.format!=source.format {return Err("history checkpoint binding differs".into());}
        history.open(&source.path)?;
        let children=serde_json::from_value(resolved["children"].clone()).map_err(|_|"history checkpoint children invalid")?;
        return Ok(Session {path:source.path.clone(),resolved:source,history,runtime:runtime.into(),parent:resolved["parent"].as_str().map(String::from),parent_step:resolved["parent_step"].as_str().map(String::from),children});
    }
    let sources=if source.is_some(){vec![]}else{discover(runtime,home)?};
    let source=if let Some(path)=source {
        let found=if super::sources::get(runtime).is_some_and(|s|super::sources::external(s.format)) {
            super::sources::identify(runtime,path)?.into_iter().find(|s|s.session==session || s.id==session).ok_or("history session identity unavailable")?
        }else{identify(runtime,path)?.ok_or("history session identity unavailable")?};
        if found.session!=session && found.native_session!=session && found.id!=session {return Err("history native session differs".into());}found
    } else {
        let matches:Vec<_>=sources.iter().filter(|s|s.id==session || s.session==session).cloned().collect();
        if matches.len()!=1 {return Err(if matches.is_empty(){"history session not found"}else{"history session has multiple sources; pass a source id from history_sessions"}.into());}
        let selected=matches.into_iter().next().unwrap();
        let mut family=vec![selected];
        if let Some(parent)=parent_source(runtime,&family[0],&sources)? {family.push(parent.clone());link_parents(runtime,&mut family)?;}
        family.remove(0)
    };
    source_format(runtime,&source.format)?;
    let label=source.path.to_str().ok_or("history source path invalid UTF-8")?;
    let mut history=if let Some(saved)=saved {
        let h=History::recover(saved)?;
        if h.source!=label || h.session!=source.session || h.native_session!=source.native_session || h.format!=source.format {return Err("history checkpoint binding differs".into());}h
    }else{History::new(&source.format,label,&source.session)?};
    history.native_session=source.native_session.clone();history.native_parent=if runtime=="claude" && source.parent.is_some(){source.parent.clone()}else{None};
    let mut children=std::collections::BTreeMap::new();
    for child in sources.iter().filter(|s|s.parent.as_ref()==Some(&source.session)) {
        if parent_source(runtime,child,&sources)?.is_some_and(|parent|parent.id==source.id) {
            if children.insert(child.native_session.clone(),child.id.clone()).is_some(){return Err("history child has multiple sources; native family ambiguous".into());}
        }
    }
    let parent=parent_source(runtime,&source,&sources)?.map(|s|s.id.clone()).or(source.parent.clone());
    Ok(Session {resolved:source.clone(),history,path:source.path,runtime:runtime.into(),parent,parent_step:source.parent_step,children})
}

pub(crate) fn resume(runtime:&str, session:&str)->Res<Vec<String>> {
    if !super::super::native::cli_id(session) {return Err("history session invalid".into());}
    Ok(match runtime {
        "codex"=>vec!["codex".into(),"resume".into(),session.into()],
        "claude"=>vec!["claude".into(),"--resume".into(),session.into()],
        "antigravity"=>vec!["agy".into(),"--conversation".into(),session.into()],
        _=>return Err("history resume unsupported".into()),
    })
}

pub fn request(request:&str)->Res<Value> {
    let v:Value=serde_json::from_str(request).map_err(|e|format!("history request: {e}"))?;
    let path=|key:&str|->Res<Option<&Path>> {match &v[key] {
        Value::Null=>Ok(None),Value::String(s) if Path::new(s).is_absolute()=>Ok(Some(Path::new(s))),
        _=>Err(format!("history {key} requires an absolute path or null")),
    }};
    let runtime=v["runtime"].as_str().ok_or("history runtime required")?;
    let rt=super::super::runtime(runtime).ok_or("history runtime unsupported")?;
    let home=path("home")?;
    if v["action"]=="list" {return serde_json::to_value(rt.history_sessions(home)?).map_err(|e|e.to_string());}
    let session=v["session"].as_str().ok_or("history session required")?;
    let mut reader=rt.history(session,home,path("source")?,v.get("checkpoint").filter(|v|!v.is_null()))?;
    if v["action"]=="media" || v["action"]=="media_bytes" {
        let media=reader.media()?;
        if v["action"]=="media_bytes" {
            let key=v["key"].as_str().ok_or("media key required")?;
            let start=v["start"].as_u64().ok_or("media start required")?;
            let length=v["length"].as_u64().ok_or("media length required")?;
            return Ok(json!({"bytes":media.read(key,start,length)?,"checkpoint":reader.checkpoint()}));
        }
        return Ok(json!({"media":media.items,"steps":media.steps(),"checkpoint":reader.checkpoint()}));
    }
    if v["action"]=="live" {
        return serde_json::to_value(reader.live_data(path("statusline")?)?).map_err(|e|e.to_string());
    }
    if v["action"]=="source_bytes" {
        return Ok(json!({"bytes":reader.source_bytes(v["key"].as_str().ok_or("history source key required")?,v["start"].as_u64().ok_or("history start required")?,v["length"].as_u64().ok_or("history length required")?)?,"checkpoint":reader.checkpoint()}));
    }
    if v["action"]=="bytes" {
        let start=v["start"].as_u64().ok_or("history start required")?;
        let length=v["length"].as_u64().ok_or("history length required")?;
        return Ok(json!({"start":start,"bytes":reader.read_bytes(start,length)?,"checkpoint":reader.checkpoint()}));
    }
    let bytes=v["bytes"].as_u64().ok_or("history bytes required; caller chooses the page window")?;
    serde_json::to_value(reader.steps(bytes)?).map_err(|e|e.to_string())
}
