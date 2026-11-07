//! Native references only; bytes remain authoritative. Shapes adapted from Orca (NOTICE).
use super::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone,Copy,Debug,PartialEq,Eq,Serialize)]
#[serde(rename_all="snake_case")]
pub enum MediaKind { Image, Video, Audio, Document }

#[derive(Clone,Debug,Serialize)]
pub struct Media {
    pub id:String,pub kind:Option<MediaKind>,pub mime:Option<String>,pub name:Option<String>,
    pub size:Option<u64>,pub sha256:Option<String>,pub source:Value,pub state:String,
    pub unknown:BTreeMap<String,String>,pub step:String,pub origin:Value,
}

enum Bytes { Inline(Vec<u8>), Path(PathBuf) }
/// A read-scoped catalog, not persisted derived state. Inline data is decoded once.
pub struct MediaCatalog { pub items:Vec<Media>,backing:BTreeMap<String,Bytes> }

impl MediaCatalog {
    pub fn read(&self,key:&str,start:u64,length:u64)->Res<Vec<u8>> {
        let media=self.items.iter().find(|m|m.id==key || m.sha256.as_deref()==Some(key) || m.source["path"].as_str()==Some(key)).ok_or("media source not allowed by this session")?;
        let hash=media.sha256.as_ref().ok_or("media bytes unavailable; inspect descriptor reason")?;
        let end=start.checked_add(length).ok_or("media range overflow")?;
        if end>media.size.ok_or("media size unavailable")? {return Err("media range unavailable".into());}
        let native_path=media.source["path"].as_str().map(|p|Bytes::Path(p.into()));
        match native_path.as_ref().or_else(||self.backing.get(hash)).ok_or("media bytes unavailable")? {
            Bytes::Inline(bytes)=>Ok(bytes[start as usize..end as usize].to_vec()),
            Bytes::Path(path)=> {
                check_anchor(&media.source)?;
                let mut file=File::open(path).map_err(|e|format!("media missing: {e}"))?;
                let mut digest=ring::digest::Context::new(&ring::digest::SHA256);let mut offset=0u64;let mut bytes=Vec::new();let mut buffer=[0;8192];
                loop {
                    let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}
                    digest.update(&buffer[..n]);
                    let from=start.saturating_sub(offset).min(n as u64) as usize;let to=end.saturating_sub(offset).min(n as u64) as usize;
                    if from<to{bytes.extend_from_slice(&buffer[from..to]);}offset+=n as u64;
                }
                let read_hash:String=digest.finish().as_ref().iter().map(|b|format!("{b:02x}")).collect();
                check_anchor(&media.source)?;
                if read_hash!=*hash {return Err("media source content changed".into());}
                if bytes.len() as u64!=length {return Err("media range truncated".into());}Ok(bytes)
            }
        }
    }

    pub fn steps(&self)->Vec<Step> {
        self.items.iter().map(|m| {
            let mut step=super::steps::base_step(&m.origin,&Value::Null,&json!({"id":m.id}),StepKind::Media,serde_json::to_value(m).expect("media descriptor"),0);
            step.id=m.id.clone();step.parent=Some(m.step.clone());step.author="system".into();step.state=m.state.clone();step
        }).collect()
    }

    fn add(&mut self,source:Value,declared:Option<MediaKind>,mime:Option<&str>,name:Option<&str>,inline:Option<Res<Vec<u8>>>,forced:bool,step:&Step,pointer:&str) {
        let mut unknown=BTreeMap::new();let mut bytes=None;let mut facts=None;
        let mut state="unavailable".to_string();
        if let Some(inline)=inline {
            match inline {Ok(value)=>{facts=Some((crate::sha256_hex(&value),value.len() as u64,signature(&value)));bytes=Some(Bytes::Inline(value));state="available".into();},Err(e)=>{unknown.insert("bytes".into(),e);}}
        } else if let Some(path)=source["path"].as_str() {
            if check_anchor(&source).is_err(){return;}
            match File::open(path) {
                Ok(mut file)=>{
                    if !forced {let mut head=[0;8192];match file.read(&mut head) {Ok(n) if signature(&head[..n]).is_some()=>{},_=>return,}}
                    match digest(&mut file) {Ok(value)=>{facts=Some(value);bytes=Some(Bytes::Path(path.into()));state="available".into();},Err(e)=>{unknown.insert("bytes".into(),e);}}},
                Err(e)=>{state=if e.kind()==std::io::ErrorKind::NotFound {"missing"}else{"unavailable"}.into();unknown.insert("bytes".into(),format!("native media file unavailable: {e}"));}
            }
        }else{unknown.insert("bytes".into(),"external URL is not fetched, or native bytes were not reported".into());}
        if let Some(error)=source["error"].as_str(){unknown.insert("source".into(),error.into());}
        if check_anchor(&source).is_err(){return;}
        let detected=facts.as_ref().and_then(|v|v.2);
        if !forced && detected.is_none(){return;}
        let kind=detected.map(|v|v.0).or(declared.filter(|k|matches!(k,MediaKind::Image|MediaKind::Document)));
        let mime=detected.map(|v|v.1.to_string()).or_else(||mime.map(String::from));
        let (hash,size)=facts.map_or((None,None),|(h,n,_)|(Some(h),Some(n)));
        if let (Some(hash),Some(bytes))=(&hash,bytes){self.backing.entry(hash.clone()).or_insert(bytes);}
        let name=name.map(String::from).or_else(||source["path"].as_str().and_then(|p|Path::new(p).file_name()).map(|p|p.to_string_lossy().into_owned()));
        if kind.is_none(){unknown.insert("kind".into(),"no recognized byte signature; video/audio are never inferred from a name".into());}
        if mime.is_none(){unknown.insert("mime".into(),"native MIME and byte signature unavailable".into());}
        if name.is_none(){unknown.insert("name".into(),"native name not reported".into());}
        if hash.is_none(){unknown.insert("sha256".into(),"bytes unavailable".into());unknown.insert("size".into(),"bytes unavailable".into());}
        let id=crate::sha256_hex(json!([step.source["id"],pointer,source,hash]).to_string().as_bytes());
        self.items.push(Media {id,kind,mime,name,size,sha256:hash,source,state,unknown,step:step.id.clone(),origin:step.source.clone()});
    }
}

/// Recorded image fields only; never opens a reference or validates a preview.
pub(in crate::agent) fn recorded_image(value:&Value)->Value {
    let name=value["alt"].as_str().or(value["name"].as_str());
    let url=value["image_url"].as_str().or(value["image_url"]["url"].as_str()).or(value["source"]["url"].as_str());
    let inline=url.and_then(|url|url.strip_prefix("data:image/")).and_then(|url|url.split_once(";base64,"));
    let data=inline.map(|(_,data)|data).or_else(||(value["source"]["type"]=="base64").then(||value["source"]["data"].as_str()).flatten());
    let mime=inline.map(|(mime,_)|format!("image/{mime}")).or_else(||value["source"]["media_type"].as_str().map(String::from));
    let mut image=json!({"name":name,"url":url.filter(|_|inline.is_none()),"mime":mime,"encoding":data.map(|_|"base64"),"data":data,"unknown":{}});
    for field in ["name","url","mime","encoding","data"] {if image[field].is_null(){image["unknown"][field]=json!("native field not recorded");}}
    image
}

fn digest(file:&mut File)->Res<(String,u64,Option<(MediaKind,&'static str)>)> {
    if !file.metadata().map_err(|e|e.to_string())?.is_file(){return Err("media reference is not a file".into());}
    file.seek(SeekFrom::Start(0)).map_err(|e|e.to_string())?;
    let mut digest=ring::digest::Context::new(&ring::digest::SHA256);let mut size=0;let mut head=Vec::new();
    let mut buffer=[0;8192];
    loop {let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}if head.is_empty(){head.extend_from_slice(&buffer[..n]);}digest.update(&buffer[..n]);size+=n as u64;}
    Ok((digest.finish().as_ref().iter().map(|b|format!("{b:02x}")).collect(),size,signature(&head)))
}

fn signature(b:&[u8])->Option<(MediaKind,&'static str)> {
    use MediaKind::*;
    Some(if b.starts_with(b"\x89PNG\r\n\x1a\n"){(Image,"image/png")}
    else if b.starts_with(b"\xff\xd8\xff"){(Image,"image/jpeg")}
    else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a"){(Image,"image/gif")}
    else if b.starts_with(b"RIFF") {match b.get(8..12)? {b"WEBP"=>(Image,"image/webp"),b"WAVE"=>(Audio,"audio/wav"),b"AVI "=>(Video,"video/x-msvideo"),_=>return None}}
    else if b.get(4..8)==Some(b"ftyp") {if matches!(b.get(8..12),Some(b"avif"|b"avis")){return Some((Image,"image/avif"));}else if matches!(b.get(8..12),Some(b"heic"|b"heix"|b"heif")){return Some((Image,"image/heic"));}else if matches!(b.get(8..12),Some(b"M4A "|b"M4B "|b"M4P ")){(Audio,"audio/mp4")}else{(Video,"video/mp4")}}
    else if b.starts_with(b"\x1aE\xdf\xa3") && b.windows(4).any(|v|v==b"webm"){(Video,"video/webm")}
    else if b.starts_with(b"OggS") && (b.windows(8).any(|v|v==b"OpusHead") || b.windows(6).any(|v|v==b"vorbis")){(Audio,"audio/ogg")}
    else if b.starts_with(b"fLaC"){(Audio,"audio/flac")}
    else if b.starts_with(b"ID3") || b.len()>1 && b[0]==255 && b[1]&0xe0==0xe0 {(Audio,"audio/mpeg")}
    else if b.starts_with(b"MThd"){(Audio,"audio/midi")}
    else if b.starts_with(b"FLV"){(Video,"video/x-flv")}
    else if b.starts_with(b"%PDF-"){(Document,"application/pdf")}
    else if b.starts_with(b"{\\rtf"){(Document,"application/rtf")}
    else if b.starts_with(b"PK\x03\x04"){(Document,"application/zip")}
    else{return None})
}

fn base64(text:&str)->Res<Vec<u8>> {
    let text:Vec<_>=text.bytes().filter(|b|!b.is_ascii_whitespace()).collect();
    if text.len()%4!=0 {return Err("invalid native base64 length".into());}
    let mut bytes=Vec::new();
    for (index,c) in text.chunks_exact(4).enumerate() {
        let mut n=0u32;let mut pad=0;
        for (i,b) in c.iter().enumerate() {
            let v=match b {b'A'..=b'Z'=>b-b'A',b'a'..=b'z'=>b-b'a'+26,b'0'..=b'9'=>b-b'0'+52,b'+'=>62,b'/'=>63,b'=' if i>=2 && index==text.len()/4-1=>{pad+=1;0},_=>return Err("invalid native base64".into())};
            if pad>0 && *b!=b'=' {return Err("invalid native base64 padding".into());}n=n<<6|v as u32;
        }
        if pad==1 && n&255!=0 || pad==2 && n&65535!=0 {return Err("invalid native base64 padding bits".into());}
        bytes.push((n>>16) as u8);if pad<2{bytes.push((n>>8) as u8);}if pad==0{bytes.push(n as u8);}
    }
    Ok(bytes)
}

fn visit(catalog:&mut MediaCatalog,value:&Value,step:&Step,cwd:Option<&Path>,pointer:&str,tool:bool) {
    if let Some(text)=value.as_str().filter(|_|tool && ["/output","/result","/content","/text","/arguments","/args"].iter().any(|k|pointer.ends_with(k))) {
        if let Ok(value)=serde_json::from_str::<Value>(text) {if value.is_object() || value.is_array(){visit(catalog,&value,step,cwd,pointer,true);}}
        return;
    }
    if let Some(rows)=value.as_array(){for (i,row) in rows.iter().enumerate(){visit(catalog,row,step,cwd,&format!("{pointer}/{i}"),tool);}return;}
    let Some(object)=value.as_object() else{return};
    let native=value["type"].as_str().unwrap_or_default();
    if matches!(native,"text"|"input_text"|"output_text"|"thinking"|"reasoning")
        || tool && (matches!(native,"message"|"assistant"|"user"|"system") || value["role"].is_string()) {return;}
    let sidecar=pointer.is_empty() && step.source["format"]=="antigravity-sidecar" && value["step_index"].is_number()
        && matches!(native,"GENERIC"|"PLANNER_RESPONSE"|"TOOL_RESPONSE");
    let tool=tool || matches!(native,"tool_use"|"server_tool_use"|"tool_result"|"function_call"|"custom_tool_call"|"function_call_output"|"custom_tool_call_output"|"FileChange"|"file_change");
    let kind=match value["type"].as_str() {
        Some("image"|"Image"|"input_image"|"image_url"|"local_image"|"localImage"|"LocalImage")=>Some(MediaKind::Image),
        Some(name) if super::super::registry::codex_image_kind(name).is_some()=>Some(MediaKind::Image),
        Some("document")=>Some(MediaKind::Document),Some("video"|"input_video")=>Some(MediaKind::Video),Some("audio"|"input_audio")=>Some(MediaKind::Audio),_=>None,
    };
    let mime=value["mimeType"].as_str().or(value["source"]["media_type"].as_str()).or(value["mime_type"].as_str()).or(value["mime"].as_str()).or(value["file"]["type"].as_str());
    let kind=kind.or_else(||mime.filter(|_|tool || native=="file" || pointer.starts_with("/media/")).and_then(|m|if m.starts_with("image/"){Some(MediaKind::Image)}else if m=="application/pdf"{Some(MediaKind::Document)}else{None}));
    let name=value["name"].as_str().or(value["title"].as_str()).or(value["filename"].as_str());
    let encoded=value["data"].as_str().filter(|_|value["type"]=="image").or(value["source"]["data"].as_str().filter(|_|value["source"]["type"]=="base64")).or(value["file"]["base64"].as_str());
    let url=value["image_url"].as_str().or(value["image_url"]["url"].as_str()).or(value["url"].as_str()).or(value["source"]["url"].as_str()).or(value["uri"].as_str());
    let path=value["path"].as_str().or(value["file_path"].as_str()).or(value["filePath"].as_str()).or(value["savedPath"].as_str()).or(value["saved_path"].as_str()).or(value["source"]["path"].as_str()).or(value["Path"].as_str()).or(value["AbsolutePath"].as_str());
    if let Some(encoded)=encoded.filter(|_|kind.is_some() || tool) {
        catalog.add(json!({"kind":"inline"}),kind,mime,name,Some(base64(encoded)),true,step,pointer);
    }else if let Some(url)=url.filter(|_|kind.is_some()) {
        if url.starts_with("file://") {
            let path=file_uri(url);
            catalog.add(json!({"kind":"path","path":path.as_ref().ok(),"error":path.as_ref().err()}),kind,mime,name,None,true,step,pointer);
        }else if let Some((header,data))=url.strip_prefix("data:").and_then(|s|s.split_once(',')) {
            let mime=header.strip_suffix(";base64");
            catalog.add(json!({"kind":"inline"}),kind,mime,name,Some(if mime.is_some(){base64(data)}else{Err("native data URL is not base64".into())}),true,step,pointer);
        }else{catalog.add(json!({"kind":"url","url":url}),kind,mime,name,None,true,step,pointer);}
    }else if let Some(path)=path.filter(|_|kind.is_some() || tool) {
        let decoded=serde_json::from_str::<String>(path).ok();let path=Path::new(decoded.as_deref().unwrap_or(path));
        let path=if path.is_absolute(){Some(path.to_path_buf())}else{cwd.map(|cwd|cwd.join(path))};
        let generated=value.get("savedPath").or(value.get("saved_path")).is_some() || super::super::registry::codex_image_kind(native)==Some(true);
        catalog.add(json!({"kind":if generated{"generated"}else{"path"},"path":path}),kind,mime,name,None,kind.is_some(),step,pointer);
    }else if kind.is_some() && !catalog.items.iter().any(|m|m.step==step.id) {
        catalog.add(json!({"kind":"unreported"}),kind,mime,name,None,true,step,pointer);
    }
    for (key,child) in object {
        if key=="changes" && tool {if let Some(changes)=child.as_object() {
            for path in changes.keys() {
                let path=Path::new(path);let path=if path.is_absolute(){Some(path.to_path_buf())}else{cwd.map(|cwd|cwd.join(path))};
                catalog.add(json!({"kind":"path","path":path}),None,None,None,None,false,step,&format!("{pointer}/changes/{}",path.as_ref().map(|p|p.to_string_lossy()).unwrap_or_default()));
            }
        }}
        // Traverse native containers, not arbitrary user-supplied JSON subtrees.
        let native_container=match key.as_str() {
            "payload"=>matches!(native,"event_msg"|"response_item"),
            "params"=>value["method"].as_str().is_some_and(|m|m.starts_with("item/")),
            "item"=>matches!(native,"item_started"|"item_completed"|"item.started"|"item.completed") || pointer=="/params",
            "message"=>matches!(native,"user"|"assistant"),
            "content"=>pointer=="/message" || matches!(native,"user"|"assistant"|"message"|"tool_result") || tool,
            "toolUseResult"=>native=="user" && value["message"]["content"].as_array().is_some_and(|a|a.iter().any(|b|b["type"]=="tool_result")),
            "tool_calls"|"media"=>sidecar,
            "input"|"arguments"|"args"|"output"|"result"|"files"|"images"|"artifacts"|"text"=>tool,
            _=>false,
        };
        if native_container {visit(catalog,child,step,cwd,&format!("{pointer}/{key}"),tool || matches!(key.as_str(),"toolUseResult"|"tool_calls"));}
    }
}

fn visit_step(catalog:&mut MediaCatalog,record:&Value,step:&Step,cwd:Option<&Path>) {
    if step.source["format"]=="acp" {
        if record["type"]!="acp" || record["message"]["method"]!="session/update" {return;}
        let update=&record["message"]["params"]["update"];
        if step.kind==StepKind::Image {
            visit(catalog,&step.data["content"],step,cwd,"/message/params/update/content",false);
        }else if matches!(update["sessionUpdate"].as_str(),Some("tool_call"|"tool_call_update")) {
            for (i,block) in update["content"].as_array().into_iter().flatten().enumerate() {
                if block["type"]=="content" && matches!(block["content"]["type"].as_str(),Some("image"|"audio")) {
                    visit(catalog,&block["content"],step,cwd,&format!("/message/params/update/content/{i}/content"),false);
                }
            }
        }
        return;
    }
    let p=record.get("payload").or(record.get("params")).unwrap_or(record);let item=p.get("item").unwrap_or(p);
    let content=record["message"]["content"].as_array().or_else(||item["content"].as_array());
    if let Some(blocks)=content {
        if let Some(block)=blocks.get(step.part) {
            visit(catalog,block,step,cwd,&format!("/content/{}",step.part),false);
            if block["type"]=="tool_result" {visit(catalog,&record["toolUseResult"],step,cwd,"/toolUseResult",true);}
        }
    }else{visit(catalog,record,step,cwd,"",false);}
}

impl History {
    /// Caller-only references; a caller label never authorizes native artifact discovery.
    pub fn media(&self,path:&Path)->Res<MediaCatalog> {self.media_catalog(path,false)}

    fn media_catalog(&self,path:&Path,native_artifacts:bool)->Res<MediaCatalog> {
        self.open(path)?;
        let mut history=self.clone();history.offset=0;history.prefix=crate::sha256_hex(b"");
        if super::sources::external(&history.format) {
            let mut catalog=MediaCatalog {items:Vec::new(),backing:BTreeMap::new()};let mut cwd=None::<PathBuf>;
            for (step,normalized) in super::sources::media_steps(&history,path)? {
                if let Some(path)=step.source["record"]["cwd"].as_str().filter(|s|Path::new(s).is_absolute()){cwd=Some(path.into());}
                if step.kind!=StepKind::Metadata{visit_step(&mut catalog,&normalized,&step,cwd.as_deref());}
            }
            return Ok(catalog);
        }
        let steps=history.steps(path,u64::MAX)?.steps;
        let mut catalog=MediaCatalog {items:Vec::new(),backing:BTreeMap::new()};let mut physical=BTreeMap::new();let mut cwd=None::<PathBuf>;
        for step in &steps {
            for origin in &step.origins {
                if let (Some(start),Some(end))=(origin["start"].as_u64(),origin["end"].as_u64()) {physical.entry((start,end)).or_insert((step,origin));}
            }
        }
        // Validate once, then verify each contributing physical record on the same stream.
        let (mut native_file,_)=history.open(path)?;
        for ((start,end),(_,origin)) in physical {
            let length=end.checked_sub(start).ok_or("media source range invalid")?;
            native_file.seek(SeekFrom::Start(start)).map_err(|e|e.to_string())?;
            let mut bytes=Vec::new();(&mut native_file).take(length).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
            if bytes.len() as u64!=length{return Err("media native source truncated".into());}
            if origin["sha256"]!=crate::sha256_hex(&bytes){return Err("media native source changed".into());}
            let raw=std::str::from_utf8(&bytes).map_err(|_|"media native source invalid UTF-8")?;
            let v:Value=serde_json::from_str(raw).map_err(|e|e.to_string())?;
            let p=v.get("payload").or(v.get("params")).unwrap_or(&v);
            if let Some(path)=v["cwd"].as_str().or(p["cwd"].as_str()).filter(|s|Path::new(s).is_absolute()){cwd=Some(path.into());}
            let event=history.event(&v,raw,start)?;
            for step in super::steps::decode(&self.format,&event,&v) {
                if step.kind!=StepKind::Metadata {visit_step(&mut catalog,&v,&step,cwd.as_deref());}
            }
        }
        if native_artifacts && self.format.starts_with("codex-") && super::super::native::plain_id(&self.native_session) {
            if let (Some(home),Some(step))=(path.ancestors().find(|p|p.file_name().is_some_and(|n|n=="sessions")).and_then(Path::parent),steps.last()) {
                let anchor=home.canonicalize().map_err(|e|e.to_string())?;
                let root=anchor.join("generated_images").join(&self.native_session);
                if anchored(&root,&anchor).is_ok() && root.is_dir(){artifacts(&mut catalog,&root,&anchor,step)?;}
            }
        }
        if native_artifacts && self.format=="antigravity-sidecar" {
            if let (Some(root),Some(step))=(path.ancestors().nth(3),steps.last()) {
                let anchor=root.parent().ok_or("media artifact parent unavailable")?.canonicalize().map_err(|e|e.to_string())?;
                let root=anchor.join(root.file_name().ok_or("media artifact directory invalid")?);
                if anchored(&root,&anchor).is_ok(){artifacts(&mut catalog,&root,&anchor,step)?;}
            }
        }
        Ok(catalog)
    }
}

impl super::sessions::Session {
    pub fn media(&self)->Res<MediaCatalog> {if self.copy_witness.is_some(){return Err("copied history native artifact discovery unsupported".into());}self.history.media_catalog(&self.path,true)}
}

fn file_uri(url:&str)->Res<PathBuf> {
    let path=url.strip_prefix("file://").ok_or("native file URI invalid")?;
    let path=if let Some(path)=path.strip_prefix("localhost/"){format!("/{path}")}else if path.starts_with('/') {path.to_string()}else{return Err("native remote file URI is not read".into())};
    let mut bytes=Vec::new();let mut iter=path.bytes();
    while let Some(b)=iter.next() {
        if b==b'%' {let hi=iter.next().and_then(|b|(b as char).to_digit(16)).ok_or("invalid native file URI escape")?;let lo=iter.next().and_then(|b|(b as char).to_digit(16)).ok_or("invalid native file URI escape")?;bytes.push((hi*16+lo) as u8);}else{bytes.push(b);}
    }
    let path=String::from_utf8(bytes).map_err(|_|"native file URI invalid UTF-8")?;
    #[cfg(windows)] let path=if path.as_bytes().get(2)==Some(&b':'){path[1..].to_string()}else{path};
    Ok(path.into())
}

fn anchored(path:&Path,anchor:&Path)->Res<()> {
    if anchor.canonicalize().map_err(|e|e.to_string())?!=anchor {return Err("media artifact anchor changed".into());}
    let relative=path.strip_prefix(anchor).map_err(|_|"media artifact escaped session root")?;
    let mut current=anchor.to_path_buf();
    for component in relative.components() {
        if !matches!(component,std::path::Component::Normal(_)){return Err("media artifact path invalid".into());}
        current.push(component);
        if std::fs::symlink_metadata(&current).map_err(|e|e.to_string())?.file_type().is_symlink(){return Err("media artifact symlink refused".into());}
    }
    Ok(())
}
fn check_anchor(source:&Value)->Res<()> {
    if let (Some(path),Some(anchor))=(source["path"].as_str(),source["anchor"].as_str()){anchored(Path::new(path),Path::new(anchor))?;}
    Ok(())
}
fn artifacts(catalog:&mut MediaCatalog,root:&Path,anchor:&Path,step:&Step)->Res<()> {
    anchored(root,anchor)?;
    for entry in std::fs::read_dir(root).map_err(|e|format!("media artifact discovery: {e}"))? {
        let entry=entry.map_err(|e|e.to_string())?;let kind=entry.file_type().map_err(|e|e.to_string())?;
        if kind.is_dir(){artifacts(catalog,&entry.path(),anchor,step)?;}
        else if kind.is_file() && !catalog.items.iter().any(|m|m.source["path"].as_str()==entry.path().to_str()){catalog.add(json!({"kind":"generated","path":entry.path(),"anchor":anchor}),None,None,None,None,false,step,&format!("artifact:{}",entry.path().display()));}
    }
    Ok(())
}

