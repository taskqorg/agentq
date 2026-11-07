//! Explicit owner-bound native canary; synthetic qualification never authorizes a paid run.
mod collect;
use serde_json::{Value, json};
use std::path::Path;

const FIELDS: &[&str] = &["installation", "host", "runtime", "account", "home", "model", "permissions", "session_policy", "scenario_revision", "approved_operation", "executable"];
const PROMPT: &str = "Reply exactly AGENTQ_CANARY_OK";
const DIMENSIONS: &[&str] = &["terminal", "events", "history", "hook_correlation", "usage"];

fn hash(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn unknown(reason: &str) -> Value {
    json!({"state":"unknown","reason":reason,"detected_version":null,"scenario_revision":1,
        "conditions":{"model":null,"permissions":null,"session_policy":null,"account":null},
        "dimensions":DIMENSIONS.iter().map(|k|((*k).to_string(), Value::Null)).collect::<serde_json::Map<_,_>>()})
}
fn approved(data: &Path, request: &Value, host: Option<&str>) -> bool {
    let receipt = std::fs::read(data.join("canary/opt-in.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    let installation = std::fs::read_to_string(data.join("installation")).ok();
    let Some(object) = request.as_object() else { return false };
    receipt.as_ref() == Some(request) && object.len() == FIELDS.len()
        && FIELDS.iter().all(|k| object.contains_key(*k))
        && FIELDS.iter().filter(|k| !["scenario_revision","permissions","session_policy","model","executable"].contains(k)).all(|k| request[k].as_str().is_some_and(|s| !s.is_empty()))
        && collect::parameters(request).is_ok()
        && protected_receipt(&data.join("canary/opt-in.json"))
        && request["scenario_revision"] == 1 && request["approved_operation"] == "native-turn"
        && installation.as_deref().map(str::trim).filter(|s| !s.is_empty()) == request["installation"].as_str()
        && host.is_some() && host == request["host"].as_str()
        && request["home"].as_str().is_some_and(|s| Path::new(s).is_absolute())
        && request["runtime"].as_str().and_then(agentq_runtime::agent::registry::entry).is_some_and(|e|e.runtime.is_some())
}

// No executable is needed to establish local host identity before the authority gate.
fn host() -> Option<String> {
    #[cfg(unix)] {
        let mut name = [0u8; 256];
        if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 { return None; }
        let end = name.iter().position(|b| *b == 0)?;
        String::from_utf8(name[..end].to_vec()).ok().filter(|s| !s.is_empty())
    }
    #[cfg(not(unix))] { None }
}

fn numeric_version(text: &str) -> Option<String> {
    let token = crate::host::version_in(text)?;
    (token.split('.').count() >= 2 && token.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
        .then_some(token)
}

fn protected_receipt(path: &Path) -> bool {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(path).ok().is_some_and(|m|m.is_file() && m.uid()==unsafe {libc::geteuid()} && m.mode() & 0o077 == 0)
    }
    #[cfg(not(unix))] {let _=path; false}
}

pub fn request(data: &Path, raw: &str) -> Value { request_with(data,raw,host().as_deref()) }
fn request_with(data: &Path, raw: &str, host: Option<&str>) -> Value {
    let Ok(binding) = serde_json::from_str::<Value>(raw) else {return unknown("opt-in-unverified")};
    if !approved(data,&binding,host) {return unknown("opt-in-unverified")}
    let result=collect::run(data,&binding,host);
    let mut report=match result {Ok(v)=>v,Err(reason)=>unknown(reason)};
    if let Some(version)=report["detected_version"].as_str() {
        if matches!(report["state"].as_str(),Some("fail"|"drift")) {
            let found=finding(data,&binding,version,report["state"].as_str().unwrap(),&report);
            match found {Ok(id)=>report["finding"]=json!({"identity":id,"state":"open","delivery":"not-submitted"}),Err(_)=>report["finding"]=json!({"state":"unknown","reason":"state-unavailable"})}
        }
    }
    report
}


fn evaluate(dimensions: &[Option<bool>], drift: bool) -> &'static str {
    if dimensions.contains(&Some(false)) {"fail"} else if drift {"drift"}
    else if dimensions.contains(&None) {"unknown"} else {"pass"}
}

fn finding(data: &Path, binding: &Value, version: &str, state: &str, evidence: &Value) -> Result<String, String> {
    if !matches!(state,"fail"|"drift") || numeric_version(version).as_deref()!=Some(version) {return Err("finding-invalid".into())}
    let identity = hash(&serde_json::to_vec(&json!([binding["host"],binding["runtime"],version,binding["scenario_revision"]])).map_err(|_|"finding-invalid")?);
    let _lock = crate::executor::lock(data).map_err(|_|"state-unavailable")?;
    crate::control::save(&data.join("canary/findings"), &format!("{identity}.json"),
        &json!({"identity":identity,"state":"open","delivery":"not-submitted","runtime":binding["runtime"],
            "detected_version":version,"scenario_revision":1,"verdict":state,"evidence":finding_evidence(evidence)})).map_err(|_|"state-unavailable")?;
    Ok(identity)
}

fn finding_evidence(v: &Value)->Value {
    // This boundary never copies caller-selected keys or strings, even from an internal report.
    let dimensions=DIMENSIONS.iter().map(|k|((*k).to_string(),json!(v["dimensions"][k].as_bool()))).collect::<serde_json::Map<_,_>>();
    let conditions=["model","permissions","session_policy","account"].iter().map(|k|((*k).to_string(),json!(v["conditions"][k].as_bool()))).collect::<serde_json::Map<_,_>>();
    json!({"dimensions":dimensions,"conditions":conditions,"drift":v["drift"].as_bool(),"shapes":v["shapes"].as_array().map(|a|a.iter().map(project_shape).collect::<Vec<_>>())})
}

fn project_shape(v:&Value)->Value {
    let digest=|v:&Value|v.as_str().filter(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())).map(String::from);
    let kind=match v["kind"].as_str() {Some(s) if agentq_runtime::agent::RecordShape::known_kind(s)=>json!(s),_=>json!({"unknown_sha256":digest(&v["kind"]["unknown_sha256"])})};
    json!({"kind":kind,"unknown_keys":v["unknown_keys"].as_array().map(|a|a.iter().filter_map(digest).collect::<Vec<_>>()),
        "unknown_types":v["unknown_types"].as_array().map(|a|a.iter().filter_map(digest).collect::<Vec<_>>()),"field_count":v["field_count"].as_u64(),"input_tokens":v["input_tokens"].as_u64(),"output_tokens":v["output_tokens"].as_u64()})
}

pub fn selftest() -> bool {
    let private=json!({"private prompt":"secret","type":"credential"}).to_string();
    let scrubbed=agentq_runtime::agent::runtime("codex").and_then(|runtime|runtime.record_shape(&private).ok())
        .and_then(|shape|serde_json::to_string(&shape).ok());
    evaluate(&[Some(true);5],false)=="pass" && evaluate(&[None;5],false)=="unknown"
        && scrubbed.is_some_and(|shape|!shape.contains("secret"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shape(raw:&Value)->Value {serde_json::to_value(agentq_runtime::agent::runtime("codex").unwrap().record_shape(&raw.to_string()).unwrap()).unwrap()}
    #[test]
    fn authority_unknown_dimensions_privacy_and_restart_dedup() {
        let root = std::env::temp_dir().join(format!("agentq-canary-{}",hash(format!("{:?}",std::time::SystemTime::now()).as_bytes())));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("installation"),"fixture").unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        let b=collect::tests::binding(&root,"/fixture/client");
        assert!(!approved(&root,&b,Some("fixture-host")));
        crate::control::save(&root.join("canary"),"opt-in.json",&b).unwrap();
        assert!(approved(&root,&b,Some("fixture-host")));
        for field in FIELDS {
            let mut changed=b.clone();changed[field]=json!("changed");
            assert!(!approved(&root,&changed,Some("fixture-host")),"{field}");
            assert_eq!(request_with(&root,&changed.to_string(),Some("fixture-host"))["reason"],"opt-in-unverified");
        }
        assert!(!approved(&root,&b,Some("other-host")));
        assert!(!approved(&root,&b,None));
        // No executable lookup or process can be reached by a rejected receipt.
        assert_eq!(request_with(&root,"{}",Some("fixture-host")),unknown("opt-in-unverified"));
        for i in 0..5 {let mut d=[Some(true);5];d[i]=None;assert_eq!(evaluate(&d,false),"unknown");d[i]=Some(false);assert_eq!(evaluate(&d,false),"fail");}
        assert_eq!(evaluate(&[Some(true);5],true),"drift");
        let fixture:Value=serde_json::from_str(include_str!("../docs/spec/version-canary-drift.json")).unwrap();
        assert_eq!(shape(&json!({"type":"synthetic-new-record","synthetic-new-field":"never persist content"})),fixture);
        let raw=json!({"type":"secret-discriminator","secret-key":"session prompt bearer token", "usage":{"input_tokens":null,"output_tokens":2}});
        let scrub=shape(&raw).to_string();
        for secret in ["secret-discriminator","secret-key","session prompt","bearer token"] {assert!(!scrub.contains(secret));}
        assert_eq!(shape(&raw)["input_tokens"],Value::Null);
        let id=finding(&root,&b,"1.2.3","drift",&raw).unwrap();
        assert_eq!(id,finding(&root,&b,"1.2.3","drift",&raw).unwrap());
        assert_eq!(std::fs::read_dir(root.join("canary/findings")).unwrap().count(),1);
        let stored=std::fs::read_to_string(root.join(format!("canary/findings/{id}.json"))).unwrap();
        assert!(!stored.contains("secret"));assert!(!stored.contains("fixture-host"));
        assert_eq!(serde_json::from_str::<Value>(&stored).unwrap()["state"],"open");
        std::fs::write(root.join("canary/opt-in.json"),"broken").unwrap();
        assert!(!approved(&root,&b,Some("fixture-host")));
        assert!(numeric_version("1.2.secret").is_none());assert_eq!(numeric_version("codex-cli 1.2.3"),Some("1.2.3".into()));
        let metadata=json!({"type":"session_meta","payload":{"id":"fixture-thread","cwd":"/invented/never-open-this","forked_from_id":"fixture-parent"}});
        let projected=shape(&metadata);
        assert_eq!(projected["unknown_keys"],json!([]));
        assert!(!projected.to_string().contains("/invented/never-open-this"));
        assert!(!projected.to_string().contains("fixture-thread"));
        assert!(!projected.to_string().contains("fixture-parent"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
