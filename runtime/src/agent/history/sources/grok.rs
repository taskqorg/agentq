//! grok native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"grok",format:"history-grok",home:".grok",env:"GROK_HOME",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="chat_history.jsonl" || file=="summary.json" && !path.with_file_name("chat_history.jsonl").exists()}
