//! rovo native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"rovo",format:"history-rovo",home:".rovodev",env:"ROVODEV_HOME",roots:&["sessions"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="session_context.json" || file=="metadata.json" && !path.with_file_name("session_context.json").exists()}
