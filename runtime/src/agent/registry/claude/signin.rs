//! Claude's printed pasted-code flow. Native credentials never leave this module.
use super::*;
use super::super::super::sign_in::{self,Context,Conversation,Action,LoginFlow,LoginPrompt};
pub(super) static PROTOCOL:sign_in::Protocol=sign_in::Protocol {executable,configure,credentials,verify:|c|login(c.command(),true)};

fn executable(program:PathBuf)->Res<PathBuf> {
    #[cfg(windows)] if program.file_name().and_then(OsStr::to_str).is_some_and(|name|name.eq_ignore_ascii_case("claude.cmd")) {
        let package=program.parent().ok_or("login-native-executable-required")?.join("node_modules/@anthropic-ai/claude-code");
        let metadata:Value=serde_json::from_slice(&std::fs::read(package.join("package.json")).map_err(|_|"login-native-executable-required")?).map_err(|_|"login-native-executable-required")?;
        if metadata["name"]!="@anthropic-ai/claude-code" {return Err("login-native-executable-required".into());}
        let native=package.join("bin/claude.exe");let mut magic=[0;2];
        File::open(&native).and_then(|mut file|file.read_exact(&mut magic)).map_err(|_|"login-native-executable-required")?;
        if magic!=*b"MZ" {return Err("login-native-executable-required".into());}return Ok(native);
    }
    sign_in::direct_executable(program)
}
fn configure(c:&Context,flow:LoginFlow,command:&mut Command)->Result<Box<dyn Conversation>,&'static str> {
    if flow!=LoginFlow::Browser {return Err("login-device-unsupported".into());}
    let helper=c.home.join(format!(".agentq-login-browser-{}{}",crate::incident::uuid(),if cfg!(windows) {".cmd"}else{".sh"}));
    let mut options=std::fs::OpenOptions::new();options.write(true).create_new(true);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o700);}
    #[cfg(not(test))] let script:&[u8]=if cfg!(windows) {b"@exit /b 0\r\n"}else{b"#!/bin/sh\nexit 0\n"};
    #[cfg(test)] let script:&[u8]=if cfg!(windows) {b"@type nul > \"%~f0.called\"\r\n@exit /b 0\r\n"}else{b"#!/bin/sh\n: > \"$0.called\"\nexit 0\n"};
    options.open(&helper).and_then(|mut file|file.write_all(script)).map_err(|_|"login-browser-suppression-failed")?;
    command.args(["auth","login","--claudeai"]).env("BROWSER",&helper);
    Ok(Box::new(Claude {text:sign_in::Text::default(),prompt:false,helper}))
}
struct Claude {text:sign_in::Text,prompt:bool,helper:PathBuf}
impl Conversation for Claude {
    fn output(&mut self,stream:usize,bytes:&[u8])->Res<Vec<Action>> {
        let text=self.text.append(stream,bytes);let lower=text.to_ascii_lowercase();
        for (marker,reason) in [("access_denied","login-denied"),("authorization request was denied","login-denied"),("invalid authorization code","login-code-rejected"),("code expired","login-code-expired")] {if lower.contains(marker) {return Ok(vec![Action::Failure(reason)]);}}
        if self.prompt {return Ok(vec![]);}
        if let Some(url)=sign_in::tokens(&text).find(|u|valid_url(u)) {self.prompt=true;return Ok(vec![Action::Prompt(LoginPrompt {url:url.into(),user_code:None,needs_code:true})]);}Ok(vec![])
    }
    fn code(&mut self,code:&str)->Res<Vec<u8>> {
        if !self.prompt || code.is_empty() || !code.bytes().all(|b|b.is_ascii_graphic()) {return Err("login-code-invalid".into());}
        Ok(format!("{code}\n").into_bytes())
    }
    fn completed(&self,exit:Option<i32>)->bool {exit==Some(0)}
    fn cleanup(&mut self)->Res<()> {
        #[cfg(test)] {let mut marker=self.helper.as_os_str().to_os_string();marker.push(".called");match std::fs::remove_file(PathBuf::from(marker)) {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>return Err("login-helper-cleanup-unknown".into())}}
        std::fs::remove_file(&self.helper).map_err(|_|"login-helper-cleanup-unknown".into())
    }
}
fn valid_url(url:&str)->bool {
    let Some(query)=["https://claude.ai/oauth/authorize?","https://claude.com/oauth/authorize?","https://claude.com/cai/oauth/authorize?"].iter().find_map(|p|url.strip_prefix(p)) else {return false};
    let params:Vec<_>=query.split('&').filter_map(|s|s.split_once('=')).collect();
    params.iter().any(|p|*p==("code","true")) && params.iter().any(|(k,v)|*k=="redirect_uri" && v.to_ascii_lowercase().starts_with("https%3a%2f%2f") && !v.chars().any(char::is_whitespace))
}
fn credentials(c:&Context)->Res<Vec<Option<String>>> {
    let mut result=vec![sign_in::file_fingerprint(&c.home.join(".credentials.json"))?];
    #[cfg(target_os="macos")] {
        let service=if c.explicit {format!("Claude Code-credentials-{}",&crate::sha256_hex(nfc(c.home.to_str().ok_or("login-home-unverified")?)?.as_bytes())[..8])}else{"Claude Code-credentials".into()};
        let user=c.env_value("USER").or_else(||c.env_value("USERNAME")).ok_or("login-keychain-account-unverified")?;
        let user=user.to_str().filter(|s|!s.is_empty()).ok_or("login-keychain-account-unverified")?;
        let account=if user.bytes().all(|b|b.is_ascii_alphanumeric() || b"._-".contains(&b)) {user}else{"claude-code-user"};
        result.push(limits::keychain_fingerprint(&service,Some(account))?);
    }
    Ok(result)
}
#[cfg(target_os="macos")]
fn nfc(text:&str)->Res<String> {
    use std::ffi::c_void;type Ptr=*const c_void;
    #[link(name="CoreFoundation",kind="framework")] unsafe extern "C" {
        fn CFStringCreateWithBytes(a:Ptr,b:*const u8,n:isize,e:u32,external:bool)->Ptr;
        fn CFStringCreateMutableCopy(a:Ptr,n:isize,s:Ptr)->*mut c_void;
        fn CFStringNormalize(s:*mut c_void,form:isize);
        fn CFStringGetLength(s:Ptr)->isize;
        fn CFStringGetMaximumSizeForEncoding(n:isize,e:u32)->isize;
        fn CFStringGetCString(s:Ptr,out:*mut u8,n:isize,e:u32)->bool;
        fn CFRelease(s:Ptr);
    }
    struct StringRef(Ptr);impl Drop for StringRef {fn drop(&mut self) {unsafe {CFRelease(self.0);}}}
    let original=unsafe {CFStringCreateWithBytes(std::ptr::null(),text.as_ptr(),text.len() as isize,0x08000100,false)};
    if original.is_null() {return Err("login-home-unverified".into());}let original=StringRef(original);
    let normalized=unsafe {CFStringCreateMutableCopy(std::ptr::null(),0,original.0)};
    if normalized.is_null() {return Err("login-home-unverified".into());}let normalized=StringRef(normalized);
    unsafe {CFStringNormalize(normalized.0.cast_mut(),2);}
    let size=unsafe {CFStringGetMaximumSizeForEncoding(CFStringGetLength(normalized.0),0x08000100)};
    if size<0 {return Err("login-home-unverified".into());}let mut buffer=vec![0;(size as usize)+1];
    if !unsafe {CFStringGetCString(normalized.0,buffer.as_mut_ptr(),buffer.len() as isize,0x08000100)} {return Err("login-home-unverified".into());}
    buffer.truncate(buffer.iter().position(|b|*b==0).ok_or("login-home-unverified")?);String::from_utf8(buffer).map_err(|_|"login-home-unverified".into())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test] fn windows_login_resolves_only_the_native_claude_package() {
        let root=std::env::temp_dir().join(format!("agentq-login-executable-{}",crate::incident::uuid()));
        let package=root.join("node_modules/@anthropic-ai/claude-code");std::fs::create_dir_all(package.join("bin")).unwrap();
        let shim=root.join("claude.cmd");std::fs::write(&shim,"@exit /b 99").unwrap();
        let native=package.join("bin/claude.exe");std::fs::copy(std::env::current_exe().unwrap(),&native).unwrap();
        std::fs::write(package.join("package.json"),r#"{"name":"@anthropic-ai/claude-code"}"#).unwrap();assert_eq!(executable(shim.clone()).unwrap(),native);
        std::fs::write(package.join("package.json"),r#"{"name":"foreign"}"#).unwrap();assert!(executable(shim).is_err());
        assert!(sign_in::direct_executable(root.join("foreign.cmd")).is_err());std::fs::remove_dir_all(root).unwrap();
    }
    #[test] fn pasted_code_url_refuses_callback_and_foreign_links() {
        assert!(valid_url("https://claude.ai/oauth/authorize?code=true&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode"));
        assert!(valid_url("https://claude.com/cai/oauth/authorize?code=true&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback"));
        for url in ["https://claude.ai.evil/oauth/authorize?code=true&redirect_uri=https%3A%2F%2Fconsole.anthropic.com", "https://claude.ai/oauth/authorize?code=true&redirect_uri=http%3A%2F%2Flocalhost%3A1234", "https://claude.ai/oauth/authorize?redirect_uri=https%3A%2F%2Fconsole.anthropic.com"] {assert!(!valid_url(url));}
    }
    #[cfg(target_os="macos")]
    #[test] fn keychain_service_uses_native_nfc_spelling() {assert_eq!(nfc("cafe\u{301}").unwrap(),"café");}
}
