//! Grok candidate. Missing native proof keeps it out of the offered runtime list.
use super::*;
const CLIENT:Client=Client("grok");
static CONFIG:crate::agent::acp::Config=crate::agent::acp::Config{args:&["agent","stdio"],env:&[],home_var:"GROK_HOME",home_arg:None,modes:&[],rejected_versions:&[],bypass_args:&[],question_prefixes:&[],home:||Some(home())};
static ADAPTER:Adapter=Adapter{client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor{name:"grok",label:"Grok",brand:"xAI",family:"grok",models:&[],minimum:"",capabilities:&["history","acp","turn","resume","turn-deltas"]},ops:NativeOps{auth_vars:&["XAI_API_KEY"],..NativeOps::acp(&CONFIG,|path|Ok(which_in("grok",path)),||Some(home()))}};
pub(super) const ENTRY:Entry=Entry{name:"grok",aliases:&[],order:5,profile_order:5,worker_order:3,onboard:false,
    profile:Profile{kind:"grok",label:"Grok",aliases:&["grok"],operations:&[]},runtime:None,builtin:Some(&ADAPTER),unqualified:Some("native version/platform execution, resume, cancel and history proof missing"),
    root:home,files:|_|vec![],setup:|_,_,_|vec![],inspect:no_inspection,hook:|_|Hook::None,hook_key:"hooks",fallback:|_|json!({}),executable:||Some(which_in("grok",&std::env::var_os("PATH").unwrap_or_default())),diagnose:no_diagnosis,provider:None};
fn home()->PathBuf{root("GROK_HOME",".grok")}
