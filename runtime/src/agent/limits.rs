//! All native quota windows. Missing fields remain unknown rather than becoming zero.
use super::*;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Window {
    pub window: Option<String>,
    /// Used fraction (1 means exhausted).
    pub used: Option<f64>,
    pub resets_at: Option<u64>,
    pub event_id: Option<String>,
    /// Explicit native status or measured exhaustion; None when neither is reported.
    #[serde(default)]
    pub exhausted: Option<bool>,
    pub unknown: Option<String>,
}

pub(super) fn unknown(reason: &str) -> Vec<Window> {
    vec![Window { window: None, used: None, resets_at: None, event_id: None, exhausted: None, unknown: Some(reason.into()) }]
}

fn window(name: Option<String>, used: Option<f64>, reset: Option<u64>, id: Option<String>) -> Window {
    let used = used.filter(|u| u.is_finite() && *u >= 0.0);
    let partial = used.is_none() || reset.is_none() || name.is_none();
    Window { window: name, used, exhausted: used.map(|u|u>=1.0), resets_at: reset, event_id: id, unknown: partial.then(|| "partial-snapshot".into()) }
}

pub(super) fn reported(client: Client, v: &Value, now: u64) -> Option<Vec<Window>> {
    let id = v["event_id"].as_str().or(v["uuid"].as_str()).or(v["id"].as_str()).map(String::from);
    let claude_info = if v["type"] == "rate_limit_event" {Some(&v["rate_limit_info"])}
        else {v.get("quotaLimits").filter(|v|v.is_object()).or_else(||v.pointer("/apiErrorParams/rate_limit_info").filter(|v|v.is_object()))};
    if let Some(i) = claude_info.filter(|_|client == Client::Claude) {
        let mut windows = vec![];
        if let Some(all) = i["unifiedWindows"].as_object() {
            for (name, w) in all {
                windows.push(claude_window(Some(name.clone()), w, id.clone()));
            }
        }
        if let Some(name) = i["rateLimitType"].as_str() {
            let top = claude_window(Some(name.into()), i, id.clone());
            if let Some(at) = windows.iter().position(|w| w.window.as_deref() == Some(name)) { windows[at] = top; }
            else { windows.push(top); }
        }
        return Some(if windows.is_empty() {vec![claude_window(None,i,id)]} else {windows});
    }
    if client != Client::Codex { return None; }
    let mut windows = vec![];
    for (_, r) in codex_groups(v)? {
        windows.extend(codex_windows(r, id.clone(), now));
    }
    Some(if windows.is_empty() { unknown("partial-snapshot") } else { windows })
}

fn claude_window(name: Option<String>, info: &Value, id: Option<String>) -> Window {
    let mut w=window(name,info["utilization"].as_f64(),info["resetsAt"].as_u64(),id);
    w.exhausted=match (info["status"].as_str(),w.exhausted) {(Some("rejected"),_)|(_,Some(true))=>Some(true),(Some("allowed"|"allowed_warning"),_)=>Some(false),(_,used)=>used};
    w
}

fn codex_groups(v: &Value) -> Option<Vec<(&str, &Value)>> {
    if let Some(groups) = v["result"].get("rateLimitsByLimitId").or(v["params"].get("rateLimitsByLimitId")).or(v.get("rateLimitsByLimitId")) {
        return Some(match groups.as_object() { Some(g) => g.iter().map(|(id, r)| (id.as_str(), r)).collect(), None => vec![("codex", groups)] });
    }
    let r = [v.get("rate_limits"), v["payload"].get("rate_limits"), v["msg"].get("rate_limits"), v["params"].get("rateLimits"), v["result"].get("rateLimits"), v.get("rateLimits")].into_iter().flatten().next()?;
    Some(vec![(r["limit_id"].as_str().or(r["limitId"].as_str()).unwrap_or("codex"), r)])
}

fn codex_windows(r: &Value, id: Option<String>, now: u64) -> Vec<Window> {
    if !r.is_object() { return unknown("limit-group-unreadable"); }
    let mut windows = vec![];
    for key in ["primary", "secondary"] {
        let Some(w) = r.get(key).filter(|w| !w.is_null()) else { continue; };
        if !w.is_object() { windows.push(window(Some(key.into()),None,None,id.clone())); continue; }
        let minutes = w["window_minutes"].as_u64().or(w["windowDurationMins"].as_u64());
        let used = w["used_percent"].as_f64().or(w["usedPercent"].as_f64()).map(|u| u / 100.0);
        let reset = w["resets_at"].as_u64().or(w["resetsAt"].as_u64()).or_else(|| w["resets_in_seconds"].as_u64().and_then(|s| now.checked_add(s)));
        windows.push(window(minutes.map(|m| format!("{m}m")), used, reset, id.clone()));
    }
    if windows.is_empty() { unknown("partial-snapshot") } else { windows }
}

/// Merge independent native snapshots, never key Codex groups by their shared window lengths.
pub(super) fn update(client: Client, v: &Value, now: u64, all: &mut BTreeMap<String, Vec<Window>>) -> bool {
    if client == Client::Codex {
        let Some(groups) = codex_groups(v) else { return false; };
        if groups.is_empty() { all.clear(); all.insert("codex".into(), unknown("partial-snapshot")); }
        for (id, r) in groups {
            let event_id = v["event_id"].as_str().or(v["uuid"].as_str()).or(v["id"].as_str()).map(String::from);
            all.insert(id.into(), codex_windows(r, event_id, now));
        }
        return true;
    }
    let Some(windows) = reported(client, v, now) else { return false; };
    for w in windows { all.insert(w.window.clone().unwrap_or_default(), vec![w]); }
    true
}

pub(super) fn read(client: Client, events: &[Value], rollout: Option<&Path>) -> Vec<Window> {
    let now = (unix_ms() / 1000).max(0) as u64;
    let mut all=BTreeMap::new();
    for v in events { update(client, v, now, &mut all); }
    if !all.is_empty() { return all.into_values().flatten().collect(); }
    if let Some(path) = rollout {
        let text = match std::fs::read_to_string(path) { Ok(t) => t, Err(_) => return unknown("evidence-unreadable") };
        for v in text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) { update(client, &v, now, &mut all); }
        if !all.is_empty() { return all.into_values().flatten().collect(); }
    }
    unknown("not-reported")
}

fn exchange(server: &mut app::AppServer, session: &mut app::AppSession, request: Value, until: Instant) -> Res<Value> {
    server.send(&request, until)?;
    loop {
        let m=server.receive(until)?.ok_or("native read deadline")?;
        if m["method"].is_string() && m.get("id").is_some() {
            server.send(&json!({"id":m["id"],"error":{"code":-32601,"message":"read-only session"}}),until)?;
            continue;
        }
        session.observe(&m)?;
        if m["id"]==request["id"] && m.get("method").is_none() {
            return if m.get("error").is_some() {Err("native read refused".into())} else {Ok(m)};
        }
    }
}

/// One bounded native read session with no thread or turn. Credentials stay in the client's home.
fn codex_read<T>(home: Option<&Path>, executable: Option<&Path>, read: impl FnOnce(&mut app::AppServer, &mut app::AppSession, Instant)->Res<T>) -> Res<T> {
    let root=std::env::current_dir().map_err(|_|"working directory unavailable")?;
    let spec=Spec {client:Client::Codex,root,session:"-".into(),resume:false,args:vec![],env:vec![],home:home.map(Path::to_path_buf)};
    let log=std::env::temp_dir().join(format!("agentq-read-{}.log",crate::incident::uuid()));
    let exe=match executable {Some(path)=>super::codex_native(path.to_path_buf())?,None=>adapter::runtime("codex").ok_or("runtime missing")?.executable(None)?};
    let mut server=app::AppServer::spawn(app::app_server_command(Command::new(exe),&spec)?,&log,&Process::default())?;
    let result=(|| {
        let until=Instant::now()+Duration::from_secs(30);
        let mut session=app::AppSession::new(&spec)?;
        let initialize=session.initialize(json!({"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}))?;
        exchange(&mut server,&mut session,initialize,until)?;
        server.send(&session.initialized()?,until)?;
        read(&mut server,&mut session,until)
    })();
    let stopped=server.close(); drop(server); let _=std::fs::remove_file(log);
    let result=result?; stopped?; Ok(result)
}

pub(super) fn catalogue(home: Option<&Path>, executable: Option<&Path>) -> Res<Vec<Model>> {
    codex_read(home, executable, |server,session,until| {
        let mut models=vec![]; let mut cursor=Value::Null;
        for _ in 0..100 {
            let request=session.request("model/list",json!({"cursor":cursor,"limit":100,"includeHidden":false}))?;
            let reply=exchange(server,session,request,until)?;
            for m in reply["result"]["data"].as_array().ok_or("catalogue data missing")? {
                let id=m["model"].as_str().filter(|m|model_code(m)).ok_or("catalogue model invalid")?;
                models.push(Model {id:id.into(),name:m["displayName"].as_str().unwrap_or(id).into()});
            }
            let next=&reply["result"]["nextCursor"];
            if next.is_null() {return Ok(models);}
            if !next.is_string() || *next==cursor {return Err("catalogue cursor invalid".into());}
            cursor=next.clone();
        }
        Err("catalogue page bound exceeded".into())
    })
}

pub(super) fn account(client: Client, home: Option<&Path>, now: i64) -> Vec<Window> {
    let explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let Some(home) = home.map(Path::to_path_buf).or_else(|| base_home(client)) else { return unknown("home-unavailable"); };
    if client != Client::Codex { return oauth_account(client, &home, explicit, now); }
    let path = home.join("auth.json");
    let before = match credential_snapshot(&path) { Ok(b) => b, Err(_) => return unknown("credential-unreadable") };
    let response = codex_read(Some(&home), None, |server,session,until| {let request=session.request("account/rateLimits/read",json!({}))?;exchange(server,session,request,until)});
    if credential_snapshot(&path).ok().as_ref() != Some(&before) { return unknown("credential-changed"); }
    match response {
        Ok(v) => reported(client, &v, now.max(0) as u64).unwrap_or_else(|| unknown("not-reported")),
        Err(_) => unknown("native-account-read-failed"),
    }
}

fn credential_snapshot(path: &Path) -> Res<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes)=>Ok(Some(bytes)),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>Ok(None),
        Err(_)=>Err("credential-unreadable".into()),
    }
}

pub(super) fn identity(client: Client, home: Option<&Path>) -> Option<String> {
    let explicit=home.is_some() || std::env::var_os(client.home_var()).is_some();
    let home=home.map(Path::to_path_buf).or_else(||base_home(client))?;
    match client {
        Client::Codex=>{
            let path=home.join("auth.json"); let before=credential_snapshot(&path).ok()?;
            let reply=codex_read(Some(&home),None,|server,session,until|{
                let request=session.request("account/read",json!({"refreshToken":false}))?;
                exchange(server,session,request,until)
            }).ok()?;
            if credential_snapshot(&path).ok()? != before {return None;}
            codex_identity(&reply["result"]["account"])
        }
        Client::Antigravity=>{
            let before=read_credentials(client,&home,explicit).ok()?;
            let credential=token_text(&before).ok()?;
            let oauth=credential.get("token").filter(|v|v.is_object()).unwrap_or(&credential);
            let token=access_token(client,oauth,(unix_ms()/1000).max(0)).ok()?;
            let load=https(&code_assist_call(token,"cloudcode-pa.googleapis.com","loadCodeAssist",json!({"metadata":{"ideType":"ANTIGRAVITY"}}))).ok()?;
            if read_credentials(client,&home,explicit).ok()?.as_str()!=before {return None;}
            project_identity(&load)
        }
        Client::Claude=>None,
    }
}

fn codex_identity(account: &Value) -> Option<String> {
    ["email","chatgptAccountId"].into_iter().find_map(|key|account[key].as_str().filter(|s|!s.is_empty()).map(String::from))
}

fn project_identity(load: &Value) -> Option<String> {
    load["cloudaicompanionProject"].as_str().filter(|s|!s.is_empty()).map(String::from)
}

fn reset(value: &Value) -> Option<u64> {
    let s = value.as_str()?;
    let base = s.get(..19)?;
    let mut tail = s.get(19..)?;
    if let Some(fraction) = tail.strip_prefix('.') { tail = fraction.trim_start_matches(|c:char|c.is_ascii_digit()); }
    let seconds = capture::time_ms(&format!("{base}Z"))? / 1000;
    if tail == "Z" { return Some(seconds); }
    if tail.len() != 6 || !matches!(tail.as_bytes()[0], b'+' | b'-') || tail.as_bytes()[3] != b':' { return None; }
    let hours: u64 = tail[1..3].parse().ok()?; let minutes: u64 = tail[4..6].parse().ok()?;
    if hours > 23 || minutes > 59 { return None; }
    let offset = hours * 3600 + minutes * 60;
    if tail.starts_with('+') { seconds.checked_sub(offset) } else { seconds.checked_add(offset) }
}

fn claude_windows(v: &Value) -> Vec<Window> {
    let Some(fields) = v.as_object() else { return unknown("partial-snapshot"); };
    let mut windows = vec![];
    for (name, w) in fields {
        if (name.starts_with("five_hour") || name.starts_with("seven_day")) && name != "seven_day_breakdown" && !w.is_null() {
            windows.push(window(Some(name.clone()), w["utilization"].as_f64().map(|n|n/100.0), reset(&w["resets_at"]), None));
        }
    }
    if let Some(list) = v.get("limits").filter(|l| !l.is_null()) {
        if let Some(list) = list.as_array() {
            for (index, l) in list.iter().enumerate() {
                let mut name = match l["group"].as_str() { Some("session") => "five_hour".to_string(), Some("weekly") => "seven_day".to_string(), _ => format!("limits[{index}]") };
                let scope = l.get("scope").filter(|s|!s.is_null());
                let model = scope.and_then(|s|s.get("model")).filter(|m|!m.is_null());
                let mut invalid = name.starts_with("limits[") || scope.is_some_and(|s|!s.is_object() || s.as_object().is_some_and(|s|s.iter().any(|(k,v)|k!="model" && !v.is_null())));
                if let Some(model) = model {
                    if let Some(model) = model["display_name"].as_str().or(model["id"].as_str()).filter(|m|!m.is_empty()) { name.push('_'); name.push_str(&model.to_ascii_lowercase().replace(' ', "_")); }
                    else { invalid = true; }
                }
                windows.retain(|w|w.window.as_deref()!=Some(&name));
                windows.push(window(Some(name), if invalid {None} else {l["percent"].as_f64().map(|n|n/100.0)}, reset(&l["resets_at"]), None));
            }
        } else { windows.extend(unknown("limits-list-unreadable")); }
    }
    if windows.is_empty() { unknown("not-reported") } else { windows }
}

fn https(config: &str) -> Res<Value> {
    let executable = which_in("curl", &std::env::var_os("PATH").unwrap_or_default());
    let mut command = Command::new(executable);
    command.args(["-q","--silent","--fail","--proto","=https","--max-time","20","--max-filesize","1048576","--config","-"]);
    let output = crate::process::exchange("agent.account-limits", &mut command, config.as_bytes(), Duration::from_secs(25)).map_err(|_|"native-account-read-failed")?;
    if output.stdout.len() > 1024*1024 { return Err("native-account-response-too-large".into()); }
    serde_json::from_slice(&output.stdout).map_err(|_|"native-account-response-invalid".into())
}

fn token_text(text: &str) -> Res<Value> {
    let text = text.trim();
    let decoded;
    let text = if let Some(encoded) = text.strip_prefix("go-keyring-base64:") {
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut bytes = vec![]; let (mut bits,mut available) = (0u32,0u32);
        for c in encoded.trim().bytes().take_while(|c|*c!=b'=') {
            let n = alphabet.iter().position(|b|*b==c).ok_or("credential-invalid")?;
            bits=(bits<<6)|n as u32; available+=6;
            if available>=8 { available-=8; bytes.push((bits>>available) as u8); }
        }
        if adapter::b64(&bytes)!=encoded.trim() {return Err("credential-invalid".into());}
        decoded=String::from_utf8(bytes).map_err(|_|"credential-invalid")?; decoded.as_str()
    } else {text};
    serde_json::from_str(text).map_err(|_|"credential-invalid".into())
}

fn read_credentials(client: Client, home: &Path, explicit: bool) -> Res<String> {
    #[cfg(target_os="macos")]
    if client == Client::Claude || !explicit {
        use std::sync::atomic::{AtomicBool,Ordering};
        static PROMPTED: AtomicBool = AtomicBool::new(false);
        if PROMPTED.load(Ordering::Relaxed) {return Err("keychain-would-prompt".into());}
        let service = if client==Client::Claude {
            let suffix=if explicit {format!("-{}", &crate::sha256_hex(home.to_string_lossy().as_bytes())[..8])} else {String::new()};
            format!("Claude Code-credentials{suffix}")
        } else {"gemini".into()};
        let mut command=Command::new("/usr/bin/security"); command.args(["find-generic-password","-s",&service]);
        if client==Client::Antigravity {command.args(["-a","antigravity"]);}
        command.arg("-w");
        let output=match crate::process::output("agent.account-credential",&mut command,Duration::from_secs(5)) {
            Ok(o)|Err(crate::process::Failure::Exit(o))=>o,
            Err(crate::process::Failure::Timeout)=>{PROMPTED.store(true,Ordering::Relaxed);return Err("keychain-would-prompt".into());},
            Err(_)=>return Err("credential-unreadable".into()),
        };
        match output.status.code() {
            Some(0)=>return String::from_utf8(output.stdout).map(|s|s.trim().into()).map_err(|_|"credential-invalid".into()),
            Some(44)=>{}, Some(36)=>{PROMPTED.store(true,Ordering::Relaxed);return Err("keychain-would-prompt".into());},
            _=>return Err("credential-unreadable".into()),
        }
    }
    let file=if client==Client::Claude {home.join(".credentials.json")} else {home.join("antigravity-oauth-token")};
    // shortcut: Windows keyring-only logins read unknown; add its native reader when qualified.
    if let Ok(text)=std::fs::read_to_string(file) {return Ok(text);}
    if client==Client::Antigravity && !explicit {
        if let Some(parent)=home.parent() {if let Ok(text)=std::fs::read_to_string(parent.join("oauth_creds.json")) {return Ok(text);}}
    }
    Err("credential-unreadable".into())
}

fn access_token(client: Client, oauth: &Value, now: i64) -> Res<&str> {
    let token=if client==Client::Claude {oauth["accessToken"].as_str()} else {oauth["access_token"].as_str()}.filter(|s|!s.is_empty() && s.bytes().all(|b|b.is_ascii_alphanumeric() || b"_.-".contains(&b))).ok_or("oauth-unavailable")?;
    let expiry=if client==Client::Claude {oauth["expiresAt"].as_i64().map(|n|n/1000)} else {reset(&oauth["expiry"]).and_then(|n|i64::try_from(n).ok()).or_else(||oauth["expiry_date"].as_i64().map(|n|n/1000))};
    if expiry.is_some_and(|end|end<=now) {return Err("credential-expired".into());}
    Ok(token)
}

/// Native routes moved from the consumer's read-only usage adapters; no refresh, price, ranking or fleet policy.
fn oauth_account(client: Client, home: &Path, explicit: bool, now: i64) -> Vec<Window> {
    let result = (|| -> Res<Vec<Window>> {
        let before=read_credentials(client,home,explicit)?;
        let credential=token_text(&before)?;
        let oauth=if client==Client::Claude {&credential["claudeAiOauth"]} else {credential.get("token").filter(|v|v.is_object()).unwrap_or(&credential)};
        let token=access_token(client,oauth,now)?;
        let response=if client==Client::Claude {
            https(&format!("url = \"https://api.anthropic.com/api/oauth/usage\"\nheader = \"Authorization: Bearer {token}\"\nheader = \"anthropic-beta: oauth-2025-04-20\"\n")).map(|v|claude_windows(&v))
        } else {code_assist(token,https)};
        if read_credentials(client,home,explicit).ok().as_deref()!=Some(before.as_str()) {return Err("credential-changed".into());}
        response
    })();
    result.unwrap_or_else(|e|unknown(&e))
}

fn code_assist_call(token: &str, host: &str, method: &str, body: Value) -> String {
    format!("url = \"https://{host}/v1internal:{method}\"\nheader = \"Authorization: Bearer {token}\"\nheader = \"Content-Type: application/json\"\nheader = \"User-Agent: antigravity/hub/2.28.2\"\ndata = {}\n",json!(body.to_string()))
}

fn code_assist(token: &str, mut read: impl FnMut(&str)->Res<Value>) -> Res<Vec<Window>> {
    let load=read(&code_assist_call(token,"cloudcode-pa.googleapis.com","loadCodeAssist",json!({"metadata":{"ideType":"ANTIGRAVITY"}})))?;
    let host=match load.pointer("/currentTier/usesGcpTos") {Some(Value::Bool(true))=>"cloudcode-pa.googleapis.com",None|Some(Value::Bool(false))=>"daily-cloudcode-pa.googleapis.com",_=>return Err("quota-tier-unknown".into())};
    let body=match load.get("cloudaicompanionProject").filter(|p|!p.is_null()) {Some(Value::String(p))=>json!({"project":p}),None=>json!({}),_=>return Err("quota-project-unknown".into())};
    let quota=read(&code_assist_call(token,host,"retrieveUserQuotaSummary",body))?;
    let mut windows=vec![];
    for group in quota["groups"].as_array().ok_or("quota-groups-unreadable")? {
        let Some(buckets)=group["buckets"].as_array().filter(|b|!b.is_empty()) else {windows.extend(unknown("quota-buckets-unreadable"));continue;};
        for b in buckets {
            windows.push(window(b["bucketId"].as_str().map(String::from),b["remainingFraction"].as_f64().filter(|f|(0.0..=1.0).contains(f)).map(|n|1.0-n),reset(&b["resetTime"]),None));
        }
    }
    Ok(if windows.is_empty() {unknown("not-reported")} else {windows})
}

#[cfg(test)]
mod tests {
    use super::*;
    // #258 A3/P2: rejection retains name/reset without manufacturing utilization.
    #[test]
    fn rejected_windows_without_utilization() {
        for row in [
            json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day","resetsAt":1900000000}}),
            json!({"type":"assistant","quotaLimits":{"status":"rejected","rateLimitType":"seven_day","resetsAt":1900000000}}),
            json!({"type":"assistant","apiError":"usage_limit_reached","apiErrorParams":{"rate_limit_info":{"status":"rejected","rateLimitType":"seven_day","resetsAt":1900000000}}}),
        ] {
            let runtime=adapter::runtime("claude").unwrap();
            let log=row.to_string();
            let w=runtime.log_limits(&log,None,None);
            assert_eq!(w[0].window.as_deref(),Some("seven_day"));
            assert_eq!(w[0].used,None);
            assert_eq!(serde_json::to_value(&w[0]).unwrap()["exhausted"],true);
            assert_eq!(runtime.classify(&log).reached,Some(Some(1900000000)));
            assert_eq!(w[0].resets_at,Some(1900000000));
            let worked=format!("{}\n{log}",json!({"type":"assistant","message":{"content":[{"type":"text","text":"work"}]}}));
            assert_eq!(runtime.classify(&worked).reached,None,"after work is not a start refusal");
        }
        for (info,expected) in [
            (json!({"status":"rejected","utilization":0.2}),Some(true)),
            (json!({"status":"allowed","utilization":1.0}),Some(true)),
            (json!({"status":"allowed_warning"}),Some(false)),
            (json!({"utilization":0.2}),Some(false)),
            (json!({}),None),
        ] {assert_eq!(claude_window(None,&info,None).exhausted,expected);}
    }

    // #258 A4: optional null windows/scopes are absent; malformed siblings remain unknown.
    #[test]
    fn claude_null_windows_and_scope_dimensions_are_absent() {
        let windows=claude_windows(&json!({"five_hour":null,"seven_day_sonnet":null,"seven_day":"malformed","limits":[{"group":"session","percent":25,"resets_at":"2026-11-09T00:00:00Z","scope":{"model":null,"tier":null}},{"group":"weekly","percent":30,"scope":{"model":false}}]}));
        assert_eq!(windows.len(),2,"null legacy windows are not partial snapshots");
        assert_eq!(windows[0].used,Some(0.25));assert_eq!(windows[0].unknown,None);
        assert_eq!(windows[1].used,None);assert!(windows[1].unknown.is_some());
        assert!(claude_windows(&json!({"five_hour":false}))[0].unknown.is_some());
    }

    // #258 A5: only explicit identity fields leave the adapter, never a token or a fabricated identity.
    #[test]
    fn identity_fields_are_allowlisted() {
        for (account,expected) in [
            (json!({"email":"invented@example.invalid","chatgptAccountId":"invented-id","access_token":"invented-token"}),Some("invented@example.invalid")),
            (json!({"email":null,"chatgptAccountId":"invented-id"}),Some("invented-id")),
            (json!({"email":"","chatgptAccountId":false,"access_token":"invented-token"}),None),
            (Value::Null,None),
        ] {assert_eq!(codex_identity(&account).as_deref(),expected);}
        for value in [Value::Null,json!(false),json!("")] {assert_eq!(project_identity(&json!({"cloudaicompanionProject":value,"access_token":"invented-token"})),None);}
        assert_eq!(project_identity(&json!({"cloudaicompanionProject":"invented-project","access_token":"invented-token"})).as_deref(),Some("invented-project"));
        assert_eq!(adapter::runtime("claude").unwrap().account_identity(None),None);
    }

    // #246 A4/P2: logs, reports and rollout fallback retain independent groups, even with equal lengths.
    #[test]
    fn codex_group_updates_keep_sibling_windows() {
        let row=|group,used,id|json!({"event_id":id,"type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":group,"primary":{"used_percent":used,"window_minutes":300,"resets_at":60},"secondary":{"used_percent":90,"window_minutes":10080,"resets_at":90}}}});
        let mut rows=vec![row("codex",20,"a"),row("other",30,"b"),row("codex",40,"c")];
        rows.push(json!({"event_id":"d","rate_limits":{"limit_id":"other","primary":{"window_minutes":300},"secondary":{"used_percent":80,"window_minutes":10080,"resets_at":100}}}));
        let log=rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let windows=adapter::runtime("codex").unwrap().log_limits(&log,None,None);
        assert_eq!(windows.len(),4,"{windows:?}");
        assert_eq!(windows.iter().map(|w|w.used).collect::<Vec<_>>(),vec![Some(0.4),Some(0.9),None,Some(0.8)]);
        assert_eq!(windows[0].event_id.as_deref(),Some("c"));assert_eq!(windows[2].event_id.as_deref(),Some("d"));
        assert_eq!(windows[2].unknown.as_deref(),Some("partial-snapshot"));
        assert_eq!(Report::read(Client::Codex,&log,&rows,None).limits,windows);
        let root=crate::scratch("quota-groups");let path=root.join("rollout.jsonl");std::fs::write(&path,&log).unwrap();
        assert_eq!(read(Client::Codex,&[],Some(&path)),windows);
        std::fs::remove_dir_all(root).unwrap();
    }

    // #246 A4: retain every window and event identity; partial data must never look unused.
    #[test]
    fn all_windows_keep_unknowns_and_ids() {
        let c = json!({"type":"rate_limit_event","uuid":"native-id","rate_limit_info":{"rateLimitType":"five_hour","utilization":0.4,"resetsAt":50,"unifiedWindows":{"five_hour":{"utilization":0.3,"resetsAt":49},"seven_day":{"utilization":0.7}}}});
        let w = reported(Client::Claude, &c, 1).unwrap();
        assert_eq!(w.len(), 2); assert_eq!(w[0].used, Some(0.4)); assert_eq!(w[1].resets_at, None);
        assert_eq!(w[1].unknown.as_deref(), Some("partial-snapshot")); assert_eq!(w[0].event_id.as_deref(), Some("native-id"));
        let c = json!({"result":{"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":20,"windowDurationMins":300,"resetsAt":60},"secondary":{"usedPercent":90,"windowDurationMins":10080,"resetsAt":90}},"other":{"primary":{"windowDurationMins":5}}}}});
        let w = reported(Client::Codex, &c, 1).unwrap();
        assert_eq!(w.len(), 3); assert_eq!(w[1].used, Some(0.9)); assert_eq!(w[2].used, None);
        let w=reported(Client::Codex,&json!({"result":{"rateLimitsByLimitId":{"ok":{"primary":{"usedPercent":20,"resetsAt":50,"windowDurationMins":300},"secondary":"bad"},"bad":"bad"}}}),1).unwrap();
        assert_eq!(w.len(),3);assert!(w.iter().filter(|w|w.unknown.is_some()).count()==2);
        let w = claude_windows(&json!({"five_hour":null,"seven_day":{"utilization":61.0,"resets_at":"2026-11-09T05:00:00.892086+00:00"}}));
        assert_eq!(w.len(),1); assert_eq!(w[0].used,Some(0.61)); assert!(w[0].resets_at.is_some());
        let w = claude_windows(&json!({"seven_day_fable":{"utilization":2,"resets_at":"2026-11-09T00:00:00Z"},"limits":[{"group":"weekly","percent":99,"resets_at":"2026-11-09T01:00:00+01:00","scope":{"model":{"display_name":"Fable"}}},{"group":"session","percent":0,"scope":{"extra":true}}]}));
        assert_eq!(w.len(),2); assert_eq!(w[0].used,Some(0.99)); assert!(w[1].used.is_none());
        assert_eq!(reset(&json!("2026-11-09T01:00:00+01:00")),reset(&json!("2026-11-09T00:00:00Z")));
    }

    // #246 A4: retain native Code Assist endpoint routing and every model bucket from the consumer adapter.
    #[test]
    fn code_assist_keeps_model_buckets_and_named_unknowns() {
        let mut calls=0;
        let w=code_assist("invented-token", |config| {
            calls+=1;
            if calls==1 {assert!(config.contains(":loadCodeAssist"));return Ok(json!({"cloudaicompanionProject":"invented-project"}));}
            assert!(config.contains("daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary"));
            assert!(config.contains("antigravity/hub/2.28.2"));
            Ok(json!({"groups":[{"buckets":[{"bucketId":"gemini-5h","remainingFraction":1.0,"resetTime":"2026-11-09T00:00:00Z"},{"bucketId":"gemini-weekly","remainingFraction":0.0}]},{"buckets":[{"bucketId":"3p-weekly"}]}]}))
        }).unwrap();
        assert_eq!(calls,2); assert_eq!(w.len(),3); assert_eq!(w[0].used,Some(0.0)); assert_eq!(w[1].used,Some(1.0)); assert_eq!(w[2].used,None);
        assert_eq!(token_text("go-keyring-base64:eyJhY2Nlc3NfdG9rZW4iOiJpbnZlbnRlZCJ9").unwrap()["access_token"],"invented");
        assert!(token_text("go-keyring-base64:!!").is_err());
    }
}
