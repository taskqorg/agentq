//! Native Codex cancels an occupied registered callback; only an admitted, unoccupied resource may be submitted.
use super::*;
const RESOURCE:&str="tcp/ipv4/127.0.0.1:1455";
const FILE:&str="codex-oauth-127.0.0.1-1455";
pub(super) struct CallbackAdmission {directory:sign_in::Home,path:PathBuf,root:PathBuf,token:String}
impl CallbackAdmission {
    pub fn acquire(root:&Path)->Result<Arc<Self>,&'static str> {
        let directory=sign_in::Home::with_lock(root,&format!("{FILE}.lock")).map_err(|_|"login-callback-authority-unavailable")?;
        directory.check(root).map_err(|_|"login-callback-authority-changed")?;
        let path=root.join(format!("{FILE}.json"));
        if let Some(row)=read(&path)? {
            if row["schema"]!="agentq-login-callback/1" || row["resource"]!=RESOURCE || !row["token"].as_str().is_some_and(plain_id) || row["phase"]!="settled" {return Err("login-callback-owner-unknown");}
        }
        probe()?;
        directory.check(root).map_err(|_|"login-callback-authority-changed")?;
        let token=crate::incident::uuid();
        save(&path,&json!({"schema":"agentq-login-callback/1","resource":RESOURCE,"token":token,"phase":"starting"}))?;
        Ok(Arc::new(Self {directory,path,root:root.into(),token}))
    }
    pub fn check_url(&self,url:&str)->Res<()> {
        let redirect=url.split_once('?').and_then(|(_,query)|query.split('&').find_map(|p|p.strip_prefix("redirect_uri="))).ok_or("login-callback-resource-unverified")?;
        if !["http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback","http://127.0.0.1:1455/auth/callback","http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback","http://localhost:1455/auth/callback"].iter().any(|expected|redirect.eq_ignore_ascii_case(expected)) {return Err("login-callback-resource-unverified".into());}Ok(())
    }
    pub fn check(&self)->Res<()> {self.owned().map_err(String::from)?;probe().map_err(String::from)}
    fn owned(&self)->Result<Value,&'static str> {
        self.directory.check(&self.root).map_err(|_|"login-callback-authority-changed")?;
        let row=read(&self.path)?.ok_or("login-callback-owner-unknown")?;
        if row["schema"]!="agentq-login-callback/1" || row["resource"]!=RESOURCE || row["token"]!=self.token || row["phase"]!="starting" {return Err("login-callback-owner-unknown");}Ok(row)
    }
    pub fn finish(&self)->Res<()> {
        let mut row=self.owned().map_err(String::from)?;row["phase"]=json!("settled");save(&self.path,&row).map_err(String::from)
    }
}
fn probe()->Result<(),&'static str> {
    // Binding reads occupancy without contacting or cancelling the occupant. External claimants can still race the later native bind.
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,1455)).map(drop).map_err(|e|if e.kind()==std::io::ErrorKind::AddrInUse {"login-browser-callback-busy"}else{"login-browser-callback-unverified"})
}
fn read(path:&Path)->Result<Option<Value>,&'static str> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink()=>return Err("login-callback-owner-unknown"),
        Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(_)=>return Err("login-callback-owner-unknown"),_=>{},
    }
    serde_json::from_slice(&std::fs::read(path).map_err(|_|"login-callback-owner-unknown")?).map(Some).map_err(|_|"login-callback-owner-unknown")
}
fn save(path:&Path,row:&Value)->Result<(),&'static str> {
    coreq::write_durable(path,serde_json::to_vec(row).map_err(|_|"login-callback-owner-unknown")?).map_err(|_|"login-callback-write-unknown")
}
