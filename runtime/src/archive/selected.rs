//! Private bridge from one measured control-journal record to the actual selected physical capture.
use super::*;
use super::store::{active,allowed,enabled,private,append_stream,excluded};
use crate::agent;
use std::{fs,io::{Read,Seek,SeekFrom,Write}};
use super::JournalRecord;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Selected {
    pub journal:JournalRecord, pub captured:JournalRecord, pub session:String, pub turn:Option<String>,
    pub operation:Option<String>, pub first:bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Binding {pub operation:String,pub captured:JournalRecord,pub source:String,pub start:u64,pub manifest_sha256:String}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag="kind")]
enum Witness {Selected(Selected),Binding(Binding)}
fn proof(path:&Path)->PathBuf {path.with_extension("lineage.jsonl")}
fn append(path:&Path,witness:&Witness)->Res<()> {
    let mut options=fs::OpenOptions::new();options.create(true).append(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let mut file=options.open(proof(path)).map_err(|_|"archive selection witness unavailable")?;
    let mut bytes=serde_json::to_vec(witness).map_err(|_|"archive selection witness invalid")?;bytes.push(b'\n');
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|_|"archive selection witness unavailable".into())
}
fn records(path:&Path)->Res<Vec<Witness>> {
    if !proof(path).exists(){return Ok(vec![]);}
    let bytes=fs::read(proof(path)).map_err(|_|"archive selection witness unavailable")?;
    bytes.split_inclusive(|b|*b==b'\n').map(|row| {
        if !row.ends_with(b"\n"){return Err("archive selection witness incomplete".into());}
        serde_json::from_slice(row).map_err(|_|"archive selection witness invalid".into())
    }).collect()
}
pub(super) fn selections(path:&Path)->Res<Vec<Selected>> {Ok(records(path)?.into_iter().filter_map(|r|if let Witness::Selected(r)=r {Some(r)}else{None}).collect())}
pub(super) fn bindings(path:&Path)->Res<Vec<Binding>> {Ok(records(path)?.into_iter().filter_map(|r|if let Witness::Binding(r)=r {Some(r)}else{None}).collect())}
fn physical(record:&JournalRecord)->Res<Vec<u8>> {
    let mut file=fs::OpenOptions::new().read(true).write(true).open(&record.path).map_err(|_|"archive journal unavailable")?;
    if agent::archive_file_identity(&file)?!=record.identity || record.start>=record.end{return Err("archive journal incarnation changed".into());}
    file.seek(SeekFrom::Start(record.start)).map_err(|_|"archive journal unavailable")?;
    let mut bytes=Vec::new();(&mut file).take(record.end-record.start).read_to_end(&mut bytes).map_err(|_|"archive journal unavailable")?;
    if bytes.len() as u64!=record.end-record.start || crate::sha256_hex(&bytes)!=record.sha256 || !bytes.ends_with(b"\n") || bytes[..bytes.len()-1].contains(&b'\n'){return Err("archive journal record changed".into());}
    file.sync_all().map_err(|_|"archive journal witness not durable")?;Ok(bytes)
}
pub(super) fn write_selected(path:&Path,bytes:&[u8],journal:Option<&JournalRecord>,session:&str,turn:Option<&str>,operation:Option<&str>,first:bool)->Res<()> {
    let data=path.parent().and_then(Path::parent).and_then(Path::parent).ok_or("archive selection path invalid")?;
    if active(data)?.is_none_or(|g|!allowed(&g,"launched")) || !enabled(data)? {return Ok(());}
    private(path.parent().ok_or("archive selection path invalid")?)?;
    let _lock=coreq::file_lock::lock(&path.with_extension("selection.lock")).map_err(|_|"archive selection locked")?;
    if let Some(journal)=journal {if excluded(&journal.root,&journal.root)? || excluded(&journal.root,&journal.path)? || excluded(&journal.root,path)? {return Ok(());}}
    let before=fs::metadata(path).ok().map_or(0,|m|m.len());
    append_stream(path,bytes)?;
    let Some(journal)=journal else{return Ok(());};
    let original=physical(journal)?;
    let a:Value=serde_json::from_slice(&original).map_err(|_|"archive journal JSON invalid")?;
    let b:Value=serde_json::from_slice(bytes).map_err(|_|"archive selection JSON invalid")?;
    if a!=b || !bytes.ends_with(b"\n") || bytes[..bytes.len()-1].contains(&b'\n') {return Err("archive selection representation differs".into());}
    let file=fs::File::open(path).map_err(|_|"archive selection unavailable")?;
    let captured=JournalRecord {root:journal.root.clone(),path:path.into(),identity:agent::archive_file_identity(&file)?,start:before,end:before.checked_add(bytes.len() as u64).ok_or("archive selection overflow")?,sha256:crate::sha256_hex(bytes)};
    if file.metadata().map_err(|_|"archive selection unavailable")?.len()!=captured.end {return Err("archive selection concurrent append".into());}
    append(path,&Witness::Selected(Selected {journal:journal.clone(),captured,session:session.into(),turn:turn.map(String::from),operation:operation.map(String::from),first}))
}
pub(super) fn bind(frame:&super::lineage::Frame,manifest:&str)->Res<()> {
    let _lock=coreq::file_lock::lock(&frame.path.with_extension("selection.lock")).map_err(|_|"archive selection locked")?;
    let existing=bindings(&frame.path)?;
    for selected in selections(&frame.path)? {
        let Some(operation)=selected.operation.filter(|_|selected.first) else{continue;};
        if existing.iter().any(|b|b.operation==operation){continue;}
        if selected.captured.identity!=frame.identity || !frame.records.iter().any(|r|r.start==selected.captured.start && r.end==selected.captured.end && r.native_sha256==selected.captured.sha256){continue;}
        let start=selected.captured.start.checked_sub(frame.base).ok_or("archive selection segment mismatch")?;
        append(&frame.path,&Witness::Binding(Binding {operation,captured:selected.captured,source:frame.source.clone(),start,manifest_sha256:manifest.into()}))?;
    }Ok(())
}

/// A measured one-to-one physical contribution mapping, not content/hash search or offset subtraction.
pub(super) fn map(step:&crate::agent::history::Step,incarnation:&str,frame:&super::lineage::Frame,parent:&Capture)->Option<crate::agent::history::Step> {
    if frame.format!=crate::agent::history::CODEX_APP_SERVER_SELECTED || step.source["format"]!="codex-app-server" || step.origins.is_empty(){return None;}
    let records=selections(&frame.path).ok()?;
    let convert=|origin:&Value|->Option<Value> {
        let mut matching=records.iter().filter(|r| parent.manifest["session"].as_str()==Some(r.session.as_str()) && r.journal.identity==incarnation && r.journal.path.to_str()==origin["source"].as_str() && origin["start"]==r.journal.start && origin["end"]==r.journal.end && origin["sha256"]==r.journal.sha256 && r.captured.identity==frame.identity && r.captured.path==frame.path && r.journal.root==parent.root && r.captured.root==parent.root);
        let r=matching.next()?;if matching.next().is_some() || excluded(&parent.root,&r.journal.path).ok()? {return None;}
        Some(json!({"source":r.captured.path,"start":r.captured.start,"end":r.captured.end,"sha256":r.captured.sha256}))
    };
    let mut mapped=step.clone();mapped.origins=step.origins.iter().map(convert).collect::<Option<Vec<_>>>()?;
    let mut source=convert(&step.source)?;source["format"]=json!(frame.format);mapped.source=source;Some(mapped)
}
