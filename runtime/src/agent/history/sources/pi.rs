//! pi native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"pi",format:"history-pi",home:".pi/agent",env:"PI_CODING_AGENT_DIR",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
