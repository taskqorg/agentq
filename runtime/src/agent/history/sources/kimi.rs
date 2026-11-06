//! kimi native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"kimi",format:"history-kimi",home:".kimi-code",env:"KIMI_CODE_HOME",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="wire.jsonl" || file=="state.json" && !std::fs::read_dir(path.parent().unwrap_or(path).join("agents")).into_iter().flatten().flatten().any(|e|e.path().join("wire.jsonl").is_file())}
pub(super) fn session(path:&Path)->Option<String> {path.ancestors().filter_map(Path::file_name).find(|n|n.to_string_lossy().starts_with("session_")).map(|n|n.to_string_lossy().into_owned())}
pub(super) fn companion(path:&Path)->Option<PathBuf> {path.ancestors().find(|p|p.file_name().is_some_and(|n|n.to_string_lossy().starts_with("session_"))).map(|p|p.join("state.json")).filter(|p|p!=path)}
pub(super) fn identity(path:&Path,state:&Value)->Res<Option<(String,Option<String>)>> {
    let Some(base)=session(path) else{return Ok(None)};
    let Some(agent)=path.parent().filter(|p|p.parent().is_some_and(|p|p.file_name().is_some_and(|n|n=="agents"))).and_then(Path::file_name).and_then(|s|s.to_str()) else{return Ok(None)};
    let mut primary=state["agents"].as_object().into_iter().flat_map(|a|a.iter()).filter(|(_,v)|v["type"]=="main" && v["parentAgentId"].is_null()).map(|(id,_)|id.as_str());
    let primary_id=primary.next().unwrap_or("main");
    if primary.next().is_some(){return Err("history Kimi primary agent ambiguous".into());}
    let primary=primary_id;
    let bound=|id:&str|if id==primary{base.clone()}else{format!("{base}:agent:{id}")};
    Ok(Some((bound(agent),state["agents"][agent]["parentAgentId"].as_str().map(bound))))
}

pub(super) fn family(parent:&Path,child:&Path)->Res<bool> {
    let root=|path:&Path|->Res<PathBuf> {path.ancestors().find(|p|p.file_name().is_some_and(|n|n.to_string_lossy().starts_with("session_"))).ok_or("history Kimi session family unavailable")?.canonicalize().map_err(|e|e.to_string())};
    Ok(root(parent)?==root(child)?)
}
