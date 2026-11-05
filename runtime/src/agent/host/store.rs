//! M12.8: bounded durable journal; missing/corrupt/locked state never resets occupied operations.
use super::*;
use std::{collections::BTreeMap,fs::OpenOptions,io::Write,time::{Duration,Instant}};
pub(super) const WAIT:Duration=Duration::from_secs(5);
#[derive(Default,Clone,Serialize,Deserialize,PartialEq,Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct State {pub ops:BTreeMap<String,Op>}
#[derive(Clone,Serialize,Deserialize,PartialEq,Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct Op {pub bound:Value,pub receipt:Value}
pub(super) fn write_synced(path:&Path,text:&str)->Res<()> {
    coreq::write_atomic(path,text).map_err(|_|"protected journal write unknown; retain occupancy")?;
    fs::File::open(path).and_then(|f|f.sync_all()).map_err(|_|"protected journal sync unknown")?;
    #[cfg(unix)] fs::File::open(path.parent().ok_or("journal parent absent")?).and_then(|f|f.sync_all()).map_err(|_|"protected journal directory sync unknown")?;
    if fs::read(path).map_err(|_|"protected journal readback unknown")?!=text.as_bytes() {return Err("protected journal readback changed".into());}Ok(())
}
pub(super) fn within<T>(install:&Path,budget:Duration,f:impl FnOnce(&mut State)->Res<T>)->Res<T> {
    let mut options=OpenOptions::new();options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
    let lock=options.open(install.join("operations.lock")).map_err(|_|"protected journal lock unavailable")?;
    let until=Instant::now()+budget;
    loop {match lock.try_lock() {
        Ok(())=>break,
        Err(std::fs::TryLockError::WouldBlock) if Instant::now()<until=>std::thread::sleep(Duration::from_millis(10)),
        Err(_)=>return Err("protected journal locked or unavailable; retain occupancy".into()),
    }}
    let path=install.join("operations.json");
    let mut state:State=match fs::File::open(&path) {
        Ok(mut file)=>{
            let mut bytes=Vec::new();Read::by_ref(&mut file).take(16*1024*1024+1).read_to_end(&mut bytes).map_err(|_|"protected journal read unknown")?;
            if bytes.len()>16*1024*1024 {return Err("protected journal full; retain occupancy".into());}
            serde_json::from_slice(&bytes).map_err(|_|"protected journal corrupt; retain occupancy")?
        },
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{
            // Only explicit operator initialization permits an empty journal. Loss is never a reset.
            let initial=read(&install.join("initial.json"))?;
            if initial!=json!({"schema":"agentq.host-initial/1","operations":0}) {return Err("protected journal absent; authority unknown".into());}
            let mut marker=OpenOptions::new();marker.write(true).create_new(true);
            #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;marker.mode(0o600);}
            marker.open(install.join("initialized")).and_then(|mut f|{f.write_all(b"journal initialized\n")?;f.sync_all()}).map_err(|_|"protected journal missing after initialization; retain occupancy")?;
            let empty=State::default();
            write_synced(&path,&serde_json::to_string(&empty).map_err(|_|"empty journal serialization failed")?)?;
            empty
        },
        Err(_)=>return Err("protected journal unreadable; retain occupancy".into()),
    };
    let value=f(&mut state)?;
    let text=serde_json::to_string(&state).map_err(|_|"journal serialization unknown")?;
    if text.len()>16*1024*1024 {return Err("protected journal full; retain occupancy".into());}
    write_synced(&path,&text)?;
    Ok(value)
}
