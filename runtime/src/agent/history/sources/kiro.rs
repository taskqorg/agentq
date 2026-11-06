//! kiro native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"kiro",format:"history-kiro",home:".kiro",env:"KIRO_HOME",roots:&["sessions/cli"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl") || file.ends_with(".json") && file.len()==41 && !path.with_extension("jsonl").exists()}
pub(super) fn companion(path:&Path)->Option<PathBuf> {path.extension().filter(|e|*e=="jsonl").map(|_|path.with_extension("json"))}
