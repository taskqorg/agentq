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

pub(crate) struct NativeFrame {
    pub(crate) value:Value,
    pub(crate) raw:Option<Vec<u8>>,
    pub(crate) journal:Option<crate::archive::JournalRecord>,
}
impl NativeFrame {pub(crate) fn parsed(value:Value)->Self {Self {value,raw:None,journal:None}}}

type WriteRequest=(Vec<u8>,mpsc::Sender<Res<()>>);

/// Shared native stdio transport. Dropping the handle does not kill an active session; callers own its lifecycle.
pub(crate) struct Stdio {
    child:Option<Child>,
    reap:mpsc::Sender<Child>,
    input:Option<Pipe<WriteRequest>>,
    bounded:bool,
    stopped:bool,
    #[cfg(windows)] job:Option<std::sync::Arc<windows_job::Job>>,
    events:Receiver<Res<NativeFrame>>,
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
        let (reap,reaping)=mpsc::channel::<Child>();
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
                if output.send(event.map(|value|NativeFrame {value,raw:Some(line),journal:None})).is_err() || failed {break;}
            }
            drop(output);
            drop(reader);
            // Keep the exact owned handle until wait reaps it; never signal a protected scope.
            if let Ok(mut child)=reaping.recv() {
                if let Err(error)=child.wait() {eprintln!("app-server frontend reap unknown: {error}");}
            }
        });
        Ok(Self {child:Some(child),reap,input:Some(input),events,bounded,stopped:false,#[cfg(windows)] job})
    }

    pub fn id(&self)->u32 {self.child.as_ref().expect("owned frontend").id()}

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
    pub fn receive(&mut self,until:Instant)->Res<Option<Value>> {Ok(self.receive_frame(until)?.map(|frame|frame.value))}

    pub(crate) fn receive_frame(&mut self,until:Instant)->Res<Option<NativeFrame>> {
        match self.events.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(event)=>event.map(Some),Err(RecvTimeoutError::Timeout)=>Ok(None),
            Err(RecvTimeoutError::Disconnected)=>Err("app-server stream ended; native turn outcome unknown".into()),
        }
    }

    pub(crate) fn wait(&mut self,until:Option<Instant>)->Res<Value> {Ok(self.wait_frame(until)?.value)}

    pub(crate) fn wait_frame(&mut self,until:Option<Instant>)->Res<NativeFrame> {
        match until {
            Some(d)=>self.receive_frame(d)?.ok_or_else(||"native reply deadline; outcome unknown".into()),
            None=>self.events.recv().map_err(|_|"native stream ended; outcome unknown")?,
        }
    }

    /// A real child wait proves direct termination; its native numeric code remains independently optional.
    pub(crate) fn observe_exit(&mut self,report:&mut Report)->Res<()> {
        let status=self.child.as_mut().expect("owned frontend").try_wait().map_err(|e|format!("app-server wait: {e}"))?;
        report.exit=status.filter(|_|!self.stopped).and_then(|s|s.code());
        report.termination=if status.is_some() {ProcessTermination::Observed} else {ProcessTermination::Unknown {reason:"session-process-running"}};
        if status.is_none() {report.process.exit_unknown="session-process-running";}
        else if status.is_some_and(|s|s.code().is_none()) {report.process.exit_unknown="signalled";}
        else if self.stopped {report.process.exit_unknown="post-terminal-cleanup";}
        Ok(())
    }

    pub fn poll(&mut self)->Res<Option<i32>> {self.child.as_mut().expect("owned frontend").try_wait().map(|s|s.map(|s|s.code().unwrap_or(-1))).map_err(|e|format!("app-server wait: {e}"))}

    /// Stop the owned server (and its Windows job); caller authority outside that scope stays unchanged.
    pub fn kill(&mut self)->Res<()> {
        let running=self.poll()?.is_none();
        #[cfg(windows)] if let Some(job)=&self.job {job.kill()?;self.stopped|=running;}
        if !running || self.poll()?.is_some() {return Ok(());}
        self.child.as_mut().expect("owned frontend").kill().map_err(|e|format!("app-server stop unconfirmed: {e}"))?;self.stopped=true;
        self.child.as_mut().expect("owned frontend").wait().map_err(|e|format!("app-server reap unconfirmed: {e}"))?;
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

impl Drop for Stdio {
    fn drop(&mut self) {
        self.input.take();
        if let Some(child)=self.child.take() {
            // The reader cannot exit before receiving this handle, even after a stream error.
            if self.reap.send(child).is_err() {eprintln!("app-server frontend reap unknown: reader unavailable");}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Protected frontends must be reaped through their original Child after graceful EOF.
    #[test]
    fn stdio_drop_reaps_original_frontend_after_eof() {
        if let Some(marker)=std::env::var_os("AGENTQ_STDIO_REAP_FIXTURE") {
            std::io::stdout().write_all(b"\n{}\n").unwrap();
            std::io::stdout().flush().unwrap();
            let mut input=vec![];
            std::io::stdin().read_to_end(&mut input).unwrap();
            std::fs::write(marker,b"original input EOF").unwrap();
            std::process::exit(0);
        }
        let root=crate::scratch("stdio-frontend-reap");std::fs::create_dir_all(&root).unwrap();
        let marker=root.join("eof");
        let mut command=Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","agent::stdio::tests::stdio_drop_reaps_original_frontend_after_eof","--nocapture"])
            .env("AGENTQ_STDIO_REAP_FIXTURE",&marker);
        let mut server=Stdio::spawn(command,&root.join("private.log"),&super::super::Process::default()).unwrap();
        let original=coreq::process::identity(server.id()).unwrap().unwrap();
        server.receive_frame(Instant::now()+Duration::from_secs(10)).unwrap().unwrap();
        drop(server);
        let until=Instant::now()+Duration::from_secs(10);
        loop {
            match coreq::process::identity(original.pid) {
                Ok(None)=>break,
                Ok(Some(current))=>assert_eq!(current,original,"original PID reused; no reap proof"),
                Err(_)=>{}, // Unreadable creation identity stays pending, never absent.
            }
            assert!(Instant::now()<until,"original frontend was not reaped after drop");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read(&marker).unwrap(),b"original input EOF");
        std::fs::remove_dir_all(root).unwrap();
    }

    // S283: the pipe owns original bytes; diagnostics and incomplete/invalid frames never become records.
    #[test]
    fn native_frames_keep_physical_bytes_and_private_errors() {
        const RAW:&[u8]=b" { \"z\": \"\\u4e2d\", \"a\":1 }\r\n";
        if let Ok(mode)=std::env::var("AGENTQ_FRAME_READER_FIXTURE") {
            eprintln!("private stderr");
            let mut out=std::io::stdout().lock();out.write_all(b"private startup diagnostic\n").unwrap();
            out.write_all(match mode.as_str() {"valid"=>RAW,"json"=>b"{broken}\n","shape"=>b"[]\n","utf8"=>b"{\xff}\n",_=>b"{\"partial\":1}"}).unwrap();out.flush().unwrap();std::process::exit(0);
        }
        let root=crate::scratch("native-frame-reader");std::fs::create_dir_all(&root).unwrap();
        for mode in ["valid","json","shape","utf8","partial"] {
            let mut command=Command::new(std::env::current_exe().unwrap());
            command.args(["--exact","agent::stdio::tests::native_frames_keep_physical_bytes_and_private_errors","--nocapture"]).env("AGENTQ_FRAME_READER_FIXTURE",mode);
            let log=root.join(mode);let mut server=Stdio::spawn(command,&log,&super::super::Process::default()).unwrap();
            let received=server.receive_frame(Instant::now()+Duration::from_secs(30));
            if mode=="valid" {let frame=received.unwrap().unwrap();assert_eq!(frame.raw.unwrap(),RAW);assert_eq!(frame.value["a"],1);}else{assert!(received.is_err(),"{mode}");}
            server.kill().unwrap();let private=std::fs::read(&log).unwrap();assert!(private.windows(b"private startup diagnostic".len()).any(|w|w==b"private startup diagnostic"));
        }
        let _=std::fs::remove_dir_all(root);
    }
}
