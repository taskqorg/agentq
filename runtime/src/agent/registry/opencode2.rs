//! OpenCode 2 candidate. Missing native proof keeps it out of the offered runtime list.
use super::*;
const CLIENT:Client=Client("opencode2");
static CONFIG:crate::agent::acp::Config=crate::agent::acp::Config{args:&["acp"],env:&[("OPENCODE_CLIENT","acp"),("OPENCODE_ENABLE_QUESTION_TOOL","false")],home_var:"XDG_DATA_HOME",home_arg:None,modes:&[],rejected_versions:&[],bypass_args:&[],question_prefixes:&[],home:||Some(home())};
static ADAPTER:Adapter=Adapter{client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor{name:"opencode2",label:"OpenCode 2",brand:"OpenCode",family:"opencode",models:&[],minimum:"2.0.14",capabilities:&["history","acp","turn","resume","turn-deltas"]},ops:NativeOps::acp(&CONFIG,|path|Ok(which_in("opencode2",path)),||Some(home()))};
pub(super) const ENTRY:Entry=Entry{name:"opencode2",aliases:&[],order:7,profile_order:7,worker_order:5,onboard:false,
    profile:Profile{kind:"opencode2",label:"OpenCode 2",aliases:&["opencode2"],operations:&[]},runtime:None,builtin:Some(&ADAPTER),unqualified:Some("native version/platform execution, resume, cancel and history proof missing"),
    root:home,files:|_|vec![],setup:|_,_,_|vec![],inspect:no_inspection,hook:|_|Hook::None,hook_key:"hooks",fallback:|_|json!({}),executable:||Some(which_in("opencode2",&std::env::var_os("PATH").unwrap_or_default())),diagnose:no_diagnosis,provider:None};
fn home()->PathBuf{root("XDG_DATA_HOME",".local/share")}
