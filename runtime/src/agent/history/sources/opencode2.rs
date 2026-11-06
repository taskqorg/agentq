//! opencode2 native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"opencode2",format:"history-opencode2",home:".local/share/opencode",env:"XDG_DATA_HOME",roots:&["."],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".db") || file.ends_with(".sqlite")}
