//! kimi native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"kimi",format:"history-kimi",home:".kimi-code",env:"KIMI_CODE_HOME",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="wire.jsonl" || file=="state.json" && !path.parent().unwrap_or(path).join("agents/main/wire.jsonl").exists()}
