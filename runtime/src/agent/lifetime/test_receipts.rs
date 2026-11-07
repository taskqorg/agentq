//! Actual callback handles, scoped to fixture-owned roots. Never included in a normal library build.
use super::*;
use std::sync::Mutex;
use std::thread::JoinHandle;
static WRITERS:Mutex<Vec<(PathBuf,JoinHandle<()>,std::sync::mpsc::Receiver<Res<()>>)>>=Mutex::new(Vec::new());
pub(crate) fn register(root:&Path,writer:JoinHandle<()>,result:std::sync::mpsc::Receiver<Res<()>>) {WRITERS.lock().unwrap().push((root.to_path_buf(),writer,result));}
pub(crate) fn finish(roots:&[PathBuf],unwinding:bool)->Res<()> {
    let mut writers=WRITERS.lock().unwrap();let mut owned=vec![];let mut i=0;
    while i<writers.len() {if roots.iter().any(|root|writers[i].0.starts_with(root)) {owned.push(writers.swap_remove(i));}else{i+=1;}}
    drop(writers);
    let mut errors=vec![];
    for (root,writer,result) in owned {
        if unwinding && !writer.is_finished() {eprintln!("fixture failed; retaining {} while receipt callback is unfinished",root.display());continue;}
        match writer.join() {Ok(())=>match result.recv() {Ok(Ok(()))=>{},Ok(Err(error))=>errors.push(error),Err(_)=>errors.push("scope receipt callback result missing".into())},Err(_)=>errors.push("scope receipt callback panicked".into())}
    }
    if WRITERS.lock().unwrap().iter().any(|(r,_,_)|roots.iter().any(|root|r.starts_with(root))) {errors.push("scope receipt callback registered during teardown".into());}
    if errors.is_empty() {Ok(())}else{Err(errors.join("; "))}
}

pub(crate) fn count(root:&Path)->usize {WRITERS.lock().unwrap().iter().filter(|(r,_,_)|r==root).count()}
static STOP_PAUSES:Mutex<Vec<(PathBuf,Pause)>>=Mutex::new(Vec::new());
pub(crate) struct Pause {entered:std::sync::mpsc::Sender<()>,release:std::sync::mpsc::Receiver<()>}
impl Pause {pub(crate) fn wait(self) {let _=self.entered.send(());let _=self.release.recv();}}
pub(crate) fn hold_next_stop(root:&Path)->(std::sync::mpsc::Sender<()>,std::sync::mpsc::Receiver<()>) {
    let (release,wait)=std::sync::mpsc::channel();let (entered,ready)=std::sync::mpsc::channel();
    STOP_PAUSES.lock().unwrap().push((root.canonicalize().unwrap(),Pause {entered,release:wait}));(release,ready)
}
pub(crate) fn take_stop_pause(root:&Path)->Option<Pause> {
    let mut pauses=STOP_PAUSES.lock().unwrap();let at=pauses.iter().position(|(r,_)|r==root)?;Some(pauses.swap_remove(at).1)
}

#[test]
fn joins_are_scoped_and_callback_failures_fail_the_fixture() {
    let root=crate::scratch("receipt-joins");let other=crate::scratch("receipt-other");
    let (send,result)=std::sync::mpsc::channel();register(&root,std::thread::spawn(move||send.send(Err("durability failure".into())).unwrap()),result);
    let (send,result)=std::sync::mpsc::channel();register(&other,std::thread::spawn(move||send.send(Ok(())).unwrap()),result);
    assert_eq!(finish(&[root.clone()],false),Err("durability failure".into()));assert_eq!(count(&other),1);
    let (_send,result)=std::sync::mpsc::channel();register(&root,std::thread::spawn(||panic!("callback failed")),result);
    assert_eq!(finish(&[root.clone()],false),Err("scope receipt callback panicked".into()));
    finish(&[other.clone()],false).unwrap();std::fs::remove_dir_all(root).unwrap();std::fs::remove_dir_all(other).unwrap();
}
