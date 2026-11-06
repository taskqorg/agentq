//! Native process scopes; owner death never becomes replay or protected-host authority.
use super::*;
use serde::{Deserialize,Serialize};

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,Serialize,Deserialize)]
#[serde(rename_all="kebab-case")]
pub enum Lifetime {#[default] Recoverable,StopWithOwner}

#[cfg(any(target_os="macos",target_os="linux"))]
mod platform {
    use super::*;
    use std::{fs::File,io::Read,os::fd::{AsRawFd,FromRawFd,OwnedFd},os::unix::process::CommandExt,sync::{Arc,Mutex}};

    fn pipe()->Res<(File,File)> {
        let mut fds=[-1;2];
        if unsafe {libc::pipe(fds.as_mut_ptr())}!=0 {return Err(std::io::Error::last_os_error().to_string());}
        let originals=unsafe {(File::from_raw_fd(fds[0]),File::from_raw_fd(fds[1]))};
        let duplicate=|fd| {let raw=unsafe {libc::fcntl(fd,libc::F_DUPFD_CLOEXEC,3)};if raw<0 {Err(std::io::Error::last_os_error().to_string())} else {Ok(unsafe {File::from_raw_fd(raw)})}};
        Ok((duplicate(originals.0.as_raw_fd())?,duplicate(originals.1.as_raw_fd())?))
    }

    #[cfg(target_os="linux")]
    fn watch(pid:u32)->std::io::Result<OwnedFd> {
        let fd=unsafe {libc::syscall(libc::SYS_pidfd_open,pid as libc::pid_t,0)};
        if fd<0 {Err(std::io::Error::last_os_error())} else {Ok(unsafe {OwnedFd::from_raw_fd(fd as i32)})}
    }
    #[cfg(target_os="macos")]
    fn watch(pid:u32)->std::io::Result<OwnedFd> {
        let fd=unsafe {libc::kqueue()};if fd<0 {return Err(std::io::Error::last_os_error());}
        let owned=unsafe {OwnedFd::from_raw_fd(fd)};
        if unsafe {libc::fcntl(fd,libc::F_SETFD,libc::FD_CLOEXEC)}<0 {return Err(std::io::Error::last_os_error());}
        let event=libc::kevent {ident:pid as usize,filter:libc::EVFILT_PROC,flags:libc::EV_ADD|libc::EV_ONESHOT,fflags:libc::NOTE_EXIT,data:0,udata:std::ptr::null_mut()};
        if unsafe {libc::kevent(fd,&event,1,std::ptr::null_mut(),0,std::ptr::null())}<0 {return Err(std::io::Error::last_os_error());}
        Ok(owned)
    }

    extern "C" fn stop_group(_:i32) {unsafe {libc::kill(-libc::getpgrp(),libc::SIGKILL);libc::_exit(1)}}

    // Only async-signal-safe calls run between fork and exec; no Rust allocator, locks or inherited report pipes.
    unsafe fn guardian(owner:i32,native:u32,report:i32,policy:Lifetime,max_fd:i32)->! {unsafe {
        let monitor=match watch(native) {Ok(fd)=>fd,Err(_)=>{if policy==Lifetime::StopWithOwner {stop_group(0);}libc::_exit(0)}};
        let a=libc::fcntl(owner,libc::F_DUPFD_CLOEXEC,6);
        let b=libc::fcntl(monitor.as_raw_fd(),libc::F_DUPFD_CLOEXEC,6);
        let c=libc::fcntl(report,libc::F_DUPFD_CLOEXEC,6);
        if a<0 || b<0 || c<0 {libc::_exit(1);}
        if libc::dup2(a,3)<0 || libc::dup2(b,4)<0 || libc::dup2(c,5)<0 {libc::_exit(1);}
        for fd in 0..3 {libc::close(fd);}
        #[cfg(target_os="macos")] for fd in 6..max_fd {libc::close(fd);}
        let _=max_fd;
        #[cfg(target_os="linux")] if libc::syscall(libc::SYS_close_range,6u32,u32::MAX,0u32)<0 {libc::_exit(1);}
        let mut action:libc::sigaction=std::mem::zeroed();action.sa_sigaction=stop_group as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        let mut unblocked:libc::sigset_t=std::mem::zeroed();libc::sigemptyset(&mut unblocked);libc::sigaddset(&mut unblocked,libc::SIGUSR1);
        libc::sigprocmask(libc::SIG_UNBLOCK,&unblocked,std::ptr::null_mut());
        if libc::sigaction(libc::SIGUSR1,&action,std::ptr::null_mut())<0 {libc::_exit(1);}
        let pid=(libc::getpid() as u32).to_ne_bytes();
        if libc::write(5,pid.as_ptr().cast(),pid.len())!=pid.len() as isize {libc::_exit(1);}
        libc::close(5);
        let mut polls=[libc::pollfd {fd:3,events:libc::POLLIN,revents:0},libc::pollfd {fd:4,events:libc::POLLIN,revents:0}];
        loop {
            if libc::poll(polls.as_mut_ptr(),2,-1)<0 {if std::io::Error::last_os_error().kind()==std::io::ErrorKind::Interrupted {continue;}libc::_exit(1);}
            if polls[1].revents!=0 {
                if policy==Lifetime::StopWithOwner {stop_group(0);}
                libc::_exit(0);
            }
            if polls[0].revents!=0 {
                if policy==Lifetime::StopWithOwner {stop_group(0);}
                libc::close(3);polls[0].fd=-1;
            }
        }
    }}

    struct Pipes {read:File,write:File,report:File,send:File}
    pub struct Setup {pipes:Mutex<Option<Pipes>>,policy:Lifetime,max_fd:i32}
    impl Setup {
        pub fn new(policy:Lifetime)->Res<Arc<Self>> {
            let (read,write)=pipe()?;let (report,send)=pipe()?;
            let max_fd=unsafe {libc::sysconf(libc::_SC_OPEN_MAX)};if max_fd<0 || max_fd>i32::MAX as _ {return Err("lifetime descriptor range unknown".into());}
            Ok(Arc::new(Self {pipes:Mutex::new(Some(Pipes {read,write,report,send})),policy,max_fd:max_fd as i32}))
        }
        pub fn configure(&self,command:&mut Command) {
            let pipes=self.pipes.lock().unwrap();let pipes=pipes.as_ref().unwrap();
            let (owner,report)=(pipes.read.as_raw_fd(),pipes.send.as_raw_fd());let policy=self.policy;let max_fd=self.max_fd;
            unsafe {command.pre_exec(move|| {
                if libc::setpgid(0,0)<0 {return Err(std::io::Error::last_os_error());}
                let native=libc::getpid() as u32;
                let pid=libc::fork();if pid<0 {return Err(std::io::Error::last_os_error());}
                if pid==0 {guardian(owner,native,report,policy,max_fd);}
                Ok(())
            });}
        }
        pub fn started(&self,child:&std::process::Child)->Res<Value> {
            let Pipes {read,write,mut report,send}=self.pipes.lock().map_err(|_|"lifetime setup poisoned")?.take().ok_or("lifetime already started")?;
            drop(read);drop(send);
            let mut bytes=[0;4];report.read_exact(&mut bytes).map_err(|e|format!("lifetime guardian report: {e}"))?;
            let pid=u32::from_ne_bytes(bytes);let guard=NativeProcess::read(pid)?.ok_or("lifetime guardian identity unavailable")?;
            let monitor=watch(pid).map_err(|e|e.to_string())?;
            std::thread::Builder::new().name("agentq-lifetime".into()).spawn(move|| {
                let mut poll=libc::pollfd {fd:monitor.as_raw_fd(),events:libc::POLLIN,revents:0};
                while unsafe {libc::poll(&mut poll,1,-1)}<0 && std::io::Error::last_os_error().kind()==std::io::ErrorKind::Interrupted {}
                drop(write);
            }).map_err(|e|e.to_string())?;
            Ok(json!({"kind":"posix-group","group":child.id(),"guardian":guard}))
        }
        pub fn release(&self,_child:&std::process::Child)->Res<()> {Ok(())}
    }

    pub fn verified(scope:&Value,native:&NativeProcess)->Res<()> {
        if NativeProcess::read(native.pid)?.as_ref()!=Some(native) {return Err("orphan-native-identity-changed".into());}
        if scope["kind"]!="posix-group" || scope["group"].as_u64()!=Some(u64::from(native.pid)) {return Err("orphan-scope-unknown".into());}
        let guard:NativeProcess=serde_json::from_value(scope["guardian"].clone()).map_err(|_|"orphan-scope-unknown")?;
        if NativeProcess::read(guard.pid)?.as_ref()!=Some(&guard) || unsafe {libc::getpgid(guard.pid as i32)}!=native.pid as i32 {return Err("orphan-scope-unknown".into());}
        Ok(())
    }
    pub fn absent(scope:&Value)->Res<bool> {
        if scope["kind"]!="posix-group" {return Err("orphan-scope-unknown".into());}
        let group=scope["group"].as_u64().filter(|n|*n>1 && *n<=i32::MAX as u64).ok_or("orphan-scope-unknown")? as i32;
        if unsafe {libc::kill(-group,0)}==0 {return Ok(false);}
        let error=std::io::Error::last_os_error();if error.raw_os_error()==Some(libc::ESRCH) {Ok(true)} else {Err(format!("orphan-scope-unknown: {error}"))}
    }
    pub fn stop(scope:&Value,native:&NativeProcess)->Res<()> {
        verified(scope,native)?;
        let guard:NativeProcess=serde_json::from_value(scope["guardian"].clone()).map_err(|_|"orphan-scope-unknown")?;
        signal(&guard).map_err(|e|format!("orphan-scope-unknown: {e}"))
    }

    #[cfg(target_os="linux")]
    fn signal(expected:&NativeProcess)->std::io::Result<()> {
        let fd=watch(expected.pid)?;
        if NativeProcess::read(expected.pid).map_err(std::io::Error::other)?.as_ref()!=Some(expected) {return Err(std::io::Error::other("guardian identity changed"));}
        if unsafe {libc::syscall(libc::SYS_pidfd_send_signal,fd.as_raw_fd(),libc::SIGUSR1,std::ptr::null::<libc::siginfo_t>(),0u32)}<0 {return Err(std::io::Error::last_os_error());}
        Ok(())
    }
    #[cfg(target_os="macos")]
    fn signal(expected:&NativeProcess)->std::io::Result<()> {
        #[repr(C)] struct Unique {uuid:[u8;16],id:u64,parent:u64,version:i32,parent_version:i32,reserved:[u64;2]}
        let mut info:Unique=unsafe {std::mem::zeroed()};
        if unsafe {libc::proc_pidinfo(expected.pid as i32,17,0,(&mut info as *mut Unique).cast(),std::mem::size_of::<Unique>() as i32)}!=56 {return Err(std::io::Error::last_os_error());}
        if NativeProcess::read(expected.pid).map_err(std::io::Error::other)?.as_ref()!=Some(expected) {return Err(std::io::Error::other("guardian identity changed"));}
        let address=unsafe {libc::dlsym(libc::RTLD_DEFAULT,c"proc_signal_with_audittoken".as_ptr())};
        if address.is_null() {return Err(std::io::Error::other("identity-bound guardian signal unavailable"));}
        let send:unsafe extern "C" fn(*mut [u32;8],i32)->i32=unsafe {std::mem::transmute(address)};
        let mut token=[0;8];token[5]=expected.pid;token[7]=info.version as u32;
        let error=unsafe {send(&mut token,libc::SIGUSR1)};
        if error!=0 {return Err(std::io::Error::from_raw_os_error(error));}Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{os::windows::{io::{AsRawHandle,FromRawHandle,OwnedHandle},process::CommandExt},sync::Arc};
    type Handle=*mut std::ffi::c_void;
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes:Handle,name:*const u16)->Handle;
        fn OpenJobObjectW(access:u32,inherit:i32,name:*const u16)->Handle;
        fn SetInformationJobObject(job:Handle,class:u32,data:*const std::ffi::c_void,size:u32)->i32;
        fn QueryInformationJobObject(job:Handle,class:u32,data:*mut std::ffi::c_void,size:u32,returned:*mut u32)->i32;
        fn AssignProcessToJobObject(job:Handle,process:Handle)->i32;
        fn IsProcessInJob(process:Handle,job:Handle,result:*mut i32)->i32;
        fn TerminateJobObject(job:Handle,code:u32)->i32;
        fn GetCurrentProcess()->Handle;
        fn DuplicateHandle(source:Handle,handle:Handle,target:Handle,out:*mut Handle,access:u32,inherit:i32,options:u32)->i32;
        fn OpenProcess(access:u32,inherit:i32,pid:u32)->Handle;
        fn WaitForSingleObject(handle:Handle,millis:u32)->u32;
        fn GetLastError()->u32;
    }
    #[link(name="ntdll")]
    unsafe extern "system" {fn NtResumeProcess(process:Handle)->i32;}
    #[repr(C)] #[derive(Default)] struct Limits {process_time:i64,job_time:i64,flags:u32,min:usize,max:usize,active:u32,affinity:usize,priority:u32,scheduling:u32}
    #[repr(C)] #[derive(Default)] struct Extended {basic:Limits,io:[u64;6],process_memory:usize,job_memory:usize,peak_process:usize,peak_job:usize}
    #[repr(C)] #[derive(Default)] struct Accounting {user:i64,kernel:i64,period_user:i64,period_kernel:i64,faults:u32,total:u32,active:u32,terminated:u32}
    fn owned(raw:Handle)->Res<OwnedHandle> {if raw.is_null() {Err(std::io::Error::last_os_error().to_string())} else {Ok(unsafe {OwnedHandle::from_raw_handle(raw)})}}
    fn wide(name:&str)->Vec<u16> {name.encode_utf16().chain(Some(0)).collect()}
    fn duplicate(handle:Handle)->Res<OwnedHandle> {
        let mut out=std::ptr::null_mut();let current=unsafe {GetCurrentProcess()};
        if unsafe {DuplicateHandle(current,handle,current,&mut out,0,0,2)}==0 {return Err(std::io::Error::last_os_error().to_string());}owned(out)
    }
    pub struct Setup {job:OwnedHandle,name:String,policy:Lifetime}
    impl Setup {
        pub fn new(policy:Lifetime)->Res<Arc<Self>> {
            let name=format!("Local\\agentq-{}",crate::incident::uuid());let job=owned(unsafe {CreateJobObjectW(std::ptr::null_mut(),wide(&name).as_ptr())})?;
            if unsafe {GetLastError()}==183 {return Err("lifetime job already exists".into());}
            if policy==Lifetime::StopWithOwner {
                let limits=Extended {basic:Limits {flags:0x2000,..Default::default()},..Default::default()};
                if unsafe {SetInformationJobObject(job.as_raw_handle(),9,(&limits as *const Extended).cast(),std::mem::size_of::<Extended>() as u32)}==0 {return Err(std::io::Error::last_os_error().to_string());}
            }
            Ok(Arc::new(Self {job,name,policy}))
        }
        pub fn configure(&self,command:&mut Command) {command.creation_flags(0x08000004);}
        pub fn started(&self,child:&std::process::Child)->Res<Value> {
            if unsafe {AssignProcessToJobObject(self.job.as_raw_handle(),child.as_raw_handle())}==0 {return Err(format!("lifetime job assignment: {}",std::io::Error::last_os_error()));}
            if self.policy==Lifetime::Recoverable {
                let mut remote=std::ptr::null_mut();
                // Non-inheritable remote handle preserves this named Job across original controller death.
                if unsafe {DuplicateHandle(GetCurrentProcess(),self.job.as_raw_handle(),child.as_raw_handle(),&mut remote,0,0,2)}==0 {return Err(std::io::Error::last_os_error().to_string());}
            }
            let job=duplicate(self.job.as_raw_handle())?;let native=duplicate(child.as_raw_handle())?;
            std::thread::Builder::new().name("agentq-lifetime".into()).spawn(move|| {unsafe {WaitForSingleObject(native.as_raw_handle(),u32::MAX);}drop(job);}).map_err(|e|e.to_string())?;
            Ok(json!({"kind":"windows-job","name":self.name}))
        }
        pub fn release(&self,child:&std::process::Child)->Res<()> {
            let status=unsafe {NtResumeProcess(child.as_raw_handle())};if status<0 {Err(format!("lifetime resume: NTSTATUS {status:#x}"))} else {Ok(())}
        }
    }
    fn name(scope:&Value)->Res<&str> {
        let name=scope["name"].as_str().ok_or("orphan-scope-unknown")?;
        if scope["kind"]!="windows-job" || !name.strip_prefix("Local\\agentq-").is_some_and(native::plain_id) {return Err("orphan-scope-unknown".into());}Ok(name)
    }
    fn open(scope:&Value,access:u32)->Res<Option<OwnedHandle>> {
        let raw=unsafe {OpenJobObjectW(access,0,wide(name(scope)?).as_ptr())};
        if raw.is_null() {let code=unsafe {GetLastError()};if code==2 {Ok(None)} else {Err(format!("orphan-scope-unknown: {}",std::io::Error::from_raw_os_error(code as i32)))}} else {owned(raw).map(Some)}
    }
    fn member(job:&OwnedHandle,native:&NativeProcess)->Res<()> {
        let process=owned(unsafe {OpenProcess(0x1000,0,native.pid)})?;
        if NativeProcess::read(native.pid)?.as_ref()!=Some(native) {return Err("orphan-native-identity-changed".into());}
        let mut inside=0;if unsafe {IsProcessInJob(process.as_raw_handle(),job.as_raw_handle(),&mut inside)}==0 || inside==0 {return Err("orphan-scope-unknown".into());}Ok(())
    }
    pub fn verified(scope:&Value,native:&NativeProcess)->Res<()> {let job=open(scope,4)?.ok_or("orphan-scope-absent")?;member(&job,native)}
    pub fn stop(scope:&Value,native:&NativeProcess)->Res<()> {
        let job=open(scope,4|8)?.ok_or("orphan-scope-absent")?;member(&job,native)?;
        if unsafe {TerminateJobObject(job.as_raw_handle(),1)}==0 {return Err(std::io::Error::last_os_error().to_string());}Ok(())
    }
    pub fn absent(scope:&Value)->Res<bool> {
        let Some(job)=open(scope,4)? else {return Ok(true)};let mut accounting=Accounting::default();
        if unsafe {QueryInformationJobObject(job.as_raw_handle(),1,(&mut accounting as *mut Accounting).cast(),std::mem::size_of::<Accounting>() as u32,std::ptr::null_mut())}==0 {return Err(std::io::Error::last_os_error().to_string());}Ok(accounting.active==0)
    }
}

#[cfg(not(any(windows,target_os="linux",target_os="macos")))]
mod platform {
    use super::*;use std::sync::Arc;
    pub struct Setup;
    impl Setup {
        pub fn new(_:Lifetime)->Res<Arc<Self>> {Err("lifetime scope unsupported".into())}
        pub fn configure(&self,_:&mut Command) {}
        pub fn started(&self,_:&std::process::Child)->Res<Value> {Err("lifetime scope unsupported".into())}
        pub fn release(&self,_:&std::process::Child)->Res<()> {Ok(())}
    }
    pub fn verified(_:&Value,_:&NativeProcess)->Res<()> {Err("orphan-scope-unknown".into())}
    pub fn stop(_:&Value,_:&NativeProcess)->Res<()> {Err("orphan-scope-unknown".into())}
    pub fn absent(_:&Value)->Res<bool> {Err("orphan-scope-unknown".into())}
}

pub(super) use platform::{Setup,verified,stop,absent};
