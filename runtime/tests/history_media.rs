//! #291: references authorize bytes, never names or assistant prose.
use agentq_runtime::agent::{self,Event,history::{StepKind,media::MediaKind}};
use serde_json::{Value,json};
use std::{fs,path::PathBuf};
fn root(name:&str)->PathBuf {let p=std::env::temp_dir().join(format!("agentq-media-{}-{name}",std::process::id()));fs::create_dir_all(&p).unwrap();p}
fn source(root:&std::path::Path,runtime:&str,rows:&[Value])->PathBuf {
    let p=if runtime=="claude" {root.join("projects/project/session.jsonl")}else{root.join("sessions/session.jsonl")};fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p,rows.iter().map(|v|format!("{v}\n")).collect::<String>()).unwrap();p
}

#[test]
fn inline_media_are_decoded_once_by_hash_and_nested_results_are_not_lost() {
    let root=root("inline");
    let image=json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}});
    let rows=[json!({"type":"user","sessionId":"session","message":{"content":[image.clone(),{"type":"document","source":{"type":"url","url":"https://example.invalid/doc.pdf"},"title":"external"}]}}),
        json!({"type":"user","sessionId":"session","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":[image.clone()]}]},"toolUseResult":{"type":"image","file":{"type":"image/png","base64":"iVBORw0KGgo="}}}),
        json!({"type":"user","sessionId":"session","message":{"content":[{"type":"image","source":{"type":"base64","data":"bad="}}]}})];
    source(&root,"claude",&rows);
    let session=agent::runtime("claude").unwrap().history("session",Some(&root),None,None).unwrap();let media=session.media().unwrap();
    let images:Vec<_>=media.items.iter().filter(|m|m.kind==Some(MediaKind::Image) && m.sha256.is_some()).collect();assert_eq!(images.len(),3,"{:?}",media.items);
    assert!(images.iter().all(|m|m.sha256==images[0].sha256));
    assert_eq!(media.read(images[0].sha256.as_ref().unwrap(),1,3).unwrap(),b"PNG");
    assert!(media.read(images[0].sha256.as_ref().unwrap(),7,2).is_err());assert!(media.read("/not/referenced",0,1).is_err());
    let document=media.items.iter().find(|m|m.kind==Some(MediaKind::Document)).unwrap();assert_eq!(document.state,"unavailable");assert!(document.unknown.contains_key("bytes"));
    assert!(media.items.iter().any(|m|m.unknown.get("bytes").is_some_and(|s|s.contains("base64"))));
    assert!(media.steps().iter().all(|s|s.kind==StepKind::Media && !s.origins.is_empty()));fs::remove_dir_all(root).unwrap();
}

#[test]
fn signatures_range_reads_and_changed_or_missing_files_are_explicit() {
    let root=root("files");let wav=root.join("audio.txt");let mp4=root.join("video.bin");let fake=root.join("fake.mp4");let image=root.join("generated.png");let missing=root.join("missing.png");
    fs::write(&wav,b"RIFF\0\0\0\0WAVEfmt payload").unwrap();fs::write(&mp4,b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();fs::write(&fake,b"ordinary text").unwrap();fs::write(&image,b"\x89PNG\r\n\x1a\npayload").unwrap();
    let rows=[json!({"type":"session_meta","payload":{"id":"session"}}),
        json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"media","output":json!({"files":[{"path":wav},{"path":mp4},{"path":fake}]}).to_string()}}),
        json!({"type":"event_msg","payload":{"type":"item_completed","item":{"id":"generated","type":"imageGeneration","savedPath":image}}}),
        json!({"type":"response_item","payload":{"type":"message","content":[{"type":"local_image","path":missing},{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgo="}]}})];
    let generated_dir=root.join("generated_images/session");fs::create_dir_all(&generated_dir).unwrap();fs::write(generated_dir.join("native-asset.bin"),b"\x89PNG\r\n\x1a\nasset").unwrap();
    let foreign=root.join("generated_images/other-session");fs::create_dir_all(&foreign).unwrap();fs::write(foreign.join("foreign.png"),b"\x89PNG\r\n\x1a\nforeign").unwrap();
    source(&root,"codex",&rows);let session=agent::runtime("codex").unwrap().history("session",Some(&root),None,None).unwrap();let media=session.media().unwrap();
    assert_eq!(media.items.iter().filter(|m|m.kind==Some(MediaKind::Audio)).count(),1);assert_eq!(media.items.iter().filter(|m|m.kind==Some(MediaKind::Video)).count(),1);
    assert!(!media.items.iter().any(|m|m.source["path"]==fake.to_str().unwrap()));
    assert!(media.items.iter().any(|m|m.name.as_deref()==Some("native-asset.bin")));assert!(!media.items.iter().any(|m|m.name.as_deref()==Some("foreign.png")));
    let generated=media.items.iter().find(|m|m.source["kind"]=="generated").unwrap();assert_eq!(generated.size,Some(15));
    assert_eq!(media.read(image.to_str().unwrap(),8,7).unwrap(),b"payload");
    fs::write(&image,b"\x89PNG\r\n\x1a\nchanged").unwrap();assert!(media.read(&generated.id,0,1).unwrap_err().contains("changed"));
    let absent=media.items.iter().find(|m|m.source["path"]==missing.to_str().unwrap()).unwrap();assert_eq!(absent.state,"missing");assert!(media.read(&absent.id,0,1).is_err());
    let log=json!({"method":"item/completed","params":{"item":{"id":"generated","type":"imageGeneration","savedPath":"/fixture/generated.png","status":"completed"}}}).to_string();
    assert!(agent::runtime("codex").unwrap().log_events(&log).iter().any(|e|matches!(e,Event::Image(agent::Image {generated:true,saved_path:Some(p),evidence:Some(_),..}) if p=="/fixture/generated.png")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn real_scrubbed_media_fixtures_pass_for_all_three_runtimes() {
    for (name,fixture) in [("claude",include_str!("fixtures/history/claude-media.jsonl")),("codex",include_str!("fixtures/history/codex-media.jsonl")),("antigravity",include_str!("fixtures/history/antigravity-media.jsonl"))] {
        for private in ["/Users/","/home/","/tmp/","/private/","C:\\Users\\","2026-"] {assert!(!fixture.contains(private),"{name}: unsanitized {private}");}
        let root=root(name);let image=root.join("pixel.png");let artifact=root.join("artifact.png");
        fs::write(&image,include_bytes!("fixtures/history/pixel.png")).unwrap();fs::write(&artifact,include_bytes!("fixtures/history/pixel.png")).unwrap();
        fn paths(value:&mut Value,image:&std::path::Path,artifact:&std::path::Path) {
            match value {
                Value::String(s) if s=="/fixture/pixel.png"=>*s=image.to_string_lossy().into_owned(),
                Value::String(s) if s=="\"/fixture/pixel.png\""=>*s=serde_json::to_string(image).unwrap(),
                Value::String(s) if s=="file:///fixture/artifact.png"=>*s=format!("file://{}{}",if cfg!(windows){"/"}else{""},artifact.to_string_lossy().replace('\\',"/")),
                Value::Array(a)=>for v in a {paths(v,image,artifact);},Value::Object(o)=>for v in o.values_mut(){paths(v,image,artifact);},_=>{}
            }
        }
        let rows:Vec<Value>=fixture.lines().map(|l|{let mut v=serde_json::from_str(l).unwrap();paths(&mut v,&image,&artifact);v}).collect();
        let fixture=rows.iter().map(|v|format!("{v}\n")).collect::<String>();
        let path=if name=="antigravity" {
            let p=root.join("brain/fixture-session/.system_generated/logs/transcript.jsonl");fs::create_dir_all(p.parent().unwrap()).unwrap();fs::write(&p,fixture).unwrap();p
        }else{source(&root,name,&rows)};
        let session=agent::runtime(name).unwrap().history("fixture-session",Some(&root),Some(&path),None).unwrap();let media=session.media().unwrap();
        assert!(!media.items.is_empty(),"{name}");
        assert!(media.items.iter().all(|m|m.kind==Some(MediaKind::Image) && m.state=="available"),"{name}");
        for item in &media.items {assert_eq!(media.read(&item.id,0,8).unwrap(),b"\x89PNG\r\n\x1a\n");}
        fs::remove_dir_all(root).unwrap();
    }
}
