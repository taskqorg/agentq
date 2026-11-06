//! jcode native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"jcode",format:"history-jcode",home:".jcode",env:"JCODE_HOME",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.starts_with("session_") && file.ends_with(".json")}
