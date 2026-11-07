//! Native directory identity and one sign-in lease; no account path fallback.
use super::*;
pub(crate) struct Home {directory:File,identity:String,record:Option<(PathBuf,String)>,_writer:coreq::file_lock::Lock}
impl Home {
    pub fn open(path:&Path)->Res<Self> {
        let mut home=Self::with_lock(path,".agentq-login.lock")?;let record=path.join(".agentq-login.json");
        if let Some(row)=read(&record)? {
            if row["schema"]!="agentq-login-owner/1" || row["directory"]!=home.identity || !row["token"].as_str().is_some_and(super::super::super::native::plain_id) || row["phase"]!="settled" {return Err("login-owner-unknown".into());}
        }
        home.check(path)?;let token=crate::incident::uuid();
        save(&record,&json!({"schema":"agentq-login-owner/1","directory":home.identity,"token":token,"phase":"starting"})).map_err(String::from)?;
        home.record=Some((record,token));Ok(home)
    }
    pub fn finish(&self)->Result<(),&'static str> {
        let Some((path,token))=&self.record else {return Ok(())};self.check(path.parent().ok_or("login-owner-unknown")?).map_err(|_|"login-home-changed")?;
        let mut row=read(path).map_err(|_|"login-owner-unknown")?.ok_or("login-owner-unknown")?;
        if row["schema"]!="agentq-login-owner/1" || row["directory"]!=self.identity || row["token"]!=*token || !matches!(row["phase"].as_str(),Some("starting"|"settled")) {return Err("login-owner-unknown");}
        if row["phase"]=="settled" {return Ok(())}row["phase"]=json!("settled");save(path,&row)
    }
    pub fn with_lock(path:&Path,name:&str)->Res<Self> {
        if !path.is_absolute() {return Err("login-home-unverified".into());}
        let directory=open(path).map_err(|_|"login-home-missing")?;
        if !directory.metadata().map_err(|_|"login-home-unverified")?.is_dir() {return Err("login-home-missing".into());}
        owned(&directory)?;let identity=super::super::super::native::file_id(&directory)?;
        let lock=path.join(name);
        if std::fs::symlink_metadata(&lock).is_ok_and(|m|!m.is_file() || m.file_type().is_symlink()) {return Err("login-home-lock-unverified".into());}
        let mut options=std::fs::OpenOptions::new();options.write(true).create(true).truncate(false);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
        #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.custom_flags(0x00200000);}
        let file=options.open(lock).map_err(|_|"login-home-lock-unverified")?;
        let metadata=file.metadata().map_err(|_|"login-home-lock-unverified")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {return Err("login-home-lock-unverified".into());}
        #[cfg(windows)] {use std::os::windows::fs::MetadataExt;if metadata.file_attributes()&0x400!=0 {return Err("login-home-lock-unverified".into());}}
        // Account ownership comes from the retained directory; the lock carries no credentials.
        let _writer=coreq::file_lock::try_lock_file(file).map_err(|_|"login-home-busy")?;
        let home=Self {directory,identity,record:None,_writer};home.check(path)?;Ok(home)
    }
    pub fn check(&self,path:&Path)->Res<()> {
        let now=open(path).map_err(|_|"login-home-changed")?;owned(&now)?;
        if super::super::super::native::file_id(&now)?!=self.identity || super::super::super::native::file_id(&self.directory)?!=self.identity {return Err("login-home-changed".into());}Ok(())
    }
}
fn open(path:&Path)->std::io::Result<File> {
    let mut options=std::fs::OpenOptions::new();options.read(true);
    #[cfg(windows)] {use std::os::windows::fs::OpenOptionsExt;options.custom_flags(0x02000000);}
    options.open(path)
}
#[cfg(unix)]
fn owned(file:&File)->Res<()> {use std::os::unix::fs::MetadataExt;if file.metadata().map_err(|_|"login-home-unverified")?.uid()!=unsafe {libc::geteuid()} {return Err("login-home-foreign".into());}Ok(())}
#[cfg(windows)]
fn owned(file:&File)->Res<()> {
    use std::os::windows::io::AsRawHandle;
    type Ptr=*mut std::ffi::c_void;
    #[link(name="kernel32")] unsafe extern "system" {fn GetCurrentProcess()->Ptr;fn CloseHandle(p:Ptr)->i32;fn LocalFree(p:Ptr)->Ptr;}
    #[link(name="advapi32")] unsafe extern "system" {
        fn GetSecurityInfo(handle:Ptr,kind:u32,information:u32,owner:*mut Ptr,group:*mut Ptr,dacl:*mut Ptr,sacl:*mut Ptr,descriptor:*mut Ptr)->u32;
        fn OpenProcessToken(process:Ptr,access:u32,token:*mut Ptr)->i32;
        fn GetTokenInformation(token:Ptr,class:u32,buffer:Ptr,length:u32,needed:*mut u32)->i32;
        fn EqualSid(a:Ptr,b:Ptr)->i32;
    }
    let (mut owner,mut descriptor,mut token)=(std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut());
    if unsafe {GetSecurityInfo(file.as_raw_handle(),1,1,&mut owner,std::ptr::null_mut(),std::ptr::null_mut(),std::ptr::null_mut(),&mut descriptor)}!=0 {return Err("login-home-unverified".into());}
    struct Local(Ptr);impl Drop for Local {fn drop(&mut self) {unsafe {LocalFree(self.0);}}}let _descriptor=Local(descriptor);
    if unsafe {OpenProcessToken(GetCurrentProcess(),8,&mut token)}==0 {return Err("login-home-unverified".into());}
    struct Token(Ptr);impl Drop for Token {fn drop(&mut self) {unsafe {CloseHandle(self.0);}}}let _token=Token(token);
    let mut needed=0;unsafe {GetTokenInformation(token,1,std::ptr::null_mut(),0,&mut needed);}
    let mut buffer=vec![0usize;(needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {GetTokenInformation(token,1,buffer.as_mut_ptr().cast(),needed,&mut needed)}==0 {return Err("login-home-unverified".into());}
    if owner.is_null() || unsafe {EqualSid(owner,*(buffer.as_ptr().cast::<Ptr>()))}==0 {return Err("login-home-foreign".into());}Ok(())
}

fn read(path:&Path)->Res<Option<Value>> {
    match std::fs::symlink_metadata(path) {Ok(m) if !m.is_file() || m.file_type().is_symlink()=>return Err("login-owner-unknown".into()),Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err("login-owner-unknown".into()),_=>{}}
    serde_json::from_slice(&std::fs::read(path).map_err(|_|"login-owner-unknown")?).map(Some).map_err(|_|"login-owner-unknown".into())
}
fn save(path:&Path,row:&Value)->Result<(),&'static str> {coreq::write_durable(path,serde_json::to_vec(row).map_err(|_|"login-owner-write-unknown")?).map_err(|_|"login-owner-write-unknown")}
