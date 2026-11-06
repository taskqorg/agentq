//! opencode native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"opencode",format:"history-opencode",home:".local/share/opencode",env:"XDG_DATA_HOME",roots:&["."],admit,native_version:Some("1.18.23")};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".db") || file.ends_with(".sqlite") || file.ends_with(".json") && has(path,"session")}
