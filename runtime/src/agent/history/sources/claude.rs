//! claude native source contract; Orca MIT scanner/fixtures (../NOTICE).
use super::*;
pub(super) const SPEC:SourceSpec=SourceSpec {name:"claude",format:"claude-native",home:".claude",env:"CLAUDE_CONFIG_DIR",roots:&["projects"],admit,native_version:Some("2.1.319")};
fn admit(path:&Path)->bool {let file=path.file_name().unwrap_or_default().to_string_lossy();file.ends_with(".jsonl")}
