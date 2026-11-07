//! Read-only component opens; held parents bind lookup, never grant native control.
use super::*;
use std::{ffi::c_void,os::windows::{ffi::OsStrExt,fs::{MetadataExt,OpenOptionsExt},io::{AsRawHandle,FromRawHandle,OwnedHandle}},path::{Component,Prefix}};

const REFUSED:&str="background locator provenance unknown: secure open refused";
const NONCANONICAL:&str="background locator provenance unknown: noncanonical home";
const REPARSE:u32=0x400;

// WDK ntdef.h / wdm.h: UNICODE_STRING, OBJECT_ATTRIBUTES and IO_STATUS_BLOCK.
// repr(C) supplies pointer-width padding; IO_STATUS_BLOCK's status/pointer union is pointer-sized.
#[repr(C)]
struct UnicodeString {length:u16,maximum_length:u16,buffer:*const u16}
#[repr(C)]
struct ObjectAttributes {length:u32,root:*mut c_void,name:*const UnicodeString,attributes:u32,security:*mut c_void,quality:*mut c_void}
#[repr(C)]
struct IoStatusBlock {status:usize,information:usize}
const _:()={
    let pointer=std::mem::size_of::<usize>();
    assert!(std::mem::size_of::<UnicodeString>()==if pointer==8 {16} else {8});
    assert!(std::mem::offset_of!(UnicodeString,buffer)==pointer);
    assert!(std::mem::size_of::<ObjectAttributes>()==6*pointer);
    assert!(std::mem::offset_of!(ObjectAttributes,root)==pointer);
    assert!(std::mem::offset_of!(ObjectAttributes,security)==4*pointer);
    assert!(std::mem::size_of::<IoStatusBlock>()==2*pointer);
};
#[link(name="ntdll")]
unsafe extern "system" {
    fn NtCreateFile(handle:*mut *mut c_void,access:u32,attributes:*const ObjectAttributes,status:*mut IoStatusBlock,
        allocation:*const i64,file_attributes:u32,share:u32,disposition:u32,options:u32,ea:*const c_void,ea_length:u32)->i32;
}

fn checked(file:File,directory:bool)->Res<File> {
    let meta=file.metadata().map_err(|_|REFUSED)?;
    if meta.file_attributes()&REPARSE!=0 || if directory {!meta.is_dir()} else {!meta.is_file()} {return Err(REFUSED.into());}
    Ok(file)
}

pub(super) fn background_directory(path:&Path)->Res<File> {
    let mut parts=path.components();
    let Some(Component::Prefix(prefix))=parts.next() else {return Err(NONCANONICAL.into());};
    match prefix.kind() {Prefix::Disk(drive)|Prefix::VerbatimDisk(drive) if drive.is_ascii_alphabetic()=>{},_=>return Err(NONCANONICAL.into())}
    if parts.next()!=Some(Component::RootDir) {return Err(NONCANONICAL.into());}
    let mut root=prefix.as_os_str().to_os_string();root.push(r"\");let root=PathBuf::from(root);let mut rebuilt=root.clone();
    let names=parts.map(|part|match part {Component::Normal(name)=>Ok(name),_=>Err(NONCANONICAL)}).collect::<Result<Vec<_>,_>>()?;
    for name in &names {rebuilt.push(name);}
    // Compare raw spelling: Path components alone silently discard dots and repeated separators.
    if rebuilt.as_os_str()!=path.as_os_str() {return Err(NONCANONICAL.into());}
    let mut options=File::options();
    // BACKUP_SEMANTICS | OPEN_REPARSE_POINT; normal read/write/delete sharing preserves native access.
    options.read(true).share_mode(7).custom_flags(0x02000000|0x00200000);
    let mut held=checked(options.open(root).map_err(|_|REFUSED)?,true)?;
    for name in names {held=background_open_at(&held,name,true)?;}
    Ok(held)
}

pub(super) fn background_open_at(parent:&File,name:&OsStr,directory:bool)->Res<File> {
    let encoded:Vec<u16>=name.encode_wide().collect();
    if encoded.is_empty() || encoded.iter().any(|c|matches!(*c,0|0x2f|0x5c|0x3a)) || name=="." || name==".."
        || matches!(encoded.last(),Some(0x20|0x2e)) {return Err(REFUSED.into());}
    let length=u16::try_from(encoded.len().checked_mul(2).ok_or(REFUSED)?).map_err(|_|REFUSED)?;
    let name=UnicodeString {length,maximum_length:length,buffer:encoded.as_ptr()};
    let attributes=ObjectAttributes {length:std::mem::size_of::<ObjectAttributes>() as u32,root:parent.as_raw_handle(),name:&name,
        attributes:0x40|0x1000,security:std::ptr::null_mut(),quality:std::ptr::null_mut()}; // CASE_INSENSITIVE | DONT_REPARSE
    let mut status=IoStatusBlock {status:0,information:0};let mut raw=std::ptr::null_mut();
    // SAFETY: system ABI matches NtCreateFile; all buffers and the owned parent live through this synchronous call.
    // FILE_OPEN only; READ_DATA/LIST_DIRECTORY | READ_ATTRIBUTES | SYNCHRONIZE; no mutation or inheritable handle.
    // SYNCHRONOUS_IO_NONALERT | OPEN_REPARSE_POINT | DIRECTORY_FILE/NON_DIRECTORY_FILE; share READ|WRITE|DELETE.
    let result=unsafe {NtCreateFile(&mut raw,0x00100081,&attributes,&mut status,std::ptr::null(),0,7,1,
        0x20|0x00200000|if directory {1} else {0x40},std::ptr::null(),0)};
    // An error must not become a File. Own any returned valid handle first so every refusal closes it exactly once.
    let handle=if raw.is_null() || raw==(-1isize as *mut c_void) {None} else {
        // SAFETY: NtCreateFile returned a new non-null, non-INVALID_HANDLE_VALUE handle; ownership transfers once.
        Some(unsafe {OwnedHandle::from_raw_handle(raw)})
    };
    // Only STATUS_SUCCESS qualifies; pending/informational/warning/error statuses do not prove a completed open.
    if result!=0 || status.status!=0 {return Err(REFUSED.into());}
    checked(File::from(handle.ok_or(REFUSED)?),directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tag:&str)->(PathBuf,BackgroundCandidate,String) {
        let root=crate::scratch(tag).canonicalize().unwrap();let home=root.join("account");let project=root.join("project");
        std::fs::create_dir_all(home.join("jobs/job")).unwrap();std::fs::create_dir(&project).unwrap();
        let candidate=BackgroundCandidate {runtime:"claude",home:home.clone(),home_identity:background_home_identity(&home).unwrap(),project:project.clone(),
            row:BackgroundRow::read(&json!({"kind":"background","id":"job","sessionId":"session","cwd":project,"state":"working","status":"busy"})).unwrap()};
        let raw=json!({"cwd":project,"sessionId":"session","state":"working","intent":"  owned input\n","detail":"owned failure"}).to_string();
        std::fs::write(home.join("jobs/job/state.json"),&raw).unwrap();(root,candidate,raw)
    }
    fn refuses(candidate:&BackgroundCandidate) {
        let mut budget=8192;assert!(candidate.input("owned input",&mut budget).is_err());assert_eq!(budget,8192,"refused open reads no native bytes");
        assert!(candidate.failure().is_err());
    }
    fn junction(target:&Path,link:&Path)->bool {
        let result=std::process::Command::new("cmd.exe").args([OsStr::new("/d"),OsStr::new("/c"),OsStr::new("mklink"),OsStr::new("/J"),link.as_os_str(),target.as_os_str()]).output().unwrap();
        if !result.status.success() {println!("SKIPPED junction fixture: {}",String::from_utf8_lossy(&result.stderr));return false;}
        true
    }
    fn symlink(result:std::io::Result<()>,kind:&str)->bool {
        match result {Ok(())=>true,Err(e) if matches!(e.raw_os_error(),Some(1314|5))=>{println!("SKIPPED {kind} fixture: {e}");false},Err(e)=>panic!("{kind} fixture creation failed: {e}")}
    }

    // Protect the actual shared lazy reader: real opened bytes, identity validation and aggregate caller budget.
    #[test]
    fn windows_locator_lazy_reader_preserves_validation_and_budget() {
        let (root,candidate,raw)=fixture("windows-locator-lazy");let path=candidate.home.join("jobs/job/state.json");let size=raw.len() as u64;
        let mut budget=2*size;assert!(candidate.input("owned input",&mut budget).unwrap());assert_eq!(budget,size);
        assert!(!candidate.input("owned",&mut budget).unwrap());assert_eq!(budget,0);
        assert!(candidate.input("owned input",&mut budget).is_err());assert_eq!(budget,0);
        assert_eq!(candidate.failure().unwrap().unwrap().detail.as_deref(),Some("owned failure"));
        let base:Value=serde_json::from_str(&raw).unwrap();
        for (key,value) in [("sessionId",json!("foreign")),("cwd",json!(root)),("intent",json!(["owned input"]))] {
            let mut row=base.clone();row[key]=value;std::fs::write(&path,row.to_string()).unwrap();
            assert!(candidate.input("owned input",&mut 8192).is_err());
        }
        let mut row=base.clone();row["state"]=json!("failed");std::fs::write(&path,row.to_string()).unwrap();assert!(candidate.failure().is_err());
        let mut row=base;row.as_object_mut().unwrap().remove("intent");row.as_object_mut().unwrap().remove("detail");std::fs::write(&path,row.to_string()).unwrap();
        assert!(!candidate.input("owned input",&mut 8192).unwrap());assert!(candidate.failure().unwrap().is_none());
        std::fs::write(&path,"{").unwrap();let mut budget=8192;assert!(candidate.input("owned input",&mut budget).is_err());assert_eq!(budget,8191);
        std::fs::write(&path," ".repeat(1024*1024+1)).unwrap();let mut budget=u64::MAX;assert!(candidate.input("owned input",&mut budget).is_err());assert_eq!(budget,u64::MAX);
        std::fs::write(&path,&raw).unwrap();let exclusive=File::options().read(true).share_mode(0).open(&path).unwrap();refuses(&candidate);drop(exclusive);
        std::fs::remove_file(&path).unwrap();refuses(&candidate);std::fs::create_dir(&path).unwrap();refuses(&candidate);
        let held=candidate.home_identity.clone();let moved=root.join("original-account");std::fs::rename(&candidate.home,&moved).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();std::fs::write(&path,&raw).unwrap();refuses(&candidate);
        let spec=Spec {client:Client::Claude,root:candidate.project.clone(),session:"session".into(),resume:false,args:vec![],home:Some(candidate.home.clone()),env:vec![]};
        assert!(background_candidates(vec![],&spec,held).is_err(),"replacement during listing refuses too");
        drop(candidate);std::fs::remove_dir_all(root).unwrap();println!("WINDOWS_LOCATOR_LAZY_PASS");
    }

    // Parent replacement must not redirect held handles; fresh traversal refuses every junction position.
    #[test]
    fn windows_locator_held_parents_and_reparse_refusal() {
        let (root,candidate,raw)=fixture("windows-locator-reparse");let job=candidate.home.join("jobs/job");let path=job.join("state.json");
        let foreign=root.join("foreign");let mut foreign_row:Value=serde_json::from_str(&raw).unwrap();foreign_row["detail"]=json!("foreign failure");let foreign_raw=foreign_row.to_string();
        std::fs::create_dir_all(foreign.join("job")).unwrap();std::fs::write(foreign.join("state.json"),&foreign_raw).unwrap();std::fs::write(foreign.join("job/state.json"),&foreign_raw).unwrap();
        let before=std::fs::read(foreign.join("state.json")).unwrap();
        let held=background_directory(&job).unwrap();let mut file=background_open_at(&held,OsStr::new("state.json"),false).unwrap();
        std::fs::rename(&path,job.join("original.json")).unwrap();std::fs::write(&path,"foreign replacement").unwrap();
        let mut actual=String::new();file.read_to_string(&mut actual).unwrap();assert_eq!(actual,raw,"opened file cannot redirect after replacement");drop(file);
        std::fs::remove_file(&path).unwrap();std::fs::rename(job.join("original.json"),&path).unwrap();
        std::fs::rename(&job,candidate.home.join("original-job")).unwrap();
        if junction(&foreign,&job) {
            let mut file=background_open_at(&held,OsStr::new("state.json"),false).unwrap();let mut actual=String::new();file.read_to_string(&mut actual).unwrap();assert_eq!(actual,raw);
            assert!(background_directory(&job).is_err());refuses(&candidate);std::fs::remove_dir(&job).unwrap();println!("WINDOWS_LOCATOR_HELD_PARENT_JUNCTION_PASS");
        }
        drop(held);std::fs::rename(candidate.home.join("original-job"),&job).unwrap();
        for (directory,target) in [(candidate.home.join("jobs"),foreign.clone()),(candidate.home.clone(),foreign.clone())] {
            let moved=directory.with_extension("original");std::fs::rename(&directory,&moved).unwrap();
            if junction(&target,&directory) {refuses(&candidate);assert!(background_directory(&directory).is_err());std::fs::remove_dir(&directory).unwrap();println!("WINDOWS_LOCATOR_JUNCTION_REFUSED {}",directory.display());}
            std::fs::rename(moved,directory).unwrap();
        }
        std::fs::remove_file(&path).unwrap();
        if symlink(std::os::windows::fs::symlink_file(foreign.join("state.json"),&path),"file symlink") {
            refuses(&candidate);std::fs::remove_file(&path).unwrap();println!("WINDOWS_LOCATOR_FILE_SYMLINK_PASS");
        }
        std::fs::write(&path,&raw).unwrap();std::fs::rename(&job,candidate.home.join("original-job")).unwrap();
        if symlink(std::os::windows::fs::symlink_dir(&foreign,&job),"directory symlink") {
            refuses(&candidate);std::fs::remove_dir(&job).unwrap();println!("WINDOWS_LOCATOR_DIRECTORY_SYMLINK_PASS");
        }
        std::fs::rename(candidate.home.join("original-job"),&job).unwrap();assert_eq!(std::fs::read(foreign.join("state.json")).unwrap(),before);
        drop(candidate);std::fs::remove_dir_all(root).unwrap();
    }

    // Refuse namespace/stream/normalization ambiguity before opening a locator; error outputs never become Files.
    #[test]
    fn windows_locator_path_forms_fail_closed() {
        let (root,candidate,_)=fixture("windows-locator-paths");let home=background_directory(&candidate.home).unwrap();
        for path in [r"relative\account",r"C:relative",r"\\server\share\account",r"\\?\UNC\server\share\account",r"\\.\C:\account",r"\\?\GLOBALROOT\Device\HarddiskVolume1\account",r"C:\account\..\other",r"C:\account\.\other",r"C:\account\\other",r"C:\account:stream"] {
            assert!(background_directory(Path::new(path)).is_err(),"{path}");
        }
        for name in ["",".","..","jobs/other",r"jobs\other","jobs:stream","jobs\0","jobs.","jobs ","missing"] {assert!(background_open_at(&home,OsStr::new(name),true).is_err(),"{name:?}");}
        assert!(background_open_at(&home,OsStr::new("jobs"),false).is_err(),"directory cannot be opened as a regular locator");
        drop(home);drop(candidate);std::fs::remove_dir_all(root).unwrap();println!("WINDOWS_LOCATOR_PATH_FORMS_PASS");
    }
}
