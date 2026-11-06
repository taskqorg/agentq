//! Native references only; bytes remain authoritative. Shapes adapted from Orca (NOTICE).
use super::*;
use serde::Serialize;
use std::collections::{BTreeMap,BTreeSet};
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
                let mut file=File::open(path).map_err(|e|format!("media missing: {e}"))?;
                let mut digest=ring::digest::Context::new(&ring::digest::SHA256);let mut offset=0u64;let mut bytes=Vec::new();let mut buffer=[0;8192];
                loop {
                    let n=file.read(&mut buffer).map_err(|e|e.to_string())?;if n==0{break;}
                    digest.update(&buffer[..n]);
                    let from=start.saturating_sub(offset).min(n as u64) as usize;let to=end.saturating_sub(offset).min(n as u64) as usize;
                    if from<to{bytes.extend_from_slice(&buffer[from..to]);}offset+=n as u64;
                }
                let read_hash:String=digest.finish().as_ref().iter().map(|b|format!("{b:02x}")).collect();
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
            match File::open(path) {
                Ok(mut file)=>{
                    if !forced {let mut head=[0;8192];match file.read(&mut head) {Ok(n) if signature(&head[..n]).is_some()=>{},_=>return,}}
                    match digest(&mut file) {Ok(value)=>{facts=Some(value);bytes=Some(Bytes::Path(path.into()));state="available".into();},Err(e)=>{unknown.insert("bytes".into(),e);}}},
                Err(e)=>{state=if e.kind()==std::io::ErrorKind::NotFound {"missing"}else{"unavailable"}.into();unknown.insert("bytes".into(),format!("native media file unavailable: {e}"));}
            }
        }else{unknown.insert("bytes".into(),"external URL is not fetched, or native bytes were not reported".into());}
        if let Some(error)=source["error"].as_str(){unknown.insert("source".into(),error.into());}
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

fn visit(catalog:&mut MediaCatalog,value:&Value,step:&Step,cwd:Option<&Path>,pointer:&str) {
    if let Some(text)=value.as_str().filter(|_|["/output","/result","/content","/text"].iter().any(|k|pointer.ends_with(k))) {
        if let Ok(value)=serde_json::from_str::<Value>(text) {if value.is_object() || value.is_array(){visit(catalog,&value,step,cwd,pointer);}}
        return;
    }
    if let Some(rows)=value.as_array(){for (i,row) in rows.iter().enumerate(){visit(catalog,row,step,cwd,&format!("{pointer}/{i}"));}return;}
    let Some(object)=value.as_object() else{return};
    let kind=match value["type"].as_str() {
        Some("image"|"Image"|"input_image"|"image_url"|"local_image"|"localImage"|"LocalImage"|"ImageView"|"imageView"|"image_view"|"imageGeneration"|"image_generation")=>Some(MediaKind::Image),
        Some("document")=>Some(MediaKind::Document),Some("video"|"input_video")=>Some(MediaKind::Video),Some("audio"|"input_audio")=>Some(MediaKind::Audio),_=>None,
    };
    let mime=value["source"]["media_type"].as_str().or(value["mime_type"].as_str()).or(value["mime"].as_str()).or(value["file"]["type"].as_str());
    let kind=kind.or_else(||mime.and_then(|m|if m.starts_with("image/"){Some(MediaKind::Image)}else if m=="application/pdf"{Some(MediaKind::Document)}else{None}));
    let name=value["name"].as_str().or(value["title"].as_str()).or(value["filename"].as_str());
    let encoded=value["source"]["data"].as_str().filter(|_|value["source"]["type"]=="base64").or(value["file"]["base64"].as_str());
    let url=value["image_url"].as_str().or(value["image_url"]["url"].as_str()).or(value["url"].as_str()).or(value["source"]["url"].as_str()).or(value["uri"].as_str());
    let path=value["path"].as_str().or(value["file_path"].as_str()).or(value["filePath"].as_str()).or(value["savedPath"].as_str()).or(value["saved_path"].as_str()).or(value["source"]["path"].as_str()).or(value["Path"].as_str()).or(value["AbsolutePath"].as_str());
    if let Some(encoded)=encoded {
        catalog.add(json!({"kind":"inline"}),kind,mime,name,Some(base64(encoded)),true,step,pointer);
    }else if let Some(url)=url.filter(|_|kind.is_some()) {
        if url.starts_with("file://") {
            let path=file_uri(url);
            catalog.add(json!({"kind":"path","path":path.as_ref().ok(),"error":path.as_ref().err()}),kind,mime,name,None,true,step,pointer);
        }else if let Some((header,data))=url.strip_prefix("data:").and_then(|s|s.split_once(',')) {
            let mime=header.strip_suffix(";base64");
            catalog.add(json!({"kind":"inline"}),kind,mime,name,Some(if mime.is_some(){base64(data)}else{Err("native data URL is not base64".into())}),true,step,pointer);
        }else{catalog.add(json!({"kind":"url","url":url}),kind,mime,name,None,true,step,pointer);}
    }else if let Some(path)=path {
        let decoded=serde_json::from_str::<String>(path).ok();let path=Path::new(decoded.as_deref().unwrap_or(path));
        let path=if path.is_absolute(){Some(path.to_path_buf())}else{cwd.map(|cwd|cwd.join(path))};
        let generated=value.get("savedPath").or(value.get("saved_path")).is_some() || value["type"]=="imageGeneration";
        catalog.add(json!({"kind":if generated{"generated"}else{"path"},"path":path}),kind,mime,name,None,kind.is_some(),step,pointer);
    }else if kind.is_some() {
        catalog.add(json!({"kind":"unreported"}),kind,mime,name,None,true,step,pointer);
    }
    for (key,child) in object {
        if key=="changes" {if let Some(changes)=child.as_object() {
            for path in changes.keys() {
                let path=Path::new(path);let path=if path.is_absolute(){Some(path.to_path_buf())}else{cwd.map(|cwd|cwd.join(path))};
                catalog.add(json!({"kind":"path","path":path}),None,None,None,None,false,step,&format!("{pointer}/changes/{}",path.as_ref().map(|p|p.to_string_lossy()).unwrap_or_default()));
            }
        }}
        // A typed source/file wrapper belongs to its parent, not another media occurrence.
        if (key=="source" || key=="file") && (encoded.is_some() || url.is_some()){continue;}
        visit(catalog,child,step,cwd,&format!("{pointer}/{key}"));
    }
}

impl super::sessions::Session {
    pub fn media(&self)->Res<MediaCatalog> {
        let mut history=self.history.clone();history.offset=0;history.prefix=crate::sha256_hex(b"");
        let steps=history.steps(&self.path,u64::MAX)?.steps;
        let mut catalog=MediaCatalog {items:Vec::new(),backing:BTreeMap::new()};let mut seen=BTreeSet::new();let mut cwd=None::<PathBuf>;
        for step in &steps {
            let v:Value=serde_json::from_str(step.source["raw"].as_str().ok_or("media native raw record unavailable")?).map_err(|e|e.to_string())?;
            let p=v.get("payload").or(v.get("params")).unwrap_or(&v);
            if let Some(path)=v["cwd"].as_str().or(p["cwd"].as_str()).filter(|s|Path::new(s).is_absolute()){cwd=Some(path.into());}
            if step.kind==StepKind::Metadata {continue;}
            if seen.insert(step.source["start"].as_u64()) {visit(&mut catalog,&v,step,cwd.as_deref(),"");}
        }
        if self.history.format.starts_with("codex-") && super::super::native::plain_id(&self.history.native_session) {
            if let (Some(home),Some(step))=(self.path.ancestors().find(|p|p.file_name().is_some_and(|n|n=="sessions")).and_then(Path::parent),steps.last()) {
                let root=home.join("generated_images").join(&self.history.native_session);if root.is_dir(){artifacts(&mut catalog,&root,step)?;}
            }
        }
        if self.history.format=="antigravity-sidecar" {
            if let (Some(root),Some(step))=(self.path.ancestors().nth(3),steps.last()) {artifacts(&mut catalog,root,step)?;}
        }
        Ok(catalog)
    }
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

fn artifacts(catalog:&mut MediaCatalog,root:&Path,step:&Step)->Res<()> {
    for entry in std::fs::read_dir(root).map_err(|e|format!("media artifact discovery: {e}"))? {
        let entry=entry.map_err(|e|e.to_string())?;let kind=entry.file_type().map_err(|e|e.to_string())?;
        if kind.is_dir(){artifacts(catalog,&entry.path(),step)?;}
        else if kind.is_file() && !catalog.items.iter().any(|m|m.source["path"].as_str()==entry.path().to_str()){catalog.add(json!({"kind":"generated","path":entry.path()}),None,None,None,None,false,step,&format!("artifact:{}",entry.path().display()));}
    }
    Ok(())
}

