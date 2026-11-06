//! Bind the native server and its startup shim descendants before any protocol input.
use super::*;
use std::{ffi::c_void,os::windows::io::{AsRawHandle,FromRawHandle,OwnedHandle}};
#[link(name="kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes:*const c_void,name:*const u16)->*mut c_void;
    fn SetInformationJobObject(job:*mut c_void,class:u32,information:*const c_void,size:u32)->i32;
    fn AssignProcessToJobObject(job:*mut c_void,process:*mut c_void)->i32;
    fn TerminateJobObject(job:*mut c_void,code:u32)->i32;
    fn TerminateProcess(process:*mut c_void,code:u32)->i32;
    fn OpenProcess(access:u32,inherit:i32,pid:u32)->*mut c_void;
    fn GetProcessId(process:*mut c_void)->u32;
    fn GetProcessTimes(process:*mut c_void,created:*mut u64,exit:*mut u64,kernel:*mut u64,user:*mut u64)->i32;
    fn CreateToolhelp32Snapshot(flags:u32,pid:u32)->*mut c_void;
    fn Process32FirstW(snapshot:*mut c_void,entry:*mut Entry)->i32;
    fn Process32NextW(snapshot:*mut c_void,entry:*mut Entry)->i32;
}
#[link(name="ntdll")]
unsafe extern "system" {fn NtSuspendProcess(process:*mut c_void)->i32;fn NtResumeProcess(process:*mut c_void)->i32;}
#[repr(C)] struct Entry {size:u32,usage:u32,pid:u32,heap:usize,module:u32,threads:u32,parent:u32,priority:i32,flags:u32,exe:[u16;260]}
#[repr(C)] #[derive(Default)] struct Basic {process_time:i64,job_time:i64,flags:u32,min_working:usize,max_working:usize,active:u32,affinity:usize,priority:u32,scheduling:u32}
#[repr(C)] #[derive(Default)] struct Io {read_ops:u64,write_ops:u64,other_ops:u64,read_bytes:u64,write_bytes:u64,other_bytes:u64}
#[repr(C)] #[derive(Default)] struct Limits {basic:Basic,io:Io,process_memory:usize,job_memory:usize,peak_process:usize,peak_job:usize}
fn error(operation:&str)->String {format!("native job {operation}: {}",std::io::Error::last_os_error())}
fn created(handle:*mut c_void)->Res<u64> {
    let(mut start,mut exit,mut kernel,mut user)=(0,0,0,0);
    if unsafe {GetProcessTimes(handle,&mut start,&mut exit,&mut kernel,&mut user)}==0 {return Err(error("creation identity"));}Ok(start)
}
struct Frozen {handle:OwnedHandle,pid:u32,created:u64,resume:bool}
impl Drop for Frozen {fn drop(&mut self) {unsafe {if self.resume {NtResumeProcess(self.handle.as_raw_handle());}else{TerminateProcess(self.handle.as_raw_handle(),1);}}}}
impl Frozen {
    fn open(pid:u32,parent_created:u64)->Res<Option<Self>> {
        let raw=unsafe {OpenProcess(0x1000|0x0800|0x0100|0x0001,0,pid)};
        if raw.is_null() {if std::io::Error::last_os_error().raw_os_error()==Some(87) {return Ok(None);}return Err(error("open process"));}
        let handle=unsafe {OwnedHandle::from_raw_handle(raw)};
        let created=created(raw)?;
        if created<parent_created {return Ok(None);}
        let status=unsafe {NtSuspendProcess(raw)};
        if status<0 {return Err(format!("native job suspend: NTSTATUS {status:#x}"));}
        Ok(Some(Self {handle,pid,created,resume:false}))
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
        let root=Frozen::open(child.id(),0)?.ok_or("native server exited before job attachment")?;
        if root.created!=created(child.as_raw_handle())? || unsafe {GetProcessId(child.as_raw_handle())}!=root.pid {return Err("native server creation identity changed".into());}
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
                    if let Some(member)=Frozen::open(pid,parent.created)? {added.push(member);}
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
