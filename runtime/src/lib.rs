//! agentq-runtime (agentq.md § 12, docs/spec/RUNTIME.md): Codex and Claude worker turns for every consumer, with
//! optional AgentQ optimization per launch. Moved from CoreQ v0.18.0 `agent`; it depends only on CoreQ primitives.
pub mod agent;
pub mod cli;
pub mod optimization;

// The moved module keeps its CoreQ paths (`crate::telemetry`, `crate::process`, `crate::quiet`).
pub(crate) use coreq::{process, quiet, telemetry};

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) mod incident {
    /// A random UUID v4 (CoreQ `incident::uuid`, private there).
    pub(crate) fn uuid() -> String {
        use ring::rand::SecureRandom;
        let mut b = [0u8; 16];
        let _ = ring::rand::SystemRandom::new().fill(&mut b);
        b[6] = (b[6] & 0x0f) | 0x40;
        b[8] = (b[8] & 0x3f) | 0x80;
        let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
    }
}

#[cfg(test)]
pub(crate) fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("agentq-runtime-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
