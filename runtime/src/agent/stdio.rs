//! Shared owned JSONL process transport; protocol state belongs to adapters.
use super::*;
use std::sync::mpsc::{self, Receiver, SyncSender, RecvTimeoutError};

type WriteRequest=(Vec<u8>,mpsc::Sender<Res<()>>);

/// Shared native stdio transport. Dropping the handle does not kill an active session; callers own its lifecycle.
pub(crate) struct Stdio {
    child:Child,
    input:Option<SyncSender<WriteRequest>>,
    events:Receiver<Res<Value>>,
}

impl Stdio {
    pub(crate) fn spawn(mut command:Command,private_log:&Path,process:&super::Process)->Res<Self> {
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(private_log).map_err(|e|format!("app-server log: {e}"))?;
        let mut diagnostics=log.try_clone().map_err(|e|format!("app-server log: {e}"))?;
        crate::telemetry::capture::inherit_context(&mut command);
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(log);
        let mut child=process.spawn(command,"app-server")?;
        let mut stdin=child.stdin.take().ok_or("app-server stdin unavailable")?;
        let stdout=child.stdout.take().ok_or("app-server stdout unavailable")?;
        let (input,writing)=mpsc::sync_channel::<WriteRequest>(1);
        std::thread::spawn(move || {
            while let Ok((bytes,receipt))=writing.recv() {
                let result=stdin.write_all(&bytes).and_then(|_|stdin.flush()).map_err(|e|format!("app-server write outcome unknown: {e}"));
                let failed=result.is_err();let _=receipt.send(result);
                if failed {break;}
            }
        });
        let (output,events)=mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut reader=BufReader::new(stdout);
            loop {
                // Bytes first: the bound can fall inside a valid character, which must read as size and not as invalid UTF-8.
                let mut line=Vec::new();
                let result=(&mut reader).take(8*1024*1024+1).read_until(b'\n',&mut line);
                let event=match result {
                    Ok(0)=>break,
                    Ok(_) if line.len()>8*1024*1024=>Err("app-server record exceeds 8 MiB; native state unknown".into()),
                    Ok(n) if line.last()!=Some(&b'\n')=>Err(format!("app-server stream ended inside a record after {n} bytes; native state unknown")),
                    Ok(_)=>match std::str::from_utf8(&line) {
                        Err(_)=>Err("app-server record invalid; native state unknown".into()),
                        Ok(text) if !text.trim_start().starts_with(['{','['])=>{
                            if let Err(e)=diagnostics.write_all(&line) {let _=output.send(Err(format!("app-server diagnostic log: {e}")));break;}
                            continue;
                        },
                        Ok(text)=>serde_json::from_str::<Value>(text).map_err(|_|"app-server record invalid; native state unknown".into())
                            .and_then(|v|if v.is_object() {Ok(v)} else {Err("app-server record shape invalid; native state unknown".into())}),
                    },
                    Err(e)=>Err(format!("app-server read failed: {e}")),
                };
                let failed=event.is_err();
                if output.send(event).is_err() || failed {break;}
            }
        });
        Ok(Self {child,input:Some(input),events})
    }

    pub fn id(&self)->u32 {self.child.id()}

    /// A timeout may follow partial native delivery. Retain the original request and reconcile; never resend it.
    pub fn send(&mut self,message:&Value,until:Instant)->Res<()> {
        self.send_until(message,Some(until))
    }

    pub(crate) fn send_until(&mut self,message:&Value,until:Option<Instant>)->Res<()> {
        let mut bytes=serde_json::to_vec(message).map_err(|e|e.to_string())?;
        if bytes.len()>=8*1024*1024 {return Err("app-server input exceeds 8 MiB; not submitted".into());}
        bytes.push(b'\n');
        let left=until.map(|d|d.checked_duration_since(Instant::now()).ok_or("app-server send deadline; not submitted")).transpose()?;
        let (receipt,done)=mpsc::channel();
        self.input.as_ref().ok_or("app-server writer closed")?.try_send((bytes,receipt)).map_err(|_|"app-server writer unavailable; not submitted")?;
        match left {
            Some(left)=>done.recv_timeout(left).map_err(|_|"app-server write acknowledgment unavailable; submission unknown")?,
            None=>done.recv().map_err(|_|"app-server write acknowledgment unavailable; submission unknown")?,
        }
    }

    /// Timeout means no event yet. EOF/read error is unknown and never a completed turn.
    pub fn receive(&mut self,until:Instant)->Res<Option<Value>> {
        match self.events.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(event)=>event.map(Some),Err(RecvTimeoutError::Timeout)=>Ok(None),
            Err(RecvTimeoutError::Disconnected)=>Err("app-server stream ended; native turn outcome unknown".into()),
        }
    }

    pub(crate) fn wait(&mut self,until:Option<Instant>)->Res<Value> {
        match until {
            Some(d)=>self.receive(d)?.ok_or_else(||"native reply deadline; outcome unknown".into()),
            None=>self.events.recv().map_err(|_|"native stream ended; outcome unknown")?,
        }
    }

    pub fn poll(&mut self)->Res<Option<i32>> {self.child.try_wait().map(|s|s.map(|s|s.code().unwrap_or(-1))).map_err(|e|format!("app-server wait: {e}"))}

    /// Stop only this directly owned child. Consumer process-group/native-turn authority remains with the caller.
    pub fn kill(&mut self)->Res<()> {
        if self.poll()?.is_some() {return Ok(());}
        self.child.kill().map_err(|e|format!("app-server stop unconfirmed: {e}"))?;
        self.child.wait().map_err(|e|format!("app-server reap unconfirmed: {e}"))?;
        Ok(())
    }

    /// EOF lets the native server close its MCP children; the owned server is reaped before return.
    pub(crate) fn close(&mut self)->Res<()> {
        self.input.take();
        let until=Instant::now()+Duration::from_secs(2);
        while self.poll()?.is_none() && Instant::now()<until {std::thread::sleep(Duration::from_millis(10));}
        self.kill()
    }
}
