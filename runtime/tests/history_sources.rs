//! M12.22: all admitted native formats share the trait; snapshots converge without native writes.
use agentq_runtime::agent::{self,history::{sources,StepKind}};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
fn root(tag:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-sources-{}-{tag}",std::process::id()));let _=fs::remove_dir_all(&p);fs::create_dir_all(&p).unwrap();p}
fn write(path:&Path,value:&Value,jsonl:bool) {fs::create_dir_all(path.parent().unwrap()).unwrap();let text=if jsonl{value.as_array().unwrap().iter().map(|r|format!("{r}\n")).collect()}else{value.to_string()};fs::write(path,text).unwrap();}
#[test]
fn all_source_fixtures_read_through_runtime_with_raw_access() {
    let fixtures:Value=serde_json::from_str(include_str!("fixtures/history/agents.json")).unwrap();
    assert_eq!(sources::runtimes().len(),24);
    for (name,fixture) in fixtures.as_object().unwrap() {
        if fixture["rows"]["sqlite"]==true {continue;}
        let root=root(name);let file=root.join(fixture["path"].as_str().unwrap());write(&file,&fixture["rows"],fixture["jsonl"]==true);
        let before=fs::read(&file).unwrap();let runtime=agent::runtime(name).unwrap();
        assert!(runtime.descriptor().history_support().unwrap()["level"].is_string());
        let found=runtime.history_sessions(Some(&root)).unwrap();assert_eq!(found.len(),1,"{name}: {found:?}");
        assert_eq!(found[0].session,fixture["session"],"{name}");
        let mut session=runtime.history(&found[0].id,Some(&root),None,None).unwrap();let page=session.steps(u64::MAX).unwrap();
        assert!(page.steps.iter().any(|s|s.kind==StepKind::Message && s.author=="user" && s.data["text"].as_str().is_some_and(|t|t.contains("title"))),"{name}: {:?}",page.steps.iter().map(|s|(&s.kind,&s.author,&s.data)).collect::<Vec<_>>());
        assert!(page.steps.iter().any(|s|matches!(s.kind,StepKind::Message|StepKind::Thinking) && s.author=="assistant" && s.data["text"]==fixture["assistant_text"]),"{name}: {:?}",page.steps.iter().map(|s|(&s.kind,&s.author,&s.data)).collect::<Vec<_>>());
        for step in &page.steps {assert!(step.source["record"].is_object());assert!(step.source["files"].is_array());}
        let source=&page.steps[0].source["files"][0];let bytes=session.source_bytes(source["key"].as_str().unwrap(),0,source["size"].as_u64().unwrap()).unwrap();assert_eq!(bytes,before,"{name}");
        assert!(session.source_bytes("foreign",0,1).is_err());
        let mut recovered=runtime.history(&found[0].id,Some(&root),None,Some(&page.checkpoint)).unwrap();assert!(recovered.steps(1).unwrap().steps.is_empty(),"{name}");
        assert_eq!(fs::read(&file).unwrap(),before,"{name}: native source changed");fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn snapshots_reset_on_rewrite_and_keep_native_row_identity() {
    let root=root("rewrite");let file=root.join("sessions/session_fixture.json");
    write(&file,&json!({"id":"fixture","messages":[{"id":"prompt","role":"user","content":"first"},{"id":"reply","role":"assistant","content":"old"}]}),false);
    let rt=agent::runtime("jcode").unwrap();let id=rt.history_sessions(Some(&root)).unwrap()[0].id.clone();let mut session=rt.history(&id,Some(&root),None,None).unwrap();
    let first=session.steps(1).unwrap();assert!(first.reset);let second=session.steps(u64::MAX).unwrap();assert!(!second.reset);let reply=second.steps.iter().find(|s|s.data["text"]=="old").unwrap();
    write(&file,&json!({"id":"fixture","messages":[{"id":"reply","role":"assistant","content":"new"}]}),false);
    let mut session=rt.history(&id,Some(&root),None,Some(&second.checkpoint)).unwrap();let page=session.steps(u64::MAX).unwrap();
    assert!(page.reset);assert!(page.generation>second.generation);let next=page.steps.iter().find(|s|s.data["text"]=="new").unwrap();assert_eq!(next.id,reply.id);assert!(next.revision>reply.revision);
    write(&file,&json!({"id":"foreign","messages":[]}),false);let before=session.checkpoint();assert!(session.steps(1).is_err());assert_eq!(session.checkpoint(),before);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn sqlite_wal_sources_are_scoped_read_only_and_keep_console_fields() {
    for name in ["opencode","opencode2","zcode","hermes"] {
        let root=root(&format!("sql-{name}"));let file=root.join(if name=="zcode"{"db/db.sqlite"}else if name=="hermes"{"state.db"}else{"opencode.db"});fs::create_dir_all(file.parent().unwrap()).unwrap();
        let db=rusqlite::Connection::open(&file).unwrap();db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        if name=="hermes" {
            db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,parent_session_id TEXT,model TEXT); CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,content TEXT,tool_calls TEXT,tool_call_id TEXT,reasoning_content TEXT);
            CREATE TABLE session_model_usage(session_id TEXT,model TEXT,input_tokens INTEGER,output_tokens INTEGER,actual_cost_usd REAL,estimated_cost_usd REAL);INSERT INTO session_model_usage VALUES('fixture','fixture-model',20,10,NULL,0.99);
            INSERT INTO sessions VALUES('fixture',NULL,'fixture-model'); INSERT INTO messages VALUES(1,'fixture','user','native prompt',NULL,NULL,NULL); INSERT INTO messages VALUES(2,'fixture','assistant','answer','[{\"id\":\"shell\",\"function\":{\"name\":\"terminal\",\"arguments\":\"{\\\"command\\\":\\\"printf proof\\\"}\"}}]',NULL,'native thought'); INSERT INTO messages VALUES(3,'fixture','tool','{\"stdout\":\"proof\",\"stderr\":\"\",\"exit_code\":0}',NULL,'shell',NULL);").unwrap();
        }else if name=="opencode2" {
            db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,parent_id TEXT); CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,time_created INTEGER,data TEXT);INSERT INTO session_v2 VALUES('fixture',NULL);").unwrap();
            let rows=[("u","user",json!({"text":"native prompt"})),("a","assistant",json!({"content":[{"type":"text","text":"answer"},{"type":"tool","id":"shell","name":"bash","state":{"input":{"command":"printf proof"},"content":{"stdout":"proof","stderr":"","exit_code":0}}}]}))];
            for (id,role,data) in rows {db.execute("INSERT INTO session_message VALUES(?, 'fixture', ?, 1, ?)",rusqlite::params![id,role,data.to_string()]).unwrap();}
        }else {
            db.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT);CREATE TABLE message(id TEXT PRIMARY KEY,session_id TEXT,time_created INTEGER,data TEXT);CREATE TABLE part(id TEXT PRIMARY KEY,session_id TEXT,message_id TEXT,time_created INTEGER,data TEXT);INSERT INTO session VALUES('parent',NULL),('fixture','parent');").unwrap();
            for (id,role) in [("u","user"),("a","assistant")] {db.execute("INSERT INTO message VALUES(?, 'fixture', 1, ?)",rusqlite::params![id,json!({"role":role,"modelID":"fixture-model","tokens":{"input":10,"output":5}}).to_string()]).unwrap();}
            let rows=[("prompt","u",json!({"type":"text","text":"native prompt"})),("reply","a",json!({"type":"text","text":"answer"})),("usage","a",json!({"type":"step-finish","tokens":{"input":10,"output":5,"cache":{"read":0,"write":0}},"cost":0.01})),("shell","a",json!({"type":"tool","callID":"shell","tool":"bash","state":{"status":"completed","input":{"command":"printf proof"},"output":"proof","metadata":{"exit":0}}}))];
            for (id,msg,data) in rows {db.execute("INSERT INTO part VALUES(?, 'fixture', ?, 1, ?)",rusqlite::params![id,msg,data.to_string()]).unwrap();}
        }
        let native=|suffix:&str|fs::read(format!("{}{suffix}",file.display())).ok();let before=[native(""),native("-wal"),native("-shm")];
        let rt=agent::runtime(name).unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();let source=sources.iter().find(|s|s.native_session=="fixture").unwrap();
        if matches!(name,"opencode"|"zcode"){assert_eq!(source.parent.as_deref(),Some("parent"));}
        let mut session=rt.history(&source.id,Some(&root),None,None).unwrap();let page=session.steps(u64::MAX).unwrap();
        assert!(page.steps.iter().any(|s|s.kind==StepKind::Message && s.author=="user" && s.data["text"]=="native prompt"),"{name}");
        let shell=page.steps.iter().find(|s|s.native_id.as_deref()==Some("shell")).unwrap();assert_eq!(shell.kind,StepKind::Shell,"{name}");assert_eq!(shell.data["command"],"printf proof","{name}");assert_eq!(shell.data["exit_code"],0,"{name}");
        let recovered=rt.history(&source.id,Some(&root),None,Some(&page.checkpoint)).unwrap();let live=recovered.live_data(None).unwrap();if name=="hermes"{assert!(live.calls.is_empty());assert!(live.cost_usd.is_none(),"estimated cost is not actual cost");assert!(page.steps.iter().any(|s|s.kind==StepKind::Usage && s.data["aggregate"]==true && s.turn.id.is_none()));}if matches!(name,"opencode"|"zcode"){assert_eq!(live.calls.len(),1,"message aggregate cannot recount step-finish: {name}");assert_eq!(live.cost_usd,Some(0.01));}
        assert_eq!([native(""),native("-wal"),native("-shm")],before,"{name}: native SQLite files changed");
        if name=="hermes"{db.execute("UPDATE session_model_usage SET actual_cost_usd=0.25",[]).unwrap();let live=rt.history(&source.id,Some(&root),None,None).unwrap().live_data(None).unwrap();assert_eq!(live.cost_usd,Some(0.25));assert!(live.calls.is_empty());}drop(db);fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn scrubbed_native_sql_projections_keep_real_record_shapes() {
    for (name,raw) in [("hermes",include_str!("fixtures/history/hermes-native.json")),("opencode",include_str!("fixtures/history/opencode-native.json"))] {
        for forbidden in ["/Users/","/home/","/private/","/var/folders/","C:\\Users\\","C:\\\\Users\\\\","Bearer "] {assert!(!raw.contains(forbidden),"private path or credential in {name}");}
        let rows:Value=serde_json::from_str(raw).unwrap();let root=root(&format!("native-{name}"));let file=root.join("projection.json");
        let native=rows["records"][0]["id"].as_str().unwrap();write(&file,&json!({"session_id":native,"messages":rows["records"]}),false);
        let mut history=agent::history::History::new(&format!("history-{name}"),"fixture",native).unwrap();let page=history.steps(&file,u64::MAX).unwrap();
        assert!(page.steps.iter().any(|s|s.kind==StepKind::Shell),"{name}");assert!(page.steps.iter().any(|s|s.kind==StepKind::Message),"{name}");
        assert!(page.steps.iter().all(|s|s.source["record"].is_object()));fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn source_specific_shells_and_partial_unknown_records_keep_truth() {
    for (name,rows) in [
        ("grok",json!([{"type":"backend_tool_call","id":"shell","kind":{"tool_type":"bash","command":"printf proof"}},{"type":"tool_result","tool_call_id":"shell","content":{"stdout":"proof","exit_code":0}}])),
        ("openclaw",json!([{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"shell","name":"exec","arguments":{"command":"printf proof"}}]}},{"type":"message","message":{"role":"toolResult","toolCallId":"shell","content":"proof"}}])),
    ] {
        let root=root(&format!("shell-{name}"));let file=root.join("fixture.jsonl");let mut rows=rows;rows.as_array_mut().unwrap().push(json!({"type":"api-request-blob","message":{"role":"assistant","content":[{"type":"tool_use","id":"shell","name":"Bash","input":{"command":"old replay"}}]}}));write(&file,&rows,true);
        let mut h=agent::history::History::new(&format!("history-{name}"),"fixture","fixture").unwrap();let page=h.steps(&file,u64::MAX).unwrap();
        let shell=page.steps.iter().find(|s|s.native_id.as_deref()==Some("shell")).unwrap();assert_eq!(shell.kind,StepKind::Shell,"{name}");assert_eq!(shell.data["command"],"printf proof");assert_eq!(shell.state,"completed");fs::remove_dir_all(root).unwrap();
    }
    let root=root("unknown");let file=root.join("fixture.jsonl");let raw="{\"type\":\"future-native\",\"value\":42}\n{\"partial\":";fs::write(&file,raw).unwrap();
    let mut h=agent::history::History::new("history-pi","fixture","fixture").unwrap();let page=h.steps(&file,1).unwrap();assert_eq!(page.steps[0].kind,StepKind::Unknown);assert_eq!(page.steps[0].source["raw"],raw.lines().next().unwrap().to_owned()+"\n");assert!(page.pending_bytes>0);assert!(page.more);
    let before=h.checkpoint();fs::write(&file,"\n{\"type\":\"future-native\"}\nmalformed\n").unwrap();assert_eq!(h.steps(&file,u64::MAX).unwrap_err(),"history invalid JSON");assert_eq!(h.checkpoint(),before);
    fs::write(&file,"\n{\"type\":\"session\",\"id\":\"fixture\"}\n{\"type\":\"future-native\"}\n").unwrap();
    let mut session=agent::runtime("pi").unwrap().history("fixture",Some(&root),Some(&file),None).unwrap();assert!(session.steps(u64::MAX).unwrap().steps.iter().any(|s|s.kind==StepKind::Unknown));
    let before=session.checkpoint();fs::write(&file,"\n{\"type\":\"session\",\"id\":\"foreign\"}\n").unwrap();assert_eq!(session.steps(u64::MAX).unwrap_err(),"history native session differs");assert_eq!(session.checkpoint(),before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn xdg_home_and_discovery_use_the_same_client_root() {
    struct Restore(Option<std::ffi::OsString>);
    impl Drop for Restore {fn drop(&mut self){unsafe {if let Some(value)=&self.0 {std::env::set_var("XDG_DATA_HOME",value);}else{std::env::remove_var("XDG_DATA_HOME");}}}}
    let _restore=Restore(std::env::var_os("XDG_DATA_HOME"));let root=root("xdg");
    unsafe {std::env::set_var("XDG_DATA_HOME",&root);}
    for name in ["muse","opencode"] {assert_eq!(agent::runtime(name).unwrap().home(),Some(root.join(name)));}
    let fixtures:Value=serde_json::from_str(include_str!("fixtures/history/agents.json")).unwrap();let fixture=&fixtures["muse"];
    let rt=agent::runtime("muse").unwrap();let home=rt.home().unwrap();write(&home.join(fixture["path"].as_str().unwrap()),&fixture["rows"],true);
    let found=rt.history_sessions(None).unwrap();assert_eq!(found.len(),1);assert_eq!(found[0].session,fixture["session"]);
    assert_eq!(rt.history_sessions(Some(&home)).unwrap()[0].id,found[0].id);
    fs::remove_dir_all(root).unwrap();
}
