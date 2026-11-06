//! codebuddy native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"codebuddy",format:"history-codebuddy",home:".codebuddy",env:"CODEBUDDY_HOME",roots:&["projects"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
