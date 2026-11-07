//! Content-free native schema evidence; native decoding stays inside Runtime.
use serde_json::Value;

const TYPES: &[&str] = &["result","message","usage","session_meta","turn_context","response_item","event_msg","thread.started","turn.started","turn.completed","turn.failed","error","item.completed","agent_message","output_text","input_text","text","token_count","task_started","task_complete"];

#[derive(Clone,Debug,PartialEq,serde::Serialize)]
#[serde(untagged)]
enum ShapeKind {Known(String),Unknown {unknown_sha256:String},Absent}

/// Fixed names, hashed unknown schema names and nullable counts; never native content.
#[derive(Clone,Debug,PartialEq,serde::Serialize)]
pub struct RecordShape {
    kind:ShapeKind,
    unknown_keys:Vec<String>,
    unknown_types:Vec<String>,
    field_count:usize,
    input_tokens:Option<u64>,
    output_tokens:Option<u64>,
}
impl RecordShape {
    pub fn drift(&self)->bool {!self.unknown_keys.is_empty() || !self.unknown_types.is_empty() || matches!(self.kind,ShapeKind::Unknown {..})}
    pub fn known_kind(kind:&str)->bool {TYPES.contains(&kind)}
}

pub(super) fn read(text:&str)->Result<RecordShape,super::Cause> {
    serde_json::from_str::<Value>(text).map(|raw|shape(&raw)).map_err(|_|super::Cause("native-record-invalid"))
}

/// Fixed schema projection: arbitrary names/discriminators are opaque, values/content never survive.
fn shape(raw: &Value) -> RecordShape {
    let known = ["type", "usage", "input_tokens", "output_tokens", "cached_input_tokens", "status", "timestamp", "payload", "id", "model", "role", "content", "text", "thread_id", "item", "turn_id", "session_id", "cli_version", "version", "cwd", "forked_from_id"];
    let keys = raw.as_object().map(|o|o.keys().collect::<Vec<_>>()).unwrap_or_default();
    fn keys_only(v:&Value, known:&[&str], prefix:&str, out:&mut Vec<String>, types:&mut Vec<String>) {
        match v {
            Value::Object(o)=>for (k,v) in o {let path=if prefix.is_empty(){k.clone()}else{format!("{prefix}/{k}")};if !known.contains(&k.as_str()){out.push(crate::sha256_hex(path.as_bytes()));}if k=="type" {if let Some(kind)=v.as_str().filter(|s|!TYPES.contains(s)){types.push(crate::sha256_hex(format!("{path}:{kind}").as_bytes()));}}keys_only(v,known,&path,out,types);},
            Value::Array(a)=>for v in a {keys_only(v,known,prefix,out,types);},_=>{},
        }
    }
    let mut unknown=vec![];let mut unknown_types=vec![];keys_only(raw,&known,"",&mut unknown,&mut unknown_types);unknown.sort();unknown.dedup();unknown_types.sort();unknown_types.dedup();
    let kind=match raw["type"].as_str() {Some(s) if TYPES.contains(&s)=>ShapeKind::Known(s.into()),Some(s)=>ShapeKind::Unknown {unknown_sha256:crate::sha256_hex(s.as_bytes())},_=>ShapeKind::Absent};
    RecordShape {kind,unknown_keys:unknown,unknown_types,field_count:keys.len(),
        input_tokens:raw["usage"]["input_tokens"].as_u64(),output_tokens:raw["usage"]["output_tokens"].as_u64()}
}
