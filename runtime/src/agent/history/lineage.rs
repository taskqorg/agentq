//! F300-L20261107-2: auxiliary observations; never native Session or mutation authority.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use super::{Step, StepKind, Turn};

type Res<T> = Result<T, String>;
pub const FORMAT: &str = "agentq-lineage/1";
// Generated from sorted framed relative paths/raw source bytes and dependency lock revision.
include!(concat!(env!("OUT_DIR"), "/history-decoder.rs"));
pub fn decoder_id()->&'static str {DECODER}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    #[serde(rename="type")] pub record_type: String,
    pub version: u64,
    pub operation_id: String,
    pub revision: u64,
    pub kind: Kind,
    pub effect: Effect,
    pub session: String,
    pub parent: Parent,
    pub predecessor: Option<SourceRef>,
    pub activation: Activation,
    pub conditions: Conditions,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Kind { Fork, Rewind }
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Effect { Applied, Noop }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent { pub session: String, pub point: Point }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef { pub session: String, pub source: String }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="lowercase", deny_unknown_fields)]
pub enum Point {
    Empty,
    Unmapped { reason: Unmapped },
    Record { source: String, start: u64, end: u64, sha256: String, projection: Option<Projection> },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Unmapped { NotArchived, MappingUnavailable, SourceDomainUnsupported }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection { pub decoder: String, pub part: u64 }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag="kind", rename_all="lowercase", deny_unknown_fields)]
pub enum Activation {
    Pending { reason: Pending },
    Byte { source: String, start: u64, manifest_sha256: String },
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Pending { NoNativeBytes, MappingUnavailable }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair<T> { pub before: Option<T>, pub after: Option<T>, pub changed: Option<bool> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delta<T> { pub requested: Pair<T>, pub observed: Pair<T> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    pub model: Delta<String>, pub effort: Delta<String>, pub policy: Delta<PortablePolicy>,
    pub account_label: Delta<String>, pub cli_version: Delta<String>, pub instructions_hash: Delta<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePolicy {
    pub read: crate::agent::Scope, pub write: crate::agent::Scope,
    pub network: crate::agent::Network, pub tools: Option<Vec<String>>,
    pub approval: crate::agent::Approval, pub roots: NativeRoots,
    pub unelevated: bool, pub builtin: crate::agent::Builtin, pub isolated: bool, pub subscription: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum NativeRoots { Native }

pub fn safe_label(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c|c.is_control() || matches!(c,'/'|'\\'|':'|'@'|'*')) && !s.contains("..")
}
fn digest(s:&str)->bool {s.len()==64 && s.bytes().all(|c|c.is_ascii_digit() || (b'a'..=b'f').contains(&c))}
fn keys(v:&Value, names:&[&str])->Res<()> {
    let o=v.as_object().ok_or("lineage object required")?;
    if o.len()!=names.len() || names.iter().any(|k|!o.contains_key(*k)) {return Err("lineage closed fields invalid".into());}Ok(())
}
fn shape(v:&Value)->Res<()> {
    keys(v,&["type","version","operation_id","revision","kind","effect","session","parent","predecessor","activation","conditions"])?;
    keys(&v["parent"],&["session","point"])?;
    let p=&v["parent"]["point"];
    match p["kind"].as_str() {
        Some("empty")=>keys(p,&["kind"])?,
        Some("unmapped")=>keys(p,&["kind","reason"])?,
        Some("record")=>{keys(p,&["kind","source","start","end","sha256","projection"])?;if !p["projection"].is_null(){keys(&p["projection"],&["decoder","part"])?;}},
        _=>return Err("lineage point invalid".into()),
    }
    if !v["predecessor"].is_null(){keys(&v["predecessor"],&["session","source"])?;}
    let a=&v["activation"];
    keys(a,match a["kind"].as_str(){Some("pending")=>&["kind","reason"],Some("byte")=>&["kind","source","start","manifest_sha256"],_=>return Err("lineage activation invalid".into())})?;
    let names=["model","effort","policy","account_label","cli_version","instructions_hash"];
    keys(&v["conditions"],&names)?;
    for name in names {
        let d=&v["conditions"][name];keys(d,&["requested","observed"])?;
        for basis in ["requested","observed"] {
            let p=&d[basis];keys(p,&["before","after","changed"])?;
            for side in ["before","after"] {
                let cell=&p[side];
                if cell.is_null(){continue;}
                if name=="policy" {
                    keys(cell,&["read","write","network","tools","approval","roots","unelevated","builtin","isolated","subscription"])?;
                    if cell["tools"].as_array().is_some_and(|a|a.iter().any(|v|v.as_str().is_none_or(|s|s.is_empty() || !s.bytes().all(|b|b.is_ascii_alphanumeric() || b"_.-".contains(&b))))) {return Err("lineage policy tools unsafe".into());}
                } else {
                    let s=cell.as_str().ok_or("lineage condition type invalid")?;
                    if !safe_label(s) || name=="instructions_hash" && !digest(s) {return Err("lineage condition unsafe".into());}
                }
            }
            let expected=if p["before"].is_null() || p["after"].is_null(){Value::Null}else{Value::Bool(canonical(&p["before"])!=canonical(&p["after"]))};
            if p["changed"]!=expected {return Err("lineage comparison invalid".into());}
        }
    }
    Ok(())
}
fn canonical(v:&Value)->Value {
    match v {
        Value::Object(o)=>{let sorted:BTreeMap<_,_>=o.iter().map(|(k,v)|(k.clone(),canonical(v))).collect();serde_json::to_value(sorted).unwrap()},
        Value::Array(a)=>Value::Array(a.iter().map(canonical).collect()),_=>v.clone(),
    }
}
/// One complete JSONL record. The enclosing authorized manifest supplies `session`.
pub fn parse(bytes:&[u8],session:&str)->Res<Record> {
    if !bytes.ends_with(b"\n") || bytes[..bytes.len()-1].contains(&b'\n') {return Err("lineage complete physical line required".into());}
    // Typed deserialization rejects duplicate fields before Value's map could collapse them.
    let r:Record=serde_json::from_slice(bytes).map_err(|_|"lineage record invalid")?;
    let v:Value=serde_json::from_slice(bytes).map_err(|_|"lineage record invalid")?;shape(&v)?;
    r.validate(session)?;Ok(r)
}
impl Record {
    pub fn validate(&self,session:&str)->Res<()> {
        shape(&serde_json::to_value(self).map_err(|_|"lineage serialization invalid")?)?;
        if !crate::archive::lineage_disclosure(&serde_json::to_vec(self).map_err(|_|"lineage serialization invalid")?) {return Err("lineage disclosure unsafe".into());}
        if self.record_type!="agentq_lineage" || self.version!=1 || self.session!=session || !safe_label(session) || !safe_label(&self.parent.session) || !safe_label(&self.operation_id) {return Err("lineage identity invalid".into());}
        match self.kind {
            Kind::Fork if self.effect!=Effect::Applied || self.session==self.parent.session || self.predecessor.is_some()=>return Err("lineage fork invalid".into()),
            Kind::Rewind if self.session!=self.parent.session=>return Err("lineage rewind invalid".into()),_=>{},
        }
        if let Some(p)=&self.predecessor {if p.session!=self.session || !safe_label(&p.source){return Err("lineage predecessor invalid".into());}}
        if let Point::Record {source,start,end,sha256,projection}=&self.parent.point {
            if !safe_label(source) || start>=end || !digest(sha256) || projection.as_ref().is_some_and(|p|!p.decoder.starts_with("agentq-history-decoder/1:") || !digest(p.decoder.trim_start_matches("agentq-history-decoder/1:"))) {return Err("lineage anchor invalid".into());}
        }
        if let Activation::Byte {source,manifest_sha256,..}=&self.activation {if !safe_label(source) || !digest(manifest_sha256){return Err("lineage activation invalid".into());}}Ok(())
    }
    pub fn line(&self)->Res<Vec<u8>> {self.validate(&self.session)?;let mut b=serde_json::to_vec(self).map_err(|_|"lineage serialization invalid")?;b.push(b'\n');Ok(b)}
}
/// Closed nullable fields are mandatory even where serde would accept a missing Option.
pub fn decode(bytes:&[u8],session:&str)->Res<Record> {
    parse(bytes,session)
}

fn fill(old:&Value,new:&Value)->bool {
    if old.is_null(){return true;}match (old,new) {
        (Value::Object(a),Value::Object(b))=>a.len()==b.len() && a.iter().all(|(k,v)|b.get(k).is_some_and(|n|fill(v,n))),
        _=>old==new,
    }
}
fn refinement(old:&Record,new:&Record)->bool {
    if old.operation_id!=new.operation_id || old.kind!=new.kind || old.effect!=new.effect || old.session!=new.session || old.parent.session!=new.parent.session {return false;}
    let a=serde_json::to_value(old).unwrap();let b=serde_json::to_value(new).unwrap();
    for name in ["model","effort","policy","account_label","cli_version","instructions_hash"] {
        if a["conditions"][name]["requested"]!=b["conditions"][name]["requested"] || !fill(&a["conditions"][name]["observed"],&b["conditions"][name]["observed"]){return false;}
    }
    if !fill(&a["predecessor"],&b["predecessor"]){return false;}
    let point=match (&old.parent.point,&new.parent.point) {(Point::Unmapped{..},_)=>true,_=>fill(&a["parent"]["point"],&b["parent"]["point"])};
    let activation=match (&old.activation,&new.activation) {(Activation::Pending{..},_)=>true,_=>old.activation==new.activation};point && activation
}
/// Rebuild from the operation's complete retained raw revisions; errors poison this operation only.
#[derive(Default)]
pub struct Chain { records:BTreeMap<u64,Record>, invalid:bool }
impl Chain {
    pub fn append(&mut self,record:Record)->Res<&Record> {
        if self.invalid{return Err("lineage revision conflict".into());}
        let valid=record.validate(&record.session).is_ok() && match self.records.get(&record.revision) {
            Some(old)=>old==&record,
            None=>match self.records.last_key_value() {None=>record.revision==0,Some((n,old))=>n.checked_add(1)==Some(record.revision) && refinement(old,&record)},
        };
        if !valid {self.invalid=true;return Err("lineage revision conflict or incomplete chain".into());}
        self.records.entry(record.revision).or_insert(record);Ok(self.records.last_key_value().unwrap().1)
    }
    pub fn latest(&self)->Res<Option<&Record>> {if self.invalid {Err("lineage revision conflict".into())}else{Ok(self.records.last_key_value().map(|(_,r)|r))}}
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link { pub record: Record, pub resolved: Option<Resolved>, pub unknown: Option<String> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved { pub session:String, pub source:String, pub step:String }
impl Link {
    pub fn unresolved(record:Record,reason:&str)->Self {Self {record,resolved:None,unknown:Some(reason.into())}}
    /// No native ID, turn, reply handle, parent cursor or permission is produced.
    pub fn step(&self,source:&str,start:u64,raw:&[u8])->Res<Step> {
        let record=decode(raw,&self.record.session)?;
        if record!=self.record || !safe_label(source){return Err("lineage link source mismatch".into());}
        let end=start.checked_add(raw.len() as u64).ok_or("lineage range overflow")?;
        let origin=serde_json::json!({"source":source,"format":FORMAT,"session":record.session,"start":start,"end":end,"sha256":crate::sha256_hex(raw),"raw":std::str::from_utf8(raw).map_err(|_|"lineage UTF-8 invalid")?});
        Ok(Step {id:crate::sha256_hex(serde_json::to_vec(&(source,&record.session,&record.operation_id)).unwrap().as_slice()),native_id:None,revision:start,part:0,kind:StepKind::Link,
            turn:Turn{id:None,native_id:None,session:record.session.clone(),unknown:Some("auxiliary lineage; no native turn".into())},parent:None,parent_session:None,author:"agentq".into(),state:"observed".into(),data:serde_json::to_value(self).unwrap(),presentation:Value::Null,unknown:self.unknown.clone(),source:origin.clone(),origins:vec![origin]})
    }
}

impl<T: PartialEq> Pair<T> {
    pub fn new(before:Option<T>,after:Option<T>)->Self {
        let changed=before.as_ref().zip(after.as_ref()).map(|(a,b)|a!=b);Self {before,after,changed}
    }
}
impl<T> Default for Pair<T> {fn default()->Self {Self {before:None,after:None,changed:None}}}
impl<T> Default for Delta<T> {fn default()->Self {Self {requested:Pair::default(),observed:Pair::default()}}}
impl Default for Conditions {fn default()->Self {Self {model:Delta::default(),effort:Delta::default(),policy:Delta::default(),account_label:Delta::default(),cli_version:Delta::default(),instructions_hash:Delta::default()}}}

/// Deterministic read-only native projection of an already-authorized complete archived prefix.
/// This retains caller binding and format guards; it never constructs a native Session.
pub fn native_prefix(runtime:&str,format:&str,source:&str,session:&str,native_session:Option<&str>,bytes:&[u8])->Res<Vec<Step>> {
    let h=super::History::caller(runtime,format,source,session,native_session)?;
    let mut derived=BTreeMap::<String,Step>::new();let mut offset=0u64;
    let mut turn=None;let mut presentation=serde_json::json!({});
    for row in bytes.split_inclusive(|b|*b==b'\n') {
        if !row.ends_with(b"\n"){return Err("history prefix incomplete".into());}
        let raw=std::str::from_utf8(row).map_err(|_|"history UTF-8 invalid")?;
        if !raw.trim().is_empty() {
            let value:Value=serde_json::from_str(raw).map_err(|_|"history JSON invalid")?;
            if !value.is_object(){return Err("history record invalid".into());}
            let event=h.event(&value,raw,offset)?;
            for mut step in super::steps::contextual_steps(format,&event,&value,&mut turn,&mut presentation) {
                if let Some(old)=derived.get(&step.id){step=super::steps::revise(old,step);}
                derived.insert(step.id.clone(),step);
            }
        }
        offset=offset.checked_add(row.len() as u64).ok_or("history range overflow")?;
    }
    let mut steps:Vec<_>=derived.into_values().collect();steps.sort_by_key(|s|(s.revision,s.part));Ok(steps)
}

/// Auxiliary source normalization, separate from native History/Session construction.
/// Raw invalid complete records stay visible; an incomplete tail waits for its framing newline.
pub fn project(bytes:&[u8],source:&str,session:&str)->Res<Vec<Step>> {
    if !safe_label(source) || !safe_label(session){return Err("lineage source/session invalid".into());}
    let mut chains=BTreeMap::<String,Chain>::new();let mut rows=Vec::new();let mut offset=0u64;
    for row in bytes.split_inclusive(|b|*b==b'\n') {
        if !row.ends_with(b"\n"){break;}
        let parsed=decode(row,session);
        if let Ok(r)=&parsed {let _=chains.entry(r.operation_id.clone()).or_default().append(r.clone());}
        rows.push((offset,row,parsed));offset=offset.checked_add(row.len() as u64).ok_or("lineage range overflow")?;
    }
    let mut out=Vec::new();let mut emitted=std::collections::BTreeSet::new();
    for (start,raw,record) in rows {
        let record=match record {
            Ok(r)=>r,
            Err(reason)=>{
                let origin=serde_json::json!({"source":source,"format":FORMAT,"session":session,"start":start,"end":start+raw.len() as u64,"sha256":crate::sha256_hex(raw),"raw":String::from_utf8_lossy(raw)});
                out.push(Step {id:crate::sha256_hex(serde_json::to_vec(&(source,session,start)).unwrap().as_slice()),native_id:None,revision:start,part:0,kind:StepKind::Unknown,turn:Turn{id:None,native_id:None,session:session.into(),unknown:Some("auxiliary record; no native turn".into())},parent:None,parent_session:None,author:"unknown".into(),state:"unknown".into(),data:serde_json::json!({"reason":reason}),presentation:Value::Null,unknown:Some(reason),source:origin.clone(),origins:vec![origin]});continue;
            },
        };
        let chain=&chains[&record.operation_id];
        match chain.latest() {
            Ok(Some(latest)) if latest==&record && emitted.insert(record.operation_id.clone())=>out.push(Link::unresolved(record,"parent resolution requires guarded archive reader").step(source,start,raw)?),
            Err(reason)=>{let mut step=Link::unresolved(record,&reason).step(source,start,raw)?;step.kind=StepKind::Unknown;step.state="unknown".into();out.push(step);},
            _=>{},
        }
    }
    Ok(out)
}
