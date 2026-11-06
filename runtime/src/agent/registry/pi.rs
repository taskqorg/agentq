//! Pi candidate. Missing native proof keeps it out of the offered runtime list.
use super::*;
const CLIENT:Client=Client("pi");
static CONFIG:crate::agent::jsonl_rpc::Config=crate::agent::jsonl_rpc::Config{args:&["--mode","rpc"],home_var:"PI_CODING_AGENT_DIR",resume_arg:"--session"};
static ADAPTER:Adapter=Adapter{client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor{name:"pi",label:"Pi",brand:"Pi",family:"pi",models:&[],minimum:"",capabilities:&["turn","resume","turn-deltas"]},ops:NativeOps::rpc(&CONFIG,|path|Ok(which_in("pi",path)),||Some(home()))};
pub(super) const ENTRY:Entry=Entry{name:"pi",aliases:&[],order:9,profile_order:9,worker_order:7,onboard:false,
    profile:Profile{kind:"pi",label:"Pi",aliases:&["pi"],operations:&[]},runtime:None,builtin:Some(&ADAPTER),unqualified:Some("native version/platform execution, resume, cancel and history proof missing"),
    root:home,files:|_|vec![],setup:|_,_,_|vec![],inspect:no_inspection,hook:|_|Hook::None,hook_key:"hooks",fallback:|_|json!({}),executable:||Some(which_in("pi",&std::env::var_os("PATH").unwrap_or_default())),diagnose:no_diagnosis,provider:None};
fn home()->PathBuf{root("PI_CODING_AGENT_DIR",".pi/agent")}
