//! Hermes registration: native worker and installation behavior.
use super::*;

pub(super) const ENTRY: Entry = Entry { name:"hermes", aliases:&[], order:2, profile_order:3, worker_order:3, onboard:true,
    profile:Profile {kind:"hermes",label:"Hermes",aliases:&["hermes"],operations:&["plugin"]},
    runtime:None, builtin:None,unqualified:None,
    root:config_root,files,setup,inspect,hook,hook_key:"hooks",fallback,executable:cli,diagnose:no_diagnosis,provider:None,
};

fn config_root() -> PathBuf { root("HERMES_HOME", ".hermes") }
fn files(root: &Path) -> Vec<PathBuf> {vec![root.join("config.yaml"),root.join("AGENTS.md")]}
fn setup(root: &Path, _: &Path, exe: &Path) -> Vec<Edit> {
    vec![edit("hermes",root.join("plugins/agentq/__init__.py"),&["$file"],json!(include_str!("../../../../integrations/hermes/__init__.py").replace("@AGENTQ_EXE@",&json!(exe.display().to_string()).to_string())),false),
        edit("hermes",root.join("plugins/agentq/plugin.yaml"),&["$file"],json!(include_str!("../../../../integrations/hermes/plugin.yaml")),false),
        edit("hermes",root.join("config.yaml"),&["plugins","enabled"],json!("agentq"),true)]
}
fn inspect(texts: &[Option<String>], _: &[Option<Value>], unread: &[bool]) -> ProfileState {
    let plugin=if unread[0] {"unknown"} else if texts[0].as_deref().unwrap_or("").lines().skip_while(|l|l.trim_end()!="plugins:").skip(1)
        .take_while(|l|l.trim().is_empty()||l.starts_with([' ','\t','-'])).any(|l|l.contains("agentq")) {"enabled"} else {"absent"};
    ProfileState {hooks:"unsupported",plugin,adapter:"plugin",overrides:None}
}
fn hook(event: &str) -> Hook {match event {"post_api_request"=>Hook::ModelCall,"on_session_end"=>Hook::TurnEnd,_=>Hook::None}}
fn fallback(_: &str) -> Value {json!({})}
fn cli() -> Option<PathBuf> {Some(which_in("hermes",&std::env::var_os("PATH").unwrap_or_default()))}
