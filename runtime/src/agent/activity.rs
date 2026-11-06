//! Observation only; a caller must explicitly authorize stopping a stalled worker.
use super::*;
use serde::{Serialize,Deserialize};
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub struct ActivityObservation {pub state:String,pub idle_ms:Option<u64>,pub reason:Option<String>}
impl Default for ActivityObservation {fn default()->Self {Self {state:"unknown".into(),idle_ms:None,reason:Some("no-native-activity".into())}}}
#[derive(Serialize,Deserialize)]
pub(super) struct Activity {
    pub cursor:super::events::Cursor,pub progress:u64,pub pending_seen:usize,
    last:Option<i64>,tools:Vec<(Option<String>,Option<i64>)>,unknown:bool,latched:bool,pub pending_stalls:Vec<u64>,
}
impl Activity {
    pub fn new(progress:u64)->Self {Self {cursor:super::events::Cursor::new(false),progress,pending_seen:0,last:None,tools:vec![],unknown:false,latched:false,pending_stalls:vec![]}}
    pub fn read(&mut self,client:Client,rows:Vec<Value>)->Vec<Event> {
        rows.into_iter().flat_map(|row|if row.is_object() {self.cursor.read(client,vec![row])} else {vec![Event::Unsupported {reason:"native record invalid"}]}).collect()
    }
    pub fn feed(&mut self,events:Vec<Event>,now:i64) {
        for event in events {
            match event {
                Event::Unsupported{..}=>{self.unknown=true;continue;},
                Event::Stalled{..}=>continue,
                Event::TurnStart|Event::TurnEnd{..}=>{self.tools.clear();self.unknown=false;},
                Event::ToolStart{id,timeout_ms,..}=>self.tools.push((id,timeout_ms.and_then(|ms|i64::try_from(ms).ok()).and_then(|ms|now.checked_add(ms)))),
                Event::ToolEnd{id,..}=>{if let Some(i)=self.tools.iter().position(|(tool,_)|*tool==id) {self.tools.remove(i);}},
                _=>{},
            }
            self.last=Some(now);self.latched=false;
        }
    }
    pub fn observation(&mut self,window:Option<Duration>,now:i64,running:bool)->ActivityObservation {
        let observation=|state:&str,idle_ms,reason:Option<&str>|ActivityObservation {state:state.into(),idle_ms,reason:reason.map(String::from)};
        let Some(window)=window else {return observation("disabled",None,Some("inactivity-not-requested"));};
        if self.unknown {return observation("unknown",None,Some("native-activity-unreadable"));}
        let Some(mut last)=self.last else {return observation("unknown",None,Some("no-native-activity"));};
        if last>now {return observation("unknown",None,Some("clock-regressed"));}
        if self.tools.iter().any(|(_,until)|until.is_none()) {return observation("unknown",None,Some("tool-timeout-unknown"));}
        if self.tools.iter().filter(|(id,_)|id.is_none()).count()>1 {return observation("unknown",None,Some("tool-identity-ambiguous"));}
        for (_,until) in &self.tools {last=last.max(until.unwrap());}
        let idle=now.saturating_sub(last).max(0) as u64;
        let stalled=running && now>=last && u128::from(idle)>=window.as_millis();
        if stalled && !self.latched {self.pending_stalls.push(idle);self.latched=true;}
        observation(if stalled {"stalled"} else {"active"},Some(idle),None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_unknowns_and_declared_tools_do_not_become_false_stalls() {
        let mut a=Activity::new(0);let window=Some(Duration::from_millis(10));
        assert_eq!(a.observation(window,100,true).state,"unknown");
        a.feed(vec![Event::Activity],100);assert_eq!(a.observation(window,111,true).state,"stalled");
        assert_eq!(a.observation(window,120,true).state,"stalled");assert_eq!(a.pending_stalls.len(),1);
        a.feed(vec![Event::ToolStart{id:Some("tool".into()),kind:super::super::ToolKind::Shell,name:None,detail:None,timeout_ms:Some(100)}],120);
        assert_eq!(a.observation(window,119,true).state,"unknown");
        assert_eq!(a.observation(window,200,true).state,"active");assert_eq!(a.observation(window,229,true).state,"active");assert_eq!(a.observation(window,230,true).state,"stalled");
        a.feed(vec![Event::ToolStart{id:Some("unbounded".into()),kind:super::super::ToolKind::Shell,name:None,detail:None,timeout_ms:None}],240);
        assert_eq!(a.observation(window,1000,true).state,"unknown");
        a.feed(vec![Event::Unsupported{reason:"native record invalid"},Event::Activity],1050);
        assert_eq!(a.observation(window,1060,true).state,"unknown");
        a.feed(vec![Event::Unsupported{reason:"native record invalid"},Event::TurnStart],1080);
        assert_eq!(a.observation(window,1090,true).state,"stalled","a later native boundary clears earlier unreadable records in the same batch");
        a.feed(vec![Event::TurnEnd{outcome:"ok"}],1100);assert_eq!(a.observation(window,2000,false).state,"active");
    }
}
