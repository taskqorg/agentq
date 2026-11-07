//! M12.8: include in the actual slot owner's Mac consumer after reviewed publication.
//! No slot admission/release, input, resolver, retry loop or worker-environment override.
use agentq_runtime::agent::{ProtectedAppServer, ProtectedHost, ProtectedPreparation};
use serde_json::Value;
use std::{fs::{self, OpenOptions}, io::Write, path::Path, time::{Duration, Instant}};

type Result<T> = std::result::Result<T, String>;

fn read(path: &Path) -> Result<Value> {
    serde_json::from_slice(&fs::read(path).map_err(|_| "private handoff read failed; retain operation")?)
        .map_err(|_| "private handoff incomplete; retain operation".into())
}

fn save_once(path: &Path, value: &Value) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| "handoff already consumed or unavailable; never replace or replay")?;
    file.write_all(&serde_json::to_vec(value).map_err(|_| "handoff serialization failed")?)
        .and_then(|_| file.sync_all()).map_err(|_| "handoff durability unknown; never dispatch")?;
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or("private handoff parent absent")?).and_then(|d| d.sync_all())
        .map_err(|_| "handoff directory durability unknown; never dispatch")?;
    Ok(())
}

/// Caller supplies an existing private directory outside the native write set and its reserved slot.
pub fn prepare(host: &ProtectedHost, slot: &Value, directory: &Path) -> Result<ProtectedPreparation> {
    save_once(&directory.join("prepare-intent.json"), slot)?;
    let preparation = host.prepare(slot, Instant::now() + Duration::from_secs(5))?;
    save_once(&directory.join("preparation.json"), &preparation.checkpoint())?;
    save_once(&directory.join("launch-request.json"), preparation.request())?;
    Ok(preparation)
}

/// Restore the original private preparation only; never derives authority from an operation receipt.
pub fn recover(host: &ProtectedHost, directory: &Path) -> Result<ProtectedPreparation> {
    let preparation = host.recover_preparation(&read(&directory.join("preparation.json"))?)?;
    if preparation.request()["slot"] != read(&directory.join("prepare-intent.json"))?
        || *preparation.request() != read(&directory.join("launch-request.json"))? {
        return Err("handoff binding changed; retain original operation".into());
    }
    Ok(preparation)
}

/// Same live slot owner only. Even an error consumes this dispatch; recovery never calls launch.
pub fn launch(host: &ProtectedHost, directory: &Path, private_log: &Path) -> Result<ProtectedAppServer> {
    let preparation = recover(host, directory)?;
    save_once(&directory.join("launch-intent.json"), preparation.request())?;
    host.launch_prepared(&preparation, private_log)
}
