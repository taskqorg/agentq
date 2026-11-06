//! gemini native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"gemini",format:"history-gemini",home:".gemini",env:"GEMINI_CLI_HOME",roots:&["tmp"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();(file.ends_with(".json") || file.ends_with(".jsonl")) && (has(path,"chats") || file.starts_with("session-"))}
