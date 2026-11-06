//! One new native protocol, registered and executed without a core enum variant or parser arm.
use super::*;

const CLIENT: Client=Client("example-agent");
static ADAPTER: Adapter=Adapter {client:CLIENT,live:AtomicUsize::new(0),descriptor:Descriptor {
    name:"example-agent",label:"Example Agent",brand:"Example",family:"example",models:&[],minimum:"",capabilities:&["turn","resume"]},
    ops:NativeOps {catalogue:|_,_|Err("catalogue unsupported".into()),background_launch:|_,_,_,_|Err("background unsupported".into()),background_jobs:|_,_,_,_|Err("background unsupported".into()),background_job:|_,_,_|Err("background unsupported".into()),background_stop:|_,_,_,_|Err("background unsupported".into()),background_id:|_|None,app_queue:|_,_,_,_|Err("app-queue unsupported".into()),app_command:|_,_|Err("app-server unsupported".into()),protocol:&PROTOCOL,home_var:"EXAMPLE_HOME",alternate_home_var:None,session_vars:&["EXAMPLE_SESSION"],auth_vars:&[],home:||None,
        executable:|_|std::env::current_exe().map_err(|e|e.to_string()),new_session:||"example-session".into(),args:arguments,mapped:|_|Ok(vec![]),extras:|_|vec![],
        environment:|_,_|{},settings:|_,_|{},login:|_,_|check("unknown","fixture only"),preparation:|_,_|(LaunchCheck::new("workspace","unknown",None,"fixture only",""),LaunchCheck::new("evidence","degraded",None,"fixture only","")),restore_preparation:None,sandbox:None,hook_status:None,limits_for:|_,_|limits::unknown("fixture only"),validate:|_,_,_|Ok(()),protected:|home|vec![home],refusal:|_|None,
        transcript:|_,_|None,background_transcript:|_,_|Err("background unsupported by example-agent".into()),link:|_|Err((Cause("no-session-url"),None)),session_link:|_,_|Err((Cause("no-session-url"),None))}};
pub(super) const ENTRY: Entry=Entry {name:"example-agent",aliases:&["example"],order:0,profile_order:0,worker_order:0,onboard:true,
    profile:Profile {kind:"example-agent",label:"Example Agent",aliases:&["example-client"],operations:&["hooks"]},runtime:Some(&ADAPTER),builtin:Some(&ADAPTER),
    root:home,files:|root|vec![root.join("hooks.json")],setup,inspect:no_inspection,hook:no_hook,hook_key:"hooks",fallback:empty,
    executable:||None,diagnose:no_diagnosis,provider:None};
fn home()->PathBuf {std::env::temp_dir().join(format!("agentq-registry-example-{}",std::process::id()))}
fn setup(root:&Path,_:&Path,exe:&Path)->Vec<Edit> {vec![edit("example-agent",root.join("hooks.json"),&["hooks","Stop"],json!({"hooks":[hook_command(exe,"example-agent","Stop")]}),true)]}
fn arguments(_: &Spec)->Vec<OsString> {vec!["--exact".into(),format!("{}::scripted_cli",module_path!().split_once("::").unwrap().1).into(),"--nocapture".into()]}
fn session(text:&str)->Option<String> {text.lines().filter_map(|line|serde_json::from_str::<Value>(line).ok()).find_map(|v|v["example_session"].as_str().map(String::from))}
fn is_end(v:&Value)->bool {v["example_event"]=="done"}
fn read_report(text:&str,values:&[Value],_:Option<&Path>)->Report {
    let mut r=Report::empty();r.session=session(text);
    if let Some(v)=values.iter().rev().find(|v|is_end(v)) {
        r.outcome="ok";r.error="none";r.text=v["answer"].as_str().map(String::from);
        r.calls=read_calls(values).0;r.usage=sum_usage(&r.calls,Some(1),true);
    } r
}
fn read_calls(values:&[Value])->(Vec<Call>,Option<u64>,bool) {
    let Some(v)=values.iter().rev().find(|v|is_end(v)) else {return (vec![],None,false);};
    (vec![Call {id:None,at_ms:None,input:v["input_tokens"].as_u64(),output:v["output_tokens"].as_u64(),cache_read:None,cache_write:None,reasoning:None,model:None,outcome:"ok",error:"none",aggregate:false}],Some(1),true)
}
static PROTOCOL: Protocol=Protocol {stream_history:"example-native",native_history:"example-native",history_alias:false,history_provenance:|_,_,_|false,history_parent:|_|None,rollout_report:|_,_|{},
    history_formats:&["example-native"],history_session:|_,_,raw|session(raw),history_normalize:|_,_|("unknown","unknown",Value::Null,vec![],"unsupported-source-shape"),history_validity:|_|json!({"status":"unknown","reason":"envelope-validation-unsupported"}),
    ends:is_end,start_limit:|_|None,read_calls,final_text:|_|None,
    turn_ended:|ended,v|ended || is_end(v),
    parse_event:|_,v,out,_| {if let Some(text)=v["answer"].as_str() {out.push(Event::Message {item:None,text:text.into()});}},
    terminal:|_,v,_,_,_|is_end(v).then_some("ok"),assistant_failure:|_|None,permission_mode:|_|None,lifecycle:|_|None,advances:|_|false,
    session,snapshot:|_,_|Snapshot {unknown:Some("not-reported"),..Snapshot::default()},report:read_report,
    prepare_command:stdio_command,input_bytes:plain_input,native_record:|_,_|Err("native history unsupported by example-agent".into()),native_metadata:None,
    finish_native_report:|_,_|{},session_report:|_,_|{},reported:|_,_|None,update_limits:|_,_,_|false,
    account_limits:|_,_,_|limits::unknown("account-limits unsupported by example-agent"),identity:|_|None,
    credentials:|_,_|Err("oauth-unavailable".into()),oauth_token:|_,_|Err("oauth-unavailable".into()),oauth_usage:|_|Err("oauth-unavailable".into()),
};

// This unit-test executable doubles as a portable scripted CLI, like the existing integration fixtures.
#[test]
fn scripted_cli() {
    if std::env::var_os("AGENTQ_EXAMPLE_CLI").is_none() {return;}
    let mut text=String::new();std::io::stdin().read_to_string(&mut text).unwrap();
    if std::env::var_os("AGENTQ_EXAMPLE_STOP").is_some() {println!("{}",json!({"example_event":"working","example_session":"example-session"}));return;}
    println!("\n{}",json!({"example_event":"done","example_session":"example-session","answer":text,"input_tokens":2,"output_tokens":3}));
}

#[test]
fn one_module_launches_resumes_and_reads_its_own_protocol() {
    let root=std::env::temp_dir().join(format!("agentq-example-launch-{}",crate::incident::uuid()));std::fs::create_dir_all(&root).unwrap();
    let runtime=ENTRY.runtime.unwrap();
    let mut launch=Launch::new(&root,root.join("turn.log"),"first","fixtureq");
    launch.command=vec![std::env::current_exe().unwrap().into_os_string()];
    launch.env=vec![("AGENTQ_EXAMPLE_CLI".into(),"1".into())];
    let mut run=runtime.launch(&launch).unwrap();
    while runtime.poll(&mut run).unwrap()==State::Running {std::thread::yield_now();}
    let report=runtime.report(&run).unwrap();assert_eq!((report.outcome,report.text.as_deref(),report.usage.input),("ok",Some("first"),Some(2)));
    assert_eq!(runtime.session_id(&run).as_deref(),Some("example-session"));
    assert_eq!(report.usage.calls,Some(report.calls.len() as u64));
    assert_eq!(report.calls[0].input,report.usage.input);
    assert!(runtime.progress(&mut run).unwrap().iter().any(|e|matches!(e,Event::Message{text,..} if text=="first")));
    assert!(runtime.log_events(&std::fs::read_to_string(&launch.log).unwrap()).iter().any(|e|matches!(e,Event::TurnEnd {outcome:"ok"})));
    launch.input="second".into();
    let mut resumed=runtime.resume("example-session",&launch).unwrap();
    while runtime.poll(&mut resumed).unwrap()==State::Running {std::thread::yield_now();}
    assert_eq!(runtime.report(&resumed).unwrap().text.as_deref(),Some("second"));
    assert_eq!(runtime.session_id(&resumed).as_deref(),Some("example-session"));
    launch.log=root.join("stop.log");launch.env.push(("AGENTQ_EXAMPLE_STOP".into(),"1".into()));
    let mut stopped=runtime.launch(&launch).unwrap();
    while runtime.poll(&mut stopped).unwrap()==State::Running {std::thread::yield_now();}
    let stop=runtime.stop_event(&mut stopped,u64::MAX).unwrap().unwrap();
    assert_eq!(stop.cause,crate::agent::history::stop::StopCause::ExitWithoutTurnEnd);
    assert!(stop.activity_unknown.as_deref().is_none_or(|reason|!reason.contains("unsupported")),"registered history format reaches the shared reader");
    assert_eq!(stopped.stop_facts().unwrap().format,"example-native");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unrelated_native_shapes_are_not_interpreted() {
    let log=concat!(r#"{"type":"assistant","apiError":"usage_limit_reached","message":{"id":"foreign","usage":{"input_tokens":99}}}"#, "\n", r#"{"type":"result","is_error":true,"result":"foreign"}"#);
    let r=ADAPTER.classify(log);
    assert_eq!((r.outcome,r.error,r.reached,r.text.as_deref(),r.usage.calls),("unknown","no-end",None,None,None));
    let mut c=events::Cursor::default();
    c.read(CLIENT,log.lines().map(|l|serde_json::from_str(l).unwrap()).collect());
    assert!(matches!(c.start_limit,StartLimit::Pending));
    assert!(!(PROTOCOL.turn_ended)(false,&json!({"type":"result"})));
    let root=std::env::temp_dir().join(format!("agentq-example-history-{}",crate::incident::uuid()));
    std::fs::create_dir_all(&root).unwrap();let path=root.join("history.jsonl");std::fs::write(&path,format!("{log}\n")).unwrap();
    let mut h=crate::agent::history::History::new("example-native","fixture","example-session").unwrap();
    let page=h.poll(&path).unwrap();
    assert_eq!(page.events.len(),2);assert!(page.events.iter().all(|v|v["session_binding"]=="caller" && v["kind"]=="unknown"));
    std::fs::remove_dir_all(root).unwrap();
    assert!((PROTOCOL.history_session)("example-native",&json!({"sessionId":"foreign"}),r#"{"sessionId":"foreign"}"#).is_none());
}
