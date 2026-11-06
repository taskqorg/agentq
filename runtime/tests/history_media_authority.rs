//! #291: native source authority and physical media revisions.
use agentq_runtime::agent;
use serde_json::{json,Value};
use std::{fs,path::{Path,PathBuf}};
fn root(n:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-media-authority-{}-{n}",std::process::id()));fs::create_dir_all(&p).unwrap();p}
fn session(root:&Path,rows:&[Value])->agent::history::sessions::Session {
 let p=root.join("sessions/session.jsonl");fs::create_dir_all(p.parent().unwrap()).unwrap();
 let text=std::iter::once(json!({"type":"session_meta","payload":{"id":"session"}})).chain(rows.iter().cloned()).map(|v|format!("{v}\n")).collect::<String>();
 fs::write(&p,text).unwrap();agent::runtime("codex").unwrap().history("session",Some(root),None,None).unwrap()
}
#[test]
fn assistant_json_prose_does_not_authorize_file_bytes() {
 let r=root("prose");let secret=r.join("private.pdf");fs::write(&secret,b"%PDF-private confidential content").unwrap();
 let text=json!({"path":secret}).to_string();
 let s=session(&r,&[json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}})]);
 let media=s.media().unwrap();
 println!("PROSE_ITEMS {}",serde_json::to_string(&media.items).unwrap());
 if let Some(item)=media.items.first(){println!("PROSE_READ {:?}",media.read(&item.id,0,item.size.unwrap()));}
 assert!(media.items.is_empty(),"assistant prose is not native file authority");
}
#[cfg(unix)]
#[test]
fn generated_asset_root_symlink_does_not_authorize_foreign_session() {
 let r=root("symlink");let other=r.join("generated_images/other-session");fs::create_dir_all(&other).unwrap();
 fs::write(other.join("foreign.png"),b"\x89PNG\r\n\x1a\nforeign").unwrap();
 std::os::unix::fs::symlink(&other,r.join("generated_images/session")).unwrap();
 let media=session(&r,&[]).media().unwrap();
 println!("SYMLINK_ITEMS {}",serde_json::to_string(&media.items).unwrap());
 assert!(media.items.is_empty(),"a symlink must not import another session's assets");
}
#[test]
fn media_reference_from_earlier_item_revision_survives_completion() {
 let r=root("revision");let image=r.join("view.png");fs::write(&image,b"\x89PNG\r\n\x1a\nimage").unwrap();
 let media=session(&r,&[
 json!({"type":"event_msg","payload":{"type":"item_started","turn_id":"t","item":{"type":"ImageView","id":"view","path":image}}}),
 json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"t","item":{"type":"ImageView","id":"view","status":"completed"}}})
 ]).media().unwrap();
 println!("REVISION_ITEMS {}",serde_json::to_string(&media.items).unwrap());
 assert!(media.items.iter().any(|m|m.state=="available" && m.source["path"]==image.to_str().unwrap()),"the completed step must retain the input media reference");
}

#[cfg(unix)]
#[test]
fn generated_asset_ancestor_and_later_replacement_are_refused() {
 let r=root("ancestor");let other=root("other");let foreign=other.join("session");fs::create_dir_all(&foreign).unwrap();
 fs::write(foreign.join("foreign.png"),b"\x89PNG\r\n\x1a\nforeign").unwrap();
 std::os::unix::fs::symlink(&other,r.join("generated_images")).unwrap();
 assert!(session(&r,&[]).media().unwrap().items.is_empty());
 fs::remove_file(r.join("generated_images")).unwrap();
 let selected=r.join("generated_images/session");fs::create_dir_all(&selected).unwrap();
 fs::write(selected.join("own.png"),b"\x89PNG\r\n\x1a\nown").unwrap();
 let media=session(&r,&[]).media().unwrap();let own=media.items.iter().find(|m|m.name.as_deref()==Some("own.png")).unwrap();
 fs::rename(&selected,r.join("retained")).unwrap();std::os::unix::fs::symlink(&foreign,&selected).unwrap();
 assert!(media.read(&own.id,0,1).unwrap_err().contains("symlink"));
 fs::remove_dir_all(r).unwrap();fs::remove_dir_all(other).unwrap();
}
#[test]
fn arbitrary_subtrees_and_json_message_content_are_not_file_authority() {
 let r=root("subtrees");let file=r.join("private.pdf");fs::write(&file,b"%PDF-private").unwrap();
 let s=session(&r,&[
  json!({"type":"response_item","payload":{"type":"message","role":"user","content":json!({"path":file}).to_string()}}),
  json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","arbitrary":{"type":"image","path":file},"item":{"type":"image","path":file},"mime":"image/png","path":file}]}}),
  json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"t","output":json!({"arbitrary":{"path":file}}).to_string()}})
 ]);
 assert!(s.media().unwrap().items.is_empty());fs::remove_dir_all(r).unwrap();
}
