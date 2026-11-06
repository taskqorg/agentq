//! Claude Desktop Chat registration: native worker and installation behavior.
use super::*;

pub(super) const ENTRY: Entry = Entry { name:"claude-desktop", aliases:&[], order:3, profile_order:2, worker_order:4, onboard:false,
    profile:Profile {kind:"claude-desktop-chat",label:"Claude Desktop Chat",aliases:&["claude-desktop-chat"],operations:&[]},
    runtime:None, builtin:None,
    root:config_root,files,setup,inspect,hook,hook_key:"hooks",fallback,executable:cli,diagnose:no_diagnosis,provider:None,
};

fn config_root() -> PathBuf {
    let home=root("CLAUDE_CONFIG_DIR",".claude").parent().map(Path::to_path_buf).unwrap_or_default();
    match std::env::consts::OS {
        "macos"=>home.join("Library/Application Support/Claude"),
        "windows"=>std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("Claude"),
        _=>std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(||home.join(".config")).join("Claude"),
    }
}
fn files(root: &Path) -> Vec<PathBuf> {vec![root.join("claude_desktop_config.json")]}
fn setup(_: &Path, _: &Path, _: &Path) -> Vec<Edit> {vec![]}
fn inspect(_: &[Option<String>], _: &[Option<Value>], _: &[bool]) -> ProfileState {no_inspection(&[],&[],&[])}
fn hook(_: &str) -> Hook {Hook::None}
fn fallback(_: &str) -> Value {json!({})}
fn cli() -> Option<PathBuf> {None}
