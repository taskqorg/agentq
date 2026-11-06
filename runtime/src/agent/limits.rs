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

pub(crate) fn window(name: Option<String>, used: Option<f64>, reset: Option<u64>, id: Option<String>) -> Window {
    let used = used.filter(|u| u.is_finite() && *u >= 0.0);
    let partial = used.is_none() || reset.is_none() || name.is_none();
    Window { window: name, used, exhausted: used.map(|u|u>=1.0), resets_at: reset, event_id: id, unknown: partial.then(|| "partial-snapshot".into()) }
}

/// Merge independent native snapshots, never key Codex groups by their shared window lengths.

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

/// One bounded native read session with no thread or turn. Credentials stay in the client's home.

pub(crate) fn reset(value: &Value) -> Option<u64> {
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

pub(crate) fn https(config: &str) -> Res<Value> {
    let executable = which_in("curl", &std::env::var_os("PATH").unwrap_or_default());
    let mut command = Command::new(executable);
    command.args(["-q","--silent","--fail","--proto","=https","--max-time","20","--max-filesize","1048576","--config","-"]);
    let output = crate::process::exchange("agent.account-limits", &mut command, config.as_bytes(), Duration::from_secs(25)).map_err(|_|"native-account-read-failed")?;
    if output.stdout.len() > 1024*1024 { return Err("native-account-response-too-large".into()); }
    serde_json::from_slice(&output.stdout).map_err(|_|"native-account-response-invalid".into())
}

pub(crate) fn token_text(text: &str) -> Res<Value> {
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

/// Native routes moved from the consumer's read-only usage adapters; no refresh, price, ranking or fleet policy.
pub(crate) fn oauth_account(client: Client, home: &Path, explicit: bool, now: i64) -> Vec<Window> {
    let result = (|| -> Res<Vec<Window>> {
        let before=read_credentials(client,home,explicit)?;
        let credential=token_text(&before)?;
        let token=(protocol(client).oauth_token)(&credential,now)?;
        let response=(protocol(client).oauth_usage)(token);
        if read_credentials(client,home,explicit).ok().as_deref()!=Some(before.as_str()) {return Err("credential-changed".into());}
        response
    })();
    result.unwrap_or_else(|e|unknown(&e))
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
            json!({"type":"assistant","quotaLimits":{"status":"rejected","resetsAt":1900000000},"apiErrorParams":{"rate_limit_info":{"rateLimitType":"seven_day"}}}),
            json!({"type":"assistant","quotaLimits":{"status":"rejected"},"apiErrorParams":{"rate_limit_info":{"rateLimitType":"seven_day","resetsAt":1900000000}}}),
            json!({"type":"assistant","quotaLimits":{"status":"rejected","rateLimitType":7,"resetsAt":"bad"},"apiErrorParams":{"rate_limit_info":{"rateLimitType":"seven_day","resetsAt":1900000000}}}),
            json!({"type":"assistant","quotaLimits":{"status":"rejected","rateLimitType":"seven_day","resetsAt":1900000000},"apiErrorParams":{"rate_limit_info":{"rateLimitType":"five_hour","resetsAt":1800000000}}}),
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

    // #258 A3/P2: naming a partial top-level rejection must not erase readable native window fields.
    #[test]
    fn fallback_quota_name_preserves_readable_native_window() {
        for fallback_reset in [Value::Null,json!(1800000000)] {
            let row=json!({"type":"assistant","quotaLimits":{"status":"rejected","unifiedWindows":{"seven_day":{"utilization":0.8,"resetsAt":1900000000}}},"apiErrorParams":{"rate_limit_info":{"rateLimitType":"seven_day","resetsAt":fallback_reset}}});
            let runtime=adapter::runtime("claude").unwrap();
            let windows=runtime.log_limits(&row.to_string(),None,None);
            assert_eq!(windows.len(),1);
            assert_eq!(windows[0].window.as_deref(),Some("seven_day"));
            assert_eq!(windows[0].used,Some(0.8));
            assert_eq!(windows[0].resets_at,Some(1900000000));
            assert_eq!(windows[0].exhausted,Some(true));
            assert_eq!(windows[0].unknown,None);
            assert_eq!(runtime.classify(&row.to_string()).reached,Some(Some(1900000000)));
        }
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

pub(super) fn reported(client:Client,v:&Value,now:u64)->Option<Vec<Window>> {(protocol(client).reported)(v,now)}

pub(super) fn update(client:Client,v:&Value,now:u64,all:&mut BTreeMap<String,Vec<Window>>)->bool {(protocol(client).update_limits)(v,now,all)}

pub(super) fn account(client:Client,home:Option<&Path>,now:i64)->Vec<Window> {(protocol(client).account_limits)(home,now)}

pub(super) fn identity(client:Client,home:Option<&Path>)->Option<String> {(protocol(client).identity)(home)}

#[cfg(target_os="macos")]
pub(crate) fn keychain(service:&str,account:Option<&str>)->Res<Option<String>> {
        use std::sync::atomic::{AtomicBool,Ordering};
        static PROMPTED: AtomicBool = AtomicBool::new(false);
        if PROMPTED.load(Ordering::Relaxed) {return Err("keychain-would-prompt".into());}
        let mut command=Command::new("/usr/bin/security"); command.args(["find-generic-password","-s",&service]);
        if let Some(account)=account {command.args(["-a",account]);}
        command.arg("-w");
        let output=match crate::process::output("agent.account-credential",&mut command,Duration::from_secs(5)) {
            Ok(o)|Err(crate::process::Failure::Exit(o))=>o,
            Err(crate::process::Failure::Timeout)=>{PROMPTED.store(true,Ordering::Relaxed);return Err("keychain-would-prompt".into());},
            Err(_)=>return Err("credential-unreadable".into()),
        };
        match output.status.code() {
            Some(0)=>return String::from_utf8(output.stdout).map(|s|Some(s.trim().into())).map_err(|_|"credential-invalid".into()),
            Some(44)=>{}, Some(36)=>{PROMPTED.store(true,Ordering::Relaxed);return Err("keychain-would-prompt".into());},
            _=>return Err("credential-unreadable".into()),
        }

 Ok(None)
}

fn read_credentials(client:Client,home:&Path,explicit:bool)->Res<String> {(protocol(client).credentials)(home,explicit)}

pub(super) use registry::claude_info;
#[cfg(test)]
use registry::{claude_window,claude_windows,code_assist,project_identity,codex_identity};
