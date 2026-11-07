//! copilot native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"copilot",format:"history-copilot",home:".copilot",env:"COPILOT_HOME",roots:&["session-state"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
