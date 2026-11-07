//! Native account protocols only; no thread or model submission.
use super::*;
use super::super::super::sign_in::{self,Context,Conversation,Action,LoginFlow,LoginPrompt};
use std::sync::Arc;
#[path="signin/callback.rs"]
mod callback;
use callback::CallbackAdmission;
pub(super) static PROTOCOL:sign_in::Protocol=sign_in::Protocol {executable:sign_in::direct_executable,configure,credentials,verify:|c|login(c.command(),true)};
fn configure(context: &Context,flow:LoginFlow,command:&mut Command)->Result<Box<dyn Conversation>,&'static str> {
    match flow {
        LoginFlow::Browser=>{
            let permit=CallbackAdmission::acquire(context.callback_authority.as_deref().ok_or("login-callback-authority-required")?)?;
            command.arg("app-server");Ok(Box::new(Browser {buffer:vec![],login:None,success:false,started:false,permit:Some(permit)}))
        },
        LoginFlow::Device=>{command.args(["login","--device-auth"]);Ok(Box::new(Device {text:sign_in::Text::default(),prompt:false}))},
    }
}
fn credentials(c:&Context)->Res<Vec<Option<String>>> {
    // Read the client's effective store, including managed configuration, rather than assuming auth.json is authoritative.
    let spec=Spec {client:Client::Codex,root:c.home.clone(),session:"-".into(),resume:false,args:vec![],env:vec![],home:None};
    let store=codex_read_command(&spec,c.command(),|server,session,until| {
        let request=session.request("config/read",json!({"includeLayers":false}))?;
        let reply=exchange(server,session,request,until)?;
        let config=reply["result"]["config"].as_object().ok_or("login-credential-store-unverified")?;
        match config.get("cli_auth_credentials_store") {None|Some(Value::Null)=>Ok(None),Some(Value::String(mode))=>Ok(Some(mode.clone())),_=>Err("login-credential-store-unverified".into())}
    })?;
    if store.as_deref().is_some_and(|s|s!="file") {return Err("login-credential-store-unverified".into());}
    Ok(vec![sign_in::file_fingerprint(&c.home.join("auth.json"))?])
}
fn frame(value:Value)->Vec<u8> {format!("{value}\n").into_bytes()}
struct Browser {buffer:Vec<u8>,login:Option<String>,success:bool,started:bool,permit:Option<Arc<CallbackAdmission>>}
impl Conversation for Browser {
    fn admission(&self)->Option<Arc<dyn Send+Sync>> {self.permit.as_ref().map(|p|p.clone() as Arc<dyn Send+Sync>)}
    fn initial(&mut self)->Vec<Vec<u8>> {vec![frame(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"agentq-runtime","version":env!("CARGO_PKG_VERSION")}}}))]}
    fn output(&mut self,stream:usize,bytes:&[u8])->Res<Vec<Action>> {
        if stream!=0 {return Ok(vec![]);}
        self.buffer.extend_from_slice(bytes);let mut actions=vec![];
        while let Some(end)=self.buffer.iter().position(|b|*b==b'\n') {
            let line:Vec<_>=self.buffer.drain(..=end).collect();if line.iter().all(u8::is_ascii_whitespace) {continue;}
            let value:Value=serde_json::from_slice(&line).map_err(|_|"login-native-protocol-invalid")?;
            if value.get("error").is_some() {actions.push(Action::Failure(native_failure(value["error"]["message"].as_str().unwrap_or(""))));continue;}
            if value["id"]==1 && !self.started {
                if let Some(permit)=&self.permit {permit.check()?;}
                self.started=true;actions.push(Action::Write(frame(json!({"method":"initialized"}))));
                actions.push(Action::Write(frame(json!({"id":2,"method":"account/login/start","params":{"type":"chatgpt"}}))));
            } else if value["id"]==2 && self.started && self.login.is_none() {
                let result=&value["result"];let url=result["authUrl"].as_str().filter(|u|valid_url(u,false)).ok_or("login-url-unverified")?;
                if let Some(permit)=&self.permit {permit.check_url(url)?;}
                self.login=Some(result["loginId"].as_str().filter(|s|!s.is_empty()).ok_or("login-id-unverified")?.into());
                actions.push(Action::Prompt(LoginPrompt {url:url.into(),user_code:None,needs_code:false}));
            } else if value["method"]=="account/login/completed" && self.login.as_deref().is_some_and(|id|value["params"]["loginId"]==id) {
                self.success=value["params"]["success"]==true;
                actions.push(if self.success {Action::Complete}else{Action::Failure(native_failure(value["params"]["error"].as_str().unwrap_or("")))});
            } else if let Some(id)=value.get("id").filter(|_|value.get("method").is_some()) {
                actions.push(Action::Write(frame(json!({"id":id,"error":{"code":-32601,"message":"account sign-in only"}}))));
            }
        }Ok(actions)
    }
    fn completed(&self,exit:Option<i32>)->bool {self.success && exit==Some(0)}
    fn cleanup(&mut self)->Res<()> {if let Some(permit)=&self.permit {permit.finish()?;}Ok(())}
}
struct Device {text:sign_in::Text,prompt:bool}
impl Conversation for Device {
    fn output(&mut self,stream:usize,bytes:&[u8])->Res<Vec<Action>> {
        let text=self.text.append(stream,bytes);let lower=text.to_ascii_lowercase();
        if lower.contains("access_denied") || lower.contains("expired") || lower.contains("device authorization timed out") {return Ok(vec![Action::Failure(native_failure(&lower))]);}
        if self.prompt {return Ok(vec![]);}
        if let Some((url,code))=device_prompt(&text) {self.prompt=true;return Ok(vec![Action::Prompt(LoginPrompt {url,user_code:Some(code),needs_code:false})]);}Ok(vec![])
    }
    fn completed(&self,exit:Option<i32>)->bool {exit==Some(0)}
}
fn native_failure(text:&str)->&'static str {let text=text.to_ascii_lowercase();if text.contains("expired") || text.contains("timed out") {"login-code-expired"}else if text.contains("denied") {"login-denied"}else{"login-native-failed"}}
fn valid_url(url:&str,device:bool)->bool {
    url.strip_prefix("https://auth.openai.com/").is_some_and(|rest|if device {rest=="codex/device"}else{rest.starts_with("oauth/authorize?")}) && !url.chars().any(char::is_whitespace)
}
fn device_prompt(text:&str)->Option<(String,String)> {
    let (head,tail)=text.split_once("Enter this one-time code")?;
    let url=sign_in::tokens(head).find(|token|valid_url(token,true))?;
    // Native prints the code on its own line after the expiry explanation.
    let code=tail.lines().skip(1).map(str::trim).find(|s|!s.is_empty())?;
    if !tail.ends_with(char::is_whitespace) || !code.contains('-') || !code.bytes().all(|b|b.is_ascii_uppercase() || b.is_ascii_digit() || b==b'-') {return None;}
    Some((url.into(),code.into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn native_browser_login_correlates_completion_without_input() {
        let mut b=Browser {buffer:vec![],login:None,success:false,started:false,permit:None};
        assert_eq!(b.output(0,b"{\"id\":1,\"result\":{}}\n").unwrap().len(),2);
        let reply=frame(json!({"id":2,"result":{"loginId":"owned","authUrl":"https://auth.openai.com/oauth/authorize?state=private"}}));
        assert!(b.output(0,&reply[..20]).unwrap().is_empty());assert!(matches!(b.output(0,&reply[20..]).unwrap()[0],Action::Prompt(_)));
        assert!(b.output(0,&frame(json!({"method":"account/login/completed","params":{"loginId":"foreign","success":true}}))).unwrap().is_empty());
        assert!(!b.completed(Some(0)));assert!(matches!(b.output(0,&frame(json!({"method":"account/login/completed","params":{"loginId":"owned","success":true}}))).unwrap()[0],Action::Complete));assert!(b.completed(Some(0)));
    }
    #[test] fn device_prompt_requires_complete_native_link_and_code() {
        let text="Open https://auth.openai.com/codex/device\n2. Enter this one-time code (expires in 15 minutes)\n AB12-CD34";
        assert!(device_prompt(text).is_none());assert_eq!(device_prompt(&format!("{text}\n")).unwrap().1,"AB12-CD34");
        assert!(device_prompt(&format!("{}\n",text.replace("auth.openai.com/","auth.openai.com.evil/"))).is_none());
    }
}
