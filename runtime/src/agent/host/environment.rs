//! M12.8 / M12.13: clean environments shared by launchd and the native child.
use super::*;
fn xml(text:&str)->String {text.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&apos;")}
pub(super) fn plist(label:&str,program:&str,key:&str,root:&str,environment:&Value)->Res<String> {
    let mut vars=String::new();
    for (k,v) in environment.as_object().ok_or("scope environment unknown")? {
        vars.push_str(&format!("<key>{}</key><string>{}</string>",xml(k),xml(v.as_str().ok_or("scope environment invalid")?)));
    }
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>Program</key><string>{}</string><key>ProgramArguments</key><array><string>{}</string><string>agent</string><string>host</string><string>--action</string><string>scope-run</string><string>--request</string><string>{}</string></array><key>WorkingDirectory</key><string>{}</string><key>EnvironmentVariables</key><dict>{vars}</dict><key>RunAtLoad</key><false/><key>KeepAlive</key><false/><key>LaunchOnlyOnce</key><true/><key>AbandonProcessGroup</key><true/></dict></plist>",xml(label),xml(program),xml(program),xml(key),xml(root)))
}
pub(super) fn environment(home:&Path)->Value {
    let mut environment=serde_json::Map::new();
    for name in ["PATH","LANG","LC_ALL","TMPDIR","TERM","AGENTQ_OPTIMIZATION"] {if let Ok(value)=std::env::var(name) {environment.insert(name.into(),json!(value));}}
    environment.insert("HOME".into(),json!(home));
    use crate::telemetry::capture::TraceContext;
    let caller=TraceContext::current().or_else(||std::env::var("COREQ_TRACE_CONTEXT").ok().and_then(|s|TraceContext::decode(&s).ok()));
    if let Some(caller)=caller {environment.insert("COREQ_TRACE_CONTEXT".into(),json!(caller.encode()));}
    json!(environment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    const TEST:&str="agent::host::environment::tests::protected_child_context_survives_clean_hops";
    // #260 A2/A3, M12.13: each clean hop must retain only valid caller metadata; missing fields and tokens stay unknown.
    #[test]
    fn protected_child_context_survives_clean_hops() {
        if let Ok(stage)=std::env::var("AGENTQ_CONTEXT_FIXTURE_STAGE") {
            let home=PathBuf::from(std::env::var_os("AGENTQ_CONTEXT_FIXTURE_HOME").unwrap());
            let expected:Value=serde_json::from_str(&std::env::var("AGENTQ_CONTEXT_FIXTURE_EXPECTED").unwrap()).unwrap();
            let env=environment(&home);
            assert_eq!(env["COREQ_TRACE_CONTEXT"],expected,"protected hop {stage} lost or accepted invalid context");
            assert!(env.as_object().unwrap().keys().all(|k|["PATH","LANG","LC_ALL","TMPDIR","TERM","AGENTQ_OPTIMIZATION","HOME","COREQ_TRACE_CONTEXT"].contains(&k.as_str())));
            assert!(!env.to_string().contains("invented-private-value"));
            if stage=="scope" {
                let job=plist("invented-label","/invented/adapter","invented-operation","/invented/project",&env).unwrap();
                if let Some(text)=expected.as_str() {assert!(job.contains(&format!("<key>COREQ_TRACE_CONTEXT</key><string>{}</string>",xml(text))));}
                else {assert!(!job.contains("<key>COREQ_TRACE_CONTEXT</key>"));}
                // Launchd and sandbox-exec are represented by fresh processes, using the production clean environment.
                let out=Command::new(std::env::current_exe().unwrap()).args(["--exact",TEST,"--nocapture"])
                    .env_clear().envs(env.as_object().unwrap().iter().map(|(k,v)|(k,v.as_str().unwrap())))
                    .env("AGENTQ_CONTEXT_FIXTURE_STAGE","native").env("AGENTQ_CONTEXT_FIXTURE_HOME",&home)
                    .env("AGENTQ_CONTEXT_FIXTURE_EXPECTED",expected.to_string()).output().unwrap();
                assert!(out.status.success(),"native hop: {}{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
            } else {
                assert_eq!(std::env::var("COREQ_TRACE_CONTEXT").ok().as_deref(),expected.as_str());
                super::super::super::capture::with_environment_caller(|| {
                    use crate::agent::{capture,Client,Context,Report};
                    let ctx=Context{product:"fixtureq",version:"1",project:"other",task:None,route:None,account:"default"};
                    let events=[json!({"type":"assistant","message":{"id":"invented-call","model":"fixture-model","usage":{}}}),json!({"type":"result"})];
                    let report=Report::read(Client::Claude,"",&events,None);
                    let store=coreq::telemetry::Store::open(&home.join("observations.db")).unwrap();
                    let turn=coreq::telemetry::Event::observation(report.observation(Client::Claude,&ctx)).unwrap();
                    for _ in 0..2 {capture::record(&store,turn.clone(),&report,Client::Claude,&ctx,"invented-turn").unwrap();}
                    let rows=store.export(0,0,100).unwrap();let rows=rows["events"].as_array().unwrap();
                    assert_eq!(rows.len(),2,"idempotent turn and call");
                    let fields=expected.as_str().and_then(|s|s.strip_prefix("2 ")).map(|s|serde_json::from_str::<Value>(s).unwrap()[2].clone()).unwrap_or(json!({}));
                    for row in rows {
                        let row=&row["observation"];
                        assert!(matches!(row["kind"].as_str(),Some("agent.turn"|"agent.call")));
                        assert_eq!(row["project"],fields.get("project").cloned().unwrap_or(json!("other")));
                        for (key,reason) in [("task","no-task"),("attempt",if fields["task"].is_null(){"no-task"}else{"no-attempt"}),("actor","not-collected"),("entry","not-collected")] {
                            assert_eq!(row[key],fields[key],"{key}: {row}");
                            if fields[key].is_null() {assert_eq!(row["unknown"][key],reason,"{row}");}
                        }
                        assert!(row["input_tokens"].is_null()&&row["unknown"]["input_tokens"]=="not-reported","{row}");
                    }
                });
            }
            return;
        }
        let root=crate::scratch("protected-context");
        {
            use tracing_subscriber::prelude::*;
            let store=std::sync::Arc::new(coreq::telemetry::Store::open(&root.join("caller.db")).unwrap());
            let capture=coreq::telemetry::capture::Capture::new(store,"fixtureq","1").unwrap();
            let dispatch=tracing::Dispatch::new(tracing_subscriber::registry().with(capture.clone()));
            tracing::dispatcher::with_default(&dispatch,|| {
                let _caller=tracing::info_span!("caller",project="fixture/current",task=1_u64,attempt=2_u64,actor="worker",entry=3_u64,private="invented-private-value").entered();
                let _nested=tracing::info_span!("nested").entered();
                let current=coreq::telemetry::capture::TraceContext::current().unwrap().encode();
                assert_eq!(environment(&root)["COREQ_TRACE_CONTEXT"],current,"current nested caller wins");
                assert!(!current.contains("invented-private-value"));
            });
            capture.finish().unwrap();
        }
        let trace="00000000000000000000000000000001.1";
        let full=format!("2 {}",json!([trace,trace,{"project":"fixture/project","task":2428,"attempt":3,"actor":"worker","entry":77}]));
        let partial=format!("2 {}",json!([trace,trace,{"task":7}]));
        let legacy=format!("1 {trace} {trace}");
        let private=format!("2 {}",json!([trace,trace,{"project":"fixture/project","private":"invented-private-value"}]));
        let bad_type=format!("2 {}",json!([trace,trace,{"task":"7"}]));
        for (name,input,expected) in [("full",Some(full.as_str()),Some(full.as_str())),("partial",Some(partial.as_str()),Some(partial.as_str())),
            ("legacy",Some(legacy.as_str()),Some(legacy.as_str())),("missing",None,None),("invalid",Some("2 not-json"),None),
            ("private",Some(private.as_str()),None),("bad-type",Some(bad_type.as_str()),None),("oversize",Some("x".repeat(1025).as_str()),None)] {
            let home=root.join(name);fs::create_dir(&home).unwrap();
            let mut cmd=Command::new(std::env::current_exe().unwrap());cmd.args(["--exact",TEST,"--nocapture"])
                .env_remove("COREQ_TRACE_CONTEXT").env("AGENTQ_CONTEXT_FIXTURE_STAGE","scope")
                .env("AGENTQ_CONTEXT_FIXTURE_HOME",&home).env("AGENTQ_CONTEXT_FIXTURE_EXPECTED",json!(expected).to_string())
                .env("AGENTQ_PRIVATE_FIXTURE","invented-private-value");
            if let Some(text)=input {cmd.env("COREQ_TRACE_CONTEXT",text);}
            let out=cmd.output().unwrap();
            assert!(out.status.success(),"{name}: {}{}",String::from_utf8_lossy(&out.stdout),String::from_utf8_lossy(&out.stderr));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
