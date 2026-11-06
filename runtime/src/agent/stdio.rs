//! Shared owned JSONL process transport; protocol state belongs to adapters.
use super::*;
#[cfg(windows)] mod windows_job;
use std::sync::mpsc::{self, Receiver, SyncSender, RecvTimeoutError};

enum Pipe<T> {Bounded(SyncSender<T>),Stream(mpsc::Sender<T>)}
impl<T> Pipe<T> {
    fn send(&self,value:T)->Result<(),()> {match self {Self::Bounded(s)=>s.send(value).map_err(|_|()),Self::Stream(s)=>s.send(value).map_err(|_|())}}
    fn enqueue(&self,value:T)->Result<(),()> {match self {Self::Bounded(s)=>s.try_send(value).map_err(|_|()),Self::Stream(s)=>s.send(value).map_err(|_|())}}
}
fn channel<T>(bound:Option<usize>)->(Pipe<T>,Receiver<T>) {match bound {
    Some(n)=>{let(s,r)=mpsc::sync_channel(n);(Pipe::Bounded(s),r)},
    None=>{let(s,r)=mpsc::channel();(Pipe::Stream(s),r)},
}}

type WriteRequest=(Vec<u8>,mpsc::Sender<Res<()>>);

/// Shared native stdio transport. Dropping the handle does not kill an active session; callers own its lifecycle.
pub(crate) struct Stdio {
    child:Child,
    input:Option<Pipe<WriteRequest>>,
    bounded:bool,
    stopped:bool,
    #[cfg(windows)] job:Option<std::sync::Arc<windows_job::Job>>,
    events:Receiver<Res<Value>>,
}

impl Stdio {
    pub(crate) fn spawn(command:Command,private_log:&Path,process:&super::Process)->Res<Self> {Self::spawn_inner(command,private_log,process,true)}
    pub(crate) fn spawn_stream(command:Command,private_log:&Path,process:&super::Process)->Res<Self> {Self::spawn_inner(command,private_log,process,false)}
    fn spawn_inner(mut command:Command,private_log:&Path,process:&super::Process,bounded:bool)->Res<Self> {
        let mut options=File::options();options.create(true).append(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600);}
        let log=options.open(private_log).map_err(|e|format!("app-server log: {e}"))?;
        let mut diagnostics=log.try_clone().map_err(|e|format!("app-server log: {e}"))?;
        crate::telemetry::capture::inherit_context(&mut command);
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(log);
        #[cfg(windows)] let job=if !bounded {Some(std::sync::Arc::new(windows_job::Job::new()?))}else{None};
        #[cfg(windows)] let owned={
            let mut owned=process.clone();
            if let Some(job)=job.clone() {
                let caller=owned.spawned.clone();
                owned.spawned=Some(std::sync::Arc::new(move |child|{
                    let result=caller.as_ref().map_or(Ok(()),|callback|callback(child));
                    job.attach(child)?;
                    result
                }));
            }owned
        };
        #[cfg(windows)] let process=&owned;
        let mut child=process.spawn(command,"app-server")?;
        let mut stdin=child.stdin.take().ok_or("app-server stdin unavailable")?;
        let stdout=child.stdout.take().ok_or("app-server stdout unavailable")?;
        let (input,writing)=channel::<WriteRequest>(bounded.then_some(1));
        std::thread::spawn(move || {
            while let Ok((bytes,receipt))=writing.recv() {
                let result=stdin.write_all(&bytes).and_then(|_|stdin.flush()).map_err(|e|format!("app-server write outcome unknown: {e}"));
                let failed=result.is_err();let _=receipt.send(result);
                if failed {break;}
            }
        });
        let (output,events)=channel(bounded.then_some(16));
        std::thread::spawn(move || {
            let mut reader=BufReader::new(stdout);
            loop {
                // Bytes first: the bound can fall inside a valid character, which must read as size and not as invalid UTF-8.
                let mut line=Vec::new();
                let result=if bounded {(&mut reader).take(8*1024*1024+1).read_until(b'\n',&mut line)} else {reader.read_until(b'\n',&mut line)};
                let event=match result {
                    Ok(0)=>break,
                    Ok(_) if bounded && line.len()>8*1024*1024=>Err("app-server record exceeds 8 MiB; native state unknown".into()),
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
                if failed {
                    if let Err(e)=diagnostics.write_all(&line).and_then(|_|diagnostics.flush()) {let _=output.send(Err(format!("app-server raw evidence log: {e}")));break;}
                }
                if output.send(event).is_err() || failed {break;}
            }
        });
        Ok(Self {child,input:Some(input),events,bounded,stopped:false,#[cfg(windows)] job})
    }

    pub fn id(&self)->u32 {self.child.id()}

    /// A timeout may follow partial native delivery. Retain the original request and reconcile; never resend it.
    pub fn send(&mut self,message:&Value,until:Instant)->Res<()> {
        self.send_until(message,Some(until))
    }

    pub(crate) fn send_until(&mut self,message:&Value,until:Option<Instant>)->Res<()> {
        let mut bytes=serde_json::to_vec(message).map_err(|e|e.to_string())?;
        if self.bounded && bytes.len()>=8*1024*1024 {return Err("app-server input exceeds 8 MiB; not submitted".into());}
        bytes.push(b'\n');
        let left=until.map(|d|d.checked_duration_since(Instant::now()).ok_or("app-server send deadline; not submitted")).transpose()?;
        let (receipt,done)=mpsc::channel();
        self.input.as_ref().ok_or("app-server writer closed")?.enqueue((bytes,receipt)).map_err(|_|"app-server writer unavailable; not submitted")?;
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

    /// A real child wait proves direct termination; its native numeric code remains independently optional.
    pub(crate) fn observe_exit(&mut self,report:&mut Report)->Res<()> {
        let status=self.child.try_wait().map_err(|e|format!("app-server wait: {e}"))?;
        report.exit=status.filter(|_|!self.stopped).and_then(|s|s.code());
        report.termination=if status.is_some() {ProcessTermination::Observed} else {ProcessTermination::Unknown {reason:"session-process-running"}};
        if status.is_none() {report.process.exit_unknown="session-process-running";}
        else if status.is_some_and(|s|s.code().is_none()) {report.process.exit_unknown="signalled";}
        else if self.stopped {report.process.exit_unknown="post-terminal-cleanup";}
        Ok(())
    }

    pub fn poll(&mut self)->Res<Option<i32>> {self.child.try_wait().map(|s|s.map(|s|s.code().unwrap_or(-1))).map_err(|e|format!("app-server wait: {e}"))}

    /// Stop the owned server (and its Windows job); caller authority outside that scope stays unchanged.
    pub fn kill(&mut self)->Res<()> {
        let running=self.poll()?.is_none();
        #[cfg(windows)] if let Some(job)=&self.job {job.kill()?;self.stopped|=running;}
        if !running || self.poll()?.is_some() {return Ok(());}
        self.child.kill().map_err(|e|format!("app-server stop unconfirmed: {e}"))?;self.stopped=true;
        self.child.wait().map_err(|e|format!("app-server reap unconfirmed: {e}"))?;
        Ok(())
    }

    /// EOF requests shutdown; observation must never wait for native descendants.
    pub(crate) fn close_stream(&mut self) {self.input.take();}

    /// EOF lets the native server close its MCP children; the owned server is reaped before return.
    pub(crate) fn close(&mut self)->Res<()> {
        self.input.take();
        let until=Instant::now()+Duration::from_secs(2);
        while self.poll()?.is_none() && Instant::now()<until {std::thread::sleep(Duration::from_millis(10));}
        self.kill()
    }
}
