//! droid native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"droid",format:"history-droid",home:".factory",env:"FACTORY_HOME",roots:&["sessions", "projects"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
