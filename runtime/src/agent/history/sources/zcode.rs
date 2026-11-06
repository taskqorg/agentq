//! zcode native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"zcode",format:"history-zcode",home:".zcode/cli",env:"ZCODE_HOME",roots:&["db"],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="db.sqlite"}
