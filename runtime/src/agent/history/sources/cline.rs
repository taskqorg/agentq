//! cline native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"cline",format:"history-cline",home:".cline/data/sessions",env:"CLINE_SESSION_DATA_DIR",roots:&["."],admit,native_version:None};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".messages.json") || file.ends_with(".json") && !path.with_file_name(format!("{}.messages.json",path.file_stem().unwrap_or_default().to_string_lossy())).exists()}
