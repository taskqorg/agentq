//! Derived steps, never a second source of truth. Decoder shapes adapted from Orca (see NOTICE).
use super::*;
use serde::{Serialize, Deserialize};
use std::io::{BufRead, BufReader};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind { Message, Thinking, Shell, FileChange, Tool, Mcp, Web, Subagent, Image, Question, Approval, Compaction, Usage, Stop, Unknown }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn { pub id: Option<String>, pub session: String, pub unknown: Option<String> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub native_id: Option<String>,
    pub revision: u64,
    pub kind: StepKind,
    pub turn: Turn,
    pub parent: Option<String>,
    pub parent_session: Option<String>,
    pub author: String,
    pub state: String,
    pub data: Value,
    pub unknown: Option<String>,
    /// Exact physical record, including raw bytes, range, hash, native time and source binding.
    pub source: Value,
    /// All contributing physical records, accessible through source byte-range reads.
    pub origins: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct StepPage { pub steps: Vec<Step>, pub more: bool, pub pending_bytes: u64, pub checkpoint: Value }

impl History {
    /// Caller-selected source window. A record may cross the window; it is never cut or discarded.
    pub fn steps(&mut self, path: &Path, bytes: u64) -> Res<StepPage> {
        self.steps_from(path,bytes,0)
    }

    pub(super) fn steps_from(&mut self,path:&Path,bytes:u64,start:u64)->Res<StepPage> {
        if bytes == 0 { return Err("history bytes must be positive".into()); }
        let (mut file, identity) = self.open(path)?;
        file.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(&mut file);
        let mut consumed = start;
        let bound=self.offset.saturating_add(bytes);
        let mut derived=BTreeMap::<String,Step>::new();
        let mut changed=BTreeMap::<String,Step>::new();
        while consumed < bound {
            let mut row = Vec::new();
            if reader.read_until(b'\n', &mut row).map_err(|e|e.to_string())? == 0 { break; }
            if !row.ends_with(b"\n") { break; }
            let raw = std::str::from_utf8(&row).map_err(|_| "history invalid UTF-8")?;
            let value: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !value.is_object() { return Err("history invalid record".into()); }
            let event = self.event(&value, raw, consumed)?;
            for mut step in decode(&self.format, &event, &value) {
                if let Some(previous)=derived.get(&step.id) {step=revise(previous,step);}
                derived.insert(step.id.clone(),step.clone());
                if consumed>=self.offset {changed.insert(step.id.clone(),step);}
            }
            consumed += row.len() as u64;
        }
        drop(reader);
        let end = consumed;
        let prefix = native::digest_prefix(&mut file, end)?;
        let pending = file.metadata().map_err(|e|e.to_string())?.len().saturating_sub(end);
        self.offset = end; self.prefix = prefix; self.identity = Some(identity);
        let mut steps:Vec<_>=changed.into_values().collect();steps.sort_by_key(|s|s.revision);
        Ok(StepPage {steps, more:pending > 0, pending_bytes:pending, checkpoint:self.checkpoint()})
    }

    /// Exact bytes; no text decoding, clipping, timeout or extra storage.
    pub fn read_bytes(&self, path: &Path, start: u64, length: u64) -> Res<Vec<u8>> {
        let (mut file, _) = self.open(path)?;
        let end = start.checked_add(length).ok_or("history range overflow")?;
        if end > file.metadata().map_err(|e|e.to_string())?.len() { return Err("history range unavailable".into()); }
        file.seek(SeekFrom::Start(start)).map_err(|e|e.to_string())?;
        let mut bytes = Vec::new();
        file.take(length).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len() as u64 != length { return Err("history range truncated".into()); }
        Ok(bytes)
    }
}

fn merge(old:&mut Value,new:&Value) {
    if let (Some(old),Some(new))=(old.as_object_mut(),new.as_object()) {
        for (key,value) in new {if !value.is_null() {
            if key=="unknown" {merge(old.entry(key).or_insert(Value::Null),value);}else{old.insert(key.clone(),value.clone());}
        }}
    }else if !new.is_null(){*old=new.clone();}
}

fn revise(previous:&Step,mut next:Step)->Step {
    if matches!(next.kind,StepKind::Tool) && !matches!(previous.kind,StepKind::Tool|StepKind::Unknown) {next.kind=previous.kind;}
    let mut data=previous.data.clone();
    if next.state=="delta" {
        let output=format!("{}{}",data["output"].as_str().unwrap_or_default(),next.data["delta"].as_str().unwrap_or_default());
        next.data["output"]=json!(output);next.data["output_mode"]=json!("cumulative");
    }
    merge(&mut data,&next.data);
    if let Some(object)=data.as_object_mut() {
        let known:Vec<_>=object.iter().filter(|(k,v)|k.as_str()!="unknown" && !v.is_null()).map(|(k,_)|k.clone()).collect();
        if let Some(reasons)=object.get_mut("unknown").and_then(Value::as_object_mut) {for key in known {reasons.remove(&key);}}
    }
    next.data=data;
    if previous.state=="completed" && next.state!="completed" {next.state=previous.state.clone();}
    next.origins=previous.origins.clone();next.origins.extend(origin(&next.source));
    if next.parent.is_none(){next.parent=previous.parent.clone();}
    if next.turn.id.is_none(){next.turn=previous.turn.clone();}
    next
}

fn origin(event:&Value)->Vec<Value> {vec![json!({"source":event["source"],"start":event["start"],"end":event["end"],"sha256":event["sha256"]})]}

fn first(v: &Value, names: &[&str]) -> Value {
    names.iter().find_map(|k| v.get(*k).filter(|v| !v.is_null())).cloned().unwrap_or(Value::Null)
}
fn string(v: &Value) -> Option<String> {v.as_str().filter(|s| !s.is_empty()).map(String::from)}

fn step(event: &Value, record: &Value, item: &Value, kind: StepKind, data: Value, index: usize) -> Step {
    let native_id = first(item, &["tool_use_id", "call_id", "id", "response_id"]);
    let id = if native_id.is_null() {format!("{}:{index}", event["id"].as_str().unwrap_or_default())}
        else {crate::sha256_hex(json!([event["format"], event["source"], event["session"], native_id]).to_string().as_bytes())};
    let p = record.get("payload").or(record.get("params")).unwrap_or(record);
    let turn = first(p, &["turn_id", "turnId"]);
    let parent = first(item, &["parent_call_id", "parent_tool_use_id", "parent_id", "parentId"]);
    let parent = if parent.is_null() {first(record, &["parent_tool_use_id", "parentToolUseId"])} else {parent};
    let state = if record["method"] == "item/commandExecution/outputDelta" {"delta".into()}
        else if matches!(item["type"].as_str(),Some("tool_use"|"function_call"|"custom_tool_call")) {"started".into()}
        else if matches!(item["type"].as_str(),Some("tool_result"|"function_call_output"|"custom_tool_call_output")) {"completed".into()}
        else {first(item, &["status", "state"]).as_str().map(String::from).unwrap_or_else(||
            if matches!(p["type"].as_str(),Some("item_completed")) || record["type"] == "item.completed" || record["method"] == "item/completed" || item["type"] == "tool_result" {"completed".into()} else {"unknown".into()})};
    let author=event["author"].as_str().unwrap_or("unknown");
    let author=if author=="unknown" {match kind {
        StepKind::Thinking=>"assistant",StepKind::Shell|StepKind::FileChange|StepKind::Mcp=>"tool",StepKind::Compaction|StepKind::Usage=>"system",_=>author,
    }}else{author};
    Step {id, native_id:string(&native_id), revision:event["start"].as_u64().unwrap_or(0),kind,
        turn:Turn {id:string(&turn),session:event["session"].as_str().unwrap_or_default().into(),unknown:if turn.is_null(){Some("native turn id not reported".into())}else{None}},
        parent:string(&parent),parent_session:None,author:author.into(),state,data,
        unknown:if kind == StepKind::Unknown {Some("unsupported source shape".into())}else{None},source:event.clone(),origins:origin(event)}
}

fn shell(item: &Value, record: &Value) -> Value {
    let result = record.get("toolUseResult").unwrap_or(item);
    let arguments = item["arguments"].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok());
    let input = item.get("input").or(arguments.as_ref()).unwrap_or(item);
    let mut data = json!({
        "command":first(input,&["command","cmd"]), "cwd":first(input,&["cwd"]),
        "exit_code":first(result,&["exit_code","exitCode"]), "duration":first(result,&["duration","duration_ms","durationMs"]),
        "output":first(result,&["aggregated_output","aggregatedOutput","output","content"]),
        "stdout":result["stdout"], "stderr":result["stderr"], "interrupted":result["interrupted"], "unknown":{}});
    if data["cwd"].is_null() {data["cwd"] = record["cwd"].clone();}
    for name in ["command","cwd","exit_code","duration","output","stdout","stderr","interrupted"] {
        if data[name].is_null() {data["unknown"][name] = json!("native field not reported");}
    }
    data
}

fn text(value:&Value)->Option<String> {
    if let Some(s)=value.as_str() {return Some(s.into());}
    value.as_array().map(|a|a.iter().filter_map(|v|v.as_str().or(v["text"].as_str()).or(v["thinking"].as_str())).collect::<Vec<_>>().join("\n"))
}

fn data(kind:StepKind, item:&Value, record:&Value)->Value {
    match kind {
        StepKind::Shell=>shell(item,record),
        StepKind::Message|StepKind::Thinking=> {
            let t=first(item,&["text","thinking","summary_text","summary","raw_content","content"]);
            let t=if t.is_null(){first(&record["message"],&["content"])}else{t};
            json!({"text":text(&t),"unknown":if text(&t).is_none(){json!({"text":"native text not reported"})}else{json!({})}})
        }
        StepKind::Tool|StepKind::Mcp|StepKind::Web|StepKind::Subagent|StepKind::Question|StepKind::Approval=> {
            let output=first(item,&["output","result","content"]);
            let result=record.get("toolUseResult").cloned().unwrap_or_else(||structured(&output));
            let session=first(item,&["agent_thread_id","thread_id","session_id","agent_id","agentId"]);
            let session=if session.is_null(){first(&result,&["agent_id","agentId","thread_id","session_id"])}else{session};
            let mut data=json!({"name":first(item,&["name","tool"]),"server":item["server"],
                "input":first(item,&["input","arguments","parameters"]),
                "output":output,"session":session,"activity":item["kind"],"native":item});
            let outputs=output.as_array().cloned().unwrap_or_else(||vec![output.clone()]);
            let console:Vec<_>=outputs.iter().filter_map(|v| {
                let v=if let Some(text)=v["text"].as_str(){serde_json::from_str::<Value>(text).ok()?}else{structured(v)};
                ["command","exit_code","exitCode","stdout","stderr","output"].iter().any(|k|v.get(*k).is_some()).then(||json!({
                    "command":v["command"],"exit_code":first(&v,&["exit_code","exitCode"]),"stdout":v["stdout"],"stderr":v["stderr"],"output":v["output"]}))
            }).collect();
            if !console.is_empty(){data["console"]=json!(console);}
            if item["name"]=="exec" {
                data["code"]=item["input"].clone();
                data["command"]=Value::Null;data["unknown"]=json!({"command":"dynamic code-mode JavaScript; no native shell command reported"});
            }
            data
        }
        StepKind::FileChange=> {
            let result=record.get("toolUseResult").unwrap_or(item);
            let changes=item.get("changes").cloned().unwrap_or_else(||json!([{
                "path":first(result,&["filePath","file_path","path"]),
                "patch":first(result,&["structuredPatch","diff","patch"])}]));
            json!({"changes":changes,"stdout":result["stdout"],"stderr":result["stderr"]})
        }
        StepKind::Usage if record["type"]=="token_usage_record" => json!({"request_id":item["response_id"],"usage":item["usage"],"turn_total":item["turn_token_usage"],"session_total":item["thread_token_usage"],"source":"native-request"}),
        _=>item.clone(),
    }
}

pub(super) fn structured(value:&Value)->Value {
    if let Some(text)=value.as_str() {return serde_json::from_str(text).unwrap_or(Value::Null);}
    if let Some(blocks)=value.as_array() {
        for block in blocks {
            if let Some(text)=block["text"].as_str() {
                if let Ok(value)=serde_json::from_str::<Value>(text) {if value.is_object(){return value;}}
            }
        }
    }
    value.clone()
}

fn tool_kind(name: &str) -> StepKind {
    match name {
        "Bash"|"bash"|"shell"|"exec_command"|"write_stdin"|"run_command"|"terminal" => StepKind::Shell,
        "Edit"|"Write"|"MultiEdit"|"apply_patch" => StepKind::FileChange,
        "Task"|"Agent"|"spawn_agent"|"wait_agent"|"send_input"|"resume_agent" => StepKind::Subagent,
        "AskUserQuestion"|"request_user_input" => StepKind::Question,
        "WebSearch"|"WebFetch"|"web_search"|"web_fetch" => StepKind::Web,
        n if n.starts_with("mcp__") => StepKind::Mcp,
        _ => StepKind::Tool,
    }
}

pub(super) fn decode(format: &str, event: &Value, record: &Value) -> Vec<Step> {
    let payload = if format == "codex-rollout" {&record["payload"]} else if format == "codex-app-server" {&record["params"]} else {record};
    let item = payload.get("item").unwrap_or(payload);
    if record["method"] == "item/commandExecution/outputDelta" {
        let item = json!({"id":payload["itemId"],"status":"delta"});
        return vec![step(event,record,&item,StepKind::Shell,json!({"delta":payload["delta"],"output_mode":"delta","unknown":{"command":"not reported by delta"}}),0)];
    }
    let mut out = Vec::new();
    let content = if format.starts_with("claude-") && matches!(record["type"].as_str(),Some("user"|"assistant")) {&record["message"]["content"]}
        else if format.starts_with("claude-"){&Value::Null}else{&item["content"]};
    if let Some(blocks) = content.as_array() {
        for (index, block) in blocks.iter().enumerate() {
            let ty = block["type"].as_str().unwrap_or_default();
            let kind = match ty {
                "text"|"Text"|"input_text"|"output_text" => StepKind::Message,
                "thinking"|"redacted_thinking"|"reasoning" => StepKind::Thinking,
                "image"|"Image"|"input_image"|"image_url"|"local_image"|"LocalImage" => StepKind::Image,
                "tool_use" => tool_kind(block["name"].as_str().unwrap_or_default()),
                "tool_result" if record["toolUseResult"].get("stdout").is_some() || record["toolUseResult"].get("stderr").is_some() => StepKind::Shell,
                "tool_result" if record["toolUseResult"].get("structuredPatch").is_some() => StepKind::FileChange,
                "tool_result" => StepKind::Tool,
                _ => StepKind::Unknown,
            };
            let data = data(kind,block,record);
            let mut identity = block.clone();
            if first(&identity,&["tool_use_id","call_id","id"]).is_null() {
                if let Some(id)=item.get("id").or(record.get("uuid")) {if !identity.is_object(){identity=json!({});}identity["id"] = json!(format!("{}:{index}",id.as_str().unwrap_or_default()));}
            }
            out.push(step(event,record,&identity,kind,data,index));
        }
    }
    if !out.is_empty() {return out;}
    let ty = item["type"].as_str().unwrap_or_default();
    let kind = match ty {
        "CommandExecution"|"command_execution"|"commandExecution"|"local_shell_call" => StepKind::Shell,
        "Reasoning"|"reasoning"|"thinking" => StepKind::Thinking,
        "FileChange"|"file_change"|"fileChange" => StepKind::FileChange,
        "McpToolCall"|"mcp_tool_call"|"mcpToolCall" => StepKind::Mcp,
        "WebSearch"|"web_search_call"|"webSearch" => StepKind::Web,
        "CollabAgentToolCall"|"collabAgentToolCall" => StepKind::Subagent,
        "SubAgentActivity" => StepKind::Subagent,
        "ImageView"|"image_view"|"imageGeneration" => StepKind::Image,
        "ContextCompaction"|"context_compacted"|"compaction" => StepKind::Compaction,
        "token_count" => StepKind::Usage,
        _ if record["type"]=="token_usage_record" => StepKind::Usage,
        "function_call"|"custom_tool_call"|"tool_use" => tool_kind(item["name"].as_str().unwrap_or_default()),
        "function_call_output"|"custom_tool_call_output"|"tool_result" => StepKind::Tool,
        "user"|"assistant"|"message"|"AgentMessage"|"agent_message"|"agentMessage"|"user_message"|"UserMessage"|"userMessage" => StepKind::Message,
        _ if record["type"] == "control_request" || record["type"] == "control_response" => StepKind::Approval,
        _ if record["subtype"] == "compact_boundary" => StepKind::Compaction,
        _ if record["type"] == "attachment" => StepKind::Tool,
        _ if format == "antigravity-sidecar" => match ty {
            "PLANNER_RESPONSE"|"USER_INPUT" => StepKind::Message,
            "TOOL_RESPONSE" => StepKind::Tool,
            _ => StepKind::Unknown,
        },
        _ if format == "agy-stream" => match record["step_update"]["step_type"].as_str() {
            Some("agent_response"|"user_input") => StepKind::Message,
            Some("tool") => StepKind::Tool,
            _ => StepKind::Unknown,
        },
        _ => StepKind::Unknown,
    };
    let data = data(kind,item,record);
    vec![step(event,record,item,kind,data,0)]
}
