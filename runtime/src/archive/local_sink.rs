//! Opt-in wire qualification against an isolated DashQ main process; no real profiles or paid agents.
use super::*;
use std::net::TcpStream;

struct Local {
    address: String,
    offline: bool,
    lose: bool,
    requests: Vec<(Value, Value)>,
}
impl Local {
    fn request(
        &self,
        method: &str,
        path: &str,
        auth: Option<&str>,
        body: &[u8],
        form: bool,
    ) -> Res<(u16, Vec<u8>)> {
        let mut stream = TcpStream::connect(&self.address).map_err(|_| "local sink offline")?;
        let mut headers = format!(
            "{method} {path} HTTP/1.1\r\nHost: fixture.invalid\r\nConnection: close\r\nX-Dashq-Subject: fixture-owner\r\nX-Dashq-Archive-Proxy: {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
            "ab".repeat(32),
            if form {
                "application/x-www-form-urlencoded"
            } else {
                "application/json"
            },
            body.len()
        );
        if method == "POST" {
            headers.push_str("Origin: https://fixture.invalid\r\n");
        }
        if let Some(auth) = auth {
            headers.push_str(&format!("Authorization: Bearer {auth}\r\n"));
        }
        headers.push_str("\r\n");
        stream
            .write_all(headers.as_bytes())
            .and_then(|_| stream.write_all(body))
            .map_err(|_| "local sink write")?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(|_| "local sink read")?;
        let split = response
            .windows(4)
            .position(|s| s == b"\r\n\r\n")
            .ok_or("local response invalid")?;
        let status = std::str::from_utf8(&response[..split])
            .map_err(|_| "local status invalid")?
            .split_whitespace()
            .nth(1)
            .ok_or("local status missing")?
            .parse()
            .map_err(|_| "local status invalid")?;
        Ok((status, response[split + 4..].to_vec()))
    }
    fn raw(&self, receipt: &Value) -> Vec<u8> {
        let path = format!(
            "/archive/raw?id={}&source={}",
            receipt["id"].as_str().unwrap(),
            receipt["source"].as_str().unwrap()
        );
        let (status, bytes) = self.request("GET", &path, None, b"", false).unwrap();
        assert_eq!(status, 200);
        bytes
    }
}
impl Wire for Local {
    fn post(&mut self, _: &str, path: &str, auth: Option<&str>, body: &Value) -> Res<(u16, Value)> {
        if self.offline {
            return Err("offline fixture".into());
        }
        let (status, bytes) =
            self.request("POST", path, auth, body.to_string().as_bytes(), false)?;
        let response: Value =
            serde_json::from_slice(&bytes).map_err(|_| "local JSON response invalid")?;
        if path == "/machine/archive/1" {
            self.requests.push((body.clone(), response.clone()));
            if self.lose {
                self.lose = false;
                return Err("lost fixture reply".into());
            }
        }
        Ok((status, response))
    }
    fn now(&self) -> u64 {
        super::now()
    }
    fn sleep(&mut self, s: u64) {
        std::thread::sleep(std::time::Duration::from_secs(s));
    }
    fn show(&mut self, _: &str, code: &str) {
        let (status, body) = self
            .request("GET", &format!("/connect?code={code}"), None, b"", false)
            .unwrap();
        assert_eq!(status, 200);
        let html = String::from_utf8(body).unwrap();
        let csrf = html
            .split("name=\"csrf\" value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let body = format!("code={code}&csrf={csrf}&workspace=fixture&decision=approve");
        let (status, _) = self
            .request("POST", "/connect", None, body.as_bytes(), true)
            .unwrap();
        assert_eq!(status, 200);
    }
}

// A1–A7: real device approval, machine ingestion, guarded decrypted readback, offline restart,
// lost response, append, late bindings, replacement/truncation and explicit host-manual refusal.
#[test]
#[ignore = "requires isolated DashQ main configured by scripts/archive_e2e.py"]
fn dashq_main_wire_and_recovery() {
    let address = std::env::var("AGENTQ_ARCHIVE_TEST_ADDR").expect("local fixture address");
    assert!(address.starts_with("127.0.0.1:"));
    let (data, mut capture) = super::tests::setup("archive-local-sink");
    fs::remove_file(connection(&data).join("connection.json")).unwrap();
    let mut sink = Local {
        address,
        offline: false,
        lose: false,
        requests: vec![],
    };
    let request = Request {
        installation: Some("archive-fixture"),
        product: "agentq",
        host_label: "fixture-host",
        schemas: &["agentq-snapshot/1"],
        scope: None,
        capabilities: &[],
    };
    enroll::pair(
        &connection(&data),
        "https://fixture.invalid",
        &request,
        &mut Machine(&mut sink),
    )
    .unwrap();
    enroll(
        &data,
        "https://fixture.invalid",
        "archive-fixture",
        "fixture-host",
        false,
        &mut sink,
    )
    .unwrap();
    let combined = active(&data).unwrap().unwrap();
    assert_eq!(combined["schemas"], json!(["agentq-snapshot/1", SCHEMA]));
    let native = capture.home.as_ref().unwrap().join("projects/fixture");
    fs::create_dir_all(native.join("s-1/subagents")).unwrap();
    fs::write(
        native.join("s-1.jsonl"),
        b"{\"type\":\"user\",\"sessionId\":\"s-1\",\"message\":{\"content\":\"native parent\"}}\n",
    )
    .unwrap();
    let child=b"{\"type\":\"assistant\",\"sessionId\":\"s-1\",\"agentId\":\"child-1\",\"message\":{\"content\":\"child output\"}}\n";
    fs::write(native.join("s-1/subagents/child-1.jsonl"), child).unwrap();
    register(&data, capture.clone()).unwrap();
    sink.lose = true;
    pass(&data, &mut sink).unwrap();
    assert_eq!(status(&data)["reason"], "archive offline");
    let first = sink.requests.first().unwrap().clone();
    assert_eq!(first.1["state"], "accepted");
    pass(&data, &mut sink).unwrap();
    assert_eq!(
        sink.requests
            .iter()
            .filter(|(b, _)| b["source"] == first.0["source"])
            .last()
            .unwrap()
            .0,
        first.0
    );
    let child_receipt = &sink
        .requests
        .iter()
        .find(|(b, _)| b["manifest"]["session"] == "s-1/agent/child-1")
        .expect("native child archived")
        .1;
    assert_eq!(sink.raw(child_receipt), child);
    let original = fs::read(&capture.sources[0].path).unwrap();
    let expected = super::super::scrub::bytes(&original).0;
    assert_eq!(sink.raw(&first.1), expected);
    assert!(!String::from_utf8_lossy(&expected).contains("fixture_secret"));
    capture.turn = Some("fixture-turn-event".into());
    capture.manifest["model"] = json!("fixture-final-model");
    register(&data, capture.clone()).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(
        sink.requests.last().unwrap().0["turns"],
        json!([{"event_id":"fixture-turn-event","start":0}])
    );
    let attributed = sink.requests.last().unwrap().1.clone();
    assert_ne!(attributed["source"], first.1["source"]);
    assert_eq!(sink.raw(&attributed), expected);

    let mut original = original;
    original.extend_from_slice(b"{\"type\":\"assistant\",\"message\":{\"content\":\"second\"}}\n");
    fs::write(&capture.sources[0].path, &original).unwrap();
    control("offline");
    pass(&data, &mut sink).unwrap();
    assert!(status(&data)["pending_bytes"].as_u64().unwrap() > 0);
    control("resume");
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.raw(&attributed), super::super::scrub::bytes(&original).0);
    let replaced = capture.sources[0].path.with_extension("replaced");
    fs::rename(&capture.sources[0].path, &replaced).unwrap();
    fs::write(&capture.sources[0].path, b"{\"replacement\":true}\n").unwrap();
    pass(&data, &mut sink).unwrap();
    let replacement = sink.requests.last().unwrap().1.clone();
    assert_ne!(replacement["source"], first.1["source"]);
    assert_eq!(sink.raw(&replacement), b"{\"replacement\":true}\n");
    fs::write(&capture.sources[0].path, b"{}\n").unwrap();
    pass(&data, &mut sink).unwrap();
    let truncated = sink.requests.last().unwrap().1.clone();
    assert_ne!(truncated["source"], replacement["source"]);
    assert_eq!(sink.raw(&truncated), b"{}\n");
    assert_eq!(
        sink.raw(&first.1),
        expected,
        "accepted original bytes remain readable"
    );
    let mut manual = capture.clone();
    manual.id = "manual-fixture".into();
    manual.capture = "manual".into();
    let before = sink.requests.len();
    register(&data, manual).unwrap();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), before);
    let grant = active(&data).unwrap().unwrap();
    let mut forged = first.0.clone();
    forged["capture"] = json!("manual");
    let (code, response) = sink
        .post(
            "https://fixture.invalid",
            "/machine/archive/1",
            grant["credential"].as_str(),
            &forged,
        )
        .unwrap();
    assert_eq!(code, 403);
    assert_eq!(response["error"], "manual_disabled");
    disconnect(&data, &mut sink).unwrap();
    let before = sink.requests.len();
    pass(&data, &mut sink).unwrap();
    assert_eq!(sink.requests.len(), before);
    assert_eq!(status(&data)["state"], "off");
    println!(
        "archive e2e: approval, exact scrubbed readback, lost reply, offline resume, append, late turn receipt, replacement, truncation, manual refusal and disconnect passed"
    );
    fs::remove_dir_all(data.parent().unwrap()).unwrap();
}

fn control(action: &str) {
    let root = PathBuf::from(
        std::env::var("AGENTQ_ARCHIVE_TEST_CONTROL").expect("isolated fixture control"),
    );
    fs::write(root.join(format!("{action}.request")), b"").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !root.join(format!("{action}.ready")).exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fixture did not acknowledge {action}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
