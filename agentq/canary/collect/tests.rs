use super::*;
pub(in crate::canary) fn binding(root:&Path,exe:&str)->Value {
    json!({"installation":"fixture","host":"fixture-host","runtime":"codex","account":"fixture-account",
        "home":root.join("home").canonicalize().unwrap_or_else(|_|root.join("home")),"model":null,"permissions":Policy::default(),"session_policy":{"lifetime":"recoverable","readiness":"continue"},
        "scenario_revision":1,"approved_operation":"native-turn","executable":{"path":exe,"sha256":"0".repeat(64)}})
}

// The suite already serializes process-environment fixtures; restore even when an assertion fails.
#[cfg(unix)]
struct Ambient {vars:Vec<(&'static str,Option<std::ffi::OsString>)>,cwd:PathBuf}
#[cfg(unix)]
impl Ambient {
    fn conflicting(home:&Path)->Self {
        let mut guard=Self {vars:vec![],cwd:std::env::current_dir().unwrap()};
        for (key,value) in [("HOME",home.as_os_str()),("CODEX_HOME",home.as_os_str()),
            ("OPENAI_API_KEY",std::ffi::OsStr::new("invented-openai-token")),("CODEX_API_KEY",std::ffi::OsStr::new("invented-codex-token")),
            ("ANTHROPIC_AUTH_TOKEN",std::ffi::OsStr::new("invented-claude-token")),("CODEX_THREAD_ID",std::ffi::OsStr::new("invented-unrelated-thread")),
            ("CLAUDE_CODE_SESSION_ID",std::ffi::OsStr::new("invented-unrelated-session"))] {
            guard.vars.push((key,std::env::var_os(key)));
            unsafe {std::env::set_var(key,value);}
        }
        std::env::set_current_dir(home).unwrap();guard
    }
}
#[cfg(unix)]
impl Drop for Ambient {
    fn drop(&mut self) {
        let _=std::env::set_current_dir(&self.cwd);
        for (key,value) in &self.vars {unsafe {match value {Some(v)=>std::env::set_var(key,v),None=>std::env::remove_var(key)}}}
    }
}

#[test]
#[cfg(unix)]
fn actual_runtime_collection_binds_every_probe_context_and_keeps_unknowns_private() {
    use std::os::unix::fs::PermissionsExt;
    let mut workspace=workspace().unwrap();let root=&workspace.path;let data=root.join("data");
    std::fs::create_dir_all(root.join("home")).unwrap();std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("installation"),"fixture").unwrap();
    let ambient=root.join("ambient-unrelated-home");std::fs::create_dir(&ambient).unwrap();
    let _ambient=Ambient::conflicting(&ambient);
    let exe=root.join("native-fixture");
    let script=r##"#!/usr/bin/python3
import sys,os,json,pathlib,uuid,hashlib
base=pathlib.Path(__file__).parent
home=pathlib.Path(os.environ.get('CODEX_HOME',str(base/'home')))
args=sys.argv[1:]
# Record entry before any assertion/read: invalid authority must not execute even a failing probe.
with (base/'invocations').open('a') as f: f.write(json.dumps(args)+'\n')
assert home.resolve()==(base/'home').resolve()
assert all(k not in os.environ for k in ['OPENAI_API_KEY','CODEX_API_KEY','ANTHROPIC_AUTH_TOKEN','CODEX_THREAD_ID','CLAUDE_CODE_SESSION_ID'])
assert pathlib.Path.cwd()!=(base/'ambient-unrelated-home').resolve()
assert pathlib.Path.cwd().name.startswith('agentq-canary-')
with (base/'contexts').open('a') as f: f.write(json.dumps({'args':args,'cwd':str(pathlib.Path.cwd()),'home_bound':True,'auth_session_clean':True})+'\n')
mode=(base/'mode').read_text()
if args==['--version']:
 count=len((base/'invocations').read_text().splitlines())
 if mode=='version-change':
  n=int((base/'versions').read_text())+1 if (base/'versions').exists() else 1
  (base/'versions').write_text(str(n))
  print('codex-cli 99.1.0' if n==1 else 'codex-cli 99.2.0')
 else: print('codex-cli 99.1.0')
 sys.exit(0)
if args[:2]==['login','status']:
 print('Logged in using ChatGPT');sys.exit(0)
if args and args[0]=='app-server':
 for line in sys.stdin:
  v=json.loads(line)
  if 'id' not in v: continue
  result={}
  if v['method']=='config/read': result={'config':{'projects':{os.getcwd():{'trust_level':'untrusted' if mode=='refuse' else 'trusted'}}}}
  if v['method']=='hooks/list': result={'data':[]}
  if v['method']=='model/list': result={'data':[{'model':'fixture-model','displayName':'Fixture'}]}
  print(json.dumps({'id':v['id'],'result':result}),flush=True)
 sys.exit(0)
assert args[:2]==['exec','--json'],args
assert sys.stdin.read()=='Reply exactly AGENTQ_CANARY_OK'
session=str(uuid.uuid4())
model='other-model' if mode=='mismatch' else 'fixture-model'
rows=[{'type':'session_meta','payload':{'id':session,'model':model,'cwd':os.getcwd(),'forked_from_id':None}},
 {'type':'response_item','payload':{'type':'message','role':'assistant','content':[{'type':'output_text','text':'AGENTQ_CANARY_OK'}]}}]
if mode=='field-drift': rows[0]['payload']['secret-new-field']='private fixture conversation'
if mode=='drift': rows.append({'type':'synthetic-secret-record','secret-content-bearing-key':'private fixture conversation'})
path=home/'sessions'/(session+'.jsonl');path.parent.mkdir(exist_ok=True)
path.write_text(''.join(json.dumps(r)+'\n' for r in rows)+('{' if mode=='partial-history' else ''))
if mode!='missing-hook':
 marker=base/'data'/'sessions'/(hashlib.sha256(('session:model:'+session).encode()).hexdigest()[:32]+'.json')
 marker.parent.mkdir(exist_ok=True);marker.write_text(json.dumps({'launched':True}))
print(json.dumps({'type':'thread.started','thread_id':session}),flush=True)
print(json.dumps({'type':'turn.started'}),flush=True)
print(json.dumps({'type':'item.completed','item':{'id':'answer','type':'agent_message','text':'WRONG' if mode=='fail' else 'AGENTQ_CANARY_OK'}}),flush=True)
usage={} if mode=='missing-usage' else {'input_tokens':1,'cached_input_tokens':0,'output_tokens':1}
print(json.dumps({'type':'turn.completed','usage':usage}),flush=True)
"##;
    std::fs::write(&exe,script).unwrap();std::fs::set_permissions(&exe,std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut b=binding(root,exe.to_str().unwrap());b["executable"]["sha256"]=json!(hash(script.as_bytes()));
    b["home"]=json!(root.join("home").canonicalize().unwrap());
    b["executable"]["path"]=json!(exe.canonicalize().unwrap());
    let raw=b.to_string();
    assert_eq!(request_with(&data,&raw,Some("fixture-host"))["reason"],"opt-in-unverified");
    assert!(!root.join("invocations").exists());
    crate::control::save(&data.join("canary"),"opt-in.json",&b).unwrap();
    let receipt=data.join("canary/opt-in.json");
    std::fs::set_permissions(&receipt,std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(request_with(&data,&raw,Some("fixture-host"))["reason"],"opt-in-unverified");
    std::fs::set_permissions(&receipt,std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut unapproved=b.clone();unapproved["model"]=json!("unapproved-model");
    assert_eq!(request_with(&data,&unapproved.to_string(),Some("fixture-host"))["reason"],"opt-in-unverified");
    assert!(!root.join("invocations").exists());

    for (mode,state) in [("normal","unknown"),("missing-hook","unknown"),("missing-usage","unknown"),("fail","fail"),("drift","drift"),("field-drift","drift"),("partial-history","unknown")] {
        std::fs::write(root.join("mode"),mode).unwrap();
        let report=request_with(&data,&raw,Some("fixture-host"));
        assert_eq!(report["state"],state,"{mode}: {report}");
        assert_eq!(report["detected_version"],"99.1.0");
        let context=std::fs::read_to_string(root.join("contexts")).unwrap();
        let last:Value=serde_json::from_str(context.lines().last().unwrap()).unwrap();
        assert!(!report.to_string().contains(last["cwd"].as_str().unwrap()));
        if mode=="normal" {for k in DIMENSIONS {assert_eq!(report["dimensions"][k],true,"{k}: {report}");}assert!(report["conditions"]["permissions"].is_null());}
        if mode=="missing-hook" {assert!(report["dimensions"]["hook_correlation"].is_null());}
        if mode=="partial-history" {assert!(report["dimensions"]["history"].is_null());}
        if mode=="missing-usage" {assert!(report["dimensions"]["usage"].is_null());}
        for text in ["private fixture conversation","secret-content-bearing-key","synthetic-secret-record","secret-new-field",exe.to_str().unwrap()] {assert!(!report.to_string().contains(text));}
    }
    assert_eq!(std::fs::read_dir(data.join("canary/findings")).unwrap().count(),1);
    let stored=std::fs::read_to_string(std::fs::read_dir(data.join("canary/findings")).unwrap().next().unwrap().unwrap().path()).unwrap();
    assert!(stored.contains("unknown_keys"));
    for secret in ["private fixture conversation","secret-new-field","synthetic-secret-record"] {assert!(!stored.contains(secret));}
    let invocations=std::fs::read_to_string(root.join("invocations")).unwrap();
    assert_eq!(invocations.lines().filter(|l|l.contains("\"exec\"")).count(),7,"no retry or resume");
    let contexts=std::fs::read_to_string(root.join("contexts")).unwrap().lines().map(|line|serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();
    assert!(contexts.iter().all(|v|v["home_bound"]==true && v["auth_session_clean"]==true));
    for context in &contexts {assert!(!stored.contains(context["cwd"].as_str().unwrap()));}
    for turn in contexts.iter().filter(|v|v["args"][0]=="exec") {
        assert!(contexts.iter().any(|v|v["args"]==json!(["--version"]) && v["cwd"]==turn["cwd"]),"version and native turn must share the exact isolated root");
    }
    let mut changed=b.clone();changed["model"]=json!("fixture-model");
    std::fs::write(root.join("mode"),"mismatch").unwrap();
    crate::control::save(&data.join("canary"),"opt-in.json",&changed).unwrap();
    let report=request_with(&data,&changed.to_string(),Some("fixture-host"));
    assert_eq!(report["state"],"fail","{report}");assert_eq!(report["conditions"]["model"],false);
    std::fs::write(root.join("mode"),"refuse").unwrap();
    let before=std::fs::read_to_string(root.join("invocations")).unwrap().lines().filter(|l|l.contains("\"exec\"")).count();
    assert_eq!(request_with(&data,&changed.to_string(),Some("fixture-host"))["reason"],"runtime-preflight-refused");
    let after=std::fs::read_to_string(root.join("invocations")).unwrap().lines().filter(|l|l.contains("\"exec\"")).count();assert_eq!(before,after);
    std::fs::write(root.join("mode"),"version-change").unwrap();
    assert_eq!(request_with(&data,&changed.to_string(),Some("fixture-host"))["reason"],"executable-or-version-changed");
    let after=std::fs::read_to_string(root.join("invocations")).unwrap().lines().filter(|l|l.contains("\"exec\"")).count();assert_eq!(before,after);
    std::fs::write(&exe,"changed executable").unwrap();
    assert_eq!(request_with(&data,&changed.to_string(),Some("fixture-host"))["reason"],"executable-unverified");
    workspace.terminal=true;
}
