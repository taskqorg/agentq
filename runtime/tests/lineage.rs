//! SOURCE fixtures for F300-L20261107-2; execution belongs to the final coordinated batch.
use agentq_runtime::agent::history::{self, lineage::*};
use serde_json::{json,Value};

fn fixture()->Record {
    Record {record_type:"agentq_lineage".into(),version:1,operation_id:"fixture-operation".into(),revision:0,kind:Kind::Fork,effect:Effect::Applied,session:"fixture-child".into(),parent:Parent {session:"fixture-parent".into(),point:Point::Empty},predecessor:None,activation:Activation::Pending {reason:Pending::NoNativeBytes},conditions:Conditions::default()}
}
fn line(v:&Value)->Vec<u8> {let mut b=serde_json::to_vec(v).unwrap();b.push(b'\n');b}
#[test]
fn closed_framed_record_and_read_only_link() {
    let r=fixture();let bytes=r.line().unwrap();assert_eq!(parse(&bytes,&r.session).unwrap(),r);
    let steps=project(&bytes,"aux-source",&r.session).unwrap();
    assert_eq!(steps.len(),1);assert_eq!(steps[0].kind,history::StepKind::Link);
    assert!(steps[0].native_id.is_none() && steps[0].turn.id.is_none() && steps[0].turn.native_id.is_none() && steps[0].parent.is_none());
    assert!(history::History::new(FORMAT,"aux-source",&r.session).is_err());
    assert!(history::History::caller("codex",FORMAT,"aux-source",&r.session,None).is_err());
    assert!(parse(&bytes,"foreign-session").is_err());
    assert!(parse(&bytes[..bytes.len()-1],&r.session).is_err());
    let mut v=serde_json::to_value(&r).unwrap();v["extra"]=json!(true);assert!(parse(&line(&v),&r.session).is_err());
    v.as_object_mut().unwrap().remove("extra");v.as_object_mut().unwrap().remove("predecessor");assert!(parse(&line(&v),&r.session).is_err());
    let duplicate=String::from_utf8(bytes).unwrap().replacen("\"version\":1","\"version\":1,\"version\":1",1);
    assert!(parse(duplicate.as_bytes(),&r.session).is_err());
    let unknown=project(b"{broken}\n","aux-source",&r.session).unwrap();assert_eq!(unknown[0].kind,history::StepKind::Unknown);assert!(unknown[0].source["raw"].as_str().unwrap().contains("broken"));
}
#[test]
fn requested_and_observed_nulls_policy_and_comparisons() {
    let mut r=fixture();r.conditions.model.requested=Pair::new(Some("model-a".into()),Some("model-b".into()));
    assert_eq!(r.conditions.model.requested.changed,Some(true));assert_eq!(r.conditions.model.observed.changed,None);
    assert!(r.line().is_ok());r.conditions.model.requested.changed=Some(false);assert!(r.line().is_err());
    r.conditions.model.requested=Pair::new(None,Some("model-b".into()));r.conditions.model.requested.changed=Some(false);assert!(r.line().is_err());
    let mut redacted=fixture();redacted.conditions.model.observed=Pair::new(Some("********".into()),Some("********".into()));assert!(redacted.line().is_err());
    let mut v=serde_json::to_value(fixture()).unwrap();
    v["conditions"]["policy"]["requested"]["after"]=json!({"read":"native","write":"project","network":"off","tools":null,"approval":"deny","roots":"native","unelevated":false,"builtin":"native","isolated":true,"subscription":true});
    assert!(parse(&line(&v),"fixture-child").is_ok());
    v["conditions"]["policy"]["requested"]["after"]["roots"]=json!({"exact":["/private/path"]});assert!(parse(&line(&v),"fixture-child").is_err());
    v["conditions"]["policy"]["requested"]["after"]["roots"]=json!("native");v["conditions"]["policy"]["requested"]["after"]["tools"]=json!(["/private/tool"]);assert!(parse(&line(&v),"fixture-child").is_err());
}
#[test]
fn immutable_operation_revisions_conflicts_and_generations() {
    let mut base=fixture();base.parent.point=Point::Unmapped {reason:Unmapped::NotArchived};
    let mut c=Chain::default();c.append(base.clone()).unwrap();
    let mut mapped=base.clone();mapped.revision=1;mapped.parent.point=Point::Record {source:"generation-one".into(),start:0,end:9,sha256:"a".repeat(64),projection:None};
    c.append(mapped.clone()).unwrap();assert_eq!(c.append(base.clone()).unwrap().revision,1);
    let mut projected=mapped.clone();projected.revision=2;if let Point::Record {projection,..}=&mut projected.parent.point {*projection=Some(Projection {decoder:decoder_id().into(),part:0});}
    c.append(projected.clone()).unwrap();
    let mut wrong=projected.clone();wrong.revision=3;if let Point::Record {source,..}=&mut wrong.parent.point {*source="generation-two".into();}
    assert!(c.append(wrong).is_err());assert!(c.latest().is_err());
    let mut c=Chain::default();assert!(c.append(mapped).is_err());
    let mut c=Chain::default();c.append(base.clone()).unwrap();let mut gap=base.clone();gap.revision=2;assert!(c.append(gap).is_err());
    let mut c=Chain::default();c.append(base.clone()).unwrap();let mut conflict=base;conflict.conditions.model.requested=Pair::new(None,Some("changed-intent".into()));assert!(c.append(conflict).is_err());
}
#[test]
fn source_prefix_projection_is_deterministic_and_never_native_authority() {
    let bytes=b"{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"fixture\"}]}}\n";
    let a=native_prefix("codex","codex-rollout","archive-source","fixture-parent",Some("fixture-parent"),bytes).unwrap();
    let b=native_prefix("codex","codex-rollout","private-source","fixture-parent",Some("fixture-parent"),bytes).unwrap();
    assert!(!a.is_empty());assert_ne!(a[0].id,b[0].id);assert_eq!(a[0].source["sha256"],b[0].source["sha256"]);
    assert!(native_prefix("codex",FORMAT,"archive-source","fixture-parent",None,bytes).is_err());
}
