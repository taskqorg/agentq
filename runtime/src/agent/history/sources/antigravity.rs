//! antigravity native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"antigravity",format:"antigravity-sidecar",home:".gemini/antigravity-cli",env:"ANTIGRAVITY_HOME",roots:&["brain"],admit,native_version:Some("1.5.2")};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
