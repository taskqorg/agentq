//! Runtime-neutral report assembly; native interpretation belongs to each registered protocol.
use super::super::super::*;

pub(crate) fn limit_reached(client: Client, events: &[Value]) -> Option<Option<i64>> {
    events.iter().find_map(protocol(client).start_limit).and_then(|s| match s {StartLimit::Reached(reset)=>Some(reset),_=>None})
}

pub(crate) fn report(client: Client, text: &str, events: &[Value], rollout: Option<&Path>, status: (&'static str, &'static str, Option<String>), live_background: Vec<String>) -> Report {
    let (calls,count,known)=(protocol(client).read_calls)(events);
    Report {session:session_of(client,text), usage:sum_usage(&calls,count,known), calls, process:ProcessUse::default(),
        limit:snapshot(client,events,rollout), limits:limits::read(client,events,rollout), reached:limit_reached(client,events),
        background_job:!live_background.is_empty(),live_background,outcome:status.0,error:status.1,detail:status.2,reason:None,
        text:(protocol(client).final_text)(events),exit:None,termination:ProcessTermination::Unknown {reason:"native-process-exit-not-observed"},started_ms:unix_ms(),duration_us:0,readiness:None,recorded:Ok(())}
}
