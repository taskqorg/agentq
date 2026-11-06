//! Separate content producer; never writes to telemetry (agentq.md M6.12–M6.13).
mod scrub;
mod store;
use crate::agent::{Context, Frozen, Launch};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{OnceLock, mpsc},
};
pub use store::{disconnect, enroll, pass, status};

type Res<T> = Result<T, String>;
pub const SCHEMA: &str = "agent-history/1";
pub const MANUAL: &str = "agent-history.manual";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub format: String,
    /// Native byte position at which this capture starts.
    pub start: u64,
    /// None while the source is live.
    pub end: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Capture {
    pub id: String,
    pub root: PathBuf,
    pub runtime: String,
    pub home: Option<PathBuf>,
    pub manifest: Value,
    pub capture: String,
    pub sources: Vec<Source>,
    pub turn: Option<String>,
    pub grant: Value,
}

/// Optional asynchronous capture metadata. It has no effect on native work or its result.
#[derive(Clone)]
pub(crate) struct Handle {
    data: PathBuf,
    capture: Capture,
}

enum Job {
    Capture(PathBuf, Capture),
    Bytes(PathBuf, Vec<u8>),
}
fn queue(job: Job) {
    static WRITER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("agentq-archive-capture".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let result = match job {
                        Job::Capture(data, capture) => store::register(&data, capture),
                        Job::Bytes(path, bytes) => store::append_stream(&path, &bytes),
                    };
                    if result.is_err() {
                        eprintln!("agentq: archive capture unavailable");
                    }
                }
            });
        sender
    });
    let _ = sender.send(job);
}

impl Handle {
    pub(crate) fn launch(
        runtime: &str,
        l: &Launch,
        sources: Vec<Source>,
        session: Option<String>,
        home: Option<PathBuf>,
    ) -> Self {
        let install = l
            .install
            .clone()
            .or_else(crate::optimization::install_root)
            .unwrap_or_default();
        let mut manifest = json!({"project":"other","task":null,"attempt":null,"actor":"worker","entry":l.product,
            "runtime":runtime,"cli_version":null,"model":l.model,"effort":l.effort,
            "policy":format!("{:?}",l.policy),"account_label":null,"host":null,"session":session,
            "parent_session":null,"instructions_hash":null});
        let ctx = Context {
            product: &l.product,
            version: env!("CARGO_PKG_VERSION"),
            project: "other",
            task: None,
            route: None,
            account: crate::agent::DEFAULT,
        };
        attribution(&mut manifest, &ctx);
        let capture = Capture {
            id: crate::incident::uuid(),
            root: l.root.clone(),
            runtime: runtime.into(),
            home,
            manifest,
            capture: "launched".into(),
            sources,
            turn: None,
            grant: Value::Null,
        };
        let handle = Self {
            data: install.join("data"),
            capture,
        };
        handle.save();
        handle
    }
    pub(crate) fn legacy(
        spec: &crate::agent::Spec,
        log: &Path,
        install: Option<&Path>,
        ctx: &Context,
    ) -> Self {
        let mut launch = Launch::new(&spec.root, log, "", ctx.product);
        launch.install = install.map(Path::to_path_buf);
        launch.home = spec.home.clone();
        let format = crate::agent::registry::protocol(spec.client).stream_history;
        let start = std::fs::metadata(log).map_or(0, |m| m.len());
        let mut handle = Self::launch(
            spec.client.code(),
            &launch,
            vec![Source {
                path: log.into(),
                format: format.into(),
                start,
                end: None,
            }],
            spec.resume.then(|| spec.session.clone()),
            crate::agent::spec_home(spec),
        );
        attribution(&mut handle.capture.manifest, ctx);
        handle.save();
        handle
    }
    fn save(&self) {
        if self.data.is_absolute() {
            queue(Job::Capture(self.data.clone(), self.capture.clone()));
        }
    }
    pub(crate) fn observe(&mut self, session: Option<String>, end: Option<u64>) {
        let before = serde_json::to_value(&self.capture).ok();
        if let Some(session) = session {
            self.capture.manifest["session"] = json!(session);
        }
        if let Some(end) = end {
            for source in &mut self.capture.sources {
                source.end = Some(end);
            }
        }
        if before != serde_json::to_value(&self.capture).ok() {
            self.save();
        }
    }
    pub(crate) fn next_turn(&mut self) {
        for source in &mut self.capture.sources {
            source.end = Some(u64::MAX);
        }
        self.save();
        self.capture.id = crate::incident::uuid();
        self.capture.turn = None;
        for source in &mut self.capture.sources {
            source.start = u64::MAX;
            source.end = None;
        }
        self.save();
    }
    pub(crate) fn record(&mut self, frozen: &Frozen) {
        let saved = frozen.save();
        let observation = &saved["records"][0]["observation"];
        for key in ["project", "task", "attempt", "actor"] {
            if !observation[key].is_null() {
                self.capture.manifest[key] = observation[key].clone();
            }
        }
        for (target, key) in [
            ("session", "session_id"),
            ("model", "model"),
            ("account_label", "account"),
        ] {
            if observation[key].as_str().is_some() {
                self.capture.manifest[target] = observation[key].clone();
            }
        }
        if let Ok(events) = frozen.events() {
            self.capture.turn = events.first().map(|e| e.id().into());
        }
        self.save();
    }
}

fn attribution(manifest: &mut Value, ctx: &Context) {
    let a = crate::agent::capture::Attribution::of(ctx);
    manifest["project"] = json!(a.project);
    manifest["task"] = json!(a.task);
    manifest["attempt"] = json!(a.attempt);
    let mut fields = json!({});
    a.caller_fields(&mut fields);
    if let Some(actor) = fields["actor"].as_str() {
        manifest["actor"] = json!(actor);
    }
    manifest["account_label"] = json!(ctx.account);
}

/// A hook or another adapter can register an already identified source. No network or storage wait.
pub fn capture(data: &Path, capture: Capture) {
    queue(Job::Capture(data.into(), capture));
}

/// Raw app-server wire recording, on the archive writer rather than the pipe reader.
pub(crate) fn stream(path: &Path, bytes: &[u8]) {
    queue(Job::Bytes(path.into(), bytes.into()));
}

/// Private source for a recorded app-server transport; recording itself checks the active grant.
pub(crate) fn stream_path(install: Option<&Path>, log: &Path) -> PathBuf {
    install
        .map(Path::to_path_buf)
        .or_else(crate::optimization::install_root)
        .unwrap_or_default()
        .join("data/archive/streams")
        .join(format!(
            "{}-{}.jsonl",
            crate::sha256_hex(log.as_os_str().as_encoded_bytes()),
            crate::incident::uuid()
        ))
}

/// Native hook capture is separate from its metadata observation and never affects the hook response.
pub fn hook(data: &Path, runtime: &str, input: &Value, launched: bool, project: &str) {
    let session = input["session_id"]
        .as_str()
        .or(input["sessionId"].as_str())
        .or(input["conversationId"].as_str());
    let Some(session) = session else { return };
    let path = input["transcript_path"]
        .as_str()
        .or(input["transcriptPath"].as_str())
        .map(PathBuf::from);
    let root = input["cwd"]
        .as_str()
        .or(input["working_directory"].as_str())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let Some(root) = root else { return };
    let capture = Capture {
        id: crate::sha256_hex(json!([runtime, session, "hook"]).to_string().as_bytes()),
        root,
        runtime: runtime.into(),
        home: None,
        manifest: json!({"project":project,"task":null,"attempt":null,"actor":if launched{"manager"}else{"manual"},"entry":"hook","runtime":runtime,
            "cli_version":null,"model":null,"effort":null,"policy":null,"account_label":null,"host":null,"session":session,"parent_session":null,"instructions_hash":null}),
        capture: if launched { "launched" } else { "manual" }.into(),
        sources: path
            .map(|path| Source {
                path,
                format: "native".into(),
                start: 0,
                end: None,
            })
            .into_iter()
            .collect(),
        turn: None,
        grant: Value::Null,
    };
    self::capture(data, capture);
}
