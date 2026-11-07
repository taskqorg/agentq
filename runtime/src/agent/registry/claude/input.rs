//! Private expected input fingerprints; acceptance requires an actual matching native record.
use super::*;

#[derive(Clone,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputWitness {uuid:String,blocks:Vec<Block>}
#[derive(Clone,PartialEq,serde::Serialize,serde::Deserialize)]
#[serde(tag="type",rename_all="snake_case",deny_unknown_fields)]
enum Block {Text {sha256:String},Image {media_type:String,sha256:String}}

fn blocks(content:&Value)->Res<Vec<Block>> {
    let values=if let Some(text)=content.as_str() {vec![json!({"type":"text","text":text})]}else{content.as_array().filter(|b|!b.is_empty()).cloned().ok_or("native Claude input content unknown")?};
    values.iter().map(|block|match block["type"].as_str() {
        Some("text")=>Ok(Block::Text {sha256:crate::sha256_hex(block["text"].as_str().ok_or("native Claude input text unknown")?.as_bytes())}),
        Some("image") if block["source"]["type"]=="base64"=>{
            let media_type=block["source"]["media_type"].as_str().filter(|m|matches!(*m,"image/png"|"image/jpeg"|"image/gif"|"image/webp")).ok_or("native Claude image type unknown")?;
            let data=block["source"]["data"].as_str().ok_or("native Claude image data unknown")?;
            let bytes=crate::agent::history::media::base64(data)?;
            Ok(Block::Image {media_type:media_type.into(),sha256:crate::sha256_hex(&bytes)})
        },
        _=>Err("native Claude input block unmapped".into()),
    }).collect()
}
impl InputWitness {
    pub(crate) fn request(message:&Value)->Res<Self> {
        if message["type"]!="user" || message["message"]["role"]!="user" {return Err("native Claude input envelope unknown".into());}
        let witness=Self {uuid:message["uuid"].as_str().ok_or("native Claude input UUID missing")?.into(),blocks:blocks(&message["message"]["content"])?};
        witness.validate()?;Ok(witness)
    }
    pub(crate) fn validate(&self)->Res<()> {
        if self.uuid.len()!=36 || !self.uuid.bytes().enumerate().all(|(i,b)|if matches!(i,8|13|18|23){b==b'-'}else{b.is_ascii_hexdigit()}) || self.blocks.is_empty() {return Err("native Claude input witness invalid".into());}
        for block in &self.blocks {
            let (hash,mime)=match block {Block::Text{sha256}=>(sha256,None),Block::Image{media_type,sha256}=>(sha256,Some(media_type.as_str()))};
            if hash.len()!=64 || !hash.bytes().all(|b|b.is_ascii_hexdigit()) || mime.is_some_and(|m|!matches!(m,"image/png"|"image/jpeg"|"image/gif"|"image/webp")) {return Err("native Claude input witness invalid".into());}
        }Ok(())
    }
    pub(crate) fn matches(&self,row:&Value)->Res<bool> {
        if row["uuid"]!=self.uuid {return Ok(false);}
        if row["type"]!="user" || row["message"]["role"]!="user" || row["isMeta"]==true || row["isSidechain"]!=false || row["isCompactSummary"]==true {return Err("native Claude accepted input scope unknown".into());}
        if blocks(&row["message"]["content"])?!=self.blocks {return Err("native Claude accepted input differs".into());}Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multipart_serializer_bytes_need_matching_native_uuid_content_and_incarnation() {
        let root=crate::scratch("claude-multipart-input");let image=root.join("image.png");
        let png="iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";
        std::fs::write(&image,crate::agent::history::media::base64(png).unwrap()).unwrap();
        let parts=[Part::Text("before".into()),Part::Image(image.clone()),Part::Text("after".into())];
        let mut sent:Value=serde_json::from_str(&stream_message(&parts).unwrap()).unwrap();sent["uuid"]=json!("00000000-0000-4000-8000-000000000001");
        let expected=InputWitness::request(&sent).unwrap();
        let mut actual=sent.clone();actual["sessionId"]=json!("00000000-0000-4000-8000-000000000099");actual["isSidechain"]=json!(false);actual["parentUuid"]=Value::Null;actual["promptId"]=sent["uuid"].clone();
        // Acceptance matches the bytes sent once, never a later reread of the user's image file.
        std::fs::write(&image,b"changed after serialization").unwrap();assert!(expected.matches(&actual).unwrap());
        let spec=Spec {client:Client::Claude,root:root.clone(),session:actual["sessionId"].as_str().unwrap().into(),resume:false,args:vec![],env:vec![],home:Some(root.clone())};
        let path=root.join("native.jsonl");let mut turn=native::NativeTurn::before(&spec,&path,"").unwrap();turn.claude_input=Some(expected.clone());assert!(!turn.accepted);assert!(turn.poll().is_err());assert!(!turn.accepted);
        let assistant=json!({"type":"assistant","sessionId":spec.session,"isSidechain":false,"uuid":"00000000-0000-4000-8000-000000000002","parentUuid":sent["uuid"],"promptId":sent["uuid"],"message":{"role":"assistant","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn"}});
        let end=json!({"type":"system","subtype":"turn_duration","sessionId":spec.session,"durationMs":1});
        std::fs::write(&path,format!("{actual}\n{assistant}\n{end}\n")).unwrap();assert!(turn.poll().unwrap().is_some());assert!(turn.accepted);assert!(turn.checkpoint()["identity"].is_string());
        let saved=turn.checkpoint();let recovered=native::NativeTurn::recover(&spec,&saved).unwrap();assert!(recovered.claude_input.as_ref().unwrap().matches(&actual).unwrap());assert!(!saved["claude_input"].to_string().contains(png));
        let mut foreign=actual.clone();foreign["uuid"]=json!("00000000-0000-4000-8000-000000000003");assert!(!expected.matches(&foreign).unwrap());
        for bad in [json!({"type":"text","text":"changed"}),json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"invalid"}})] {let mut changed=actual.clone();changed["message"]["content"][1]=bad;assert!(expected.matches(&changed).is_err());}
        let mut reordered=actual.clone();reordered["message"]["content"].as_array_mut().unwrap().swap(0,2);assert!(expected.matches(&reordered).is_err());
        let mut sidechain=actual.clone();sidechain["isSidechain"]=json!(true);assert!(expected.matches(&sidechain).is_err());
        let mut duplicate=turn.clone();assert!(super::super::native_record(&mut duplicate,&actual).is_err());
        let mut foreign=turn.clone();let mut row=actual.clone();row["sessionId"]=json!("foreign");assert!(super::super::native_record(&mut foreign,&row).is_err());
        std::fs::write(&path,b"rewritten\n").unwrap();assert!(turn.poll().is_err());std::fs::remove_dir_all(root).unwrap();
    }
}
