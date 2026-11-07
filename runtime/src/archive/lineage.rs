//! Private capture witness. None of these physical identities or native hashes goes on the wire.
use super::*;
use crate::agent::history::{Step, lineage::{Point, Unmapped}};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Physical {
    pub start:u64, pub end:u64, pub native_sha256:String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Frame {
    pub source:String, pub identity:String, pub path:PathBuf, pub base:u64, pub format:String,
    pub records:Vec<Physical>,
    #[serde(default)]
    pub history:Option<Value>,
}
impl Frame {
    pub fn capture(source:&str,identity:&str,path:&Path,base:u64,format:&str,raw:&[u8])->Res<Self> {
        let mut records=Vec::new();let mut offset=base;
        for row in raw.split_inclusive(|b|*b==b'\n') {
            if !row.ends_with(b"\n"){break;}
            let end=offset.checked_add(row.len() as u64).ok_or("archive frame overflow")?;
            records.push(Physical {start:offset,end,native_sha256:crate::sha256_hex(row)});offset=end;
        }
        Ok(Self {source:source.into(),identity:identity.into(),path:path.into(),base,format:format.into(),records,history:None})
    }
    /// This proves bytes only. Projection needs all contributions and deterministic prefix decoding.
    pub fn anchor(&self,step:&Step,identity:&str,accepted:&[u8])->Option<Point> {
        if self.identity!=identity || step.source["source"].as_str()!=self.path.to_str() || step.source["format"]!=self.format {return None;}
        let start=step.source["start"].as_u64()?;let end=step.source["end"].as_u64()?;
        self.records.iter().find(|r|r.start==start && r.end==end && step.source["sha256"]==r.native_sha256)?;
        let start=start.checked_sub(self.base)?;let end=end.checked_sub(self.base)?;
        let a=usize::try_from(start).ok()?;let b=usize::try_from(end).ok()?;
        let row=accepted.get(a..b)?;
        if row.is_empty() || !row.ends_with(b"\n") || row[..row.len()-1].contains(&b'\n') || a>0 && accepted.get(a-1)!=Some(&b'\n') {return None;}
        Some(Point::Record {source:self.source.clone(),start,end,sha256:crate::sha256_hex(row),projection:None})
    }
}

pub(super) fn unsupported()->Point {Point::Unmapped {reason:Unmapped::SourceDomainUnsupported}}

impl Frame {
    /// All selected contributors must exist in this same capture frame; any scrub leaves null.
    pub fn projection(&self,step:&Step,identity:&str,accepted:&[u8],runtime:&str,session:&str,anchor:&Point)->Option<crate::agent::history::lineage::Projection> {
        use crate::agent::history::lineage::{native_prefix,Projection,decoder_id};
        let Point::Record {start,end,sha256,..}=anchor else{return None;};
        if step.origins.is_empty() || self.identity!=identity{return None;}
        let mut contributions=Vec::new();
        for origin in &step.origins {
            if origin["source"].as_str()!=self.path.to_str() {return None;}
            let n0=origin["start"].as_u64()?;let n1=origin["end"].as_u64()?;
            let physical=self.records.iter().find(|r|r.start==n0 && r.end==n1 && origin["sha256"]==r.native_sha256)?;
            let a=n0.checked_sub(self.base)?;let b=n1.checked_sub(self.base)?;
            let row=accepted.get(usize::try_from(a).ok()?..usize::try_from(b).ok()?)?;
            // This finite witness supports unchanged complete JSONL contributors only.
            if crate::sha256_hex(row)!=physical.native_sha256 {return None;}
            contributions.push((a,b,crate::sha256_hex(row)));
        }
        contributions.sort();contributions.dedup();
        let prefix=accepted.get(..usize::try_from(*end).ok()?)?;
        let (runtime,format,label,binding,native_session)=if self.format==crate::agent::history::CODEX_APP_SERVER_SELECTED {
            let h=self.history.as_ref()?;
            if h["format"]!=self.format || h["runtime"]!=runtime || h["native_session"]!=session{return None;}
            (h["runtime"].as_str()?,h["format"].as_str()?,h["source"].as_str()?,h["binding"].as_str()?,h["native_session"].as_str())
        }else{(runtime,self.format.as_str(),self.source.as_str(),session,Some(session))};
        let steps=native_prefix(runtime,format,label,binding,native_session,prefix).ok()?;
        let mut matching=steps.iter().filter(|candidate| {
            if candidate.source["start"]!=*start || candidate.source["end"]!=*end || candidate.source["sha256"]!=*sha256 {return false;}
            let mut mapped=Vec::new();
            for o in &candidate.origins {
                let (Some(a),Some(b),Some(h))=(o["start"].as_u64(),o["end"].as_u64(),o["sha256"].as_str()) else{return false;};
                if o["source"]!=label{return false;}
                mapped.push((a,b,h.to_string()));
            }
            mapped.sort();mapped.dedup();mapped==contributions
        });
        let selected=matching.next()?;
        if matching.next().is_some(){return None;}
        Some(Projection {decoder:decoder_id().into(),part:u64::try_from(selected.part).ok()?})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::history::lineage::{native_prefix, decoder_id};
    #[test]
    fn same_physical_jsonl_frame_not_latest_hash_or_local_part() {
        let raw=b"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"fixture\"}]}}\n";
        let step=native_prefix("codex","codex-rollout","private-source","fixture-parent",Some("fixture-parent"),raw).unwrap().remove(0);
        let frame=Frame::capture("generation-one","incarnation-one",Path::new("private-source"),0,"codex-rollout",raw).unwrap();
        // Accepted coverage may arrive in arbitrary chunk boundaries; the record hash is over reassembled bytes.
        let cut=raw.len()/2;let accepted=[&raw[..cut],&raw[cut..]].concat();
        let anchor=frame.anchor(&step,"incarnation-one",&accepted).unwrap();
        let projection=frame.projection(&step,"incarnation-one",&accepted,"codex","fixture-parent",&anchor).unwrap();assert_eq!(projection.decoder,decoder_id());
        assert!(frame.anchor(&step,"incarnation-two",&accepted).is_none());assert!(frame.anchor(&step,"incarnation-one",&accepted[..cut]).is_none());
        let mut foreign=step.clone();foreign.origins[0]["source"]=json!("another-source");assert!(frame.projection(&foreign,"incarnation-one",&accepted,"codex","fixture-parent",&anchor).is_none());
        let mut changed=accepted.clone();let at=changed.iter().position(|b|*b==b'f').unwrap();changed[at]=b'x';
        assert!(frame.projection(&step,"incarnation-one",&changed,"codex","fixture-parent",&anchor).is_none());
        let mut other=frame.clone();other.source="generation-two".into();assert_ne!(frame.anchor(&step,"incarnation-one",&accepted),other.anchor(&step,"incarnation-one",&accepted));
    }
}

#[cfg(test)]
mod claude_native_projection_tests {
    use super::*;
    use crate::agent::history::lineage::native_prefix;
    // #300: native capture maps the actual native record, never its stream-log lookalike.
    #[test]
    fn claude_terminal_attachment_uses_same_physical_native_generation() {
        let raw=include_bytes!("../../tests/fixtures/history/claude-fork-tool-chain.jsonl");let session="00000000-0000-4000-8000-000000000099";
        let steps=native_prefix("claude","claude-native","private-native",session,Some(session),raw).unwrap();
        let step=steps.iter().find(|s|s.source["raw"].as_str().is_some_and(|r|r.contains("\"type\":\"attachment\""))).unwrap();
        let frame=Frame::capture("native-generation","native-incarnation",Path::new("private-native"),0,"claude-native",raw).unwrap();
        let anchor=frame.anchor(step,"native-incarnation",raw).unwrap();assert!(frame.projection(step,"native-incarnation",raw,"claude",session,&anchor).is_some());
        assert!(frame.anchor(step,"different-incarnation",raw).is_none());
        let mut stream=step.clone();stream.source["source"]=json!("private-stream");stream.source["format"]=json!("claude-stream");assert!(frame.anchor(&stream,"native-incarnation",raw).is_none());
        let mut foreign=step.clone();foreign.origins[0]["source"]=json!("private-stream");assert!(frame.projection(&foreign,"native-incarnation",raw,"claude",session,&anchor).is_none());
    }
}
