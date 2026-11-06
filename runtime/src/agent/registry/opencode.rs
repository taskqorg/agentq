//! OpenCode candidate. Missing native proof keeps it out of the offered runtime list.
use super::*;
const CLIENT:Client=Client("opencode");
static CONFIG:crate::agent::acp::Config=crate::agent::acp::Config{args:&["acp"],env:&[("OPENCODE_CLIENT","acp"),("OPENCODE_ENABLE_QUESTION_TOOL","false")],home_var:"XDG_DATA_HOME",home_arg:None,modes:&[],rejected_versions:&[],bypass_args:&[],question_prefixes:&[],home:||Some(home())};
static ADAPTER:Adapter=Adapter{client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor{name:"opencode",label:"OpenCode",brand:"OpenCode",family:"opencode",models:&[],minimum:"1.18.31",capabilities:&["history","acp","turn","resume","turn-deltas"]},ops:NativeOps::acp(&CONFIG,|path|Ok(which_in("opencode",path)),||Some(home()))};
pub(super) const ENTRY:Entry=Entry{name:"opencode",aliases:&[],order:6,profile_order:6,worker_order:4,onboard:false,
    profile:Profile{kind:"opencode",label:"OpenCode",aliases:&["opencode"],operations:&[]},runtime:None,builtin:Some(&ADAPTER),unqualified:Some("native version/platform execution, resume, cancel and history proof missing"),
    root:home,files:|_|vec![],setup:|_,_,_|vec![],inspect:no_inspection,hook:|_|Hook::None,hook_key:"hooks",fallback:|_|json!({}),executable:||Some(which_in("opencode",&std::env::var_os("PATH").unwrap_or_default())),diagnose:no_diagnosis,provider:None};
fn home()->PathBuf{root("XDG_DATA_HOME",".local/share")}
