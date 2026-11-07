//! Explicit native #287 qualification; the driver supplies only private temporary homes and a loopback model.
use agentq_runtime::agent::{self, Event, NativeProcess, SessionStore, State};
use std::{path::PathBuf, time::Duration};
struct Cleanup(NativeProcess);
impl Drop for Cleanup {fn drop(&mut self) {
    let identity=coreq::process::Identity {pid:self.0.pid,started:self.0.started.clone()};
    if NativeProcess::read(self.0.pid).ok().flatten().as_ref()==Some(&self.0) {
        if let Err(error)=coreq::process::terminate(&identity) {eprintln!("private native fixture cleanup unknown: {error}");return;}
        while NativeProcess::read(self.0.pid).ok().flatten().as_ref()==Some(&self.0) {std::thread::sleep(Duration::from_millis(20));}
    }
}}
fn main() {
    let mut args=std::env::args_os().skip(1);
    let root=PathBuf::from(args.next().expect("private fixture root")).canonicalize().unwrap();
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    let native=args.next().expect("native executable");
    let rt=agent::runtime("codex").unwrap();
    let mut launch=agent::Launch::new(root.join("workspace"),root.join("turn.log"),"hi","agentq-qualification");
    launch.home=Some(root.join("account"));launch.command=vec![native];launch.inactivity=Some(Duration::from_millis(20));
    let mut run=rt.launch(&launch).unwrap();
    while !root.join("request").exists() {assert_eq!(rt.poll(&mut run).unwrap(),State::Running);std::thread::sleep(Duration::from_millis(20));}
    let _=rt.observe(&mut run);
    let session=rt.session_id(&run).expect("native thread ID");let pid=run.id().unwrap();
    let process=NativeProcess::read(pid).unwrap().unwrap();let _cleanup=Cleanup(process.clone());
    let mut independent=launch.clone();independent.session_store=SessionStore::new(root.join("independent-store"));
    assert!(rt.adopt(&session,&independent,&[process.clone()]).unwrap_err().starts_with("session-busy"));
    assert!(rt.resume(&session,&launch).err().is_some_and(|e|e.detail.starts_with("session-busy")));
    let checkpoint=rt.checkpoint(&run).unwrap();assert!(rt.recover(&checkpoint,&launch).is_err());drop(run);
    let mut recovered=rt.recover(&checkpoint,&launch).unwrap();assert_eq!(recovered.id(),Some(pid));
    std::thread::sleep(Duration::from_millis(30));
    let observation=rt.observe(&mut recovered);assert_eq!(observation.activity.state,"stalled");
    assert_eq!(NativeProcess::read(pid).unwrap(),Some(process),"detection preserves the same native child");
    assert_eq!(rt.progress(&mut recovered).unwrap().iter().filter(|e|matches!(e,Event::Stalled{..})).count(),1);
    assert!(!rt.progress(&mut recovered).unwrap().iter().any(|e|matches!(e,Event::Stalled{..})));
    std::fs::write(root.join("release"),"").unwrap();
    while rt.poll(&mut recovered).unwrap()==State::Running {std::thread::sleep(Duration::from_millis(20));}
    assert_eq!(rt.report(&recovered).unwrap().outcome,"ok");
    assert_eq!(std::fs::read_to_string(root.join("request")).unwrap(),"1","recovery submits no additional request");
    while rt.orphans(&launch).unwrap().iter().any(|scope|scope.scope!="absent") {std::thread::sleep(Duration::from_millis(20));}
    println!("PASS native scope absence, ownership, live-writer adoption refusal, same-child recovery, stall without cancellation, exactly one model request");
}
