use super::*;
use crate::subagent::runner::tests::{MockProvider,make_runner,text_response};
use std::sync::Arc;
use serde_json::json;
struct Landed;
impl archon_tools::audit_landing::LandingHost for Landed {
    fn land(&self,_:serde_json::Value)->Result<String,String>{unreachable!()}
    fn hint(&self)->Result<String,String>{Ok("25 of 41 landed; remaining file-25 through file-40; do not re-gather landed paths".into())}
    fn complete(&self,_:&serde_json::Value)->Result<(),String>{Err("16 missing".into())}
}
#[tokio::test]
async fn compaction_reinjects_host_landed_records_and_read_orientation() {
    let provider=Arc::new(MockProvider::new(vec![text_response("Summary of prior inspection."),text_response("Summary of prior inspection.")]));
    let mut runner=make_runner(provider.clone(),2);
    runner.tool_context.workflow_read_guard=Some(Arc::new(archon_tools::workflow_read_guard::WorkflowReadGuard::new(40,20,false)));
    runner.tool_context.audit_landing=Some(Arc::new(archon_tools::audit_landing::AuditLanding::new(Arc::new(Landed),None)));
    let history=(0..16).map(|i|json!({"role":if i%2==0{"user"}else{"assistant"},"content":"old evidence ".repeat(2000)})).collect();
    let mut messages=MessageHistory::new(history);
    let mut state=crate::agent::AutoCompactState::default();
    let telemetry=crate::agent::autocompact::CompactionTelemetry { provider_family:"fixture", wire_shape:"fixture", native_context_window:200_000, runtime_context_budget:200_000, context_source:"fixture", compaction_backend:"fixture" };
    let mut known=100_000;
    assert!(compact_proactively(&runner,&mut messages,&mut state,&mut known,&telemetry,
        crate::agent::CompactAction::Full,"fixture compaction failed").await);
    let last=messages.as_slice().last().unwrap()["content"].as_str().unwrap();
    assert!(last.contains("25 of 41"));
    assert!(last.contains("Historical read-set"));
    assert!(last.contains("full contracted artifact"));
}
