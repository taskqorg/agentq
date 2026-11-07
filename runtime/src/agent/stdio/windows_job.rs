//! Bind the native server and its startup shim descendants before any protocol input.
use super::*;
use std::{ffi::c_void,os::windows::io::{AsRawHandle,FromRawHandle,OwnedHandle}};
#[repr(C)] #[derive(Default)] struct Time {low:u32,high:u32}
#[link(name="kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes:*const c_void,name:*const u16)->*mut c_void;
    fn SetInformationJobObject(job:*mut c_void,class:u32,information:*const c_void,size:u32)->i32;
    fn AssignProcessToJobObject(job:*mut c_void,process:*mut c_void)->i32;
    fn TerminateJobObject(job:*mut c_void,code:u32)->i32;
    fn TerminateProcess(process:*mut c_void,code:u32)->i32;
    fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut c_void;
    fn GetProcessId(process:*mut c_void)->u32;
    fn GetCurrentProcess()->*mut c_void;
    fn DuplicateHandle(source_process:*mut c_void,source:*mut c_void,target_process:*mut c_void,target:*mut *mut c_void,access:u32,inherit:i32,options:u32)->i32;
    fn WaitForSingleObject(handle:*mut c_void,millis:u32)->u32;
    fn GetProcessTimes(process:*mut c_void,created:*mut Time,exit:*mut Time,kernel:*mut Time,user:*mut Time)->i32;
    fn CreateToolhelp32Snapshot(flags:u32,pid:u32)->*mut c_void;
    fn Process32FirstW(snapshot:*mut c_void,entry:*mut Entry)->i32;
    fn Process32NextW(snapshot:*mut c_void,entry:*mut Entry)->i32;
}
#[link(name="ntdll")]
unsafe extern "system" {
    fn NtSuspendProcess(process:*mut c_void)->i32;
    fn NtResumeProcess(process:*mut c_void)->i32;
    fn NtQueryInformationProcess(process:*mut c_void,class:u32,information:*mut c_void,size:u32,returned:*mut u32)->i32;
}
#[repr(C)] struct ProcessBasic {reserved:*mut c_void,peb:*mut c_void,reserved2:[*mut c_void;2],pid:usize,parent:usize}
#[repr(C)] struct Entry {size:u32,usage:u32,pid:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,exe:[u16;260]}
#[repr(C)] #[derive(Default)] struct Basic {process_time:i64,job_time:i64,flags:u32,min_working:usize,max_working:usize,active:u32,affinity:usize,priority:u32,scheduling:u32}
#[repr(C)] #[derive(Default)] struct Io {read_ops:u64,write_ops:u64,other_ops:u64,read_bytes:u64,write_bytes:u64,other_bytes:u64}
#[repr(C)] #[derive(Default)] struct Limits {basic:Basic,io:Io,process_memory:usize,job_memory:usize,peak_process:usize,peak_job:usize}
fn error(operation:&str)->String {format!("native job {operation}: {}",std::io::Error::last_os_error())}
fn created(handle:*mut c_void)->Res<u64> {
    let(mut start,mut exit,mut kernel,mut user)=(Time::default(),Time::default(),Time::default(),Time::default());
    if unsafe {GetProcessTimes(handle,&mut start,&mut exit,&mut kernel,&mut user)}==0 {return Err(error("creation identity"));}Ok((u64::from(start.high)<<32)|u64::from(start.low))
}
struct Frozen {handle:OwnedHandle,pid:u32,created:u64,resume:bool}
impl Drop for Frozen {fn drop(&mut self) {unsafe {if self.resume {NtResumeProcess(self.handle.as_raw_handle());}else{TerminateProcess(self.handle.as_raw_handle(),1);}}}}
impl Frozen {
    fn root(child:&Child)->Res<Self> {
        let current=unsafe {GetCurrentProcess()};let mut raw=std::ptr::null_mut();
        if unsafe {DuplicateHandle(current,child.as_raw_handle(),current,&mut raw,0,0,2)}==0 {return Err(error("duplicate root identity"));}
        Self::freeze(unsafe {OwnedHandle::from_raw_handle(raw)},None)
    }
    fn open(pid:u32,parent:&Self)->Res<Option<Self>> {
        let raw=unsafe {OpenProcess(0x00100000|0x0400|0x0800|0x0100|0x0001,0,pid)};
        if raw.is_null() {if std::io::Error::last_os_error().raw_os_error()==Some(87) {return Ok(None);}return Err(error("open process"));}
        Self::freeze(unsafe {OwnedHandle::from_raw_handle(raw)},Some(parent)).map(Some)
    }
    fn freeze(handle:OwnedHandle,parent:Option<&Self>)->Res<Self> {
        let raw=handle.as_raw_handle();let created=created(raw)?;let pid=unsafe {GetProcessId(raw)};
        if pid==0 {return Err(error("process identity"));}
        if let Some(parent)=parent {
            let mut basic:ProcessBasic=unsafe {std::mem::zeroed()};let mut returned=0;
            let status=unsafe {NtQueryInformationProcess(raw,0,(&mut basic as *mut ProcessBasic).cast(),std::mem::size_of::<ProcessBasic>() as u32,&mut returned)};
            if status<0 {return Err(format!("native job parent identity: NTSTATUS {status:#x}"));}
            // The snapshot is discovery only. Neither an unknown nor a replaced PID grants process authority.
            if returned as usize!=std::mem::size_of::<ProcessBasic>() || basic.pid!=pid as usize || basic.parent!=parent.pid as usize || created<parent.created
                || unsafe {WaitForSingleObject(parent.handle.as_raw_handle(),0)}!=258 {
                return Err("native job descendant identity changed or unknown".into());
            }
        }
        let status=unsafe {NtSuspendProcess(raw)};
        if status<0 {return Err(format!("native job suspend: NTSTATUS {status:#x}"));}
        Ok(Self {handle,pid,created,resume:false})
    }
}
pub(super) struct Job(OwnedHandle);
impl Job {
    pub fn new()->Res<Self> {
        let raw=unsafe {CreateJobObjectW(std::ptr::null(),std::ptr::null())};
        if raw.is_null(){return Err(error("create"));}
        let job=Self(unsafe {OwnedHandle::from_raw_handle(raw)});
        let mut limits=Limits::default();limits.basic.flags=0x2000; // KILL_ON_JOB_CLOSE, no breakaway.
        if unsafe {SetInformationJobObject(raw,9,(&limits as *const Limits).cast(),std::mem::size_of::<Limits>() as u32)}==0 {return Err(error("limits"));}
        Ok(job)
    }
    pub fn attach(&self,child:&Child)->Res<()> {
        let raw=self.0.as_raw_handle();
        let root=Frozen::root(child)?;
        let mut frozen=vec![root];let mut assigned=0;
        loop {
            // cmd.exe may already have started Node; suspend and enroll that existing subtree too.
            for member in &frozen[assigned..] {if unsafe {AssignProcessToJobObject(raw,member.handle.as_raw_handle())}==0 {return Err(error("assign"));}}
            assigned=frozen.len();
            let snapshot=unsafe {CreateToolhelp32Snapshot(2,0)};
            if snapshot as isize == -1 {return Err(error("process snapshot"));}
            let snapshot=unsafe {OwnedHandle::from_raw_handle(snapshot)};
            let mut entry:Entry=unsafe {std::mem::zeroed()};entry.size=std::mem::size_of::<Entry>() as u32;
            let mut rows=Vec::new();
            let mut present=unsafe {Process32FirstW(snapshot.as_raw_handle(),&mut entry)};
            while present!=0 {rows.push((entry.pid,entry.parent));present=unsafe {Process32NextW(snapshot.as_raw_handle(),&mut entry)};}
            let mut added=Vec::new();
            for (pid,parent) in rows {
                if frozen.iter().any(|p|p.pid==pid) {continue;}
                if let Some(parent)=frozen.iter().find(|p|p.pid==parent) {
                    if let Some(member)=Frozen::open(pid,parent)? {added.push(member);}
                }
            }
            if added.is_empty() {break;}frozen.extend(added);
        }
        // Handles keep creation identities bound; dropping these guards resumes exactly our suspension.
        for member in &mut frozen {member.resume=true;}
        drop(frozen);Ok(())
    }
    pub fn kill(&self)->Res<()> {if unsafe {TerminateJobObject(self.0.as_raw_handle(),1)}==0 {Err(error("terminate"))}else{Ok(())}}
}

#[cfg(test)] mod tests {
    use super::*;
    // A stale snapshot names an owned parent, but its PID now resolves to our unrelated child.
    #[test] fn stale_snapshot_refuses_unrelated_process_before_touching_it() {
        let spawn=||Command::new("cmd.exe").args(["/D","/C","set /p first= & echo unrelated-live & set /p second="])
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
        let mut parent=spawn();let root=Frozen::root(&parent).unwrap();
        let mut unrelated=spawn();
        let attempt=Frozen::open(unrelated.id(),&root);
        let refused=attempt.is_err();drop(attempt);
        if refused {
            unrelated.stdin.as_mut().unwrap().write_all(b"continue\r\n").unwrap();
            let mut line=String::new();BufReader::new(unrelated.stdout.take().unwrap()).read_line(&mut line).unwrap();
            assert!(line.contains("unrelated-live"),"unrelated process must remain runnable");
        }
        let _=unrelated.kill();let _=unrelated.wait();drop(root);let _=parent.wait();
        assert!(refused,"snapshot PID reuse must refuse before suspension, assignment or termination");
    }
}
