//! Shared wire families from Orca's MIT scanners; unknown records retain their native source.
use super::*;
fn first(v:&Value,keys:&[&str])->Value {keys.iter().find_map(|k|v.get(*k).filter(|v|!v.is_null())).cloned().unwrap_or(Value::Null)}
fn parsed(v:&Value)->Value {v.as_str().and_then(|s|serde_json::from_str(s).ok()).unwrap_or_else(||v.clone())}
fn tool(call:&Value)->Value {
    let f=call.get("function").unwrap_or(call);
    let name=first(f,&["name","tool","tool_name","function_name"]);let name=if name.is_null(){call["kind"]["tool_type"].clone()}else{name};
    let input=first(f,&["arguments","args","input","parameters"]);let input=if input.is_null(){call["kind"].clone()}else{parsed(&input)};
    json!({"type":"tool_use","id":first(call,&["id","call_id","callID","toolCallId","tool_call_id"]),"name":name,"input":input,"server":call["server"]})
}
fn block(v:&Value)->Value {
    if v["type"].is_null() && v["text"].is_string(){return json!({"type":"text","text":v["text"]});}
    if let Some(kind)=v["part_kind"].as_str(){return match kind {"text"|"user-prompt"=>json!({"type":"text","text":v["content"]}),"thinking"=>json!({"type":"thinking","thinking":v["content"]}),"tool-call"=>tool(v),"tool-return"=>json!({"type":"tool_result","tool_use_id":v["tool_call_id"],"content":v["content"]}),_=>v.clone()};}
    if let Some(kind)=v["kind"].as_str() {
        let d=&v["data"];
        return match kind {"text"=>json!({"type":"text","text":d}),"json"=>json!({"type":"text","text":d.to_string()}),
            "toolUse"=>tool(d),"toolResult"=>json!({"type":"tool_result","tool_use_id":first(d,&["id","toolUseId"]),"content":d["content"].as_array().map(|a|a.iter().map(block).collect::<Vec<_>>())}),_=>v.clone()};
    }
    match v["type"].as_str() {
        Some("thinking"|"reasoning"|"think")=>json!({"type":"thinking","thinking":first(v,&["thinking","text","content"])}),
        Some("toolCall"|"tool_call"|"function_call"|"backend_tool_call")=>tool(v),
        Some("toolResult"|"tool_result")=>{let mut b=v.clone();b["type"]=json!("tool_result");b["tool_use_id"]=first(v,&["tool_use_id","toolCallId","tool_call_id","id"]);b},
        Some("text") if v["thought"]==true=>json!({"type":"thinking","thinking":v["text"]}),
        _=>v.clone(),
    }
}
fn usage(v:&Value)->Value {
    let mut u=v.clone();if !u.is_object(){return Value::Null;}
    for (to,keys) in [("input_tokens",&["input_tokens","input"][..]),("output_tokens",&["output_tokens","output"][..]),("cache_read_input_tokens",&["cache_read_input_tokens","cache_read_tokens","cacheRead","cached"][..]),("cache_creation_input_tokens",&["cache_creation_input_tokens","cache_write_tokens","cacheWrite"][..])] {
        if u[to].is_null(){u[to]=first(v,keys);}
    }
    if u["cache_read_input_tokens"].is_null(){u["cache_read_input_tokens"]=v["cache"]["read"].clone();}
    if u["cache_creation_input_tokens"].is_null(){u["cache_creation_input_tokens"]=v["cache"]["write"].clone();}
    u
}
fn identity(v:&Value)->Value {let id=first(v,&["uuid","messageId","id"]);if id.is_number(){json!(id.to_string())}else{id}}
fn message(role:&str,content:Value,v:&Value)->Value {
    let content=content.as_array().map(|a|json!(a.iter().enumerate().flat_map(|(index,b)|{
        if b["type"]=="tool" && b["state"].is_object(){
            let id=first(b,&["id","callID","call_id"]);let id=if id.is_null(){json!(format!("{}:tool:{index}",identity(v)))}else{id};
            vec![json!({"type":"tool_use","id":id,"name":first(b,&["name","tool"]),"input":b["state"]["input"]}),json!({"type":"tool_result","tool_use_id":id,"content":first(&b["state"],&["content","output"]),"is_error":b["state"]["status"]=="error"})]
        }else{vec![block(b)]}
    }).collect::<Vec<_>>())).unwrap_or(content);
    json!({"type":role,"uuid":identity(v),"turn_id":first(v,&["turn_id","turnId","run_id"]),"timestamp":first(v,&["timestamp","time","created_at"]),
        "model":model(v),"effort":first(v,&["effort","reasoning_effort","thinkingLevel"]),"cost_usd":cost(v),"version":v["version"],"isMeta":v["isMeta"],"isSidechain":v["isSidechain"],"parent_tool_use_id":v["parent_tool_use_id"],
        "message":{"id":identity(v),"role":role,"model":model(v),"usage":usage(&first(v,&["usage","tokens"])),"content":content}})
}
fn cost(v:&Value)->Value {let cost=first(v,&["cost_usd","costUSD","total_cost_usd","actual_cost_usd","cost"]);if cost.is_number(){cost}else{cost["total"].clone()}}
fn usage_step(v:&Value,u:&Value)->Value {
    let u=usage(u);let mut unknown=serde_json::Map::new();
    let mut record=json!({"type":"usage","id":identity(v),"model":model(v),"input":u["input_tokens"],"output":u["output_tokens"],"cache_read":u["cache_read_input_tokens"],"cache_write":u["cache_creation_input_tokens"],"reasoning":u["reasoning_tokens"],"total":v["token_count"],"cost_usd":cost(v),"aggregate":v["_usage_aggregate"]==true || v["role"]=="user","scope":first(v,&["scope","usageScope"]),"time":first(v,&["timestamp","time","time_created"]),"native":v});
    for key in ["input","output","cache_read","cache_write","reasoning","cost_usd"] {if record[key].is_null(){unknown.insert(key.into(),json!("native field not reported"));}}
    record["unknown"]=json!(unknown);record
}
fn model(v:&Value)->Value {let m=first(v,&["model","modelAlias","modelID"]);if m.is_string(){m}else{first(&v["model"],&["modelID","id"]).as_str().map(|s|json!(s)).unwrap_or_else(||v["modelInfo"]["id"].clone())}}
fn result_record(id:Value,output:Value,v:&Value)->Value {
    let mut result=message("user",json!([{"type":"tool_result","tool_use_id":id,"content":output,"is_error":v["is_error"]==true || v["isError"]==true || v["success"]==false}]),v);
    let data=parsed(&output);if data.is_object(){result["toolUseResult"]=data;}result
}
pub(super) fn records(spec:&SourceSpec,original:&Value)->Vec<Value> {
    let mut v=original.clone();
    if let Some(data)=original["data"].as_str().and_then(|s|serde_json::from_str::<Value>(s).ok()).filter(Value::is_object) {
        v=data;if v["id"].is_null(){v["id"]=original["id"].clone();}if v["role"].is_null(){v["role"]=original["role"].clone();}if v["type"].is_null(){v["type"]=original["type"].clone();}
    }
    if matches!(v["type"].as_str(),Some("api-request"|"api-request-blob"|"last-prompt"|"mode"|"progress"|"file-history-snapshot"|"summary"|"custom-title"|"attachment"|"queue-operation"|"pr-link")){return vec![v];}
    if v["_session"]==true {return vec![json!({"type":"mode","model":model(&v),"version":v["version"],"native":v})];}
    if original["_usage_aggregate"]==true {v["_usage_aggregate"]=json!(true);}
    if spec.name=="rovo" && matches!(v["kind"].as_str(),Some("request"|"response")){return vec![message(if v["kind"]=="request"{"user"}else{"assistant"},v["parts"].clone(),&v)];}
    if spec.name=="devin" && v["metadata"]["is_user_input"]==true {v["role"]=json!("user");}
    if v["type"]=="custom_message" {return vec![if v["display"]==true {message("system",v["content"].clone(),&v)}else{json!({"type":"mode","native":v})}];}
    if matches!(v["message"]["role"].as_str(),Some("bashExecution"|"pythonExecution")) {
        let m=&v["message"];let bash=m["role"]=="bashExecution";let id=first(&v,&["id","uuid"]);let id=if id.is_null(){json!("execution")}else{id};
        let call=message("assistant",json!([{"type":"tool_use","id":id,"name":if bash{"bash"}else{"python"},"input":{"command":m["command"],"code":m["code"]}}]),&v);
        return vec![call,result_record(id,json!({"output":m["output"],"exit_code":m["exitCode"],"interrupted":m["cancelled"]}),&v)];
    }
    if matches!(v["message"]["role"].as_str(),Some("custom"|"hookMessage"|"fileMention")){return vec![json!({"type":"mode","native":v})];}
    if spec.name=="kiro" {
        let role=match v["kind"].as_str(){Some("Prompt")=>Some("user"),Some("AssistantMessage")=>Some("assistant"),Some("ToolResults")=>Some("user"),_=>None};
        if let Some(role)=role{return vec![message(role,v["data"]["content"].clone(),&v["data"])];}
    }
    if v["type"]=="tool" && v["state"].is_null() && v["content"].is_null(){v["type"]=json!("tool_result");}
    let ty=v["type"].as_str().unwrap_or_default().to_string();
    if spec.name=="copilot" {
        let d=&v["data"];let mut d=d.clone();if d["id"].is_null(){d["id"]=v["id"].clone();}d["timestamp"]=v["timestamp"].clone();
        match ty.as_str(){
            "user.message"=>return vec![message("user",first(&d,&["transformedContent","content"]),&d)],
            "assistant.message"=>return vec![message("assistant",d["content"].clone(),&d)],
            "assistant.reasoning"=>return vec![message("assistant",json!([{"type":"thinking","thinking":d["content"]}]),&d)],
            "tool.execution_start"=>{let c=json!({"id":d["toolCallId"],"name":d["toolName"],"input":d["arguments"]});return vec![message("assistant",json!([tool(&c)]),&v)];},
            "tool.execution_complete"=>return vec![result_record(d["toolCallId"].clone(),first(&d,&["result","content"]),&d)],_=>{}
        }
    }
    if spec.name=="kimi" {
        match ty.as_str(){
            "context.append_message"=>{let mut m=v["message"].clone();if !v["time"].is_null(){m["id"]=json!(format!("message:{}",v["time"]));}if m["origin"]["kind"]=="injection"{return vec![json!({"type":"mode","native":v})];}v=m;},
            "turn.prompt"=>return vec![json!({"type":"queue-operation","operation":"submitted","content":v["input"]})],
            "context.append_loop_event"=>{let e=&v["event"];if e["type"]=="content.part"{return vec![message("assistant",json!([block(&e["part"])]),&v)];}v=e.clone();},
            "usage.record"=>return vec![json!({"type":"usage","model":v["model"],"input":v["usage"]["inputOther"],"cache_read":v["usage"]["inputCacheRead"],"cache_write":v["usage"]["inputCacheCreation"],"output":v["usage"]["output"],"scope":v["usageScope"]})],_=>{}
        }
    }
    // OpenCode parts have one self-contained tool state, including completed output.
    if ty=="tool" && v["state"].is_object() {
        let state=&v["state"];let id=first(&v,&["callID","call_id","id"]);let name=first(&v,&["tool","name"]);
        let mut rows=vec![message("assistant",json!([{"type":"tool_use","id":id,"name":name,"input":state["input"]}]),&v)];
        if matches!(state["status"].as_str(),Some("completed"|"error"|"cancelled")) {
            let mut result=message("user",json!([{"type":"tool_result","tool_use_id":id,"content":state["output"],"is_error":state["status"]!="completed"}]),&v);
            let mut data=parsed(&state["output"]);if !data.is_object(){data=json!({"output":state["output"]});}
            if let Some(exit)=state["metadata"].get("exit").or(state["metadata"].get("exitCode")){data["exit_code"]=exit.clone();}
            result["toolUseResult"]=data;rows.push(result);
        }return rows;
    }
    if matches!(ty.as_str(),"text"|"reasoning"|"thinking"|"file") && (original["message_id"].is_string() || v["messageID"].is_string()) {
        let b=if ty=="file" {json!({"type":"image","url":v["url"],"mime":v["mime"],"name":v["filename"]})}else{block(&v)};
        return vec![message(v["role"].as_str().unwrap_or("assistant"),json!([b]),&v)];
    }
    if ty=="patch" {return vec![json!({"type":"FileChange","id":v["id"],"changes":v["files"].as_array().map(|a|a.iter().map(|p|if p.is_string(){json!({"path":p,"patch":v["diff"]})}else{p.clone()}).collect::<Vec<_>>()),"diff":v["diff"],"status":"completed","native":v})];}
    if ty=="usage" {return vec![usage_step(&v,v.get("usage").unwrap_or(&v))];}
    if matches!(ty.as_str(),"plan"|"todo"|"todos"){let mut data=v.clone();data["items"]=first(&v,&["items","todos","plan"]);if ty=="plan"{data["plan"]=data["items"].clone();}return vec![data];}
    if matches!(ty.as_str(),"session"|"session_start"|"session.start"|"metadata"|"system"){return vec![json!({"type":"mode","model":model(&v),"version":v["version"],"native":v})];}
    if matches!(ty.as_str(),"compaction"|"context_compacted"|"plan"|"todo"|"todos"|"usage"|"model_change"|"model-change"|"model.change"|"background_task"|"subagent") {return vec![v];}
    if matches!(ty.as_str(),"tool_call"|"backend_tool_call"|"function_call") {return vec![message("assistant",json!([tool(&v)]),&v)];}
    if ty=="tool_result" {return vec![result_record(first(&v,&["tool_use_id","tool_call_id","toolCallId","id"]),first(&v,&["content","output","result"]),&v)];}
    if matches!(ty.as_str(),"reasoning"|"thinking") {return vec![message("assistant",json!([{"type":"thinking","thinking":first(&v,&["text","thinking","summary","content"])}]),&v)];}
    // Pi/OMP/OpenClaw, Droid and Anthropic-compatible wrappers.
    let wrapped=v.get("message").filter(|m|m.is_object()).cloned();
    let mut m=wrapped.unwrap_or_else(||v.clone());
    let role=m["role"].as_str().or(v["role"].as_str()).or(v["source"].as_str()).or(v["type"].as_str()).unwrap_or_default();
    let role=match role {"assistant"|"agent"|"gemini"|"model"=>"assistant","user"|"human"=>"user","tool"|"toolResult"=>"tool","system"=>"system",_=>"unknown"};
    if role=="unknown" {return vec![v];}
    if role=="tool" {
        return vec![result_record(first(&m,&["tool_call_id","toolCallId","tool_use_id","id"]),first(&m,&["content","output","result"]),&m)];
    }
    let mut content=parsed(&first(&m,&["content","parts","text"]));
    if content.is_null() && v["message"].is_string(){content=v["message"].clone();}
    if content.is_null(){content=first(&v,&["content","text"]);}
    let mut blocks=content.as_array().cloned().unwrap_or_else(||if content.is_null(){vec![]}else{vec![json!({"type":"text","text":content})]});
    for key in ["reasoning","reasoning_content"] {if m[key].is_string(){blocks.push(json!({"type":"thinking","thinking":m[key]}));}}
    for thought in m["thoughts"].as_array().into_iter().flatten(){blocks.push(json!({"type":"thinking","thinking":first(thought,&["description","text","subject"])}));}
    for key in ["tool_calls","toolCalls"] {for call in parsed(&m[key]).as_array().into_iter().flatten(){blocks.push(tool(call));
        if let Some(result)=call.get("result"){blocks.push(json!({"type":"tool_result","tool_use_id":call["id"],"content":result}));}
    }}
    if m["id"].is_null(){m["id"]=first(&v,&["uuid","id","step_id"]);}if m["model"].is_null(){m["model"]=v["model"].clone();}
    for key in ["timestamp","time","version","isMeta","isSidechain","modelInfo","cost","tokens"] {if m[key].is_null(){m[key]=v[key].clone();}}
    if m["timestamp"].is_null(){m["timestamp"]=first(&v,&["time_created","created_at"]);}
    let empty=blocks.is_empty();
    let mut result=if empty {json!({"type":"mode","model":model(&m),"native":v})}else{message(role,json!(blocks),&m)};
    if !v["toolUseResult"].is_null(){result["toolUseResult"]=v["toolUseResult"].clone();}
    if !v["uuid"].is_null(){result["uuid"]=v["uuid"].clone();}
    let mut out=vec![result];
    if first(&m,&["usage","tokens"]).is_object(){out.push(usage_step(&m,&first(&m,&["usage","tokens"])));}
    else if m["token_count"].is_number(){let mut u=usage_step(&m,&Value::Null);u["aggregate"]=json!(true);out.push(u);}
    if let Some(parts)=out[0]["message"]["content"].as_array().cloned(){
        for b in parts.iter().filter(|b|b["type"]=="tool_result") {
            if b["content"].is_object(){out.push(result_record(b["tool_use_id"].clone(),b["content"].clone(),&m));}
        }
    }
    for item in v["observation"]["results"].as_array().into_iter().flatten(){out.push(result_record(item["source_call_id"].clone(),first(item,&["content","output"]),&v));}
    if spec.name=="openclaw" {for record in &mut out {for block in record["message"]["content"].as_array_mut().into_iter().flatten(){if block["type"]=="tool_use" && block["name"]=="exec"{block["_history_kind"]=json!("shell");}}}}
    out
}
