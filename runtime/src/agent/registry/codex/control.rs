//! Codex 0.162 app-server callbacks; permission amendments are never inferred.
use super::*;

fn text<'a>(value:&'a Value,key:&str)->Res<&'a str> {value[key].as_str().filter(|s|!s.is_empty()).ok_or_else(||format!("request-{key}-invalid"))}

pub(super) fn request(value:&Value,session:&str,turn:&str)->Res<Option<Event>> {
    let method=value["method"].as_str().unwrap_or("");
    if !matches!(method,"item/commandExecution/requestApproval"|"item/fileChange/requestApproval"|"item/tool/requestUserInput") {return Ok(None);}
    let p=&value["params"];
    if text(p,"threadId")?!=session || text(p,"turnId")?!=turn {return Err("request-native-scope-differs".into());}
    text(p,"itemId")?;
    let id=value["id"].clone();if !id.is_string() && !id.is_i64() && !id.is_u64() {return Err("request-native-id-invalid".into());}
    let mut choices=vec![];let mut questions=vec![];
    let kind=if method=="item/tool/requestUserInput" {
        for q in p["questions"].as_array().filter(|a|!a.is_empty()).ok_or("request-questions-invalid")? {
            let id=text(q,"id")?.to_string();if questions.iter().any(|q:&Question|q.id==id) {return Err("request-question-id-duplicate".into());}
            let mut options=vec![];
            if !q["options"].is_null() {
                for option in q["options"].as_array().ok_or("request-options-invalid")? {
                    let label=text(option,"label")?.to_string();
                    if options.iter().any(|c:&Choice|c.id==label) {return Err("request-choice-id-duplicate".into());}
                    options.push(Choice{id:label.clone(),label,kind:"answer".into()});
                }
            }
            let free_text=if q["isOther"].is_null() {false}else{q["isOther"].as_bool().ok_or("request-question-invalid")?} || q["options"].is_null();
            questions.push(Question{id,prompt:text(q,"question")?.into(),choices:options,multiple:false,free_text});
        }
        RequestKind::Question
    }else{
        let offered=if p["availableDecisions"].is_null() {vec![json!("accept"),json!("acceptForSession"),json!("decline"),json!("cancel")]}else{p["availableDecisions"].as_array().ok_or("request-decisions-invalid")?.clone()};
        for decision in offered {
            let Some(id)=decision.as_str().filter(|s|matches!(*s,"accept"|"acceptForSession"|"decline"|"cancel")) else {continue;};
            if choices.iter().any(|c:&Choice|c.id==id) {return Err("request-choice-id-duplicate".into());}
            choices.push(Choice{id:id.into(),label:match id {"accept"=>"Allow once","acceptForSession"=>"Allow for session","decline"=>"Deny",_=>"Cancel turn"}.into(),kind:if id.starts_with("accept") {"allow"}else{"deny"}.into()});
        }
        if choices.is_empty() {return Err("request-decisions-unsupported".into());}
        RequestKind::Approval
    };
    Ok(Some(Event::Request{id,kind,prompt:p["reason"].as_str().or(p["command"].as_str()).map(String::from),choices,questions}))
}

pub(super) fn reply(native:&Value,control:&Control)->Res<Value> {
    let result=match (native["method"].as_str(),control) {
        (Some("item/commandExecution/requestApproval"|"item/fileChange/requestApproval"),Control::Reply{option,..})=>json!({"decision":option.as_deref().unwrap_or("cancel")}),
        (Some("item/tool/requestUserInput"),Control::Answers{answers,..})=>{
            let answers:serde_json::Map<String,Value>=answers.iter().map(|(id,values)|(id.clone(),json!({"answers":values.iter().map(|a|match a {Answer::Choice(s)|Answer::Text(s)=>s}).collect::<Vec<_>>()}))).collect();
            json!({"answers":answers})
        },
        (Some("item/tool/requestUserInput"),Control::Reply{option:None,..})=>json!({"answers":{}}),
        _=>return Err("request-response-unsupported".into()),
    };
    Ok(json!({"id":native["id"],"result":result}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn questions_and_offered_decisions_keep_native_scope_and_exact_answers() {
        let native=json!({"id":17,"method":"item/tool/requestUserInput","params":{"threadId":"session","turnId":"turn","itemId":"item","questions":[{"id":"q","question":"Choose","isOther":true,"options":[{"label":"Exact label","description":"native"}]}]}});
        let Some(Event::Request{questions,..})=request(&native,"session","turn").unwrap() else {panic!()};
        assert_eq!(questions[0].choices[0].id,"Exact label");assert!(questions[0].free_text);assert!(!questions[0].multiple);
        assert!(request(&native,"session","other").is_err());
        let control=Control::Answers{id:json!("opaque"),answers:BTreeMap::from([("q".into(),vec![Answer::Choice("Exact label".into())])])};
        assert_eq!(reply(&native,&control).unwrap(),json!({"id":17,"result":{"answers":{"q":{"answers":["Exact label"]}}}}));
        let approval=json!({"id":"approval","method":"item/commandExecution/requestApproval","params":{"threadId":"session","turnId":"turn","itemId":"item","availableDecisions":["decline",{"acceptWithExecpolicyAmendment":{}}]}});
        let Some(Event::Request{choices,..})=request(&approval,"session","turn").unwrap() else {panic!()};
        assert_eq!(choices.len(),1);assert_eq!(choices[0].id,"decline");
    }
}
