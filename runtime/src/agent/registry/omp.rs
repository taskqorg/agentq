//! OMP candidate. Missing native proof keeps it out of the offered runtime list.
use super::*;
const CLIENT:Client=Client("omp");
static CONFIG:crate::agent::acp::Config=crate::agent::acp::Config{args:&["acp"],env:&[("PI_NO_PTY","1")],home_var:"PI_CODING_AGENT_DIR",home_arg:None,modes:&[],rejected_versions:&[],bypass_args:&[],question_prefixes:&[],legacy_command:|_|false};
static ADAPTER:Adapter=Adapter{client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor{name:"omp",label:"OMP",brand:"OMP",family:"pi",models:&[],minimum:"17.0.5",capabilities:&["turn","resume","turn-deltas"]},ops:NativeOps::acp(&CONFIG,|path|Ok(which_in("omp",path)),||Some(home()))};
pub(super) const ENTRY:Entry=Entry{name:"omp",aliases:&[],order:8,profile_order:8,worker_order:6,onboard:false,
    profile:Profile{kind:"omp",label:"OMP",aliases:&["omp"],operations:&[]},runtime:None,builtin:Some(&ADAPTER),unqualified:Some("native version/platform execution, resume, cancel and history proof missing"),
    root:home,files:|_|vec![],setup:|_,_,_|vec![],inspect:no_inspection,hook:|_|Hook::None,hook_key:"hooks",fallback:|_|json!({}),executable:||Some(which_in("omp",&std::env::var_os("PATH").unwrap_or_default())),diagnose:no_diagnosis,provider:None};
fn home()->PathBuf{root("PI_CODING_AGENT_DIR",".omp/agent")}
