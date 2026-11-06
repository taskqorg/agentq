//! M12.22: transactional identity, WAL integrity, native companions and media projections.
use agentq_runtime::agent::{self,history::{History,StepKind,media::MediaKind}};
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
fn root(n:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-source-authority-{}-{n}",std::process::id()));let _=fs::remove_dir_all(&p);fs::create_dir_all(&p).unwrap();p}
fn rows(path:&Path,values:&[Value]) {fs::create_dir_all(path.parent().unwrap()).unwrap();fs::write(path,values.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();}
#[test]
fn every_session_bearing_record_is_validated_before_a_page_or_media_read() {
 let root=root("sessions");let file=root.join("source.jsonl");
 for foreign in [json!({"session_id":"B","role":"assistant","content":"foreign"}),json!({"data":{"sessionId":"B"}}),json!({"message":{"sessionID":"B","role":"assistant","content":"foreign"}})] {
  rows(&file,&[json!({"type":"session","id":"A"}),foreign]);let mut history=History::new("history-pi","source","A").unwrap();let before=history.checkpoint();
  assert_eq!(history.steps(&file,1).unwrap_err(),"history native session differs");assert_eq!(history.checkpoint(),before);
 }
 let file=root.join("sessions/session_A.json");fs::create_dir_all(file.parent().unwrap()).unwrap();fs::write(&file,json!({"id":"A","messages":[{"session_id":"B","role":"assistant","content":"foreign"}]}).to_string()).unwrap();
 let mut history=History::new("history-jcode","source","A").unwrap();assert_eq!(history.steps(&file,1).unwrap_err(),"history native session differs");fs::remove_dir_all(root).unwrap();
}
#[test]
fn identical_bytes_with_new_native_file_identity_reset_recovered_snapshot() {
 let root=root("replacement");let file=root.join("source.jsonl");rows(&file,&[json!({"type":"session","id":"A"}),json!({"role":"assistant","id":"reply","content":"one"})]);
 let mut history=History::new("history-pi","source","A").unwrap();let first=history.steps(&file,u64::MAX).unwrap();let content=fs::read(&file).unwrap();
 assert!(first.checkpoint["identity"].is_string());assert!(first.steps[0].source["files"][0]["identity"].is_string());
 fs::rename(&file,root.join("retained.jsonl")).unwrap();fs::write(&file,content).unwrap();let mut history=History::recover(&first.checkpoint).unwrap();let next=history.steps(&file,u64::MAX).unwrap();
 assert!(next.reset);assert!(next.generation>first.generation);assert_ne!(next.checkpoint["identity"],first.checkpoint["identity"]);assert_eq!(next.steps.last().unwrap().id,first.steps.last().unwrap().id);fs::remove_dir_all(root).unwrap();
}
#[test]
fn cline_and_kiro_companions_are_bound_and_keep_model_parent_and_tool_identity() {
 let root=root("companions");let cline=root.join("cline");fs::create_dir_all(&cline).unwrap();
 fs::write(cline.join("S.json"),json!({"session_id":"S","model":"metadata-model"}).to_string()).unwrap();fs::write(cline.join("S.messages.json"),json!({"messages":[{"role":"user","content":"prompt"},{"role":"assistant","content":"answer"}]}).to_string()).unwrap();
 let rt=agent::runtime("cline").unwrap();let sources=rt.history_sessions(Some(&cline)).unwrap();assert_eq!(sources.len(),1);let mut session=rt.history(&sources[0].id,Some(&cline),None,None).unwrap();let page=session.steps(u64::MAX).unwrap();
 assert_eq!(page.steps[0].source["files"].as_array().unwrap().len(),2);assert!(page.steps.iter().filter(|s|s.kind==StepKind::Message).all(|s|s.presentation["model"]=="metadata-model"));
 let checkpoint=session.checkpoint();fs::write(cline.join("S.json"),json!({"session_id":"foreign","model":"metadata-model"}).to_string()).unwrap();assert!(session.steps(1).is_err());assert_eq!(session.checkpoint(),checkpoint);
 let kiro=root.join("kiro");let dir=kiro.join("sessions/cli");fs::create_dir_all(&dir).unwrap();let id="4243babe-c33c-4ca3-8245-689c9e34ba3b";
 fs::write(dir.join(format!("{id}.json")),json!({"session_id":id,"parent_session_id":"parent","session_state":{"rts_model_state":{"model_info":{"model_id":"kiro-model"}}}}).to_string()).unwrap();
 rows(&dir.join(format!("{id}.jsonl")),&[
  json!({"kind":"AssistantMessage","data":{"content":[{"kind":"thinking","data":{"text":"native thought"}},{"kind":"toolUse","data":{"toolUseId":"t1","name":"bash","input":{"command":"printf proof"}}}]}}),
  json!({"kind":"ToolResults","data":{"content":[{"kind":"toolResult","data":{"toolUseId":"t1","content":[{"kind":"text","data":"proof"}]}}]}})]);
 let rt=agent::runtime("kiro").unwrap();let sources=rt.history_sessions(Some(&kiro)).unwrap();assert_eq!(sources.len(),1);assert_eq!(sources[0].parent.as_deref(),Some("parent"));
 let page=rt.history(&sources[0].id,Some(&kiro),None,None).unwrap().steps(u64::MAX).unwrap();let shells:Vec<_>=page.steps.iter().filter(|s|s.native_id.as_deref()==Some("t1")).collect();assert_eq!(shells.len(),1);assert_eq!(shells[0].kind,StepKind::Shell);assert_eq!(shells[0].state,"completed");assert_eq!(shells[0].data["command"],"printf proof");assert!(page.steps.iter().any(|s|s.kind==StepKind::Thinking && s.data["text"]=="native thought"));assert!(page.steps.iter().all(|s|s.presentation["model"]=="kiro-model"));fs::remove_dir_all(root).unwrap();
}
#[test]
fn kimi_agents_have_native_layout_identity_and_state_lineage() {
 let root=root("kimi");let dir=root.join("sessions/workspace/session_S");
 for agent in ["main","child"] {rows(&dir.join(format!("agents/{agent}/wire.jsonl")),&[json!({"type":"context.append_message","session_id":"session_S","message":{"role":"user","content":agent},"time":1})]);}
 fs::write(dir.join("state.json"),json!({"agents":{"main":{"type":"main","parentAgentId":null,"homedir":"/foreign/ignored"},"child":{"type":"subagent","parentAgentId":"main"}}}).to_string()).unwrap();
 let rt=agent::runtime("kimi").unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();assert_eq!(sources.len(),2);let parent=sources.iter().find(|s|s.session=="session_S").unwrap();let child=sources.iter().find(|s|s.session!="session_S").unwrap();assert_eq!(child.parent.as_deref(),Some(parent.session.as_str()));
 for source in sources {let mut session=rt.history(&source.session,Some(&root),None,None).unwrap();let page=session.steps(u64::MAX).unwrap();assert_eq!(page.steps[0].source["files"].as_array().unwrap().len(),2);assert!(session.steps(1).unwrap().steps.is_empty());}
 fs::remove_dir_all(root).unwrap();
}
#[test]
fn corrupt_or_truncated_wal_never_silently_returns_before_state() {
 let root=root("wal");let live=root.join("live.db");let db=rusqlite::Connection::open(&live).unwrap();
 db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE sessions(id TEXT PRIMARY KEY);CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,content TEXT);INSERT INTO sessions VALUES('S');INSERT INTO messages VALUES(1,'S','assistant','before');PRAGMA wal_checkpoint(TRUNCATE);UPDATE messages SET content='after';").unwrap();
 let wal=fs::read(format!("{}-wal",live.display())).unwrap();let mut corrupt=wal.clone();corrupt[60]^=0x40;
 for (name,bytes) in [("valid",wal.clone()),("truncated",wal[..wal.len()-20].to_vec()),("corrupt",corrupt)] {
  let copy=root.join(format!("{name}.db"));fs::copy(&live,&copy).unwrap();fs::write(format!("{}-wal",copy.display()),bytes).unwrap();let mut history=History::new("history-hermes","source","S").unwrap();let before=history.checkpoint();
  let result=history.steps(&copy,u64::MAX);if name=="valid"{assert!(result.unwrap().steps.iter().any(|s|s.data["text"]=="after"));}else{assert!(result.unwrap_err().contains("WAL"));assert_eq!(history.checkpoint(),before);}
 }
 drop(db);fs::remove_dir_all(root).unwrap();
}
#[test]
fn opencode2_database_override_and_mime_appropriate_media_use_shared_readers() {
 struct Restore(Vec<(&'static str,Option<std::ffi::OsString>)>);impl Drop for Restore{fn drop(&mut self){for (key,value) in &self.0 {unsafe{if let Some(value)=value{std::env::set_var(key,value);}else{std::env::remove_var(key);}}}}}
 let root=root("opencode");let _restore=Restore(["XDG_DATA_HOME","OPENCODE_DB"].iter().map(|k|(*k,std::env::var_os(k))).collect());let dbpath=root.join("custom.db");let db=rusqlite::Connection::open(&dbpath).unwrap();db.execute_batch("CREATE TABLE session_v2(id TEXT PRIMARY KEY,parent_id TEXT);CREATE TABLE session_message(id TEXT PRIMARY KEY,session_id TEXT,type TEXT,time_created INTEGER,data TEXT);INSERT INTO session_v2 VALUES('S',NULL);").unwrap();drop(db);
 unsafe{std::env::set_var("XDG_DATA_HOME",root.join("empty-xdg"));std::env::set_var("OPENCODE_DB",&dbpath);}
 let found=agent::runtime("opencode2").unwrap().history_sessions(None).unwrap();assert_eq!(found.len(),1);assert_eq!(found[0].session,"S");
 let file=root.join("projection.json");let records=[("pdf","application/pdf","data:application/pdf;base64,JVBERi0=","document"),("audio","audio/wav","https://example.invalid/a","audio"),("video","video/mp4","https://example.invalid/v","video"),("image","image/png","data:image/png;base64,iVBORw0KGgo=","image")].map(|(id,mime,url,_)|json!({"id":id,"messageID":"m","type":"file","mime":mime,"url":url}));
 fs::write(&file,json!({"session_id":"S","messages":records}).to_string()).unwrap();let rt=agent::runtime("opencode").unwrap();let mut session=rt.history("S",Some(&root),Some(&file),None).unwrap();let page=session.steps(u64::MAX).unwrap();assert_eq!(page.steps.iter().filter(|s|s.kind==StepKind::Image).count(),1);assert_eq!(page.steps.iter().filter(|s|s.kind==StepKind::Media).count(),3);
 let media=session.media().unwrap();assert!(media.items.iter().any(|m|m.kind==Some(MediaKind::Document) && m.mime.as_deref()==Some("application/pdf")));assert_eq!(media.items.iter().filter(|m|m.kind==Some(MediaKind::Image)).count(),1);
 fs::write(&file,json!({"session_id":"S","messages":[{"role":"assistant","content":json!([{"type":"image","path":"/private/prose.png"}]).to_string()}]}).to_string()).unwrap();assert!(rt.history("S",Some(&root),Some(&file),None).unwrap().media().unwrap().items.is_empty());fs::remove_dir_all(root).unwrap();
}
#[test]
fn snapshot_media_keeps_each_physical_revision_without_decoding_prose() {
 let root=root("media-revisions");let file=root.join("source.jsonl");let image=root.join("image.png");fs::write(&image,b"\x89PNG\r\n\x1a\nrevision").unwrap();
 rows(&file,&[json!({"type":"session","id":"S"}),json!({"type":"tool_result","tool_call_id":"read","content":{"files":[{"path":image}]}}),json!({"type":"tool_result","tool_call_id":"read","content":"completed"})]);
 let session=agent::runtime("pi").unwrap().history("S",Some(&root),Some(&file),None).unwrap();let media=session.media().unwrap();let reference=media.items.iter().find(|m|m.state=="available").unwrap();assert_eq!(media.read(&reference.id,0,8).unwrap(),b"\x89PNG\r\n\x1a\n");assert!(reference.origin["raw"].as_str().unwrap().contains("files"));assert!(reference.origin["files"][0]["identity"].is_string());fs::remove_dir_all(root).unwrap();
}
#[test]
fn stable_wal_index_binds_committed_frames_and_excludes_previous_generation_tail() {
 let root=root("wal-restart");let live=root.join("live.db");let db=rusqlite::Connection::open(&live).unwrap();
 db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE sessions(id TEXT PRIMARY KEY);CREATE TABLE messages(id INTEGER PRIMARY KEY,session_id TEXT,role TEXT,content TEXT);CREATE TABLE filler(value BLOB);INSERT INTO filler VALUES(zeroblob(65536));INSERT INTO sessions VALUES('S');INSERT INTO messages VALUES(1,'S','assistant','before');PRAGMA wal_checkpoint(RESTART);UPDATE messages SET content='after';").unwrap();
 let wal=fs::read(format!("{}-wal",live.display())).unwrap();let index=fs::read(format!("{}-shm",live.display())).unwrap();let frame=u32::from_ne_bytes(index[16..20].try_into().unwrap()) as usize;let page=u32::from_be_bytes(wal[8..12].try_into().unwrap()) as usize;let end=32+frame*(page+24);assert!(end<wal.len(),"fixture must retain the previous generation tail");
 for name in ["valid","truncated","corrupt"] {
  let copy=root.join(format!("{name}.db"));fs::copy(&live,&copy).unwrap();let mut bytes=wal.clone();if name=="truncated"{bytes.truncate(end-20);}if name=="corrupt"{bytes[60]^=0x40;}
  fs::write(format!("{}-wal",copy.display()),bytes).unwrap();fs::write(format!("{}-shm",copy.display()),&index).unwrap();let mut history=History::new("history-hermes","source","S").unwrap();
  let before=history.checkpoint();let result=history.steps(&copy,u64::MAX);if name=="valid"{let page=result.unwrap();assert!(page.steps.iter().any(|s|s.data["text"]=="after"));assert_eq!(page.steps[0].source["files"].as_array().unwrap().len(),3);}else{assert!(result.unwrap_err().contains("WAL"));assert_eq!(history.checkpoint(),before);}
 }
 drop(db);fs::remove_dir_all(root).unwrap();
}
#[test]
fn copied_kimi_families_never_cross_bind_opaque_parent_and_child_sources() {
 let root=root("kimi-copies");
 for workspace in ["A","B"] {
  let dir=root.join(format!("sessions/{workspace}/session_S"));
  for agent in ["main","child"] {rows(&dir.join(format!("agents/{agent}/wire.jsonl")),&[json!({"type":"context.append_message","session_id":"session_S","message":{"role":"user","content":workspace},"time":1})]);}
  fs::write(dir.join("state.json"),json!({"agents":{"main":{"type":"main","parentAgentId":null},"child":{"type":"subagent","parentAgentId":"main"}}}).to_string()).unwrap();
 }
 let rt=agent::runtime("kimi").unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();assert_eq!(sources.len(),4);assert!(rt.history("session_S",Some(&root),None,None).is_err());
 for workspace in ["A","B"] {
  let dir=root.join(format!("sessions/{workspace}/session_S"));let parent=sources.iter().find(|s|s.path.starts_with(&dir) && s.parent.is_none()).unwrap();let child=sources.iter().find(|s|s.path.starts_with(&dir) && s.parent.is_some()).unwrap();
  let parent_session=rt.history(&parent.id,Some(&root),None,None).unwrap();let checkpoint=parent_session.checkpoint();assert_eq!(checkpoint["resolved"]["children"][&child.native_session],child.id);assert_eq!(checkpoint["resolved"]["children"].as_object().unwrap().len(),1);
  let recovered=rt.history(&parent.id,Some(&root),None,Some(&checkpoint)).unwrap();assert_eq!(recovered.checkpoint()["resolved"],checkpoint["resolved"]);
  let child_session=rt.history(&child.id,Some(&root),None,None).unwrap();let saved=child_session.checkpoint();assert_eq!(saved["resolved"]["parent"],parent.id);
  let mut recovered=rt.history(&child.id,Some(&root),None,Some(&saved)).unwrap();let page=recovered.steps(u64::MAX).unwrap();assert!(page.steps.iter().filter(|s|s.parent_session.is_some()).all(|s|s.parent_session.as_deref()==Some(parent.id.as_str())));assert!(page.steps.iter().any(|s|s.kind==StepKind::Message && s.parent_session.as_deref()==Some(parent.id.as_str())));
 }
 fs::remove_dir_all(root).unwrap();
}
#[test]
fn multiple_kimi_primary_agents_refuse_without_advancing_an_open_snapshot() {
 let root=root("kimi-primary");let dir=root.join("sessions/workspace/session_S");rows(&dir.join("agents/main/wire.jsonl"),&[json!({"type":"context.append_message","message":{"role":"user","content":"prompt"},"time":1})]);
 let state=dir.join("state.json");fs::write(&state,json!({"agents":{"main":{"type":"main","parentAgentId":null}}}).to_string()).unwrap();let rt=agent::runtime("kimi").unwrap();let mut session=rt.history("session_S",Some(&root),None,None).unwrap();session.steps(u64::MAX).unwrap();let checkpoint=session.checkpoint();
 rows(&dir.join("agents/other/wire.jsonl"),&[json!({"type":"context.append_message","message":{"role":"user","content":"other"},"time":1})]);
 fs::write(&state,json!({"agents":{"main":{"type":"main","parentAgentId":null},"other":{"type":"main","parentAgentId":null}}}).to_string()).unwrap();
 assert!(rt.history_sessions(Some(&root)).unwrap_err().contains("primary agent ambiguous"));assert!(session.steps(1).unwrap_err().contains("primary agent ambiguous"));assert_eq!(session.checkpoint(),checkpoint);fs::remove_dir_all(root).unwrap();
}
#[test]
fn an_unproven_duplicate_parent_family_refuses_instead_of_choosing_first() {
 let root=root("parent-ambiguity");let dir=root.join("sessions");fs::create_dir_all(&dir).unwrap();
 for (file,value) in [("session_parent_A.json",json!({"id":"P","messages":[]})),("session_parent_B.json",json!({"id":"P","messages":[]})),("session_child.json",json!({"id":"C","parent_session_id":"P","messages":[{"role":"user","content":"prompt"}]}))] {fs::write(dir.join(file),value.to_string()).unwrap();}
 let rt=agent::runtime("jcode").unwrap();let sources=rt.history_sessions(Some(&root)).unwrap();let child=sources.iter().find(|s|s.session=="C").unwrap();
 assert!(rt.history(&child.id,Some(&root),None,None).err().unwrap().contains("native family ambiguous"));fs::remove_dir_all(root).unwrap();
}
