//! Source registry: per-agent roots and schemas, no client name branches.
use super::*;
use std::path::PathBuf;
mod decode;
mod snapshot;
mod wal;
mod runtime;
mod claude;
mod codex;
mod antigravity;
mod hermes;
mod gemini;
mod cursor;
mod copilot;
mod cline;
mod codebuddy;
mod devin;
mod droid;
mod grok;
mod jcode;
mod kimi;
mod kiro;
mod muse;
mod omp;
mod openclaw;
mod opencode;
mod opencode2;
mod pi;
mod qoder;
mod zcode;
mod rovo;
#[derive(Clone,Copy)]
pub struct SourceSpec {pub name:&'static str,pub format:&'static str,pub home:&'static str,pub env:&'static str,pub roots:&'static [&'static str],admit:fn(&Path)->bool,pub native_version:Option<&'static str>}
pub const SPECS:&[&SourceSpec]=&[&claude::SPEC,&codex::SPEC,&antigravity::SPEC,&hermes::SPEC,&gemini::SPEC,&cursor::SPEC,&copilot::SPEC,&cline::SPEC,&codebuddy::SPEC,&devin::SPEC,&droid::SPEC,&grok::SPEC,&jcode::SPEC,&kimi::SPEC,&kiro::SPEC,&muse::SPEC,&omp::SPEC,&openclaw::SPEC,&opencode::SPEC,&opencode2::SPEC,&pi::SPEC,&qoder::SPEC,&zcode::SPEC,&rovo::SPEC];
pub fn get(name:&str)->Option<&'static SourceSpec> {SPECS.iter().copied().find(|s|s.name==name)}
pub(super) fn external(format:&str)->bool {format.starts_with("history-") && SPECS.iter().any(|s|s.format==format)}
fn has(path:&Path,part:&str)->bool {path.components().any(|c|c.as_os_str()==part)}
pub fn runtimes()->Vec<&'static dyn super::super::Runtime> {SPECS.iter().filter_map(|s|super::super::runtime(s.name)).collect()}
pub fn capability(name:&str)->Res<Value> {
    let spec=get(name).ok_or("history runtime unsupported")?;
    let native=spec.native_version.filter(|_|cfg!(target_os="macos"));
    Ok(json!({"agent":name,"format":spec.format,"level":if native.is_some(){"limited"}else{"fixture-only"},
        "reason":if native.is_some(){"native Mac read proof; unknown native shapes retain raw sources"}else{"no native read qualification on this platform"},
        "native_version":native,"os":std::env::consts::OS,"home_variable":spec.env,"roots":spec.roots,
        "launch":matches!(name,"claude"|"codex"|"antigravity")}))
}
fn source_home(spec:&SourceSpec,home:Option<&Path>)->Res<PathBuf> {
    let user=std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")).map(PathBuf::from).ok_or("history user home unknown")?;
    let override_home=std::env::var_os(spec.env).filter(|s|!s.is_empty()).map(PathBuf::from);
    let explicit=home.is_some() || override_home.is_some();
    let mut base=home.map(Path::to_path_buf).or(override_home).unwrap_or_else(||user.join(spec.home));
    if spec.name=="devin" && !explicit {base=if cfg!(windows){std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(||user.join("AppData/Roaming")).join("devin/cli")}else{std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(||user.join(".local/share")).join("devin/cli")};}
    if !base.is_absolute(){return Err("history home override requires an absolute path".into());}
    if home.is_none() && spec.env=="XDG_DATA_HOME" && std::env::var_os("XDG_DATA_HOME").is_some_and(|v|!v.is_empty()) {base=base.join(if spec.name=="muse"{"muse"}else{"opencode"});}
    Ok(base)
}
fn roots(spec:&SourceSpec,home:Option<&Path>)->Res<Vec<PathBuf>> {
    let base=source_home(spec,home)?;
    if home.is_none() && matches!(spec.name,"pi"|"omp") && std::env::var_os(spec.env).is_some_and(|v|!v.is_empty()){
        let leaf=base.file_name().and_then(|n|n.to_str()).unwrap_or_default();
        return Ok(vec![if leaf=="agent"{base.join("sessions")}else if leaf==format!(".{}",spec.name){base.join("agent/sessions")}else{base}]);
    }
    let mut roots:Vec<_>=spec.roots.iter().map(|r|if *r=="sessions" && base.file_name().is_some_and(|n|n=="sessions"){base.clone()}else{base.join(r)}).collect();
    if home.is_none() && spec.name=="openclaw"{if let Some(user)=std::env::var_os("HOME").or_else(||std::env::var_os("USERPROFILE")){roots.push(PathBuf::from(user).join(".clawdbot/agents"));}}
    if home.is_none() && matches!(spec.name,"opencode"|"opencode2") {if let Some(path)=std::env::var_os("OPENCODE_DB").filter(|p|Path::new(p).is_absolute()){roots.push(path.into());}}
    Ok(roots)
}
fn walk(path:&Path,spec:&SourceSpec,out:&mut Vec<PathBuf>)->Res<()> {
    if !path.exists(){return Ok(());}
    if path.is_file(){if (spec.admit)(path){out.push(path.into());}return Ok(());}
    for entry in std::fs::read_dir(path).map_err(|e|format!("history discovery: {e}"))? {
        let entry=entry.map_err(|e|e.to_string())?;let kind=entry.file_type().map_err(|e|e.to_string())?;
        if kind.is_dir() && !matches!(entry.file_name().to_str(),Some("node_modules"|".git"|"tools")){walk(&entry.path(),spec,out)?;}else if kind.is_file() && (spec.admit)(&entry.path()){out.push(entry.path());}
    }
    Ok(())
}
pub(super) fn source_id(runtime:&str,path:&Path,session:&str)->String {
    if get(runtime).is_some_and(|s|external(s.format)){crate::sha256_hex(json!([runtime,path,session]).to_string().as_bytes())}else{crate::sha256_hex(path.to_string_lossy().as_bytes())}
}
pub(super) fn list(name:&str,home:Option<&Path>)->Res<Vec<super::sessions::Source>> {
    let spec=get(name).ok_or("history runtime unsupported")?;let mut paths=Vec::new();
    for root in roots(spec,home)?{walk(&root,spec,&mut paths)?;}paths.sort();paths.dedup();
    let mut out=Vec::new();for path in paths {out.extend(identify(name,&path)?);}Ok(out)
}
pub(super) fn identify(name:&str,path:&Path)->Res<Vec<super::sessions::Source>> {
    let spec=get(name).ok_or("history runtime unsupported")?;
    let ids=snapshot::identities(spec,path)?;
    Ok(ids.into_iter().map(|(session,parent)|super::sessions::Source {id:source_id(name,path,&session),native_session:session.clone(),session,path:path.into(),format:spec.format.into(),parent,parent_step:None}).collect())
}
pub(super) fn same_family(runtime:&str,parent:&Path,child:&Path)->Res<bool> {
    if runtime=="kimi" {kimi::family(parent,child)}else{Ok(true)}
}
pub(in crate::agent::history) use snapshot::{steps,source_bytes,media_steps};
pub(in crate::agent) fn history_runtime(name:&str)->Option<&'static dyn super::super::Runtime> {runtime::get(name)}
