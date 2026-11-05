//! M12.8: Pipe authenticates occupancy. A permit never grants filesystem or process authority.
use super::*;

pub(super) fn validate(install:&Path,permit:&Value)->Res<String> {
    super::super::protected::validate_slot(permit)?;
    let b=&permit["binding"];
    if b["schema"]!="pipe.permits/1" {return Err("independent host requires caller-owned permit; no fallback".into());}
    let p=policy(install,Path::new(b["root"].as_str().ok_or("project unknown")?))?;
    let caller=b["caller"].as_str().ok_or("caller unknown")?;
    let issuer=p["permit_public_keys"].get(caller).and_then(Value::as_str).ok_or("permit issuer not authorized")?;
    let payload=json!({"binding":b,"key_sha256":digest(permit["key"].as_str().ok_or("permit key unknown")?.as_bytes())});
    verify_signature(issuer,permit["signature"].as_str().ok_or("permit signature absent")?,"pipe.permits/1",payload.to_string().as_bytes())?;
    Ok(format!("op-{}",digest(format!("{}\0{}",caller,b["op_id"].as_str().ok_or("operation unknown")?).as_bytes())))
}
pub(super) fn authenticate(install:&Path,s:&resources::State,permit:&Value)->Res<String> {
    let key=operation_id(&Request{schema:SCHEMA.into(),permit:permit.clone(),policy_revision:1,account:"recovery".into(),preparation:String::new()})?;
    let op=s.ops.get(&key).ok_or("original permit absent; retain occupancy")?;
    if op.bound["local_permit"]["binding"]!=permit["binding"]||op.bound["local_permit"]["key_hash"]!=digest(permit["key"].as_str().ok_or("permit key unknown")?.as_bytes())
        ||op.bound["local_permit"]["permit_sha256"]!=digest(permit.to_string().as_bytes()) {return Err("permit binding/key changed; retain occupancy".into());}
    let _=install;Ok(key)
}
pub(super) fn reserved(install:&Path,s:&resources::State,permit:&Value,runtime:&str)->Res<String> {
    let key=validate(install,permit)?;
    if authenticate(install,s,permit)?!=key||runtime!="codex" {return Err("permit identity changed".into());}
    let b=&permit["binding"];let owner=native(b)?;
    if boot_time()?.abs_diff(b["boot"].as_i64().ok_or("permit boot unknown")?)>2 {return Err("permit boot changed; retain occupancy".into());}
    exact(&owner,coreq::process::identity(owner.pid).map_err(|_|"caller liveness unknown".to_string()))?;Ok(key)
}
pub(super) fn prepare(install:&Path,s:&mut resources::State,permit:&Value)->Res<String> {
    let key=validate(install,permit)?;let b=&permit["binding"];
    if install.join("retained").join(format!("{key}.json")).try_exists().map_err(|_|"retained operation inventory unknown")? {return Err("retained operation is recovery-only; never relaunch".into());}
    #[cfg(unix)] if native(b)?.pid!=unsafe{libc::getppid()} as u32 {return Err("preparation caller differs from actual parent".into());}
    if !s.ops.contains_key(&key) {
        let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"clock unknown")?.as_secs();
        if b["expires_at"].as_u64().is_none_or(|t|t<now) {return Err("unprepared permit expired; no launch".into());}
        s.ops.insert(key.clone(),resources::Op{bound:json!({"local_permit":{"binding":b,"key_hash":digest(permit["key"].as_str().ok_or("permit key unknown")?.as_bytes()),"permit_sha256":digest(permit.to_string().as_bytes())}}),receipt:json!({"occupied":true,"instance":random_id()?})});
    }
    reserved(install,s,permit,"codex")
}
pub(super) fn exchange(install:&Path,_action:&str,permit:&Value)->Res<Value> {
    resources::within(install,resources::WAIT,|s|{let key=authenticate(install,s,permit)?;Ok(s.ops[&key].receipt.clone())})
}
#[cfg(target_os="macos")]
pub(super) fn boot_time()->Res<i64> {
    let mut time=std::mem::MaybeUninit::<libc::timeval>::zeroed();let mut size=std::mem::size_of::<libc::timeval>();
    let mut mib=[libc::CTL_KERN,libc::KERN_BOOTTIME];
    if unsafe{libc::sysctl(mib.as_mut_ptr(),2,time.as_mut_ptr().cast(),&mut size,std::ptr::null_mut(),0)}!=0||size!=std::mem::size_of::<libc::timeval>() {return Err("host boot unknown".into());}
    let seconds=unsafe{time.assume_init()}.tv_sec;
    if seconds<=0 {return Err("host boot unknown".into());}Ok(seconds)
}
#[cfg(not(target_os="macos"))]
pub(super) fn boot_time()->Res<i64> {Err("native host boot unsupported".into())}
