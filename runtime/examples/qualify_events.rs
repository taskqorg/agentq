//! Explicit paid native probe for #246 A5; normal tests never run it. All work stays in OS temp.
use agentq_runtime::agent::{self, Approval, Builtin, Event, Launch, Policy, Scope, State};
use serde_json::json;
use std::time::{Duration, Instant};

fn finish(runtime: &dyn agent::Runtime, launch: &Launch, resume: Option<&str>) -> Result<(agent::Report, Vec<Event>), String> {
    let mut run = match resume { Some(s) => runtime.resume(s, launch), None => runtime.launch(launch) }?;
    let mut events = vec![];
    loop {
        events.extend(runtime.progress(&mut run)?);
        let state = runtime.poll(&mut run)?;
        if state != State::Running {
            events.extend(runtime.progress(&mut run)?);
            return Ok((runtime.report(&run)?, events));
        }
        if launch.deadline.is_some_and(|d| Instant::now() >= d) { runtime.cancel(&mut run)?; return Err("native probe deadline; original session retained".into()); }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn antigravity_tools_completed(events: &[Event]) -> Result<(), String> {
    let statuses: Vec<_> = events.iter().filter_map(|event| match event {
        Event::ToolEnd { status, .. } => Some(status.as_deref()), _ => None,
    }).collect();
    if statuses.is_empty() || statuses.iter().any(|status| *status != Some("DONE")) {
        return Err("native Antigravity tool completion not proven; inspect retained log".into());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let name = std::env::args().nth(1).unwrap_or_else(|| "codex".into());
    let runtime = agent::runtime(&name).ok_or("unknown runtime")?;
    if std::env::args().nth(2).as_deref()==Some("limits") {
        let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        println!("{}",json!({"runtime":name,"limits":runtime.account_limits(None,now)}));return Ok(());
    }
    let root = std::env::temp_dir().join(format!("agentq-246-native-{}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|e|e.to_string())?;
    if !std::process::Command::new("git").args(["init","--quiet"]).arg(&root).status().map_err(|e|e.to_string())?.success() { return Err("probe repository initialization failed".into()); }
    let mut l = Launch::new(&root, root.join("plain.jsonl"), "Reply exactly PLAIN_OK. Do not use tools.", "agentq-qualification");
    l.deadline = Some(Instant::now()+Duration::from_secs(120));
    if name == "codex" { l.policy = Policy { write: Scope::None, approval: Approval::Deny, builtin: Builtin::Off, isolated: true, ..Policy::default() }; }
    if name=="antigravity" {l.policy.approval=Approval::Bypass;l.input="Use a native shell tool to print EVENT_TOOL_OK, then reply exactly PLAIN_OK.".into();}
    let (plain, observed) = finish(runtime, &l, None)?;
    if plain.outcome != "ok" { return Err(format!("plain outcome {}: {}", plain.outcome, plain.error)); }
    if name != "codex" {
        let mut counts=std::collections::BTreeMap::<&str,usize>::new();
        for event in &observed {let kind=match event {Event::Text{..}=>"Text",Event::Message{..}=>"Message",Event::ToolStart{..}=>"ToolStart",Event::ToolEnd{..}=>"ToolEnd",Event::Quota(_)=>"Quota",Event::Context{..}=>"Context",Event::TurnStart=>"TurnStart",Event::TurnEnd{..}=>"TurnEnd",Event::Progress=>"Progress",Event::Unsupported{..}=>"Unsupported",Event::Image{..}=>"Image",Event::Activity=>"Activity",Event::Stalled{..}=>"Stalled"};*counts.entry(kind).or_default()+=1;}
        let statuses: Vec<_> = observed.iter().filter_map(|event| match event { Event::ToolEnd { status, .. } => Some(status), _ => None }).collect();
        println!("{}",json!({"runtime":name,"outcome":plain.outcome,"session":plain.session,"limits":plain.limits,"events":counts,"tool_end_statuses":statuses,"log":l.log,"delta_capability":runtime.descriptor().has("turn-deltas")}));
        if name == "antigravity" { antigravity_tools_completed(&observed)?; }
        return Ok(());
    }
    let server = root.join("relay.py");
    std::fs::write(&server, r#"import json,os,sys
from pathlib import Path
for line in sys.stdin:
    m=json.loads(line); method=m.get('method'); result=None
    if method=='initialize': result={'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'fixture-relay','version':'1'}}
    elif method=='tools/list': result={'tools':[{'name':'fixture_probe','description':'Check the invented per-turn environment sentinel','inputSchema':{'type':'object','properties':{}}}]}
    elif method=='tools/call':
        ok=os.environ.get('PROBE_SENTINEL')=='invented-246'
        Path(__file__).with_suffix('.proof').write_text(json.dumps({'env_ok':ok}))
        result={'content':[{'type':'text','text':'probe-env-ok' if ok else 'probe-env-missing'}],'isError':not ok}
    elif method=='ping': result={}
    if 'id' in m: print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':result}),flush=True)
"#).map_err(|e|e.to_string())?;
    let mcp = root.join("mcp.json");
    std::fs::write(&mcp,json!({"mcpServers":{"fixture":{"command":"python","args":[server],"env":{"PROBE_SENTINEL":"invented-246"}}}}).to_string()).map_err(|e|e.to_string())?;
    let instructions = root.join("instructions.txt");
    std::fs::write(&instructions,"Call the fixture_probe MCP tool, then reply exactly CODEX_RUNTIME_OK after probe-env-ok. Do not use other tools.").map_err(|e|e.to_string())?;
    l.input = "Call fixture_probe and follow the developer instructions.".into(); l.log = root.join("advanced.log");
    l.mcp = Some(mcp); l.instructions = Some(instructions); l.deltas = true;
    l.deadline = Some(Instant::now()+Duration::from_secs(120));
    let (report, events) = finish(runtime, &l, None)?;
    let proof: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(server.with_extension("proof")).map_err(|_|"native MCP tool was not called")?).map_err(|_|"MCP proof invalid")?;
    if report.outcome != "ok" || proof["env_ok"] != true || report.text.as_deref() != Some("CODEX_RUNTIME_OK") { return Err("native relay turn failed its output/environment assertions".into()); }
    let deltas = events.iter().filter(|e|matches!(e,Event::Text{..})).count();
    let tools = events.iter().filter(|e|matches!(e,Event::ToolStart{..})).count();
    if deltas == 0 || tools == 0 { return Err("native delta/tool events missing".into()); }
    println!("{}",json!({"runtime":name,"plain":"ok","advanced":report.outcome,"env_ok":true,"deltas":deltas,"tools":tools,"limits":report.limits,"catalogue_models":runtime.catalogue(None)?.len(),"account_limits":runtime.account_limits(None, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64)}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // #246 A5/P2: a final model success must not hide denied or unobserved tool execution.
    #[test]
    fn antigravity_probe_requires_successful_tool_completion() {
        assert!(antigravity_tools_completed(&[]).is_err());
        for status in [None, Some("ERROR"), Some("CANCELLED")] {
            let denied = Event::ToolEnd { id: None, status: status.map(String::from) };
            assert!(antigravity_tools_completed(&[denied]).is_err());
        }
        let done = Event::ToolEnd { id: None, status: Some("DONE".into()) };
        assert!(antigravity_tools_completed(&[done.clone()]).is_ok());
        assert!(antigravity_tools_completed(&[done, Event::ToolEnd { id: None, status: Some("ERROR".into()) }]).is_err());
    }
}
