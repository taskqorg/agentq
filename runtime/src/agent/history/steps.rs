//! Derived steps, never a second source of truth. Decoder shapes adapted from Orca (see NOTICE).
use super::*;
use serde::{Serialize, Deserialize};
use std::io::{BufRead, BufReader};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind { Message, Thinking, Shell, FileChange, Tool, Mcp, Web, Subagent, Image, Question, Approval, Compaction, Usage, Context, Windows, Plan, Todo, FileRollup, Hook, ReadTruncation, Diagnostics, Queue, Link, Metadata, Stop, Unknown }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn { pub id: Option<String>, pub native_id:Option<String>, pub session: String, pub unknown: Option<String> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    pub native_id: Option<String>,
    pub revision: u64,
    pub part:usize,
    pub kind: StepKind,
    pub turn: Turn,
    pub parent: Option<String>,
    pub parent_session: Option<String>,
    pub author: String,
    pub state: String,
    pub data: Value,
    #[serde(default)]
    pub presentation: Value,
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
        let mut rows=Vec::new();let mut events=Vec::new();
        while consumed < bound {
            let mut row = Vec::new();
            if reader.read_until(b'\n', &mut row).map_err(|e|e.to_string())? == 0 { break; }
            if !row.ends_with(b"\n") { break; }
            let raw = std::str::from_utf8(&row).map_err(|_| "history invalid UTF-8")?;
            if raw.trim().is_empty() {consumed+=row.len() as u64;continue;}
            let value: Value = serde_json::from_str(raw).map_err(|_| "history invalid JSON")?;
            if !value.is_object() { return Err("history invalid record".into()); }
            let event = self.event(&value, raw, consumed)?;
            rows.push(value);events.push(event);consumed+=row.len() as u64;
        }
        let mut usage=super::live::usage_steps(&self.format,&events,&rows,self.offset);
        let mut turn:Option<Turn>=None;let mut presentation=json!({});
        for (index,(value,event)) in rows.iter().zip(&events).enumerate() {
            let p=value.get("payload").or(value.get("params")).unwrap_or(&value);
            let prompt=value["promptId"].as_str().filter(|_|matches!(value["type"].as_str(),Some("user"|"assistant")));
            let native=p["turn_id"].as_str().or(p["turnId"].as_str()).or(prompt);
            let user=value["type"]=="user" && value["isMeta"]!=true && (value["message"]["content"].is_string()
                || value["message"]["content"].as_array().is_some_and(|a|a.iter().any(|b|b["type"]=="text") && !a.iter().any(|b|b["type"]=="tool_result")));
            let boundary=user || value["type"]=="turn.started" || value["type"]=="system" && value["subtype"]=="init"
                || value["event"]=="init" || value["type"]=="USER_INPUT";
            if native.is_some() || boundary {
                turn=Some(Turn {id:Some(native.map(String::from).unwrap_or_else(||format!("source:{}",event["id"].as_str().unwrap_or_default()))),native_id:native.map(String::from),session:self.session.clone(),
                    unknown:if native.is_none(){Some("native turn id not reported; grouping derived from the source boundary".into())}else{None}});
            }
            let facts=presentation_facts(&value);
            if !facts["model"].is_null() && facts["model"]!=presentation["model"] {presentation["effort"]=Value::Null;}
            for name in ["model","effort","cli_version"] {if !facts[name].is_null(){presentation[name]=facts[name].clone();}}
            for mut step in decode(&self.format, event, value).into_iter().filter(|s|s.kind!=StepKind::Usage).chain(usage.remove(&index).unwrap_or_default()) {
                if step.kind==StepKind::Usage {step.part=value["message"]["content"].as_array().map_or(0,Vec::len)+step.part;}
                for name in ["model","effort","cli_version"] {if step.presentation[name].is_null() && !presentation[name].is_null() {step.presentation[name]=presentation[name].clone();step.presentation["unknown"].as_object_mut().unwrap().remove(name);}}
                if step.turn.id.is_none() {if let Some(turn)=&turn {step.turn=turn.clone();}}
                if let Some(previous)=derived.get(&step.id) {step=revise(previous,step);}
                derived.insert(step.id.clone(),step.clone());
                if step.revision>=self.offset {changed.insert(step.id.clone(),step);}
            }
        }
        drop(reader);
        let end = consumed;
        let prefix = native::digest_prefix(&mut file, end)?;
        let pending = file.metadata().map_err(|e|e.to_string())?.len().saturating_sub(end);
        self.offset = end; self.prefix = prefix; self.identity = Some(identity);
        let mut steps:Vec<_>=changed.into_values().collect();steps.sort_by_key(|s|(s.revision,s.part));
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
    if next.kind!=StepKind::Usage {
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
    }
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

pub(super) fn base_step(event: &Value, record: &Value, item: &Value, kind: StepKind, data: Value, index: usize) -> Step {
    let native_id = first(item, &["tool_use_id", "call_id", "id", "response_id"]);
    let id = if native_id.is_null() {format!("{}:{index}", event["id"].as_str().unwrap_or_default())}
        else {crate::sha256_hex(json!([event["format"], event["source"], event["session"], native_id]).to_string().as_bytes())};
    let p = record.get("payload").or(record.get("params")).unwrap_or(record);
    let turn = first(p, &["turn_id", "turnId"]);
    let parent = first(item, &["parent_call_id", "parent_tool_use_id", "parent_id", "parentId"]);
    let parent = if parent.is_null() {first(record, &["parent_tool_use_id", "parentToolUseId"])} else {parent};
    let state = if record["method"] == "item/commandExecution/outputDelta" {"delta".into()}
        else if matches!(item["type"].as_str(),Some("tool_use"|"server_tool_use"|"function_call"|"custom_tool_call")) {"started".into()}
        else if matches!(item["type"].as_str(),Some("tool_result"|"function_call_output"|"custom_tool_call_output")) {"completed".into()}
        else {first(item, &["status", "state"]).as_str().map(String::from).unwrap_or_else(||
            if matches!(p["type"].as_str(),Some("item_completed")) || record["type"] == "item.completed" || record["method"] == "item/completed" || item["type"] == "tool_result" {"completed".into()} else {"unknown".into()})};
    let author=event["author"].as_str().unwrap_or("unknown");
    let author=if author=="unknown" {match kind {
        StepKind::Thinking=>"assistant",StepKind::Shell|StepKind::FileChange|StepKind::Mcp=>"tool",StepKind::Compaction|StepKind::Usage=>"system",_=>author,
    }}else{author};
    Step {id, native_id:string(&native_id), revision:event["start"].as_u64().unwrap_or(0),part:index,kind,
        turn:Turn {id:string(&turn),native_id:string(&turn),session:event["session"].as_str().unwrap_or_default().into(),unknown:if turn.is_null(){Some("native turn id not reported".into())}else{None}},
        parent:string(&parent),parent_session:None,author:author.into(),state,data,presentation:presentation_facts(record),
        unknown:if kind == StepKind::Unknown {Some("unsupported source shape".into())}else{None},source:event.clone(),origins:origin(event)}
}

fn presentation_facts(record:&Value)->Value {
    let p=record.get("payload").or(record.get("params")).unwrap_or(record);
    let mut facts=json!({"model":record["message"].get("model").or(p.get("model")),
        "effort":p.get("effort").or(p.get("reasoning_effort")).or(record.get("effort")).or(record.get("reasoningEffort")).or(record.get("reasoning_effort")),
        "cli_version":p.get("cli_version").or(record.get("claude_code_version")).or(record.get("version")),
        "sidechain":record["isSidechain"],"meta":record["isMeta"],"unknown":{}});
    for name in ["model","effort","cli_version","sidechain","meta"] {if facts[name].is_null(){facts["unknown"][name]=json!("native field not reported");}}
    facts
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
                "output":output,"session":session,"activity":item["kind"],"is_error":item["is_error"],"native":item});
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
            let input=item.get("input").unwrap_or(item);
            let changes=item.get("changes").cloned().unwrap_or_else(||json!([{
                "path":if first(result,&["filePath","file_path","path"]).is_null(){first(input,&["file_path","path"])}else{first(result,&["filePath","file_path","path"])},
                "before":input["old_string"],"after":first(input,&["new_string","content"]),
                "patch":first(result,&["structuredPatch","diff","patch"])}]));
            json!({"changes":changes,"stdout":result["stdout"],"stderr":result["stderr"],"is_error":item["is_error"]})
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
    if format=="jsonl-rpc" {
        let ty=record["type"].as_str().unwrap_or_default();
        if ty=="rpc-replay" {return vec![];}
        if ty=="message_end" {return decode("claude-native",event,&json!({"type":record["message"]["role"],"message":record["message"]}));}
        let mut item=record.clone();item["id"]=record["toolCallId"].clone();
        let kind=match ty {
            "tool_execution_start"|"tool_execution_update"|"tool_execution_end"=>tool_kind(record["toolName"].as_str().unwrap_or_default()),
            "auto_compaction_start"|"auto_compaction_end"=>StepKind::Compaction,
            _=>StepKind::Unknown,
        };
        item["status"]=json!(match ty {"tool_execution_start"=>"started","tool_execution_end"=>"completed",_=>"unknown"});
        return vec![step(event,record,&item,kind,json!({"input":record["args"],"output":record["result"],"native":record}),0)];
    }
    if format=="acp" {
        if record["replay"]==true {return vec![];}
        let message=&record["message"];let update=&message["params"]["update"];
        let mut item=update.clone();
        if !item.is_object(){item=json!({});}
        item["id"]=update["toolCallId"].clone();
        let (kind,data)=match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk"|"user_message_chunk"|"agent_thought_chunk")=>{
                let kind=if update["sessionUpdate"]=="agent_thought_chunk" {StepKind::Thinking}else if update["content"]["type"]=="image" {StepKind::Image}else{StepKind::Message};
                (kind,json!({"text":update["content"]["text"],"content":update["content"]}))
            },
            Some("tool_call"|"tool_call_update")=>{
                let kind=match update["kind"].as_str() {Some("execute")=>StepKind::Shell,Some("edit"|"delete"|"move")=>StepKind::FileChange,Some("fetch"|"search")=>StepKind::Web,_=>StepKind::Tool};
                (kind,json!({"title":update["title"],"input":update["rawInput"],"output":update["rawOutput"],"content":update["content"]}))
            },
            Some("usage_update")=>(StepKind::Usage,update.clone()),
            _ if message["method"]=="session/request_permission"=>{item["id"]=message["id"].clone();(StepKind::Approval,message["params"].clone())},
            _=>(StepKind::Unknown,message.clone()),
        };
        return vec![base_step(event,record,&item,kind,data,0)];
    }
    let payload = if format == "codex-rollout" {&record["payload"]} else if format == "codex-app-server" {&record["params"]} else {record};
    let item = payload.get("item").unwrap_or(payload);
    if record["method"] == "item/commandExecution/outputDelta" {
        let item = json!({"id":payload["itemId"],"status":"delta"});
        return vec![base_step(event,record,&item,StepKind::Shell,json!({"delta":payload["delta"],"output_mode":"delta","unknown":{"command":"not reported by delta"}}),0)];
    }
    if format.starts_with("claude-") {
        let special=match record["type"].as_str() {
            Some("attachment")=>Some(match record["attachment"]["type"].as_str().unwrap_or_default() {
                n if n.starts_with("hook_") || n=="async_hook_response"=>StepKind::Hook,
                "read_truncation_notice"=>StepKind::ReadTruncation,
                "diagnostics"=>StepKind::Diagnostics,"queued_command"=>StepKind::Queue,_=>StepKind::Metadata,
            }),
            Some("system") if record["subtype"]=="stop_hook_summary"=>Some(StepKind::Hook),
            Some("queue-operation")=>Some(StepKind::Queue),Some("pr-link")=>Some(StepKind::Link),
            Some("api-request"|"api-request-blob"|"last-prompt"|"mode"|"progress"|"file-history-snapshot"|"summary"|"custom-title"|"agent-name"|"agent-color")=>Some(StepKind::Metadata),_=>None,
        };
        if let Some(kind)=special {
            let mut data=record.get("attachment").cloned().unwrap_or_else(||record.clone());
            if kind==StepKind::Queue {data["text"]=first(&data,&["prompt","content"]);}
            let mut step=base_step(event,record,&json!({"id":record["uuid"]}),kind,data,0);
            step.author="system".into();step.state="observed".into();return vec![step];
        }
    }
    let mut out = Vec::new();
    let extra=super::live::record_steps(format,event,record);
    let content = if format.starts_with("claude-") && matches!(record["type"].as_str(),Some("user"|"assistant")) {&record["message"]["content"]}
        else if format.starts_with("claude-"){&Value::Null}else{&item["content"]};
    if let Some(blocks) = content.as_array() {
        for (index, block) in blocks.iter().enumerate() {
            let ty = block["type"].as_str().unwrap_or_default();
            let kind = match ty {
                "text"|"Text"|"input_text"|"output_text" => StepKind::Message,
                "thinking"|"redacted_thinking"|"reasoning" => StepKind::Thinking,
                "image"|"Image"|"input_image"|"image_url"|"local_image"|"LocalImage" => StepKind::Image,
                "server_tool_use"=>StepKind::Web,
                "tool_search_tool_result"=>StepKind::Tool,
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
            out.push(base_step(event,record,&identity,kind,data,index));
        }
    }
    if !out.is_empty() {out.extend(extra);return out;}
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
    if matches!(kind,StepKind::Usage|StepKind::Unknown) && !extra.is_empty(){extra}else{
        let mut steps=vec![base_step(event,record,item,kind,data,0)];steps.extend(extra);steps
    }
}
