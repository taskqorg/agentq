//! One exact Launch and content-free observations, with existing Runtime guards and native ownership.
use super::*;
use agentq_runtime::agent::{self, Launch, Policy, Lifetime, ReadinessMode, State, Event, Repair};
use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};

pub(super) fn parameters(b: &Value)->Result<(Policy,Lifetime,ReadinessMode),()> {
    let p:Policy=serde_json::from_value(b["permissions"].clone()).map_err(|_|())?;
    // Policy's serde defaults must never create an unapproved part of a request.
    if serde_json::to_value(&p).map_err(|_|())? != b["permissions"] {return Err(())}
    let lifetime:Lifetime=serde_json::from_value(b["session_policy"]["lifetime"].clone()).map_err(|_|())?;
    let mode=match b["session_policy"]["readiness"].as_str() {Some("continue")=>ReadinessMode::Continue,Some("strict")=>ReadinessMode::Strict,_=>return Err(())};
    if b["session_policy"].as_object().map(|o|o.len())!=Some(2) {return Err(())}
    if !(b["model"].is_null() || b["model"].as_str().is_some_and(|s|!s.is_empty())) {return Err(())}
    let home=Path::new(b["home"].as_str().ok_or(())?);
    if !home.is_absolute() || !home.is_dir() || home.canonicalize().map_err(|_|())?!=home {return Err(())}
    let exe=&b["executable"];
    if exe.as_object().map(|o|o.len())!=Some(2) || !exe["path"].as_str().is_some_and(|s|Path::new(s).is_absolute())
        || !exe["sha256"].as_str().is_some_and(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit())) {return Err(())}
    if !b["account"].as_str().is_some_and(agent::valid_label) {return Err(())}
    Ok((p,lifetime,mode))
}
fn executable(b:&Value)->Option<PathBuf> {
    let path=PathBuf::from(b["executable"]["path"].as_str()?);
    if path.canonicalize().ok()?!=path {return None}
    (hash(&std::fs::read(&path).ok()?)==b["executable"]["sha256"].as_str()?).then_some(path)
}
struct Workspace {path:PathBuf, terminal:bool}
impl Drop for Workspace {fn drop(&mut self) {if self.terminal {let _=std::fs::remove_dir_all(&self.path);}}}
fn workspace()->Result<Workspace,&'static str> {
    let mut id=[0u8;16];use ring::rand::SecureRandom;
    ring::rand::SystemRandom::new().fill(&mut id).map_err(|_|"workspace-unavailable")?;
    let path=std::env::temp_dir().join(format!("agentq-canary-{}",hash(&id)));
    #[cfg(unix)] {use std::os::unix::fs::DirBuilderExt;std::fs::DirBuilder::new().mode(0o700).create(&path).map_err(|_|"workspace-unavailable")?;}
    #[cfg(not(unix))] {return Err("host-identity-unverified")}
    Ok(Workspace {path,terminal:true})
}
fn hook_baseline(data:&Path)->Option<std::collections::BTreeSet<String>> {
    match std::fs::read_dir(data.join("sessions")) {
        Ok(rows)=>rows.map(|r|r.ok().map(|r|r.file_name().to_string_lossy().into_owned())).collect(),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Some(Default::default()),Err(_)=>None,
    }
}
fn hook(data:&Path,session:Option<&str>,baseline:&Option<std::collections::BTreeSet<String>>)->Option<bool> {
    let name=format!("{}.json",crate::session::event_id("session",session?));
    if baseline.as_ref()?.contains(&name) {return None}
    let row:Value=serde_json::from_slice(&std::fs::read(data.join("sessions").join(name)).ok()?).ok()?;
    (row["launched"]==true).then_some(true)
}

pub(super) fn run(data:&Path,b:&Value,host:Option<&str>)->Result<Value,&'static str> {
    let (policy,lifetime,readiness)=parameters(b).map_err(|_|"opt-in-unverified")?;
    {
        let _lock=crate::executor::lock(data).map_err(|_|"state-unavailable")?;
        if !approved(data,b,host) {return Err("opt-in-unverified")}
    }
    let exe=executable(b).ok_or("executable-unverified")?;
    let runtime=agent::registry::entry(b["runtime"].as_str().unwrap()).and_then(|e|e.runtime).ok_or("runtime-unsupported")?;
    let mut workspace=workspace()?;
    let mut launch=Launch::new(&workspace.path,workspace.path.join("turn.jsonl"),PROMPT,"agentq-canary");
    launch.policy=policy;launch.model=b["model"].as_str().map(String::from);
    launch.home=Some(PathBuf::from(b["home"].as_str().unwrap()));
    launch.command=vec![exe.clone().into_os_string()];launch.lifetime=lifetime;launch.readiness=readiness;
    // Pin optional archive bookkeeping to an empty owned temporary installation, never the host install.
    launch.install=Some(workspace.path.join("install"));
    // No telemetry store, environment overrides, tools, auth or configuration repair.
    // Version observations use this same Launch context, never an ambient bare Command.
    let version=bound_version(data,b,host,runtime,&launch)?;
    let baseline=hook_baseline(data);
    let initial=runtime.preflight(&launch,&Repair::default());
    if initial.permit(readiness).is_err() {return Ok(failed(&version,"runtime-preflight-refused"))}
    if bound_version(data,b,host,runtime,&launch)?!=version || executable(b).is_none() {return Err("executable-or-version-changed")}
    {
        let _lock=crate::executor::lock(data).map_err(|_|"state-unavailable")?;
        if !approved(data,b,host) {return Err("opt-in-unverified")}
    }
    // The Runtime performs its own preflight too; recheck authority/bytes at its actual spawn boundary.
    let owned_data=data.to_path_buf();let owned_binding=b.clone();let owned_host=host.map(String::from);
    launch.process.configure=Some(Arc::new(move |command| {
        if !approved(&owned_data,&owned_binding,owned_host.as_deref()) || executable(&owned_binding).is_none() {
            // Existing Configure is infallible: an empty program refuses spawn without running any substitute.
            *command=Command::new("");
        }
    }));
    let mut run=match runtime.launch(&launch) {Ok(run)=>run,Err(_)=>{
        if !approved(data,b,host) || executable(b).is_none() {return Err("opt-in-or-executable-changed")}
        return Ok(failed(&version,"runtime-launch-refused"))
    }};
    workspace.terminal=false;
    let mut events=vec![];let mut event_unknown=false;
    let state=loop {
        match runtime.progress(&mut run) {Ok(mut next)=>events.append(&mut next),Err(_)=>event_unknown=true}
        match runtime.poll(&mut run) {
            Ok(State::Running)=>std::thread::sleep(Duration::from_millis(50)),
            Ok(state)=>break state,
            Err(_)=>return Ok(observation_unknown(&version,"runtime-poll-unavailable")),
        }
    };
    if !matches!(state,State::Finished|State::Failed) {return Ok(observation_unknown(&version,"native-terminal-unverified"))}
    workspace.terminal=true;
    match runtime.progress(&mut run) {Ok(mut next)=>events.append(&mut next),Err(_)=>event_unknown=true}
    let report=match runtime.report(&run) {Ok(report)=>report,Err(_)=>return Ok(observation_unknown(&version,"native-report-unavailable"))};
    let mut d=[None;5];
    d[0]=match report.outcome {"ok"=>report.text.as_deref().map(|s|s.trim()=="AGENTQ_CANARY_OK"),"failed"|"cancelled"=>Some(false),_=>None};
    let mut drift=events.iter().any(|e|matches!(e,Event::Unsupported{..}));
    d[1]=if event_unknown || !events.iter().any(|e|matches!(e,Event::TurnEnd{..})) {None} else {Some(true)};
    d[3]=hook(data,report.session.as_deref(),&baseline);
    d[4]=report.usage.input.zip(report.usage.output).map(|_|true);
    let mut conditions=[None;4];
    let mut models=std::collections::BTreeSet::new();
    if let Some(model)=report.usage.model.as_deref() {models.insert(hash(model.as_bytes()));}
    let mut approval_observed=None;
    conditions[0]=b["model"].as_str().zip(report.usage.model.as_deref()).map(|(want,got)|want==got);
    let mut shapes=vec![];
    // Runtime retains native format discovery and decoding; no product-native parser or source fallback.
    if let Some(session)=report.session.as_deref() {
        if let Ok(mut history)=runtime.history(session,launch.home.as_deref(),None,None) {
            if let Ok(page)=history.steps(u64::MAX) {
                if !page.steps.is_empty() && !page.more && page.pending_bytes==0 {d[2]=Some(true)}
                for step in page.steps {
                    if let Some(raw)=step.source["raw"].as_str() {
                        match runtime.record_shape(raw) {
                            Ok(projected)=>if step.kind==agent::history::StepKind::Unknown || projected.drift() {drift=true;shapes.push(projected);},
                            Err(_)=>drift=true,
                        }
                    }
                    if let Some(model)=step.presentation["model"].as_str() {models.insert(hash(model.as_bytes()));}
                    if let Some(model)=b["model"].as_str().zip(step.presentation["model"].as_str()) {
                        models.insert(hash(model.1.as_bytes()));
                        let same=model.0==model.1;
                        conditions[0]=Some(conditions[0].unwrap_or(true) && same);
                    }
                }
            }
        }
    }
    // Typed records add drift evidence without copying the underlying JSON or its keys.
    if let Ok(path)=runtime.transcript(&run) {
        if let Ok(text)=std::fs::read_to_string(path) {
            for line in text.lines().filter(|s|!s.trim().is_empty()) {
                match runtime.record_shape(line) {
                    Ok(projected)=>if projected.drift() {drift=true;if !shapes.contains(&projected) {shapes.push(projected);}},
                    Err(_)=>drift=true,
                }
            }
            for record in runtime.log_records(&text) {
                if record.events.iter().any(|e|matches!(e,Event::Unsupported{..})) {drift=true;}
                if let Some(approval)=record.permission_mode.as_deref() {
                    // Only identical neutral spellings can be confirmed; native-specific aliases stay unknown.
                    if let Ok(observed)=serde_json::from_value::<agent::Approval>(json!(approval)) {
                        approval_observed=serde_json::to_value(observed).ok();
                        if observed!=agent::Approval::Native && launch.policy.approval!=agent::Approval::Native && observed!=launch.policy.approval {conditions[1]=Some(false)}
                    }
                    // Matching approval alone never confirms the rest of a requested Policy.
                }
            }
        }
    }
    let mut all=d.to_vec();all.extend(conditions);
    let verdict=evaluate(&all,drift);
    Ok(json!({"state":verdict,"detected_version":version,"scenario_revision":1,"binding_sha256":hash(&serde_json::to_vec(b).unwrap()),
        "dimensions":DIMENSIONS.iter().zip(d).map(|(k,v)|((*k).to_string(),json!(v))).collect::<serde_json::Map<_,_>>(),
        "conditions":{"model":conditions[0],"permissions":conditions[1],"session_policy":conditions[2],"account":conditions[3]},
        "observed_conditions":{"model_sha256":models,"approval":approval_observed,"session_policy":null,"account":null},
        "condition_unknown":{"model":if conditions[0].is_none(){Some("native-model-comparison-unavailable")}else{None},
            "permissions":if conditions[1].is_none(){Some("full-policy-observation-unavailable")}else{None},"session_policy":"native-session-policy-unreported","account":"native-account-identity-unreported"},
        "drift":drift,"shapes":shapes}))
}
fn bound_version(data:&Path,b:&Value,host:Option<&str>,runtime:&dyn agent::Runtime,launch:&Launch)->Result<String,&'static str> {
    let _lock=crate::executor::lock(data).map_err(|_|"state-unavailable")?;
    if !approved(data,b,host) {return Err("opt-in-unverified")}
    if executable(b).is_none() {return Err("executable-unverified")}
    let observed=runtime.version_for(launch);
    if !approved(data,b,host) {return Err("opt-in-unverified")}
    if executable(b).is_none() {return Err("executable-unverified")}
    if !matches!(observed.state,"ok"|"outdated") {return Err("version-unverified")}
    numeric_version(&observed.detail).ok_or("version-unverified")
}
fn observation_unknown(version:&str,reason:&str)->Value {let mut v=unknown(reason);v["detected_version"]=json!(version);v}
fn failed(version:&str,reason:&str)->Value {let mut v=observation_unknown(version,reason);v["state"]=json!("fail");v}

#[cfg(test)]
pub(super) mod tests;
