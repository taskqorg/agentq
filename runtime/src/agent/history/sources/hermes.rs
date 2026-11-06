//! hermes native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"hermes",format:"history-hermes",home:".hermes",env:"HERMES_HOME",roots:&["sessions","state.db"],admit,native_version:Some("0.21.6+509.gf925b01")};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file=="state.db" || file.starts_with("session_") && file.ends_with(".json")}
