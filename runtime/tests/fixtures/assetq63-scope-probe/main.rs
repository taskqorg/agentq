use agentq_runtime::agent::{self,Launch,Mode,Part,State,Event};
use serde_json::json;
use std::{path::PathBuf,time::{Instant,Duration}};
fn main(){
 let base=PathBuf::from(std::env::args().nth(1).unwrap());std::fs::create_dir(&base).unwrap();
 let rt=agent::runtime("codex").unwrap();let mut failures=vec![];
 for mode in ["positive","foreign","unscoped","nested","alias","header","failed","unknown","conflict"] {
  let root=base.join(mode);std::fs::create_dir(&root).unwrap();
  let mut l=Launch::new(&root,root.join("run.log"),"invented","independent");l.home=Some(root.join("invented-home"));l.command=vec!["/usr/bin/python3".into(),"/tmp/assetq63-independent-21b8c271/probe/client.py".into(),mode.into(),root.clone().into()];
  l.mode=Mode::Turn;l.deltas=true;l.deadline=Some(Instant::now()+Duration::from_secs(10));
  std::fs::create_dir(l.home.as_ref().unwrap()).unwrap();let mut run=rt.launch(&l).unwrap();let binding=json!("pre-input binding covered by separate protected public fixture");
  
  let until=Instant::now()+Duration::from_secs(5);let mut events=vec![];
  loop {let state=rt.poll(&mut run).unwrap();events.extend(rt.progress(&mut run).unwrap());if state!=State::Running{break;}assert!(Instant::now()<until);std::thread::yield_now();}
  let report=rt.report(&run).unwrap();let images:Vec<_>=events.iter().filter_map(|e|if let Event::Image(i)=e{Some(i.clone())}else{None}).collect();assert_eq!(images,report.images);assert_eq!(report.turn.as_deref(),Some("turn"));
  let saved=rt.checkpoint(&run).unwrap();let cold=rt.recover(&saved,&l).unwrap();assert_eq!(rt.report(&cold).unwrap().images,images);
  std::fs::write(root.join("readback.json"),json!({"binding":binding,"report":agentq_runtime::cli::report_json(&report),"events":events}).to_string()).unwrap();
  let expected=if ["foreign","unscoped","nested","alias","header"].contains(&mode){0}else if mode=="conflict"{2}else{1};
  println!("{mode}: images={} expected={expected}",images.len());if images.len()!=expected{failures.push(mode);}
  if mode=="failed"{assert_eq!(images[0].outcome.as_deref(),Some("failed"));}if mode=="unknown"{assert!(images[0].outcome.is_none());}
  let _=rt.cancel(&mut run);drop(cold);drop(run);
 }
 assert!(failures.is_empty(),"foreign/conflicting scope was owned: {failures:?}");
}
