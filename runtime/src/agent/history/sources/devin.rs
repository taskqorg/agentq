//! devin native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"devin",format:"history-devin",home:".local/share/devin/cli",env:"DEVIN_HOME",roots:&["transcripts", "agent_logs"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".json")}
