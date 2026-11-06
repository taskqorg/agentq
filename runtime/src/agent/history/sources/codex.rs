//! codex native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"codex",format:"codex-rollout",home:".codex",env:"CODEX_HOME",roots:&["sessions"],admit,native_version:Some("0.159.3")};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
