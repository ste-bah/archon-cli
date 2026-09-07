use super::*;
use crate::WorkflowV2HostMethod;
#[test]
fn all_noop_wave_retains_noop_instead_of_claiming_implementation() {
    let call=WorkflowV2HostCall{id:"noop-wave".into(),method:WorkflowV2HostMethod::Fanout,write_mode:Some(WorkflowV2WriteMode::Worktree),options:Default::default()};
    let mut branch=WorkflowV2Result::noop("already satisfied");
    branch.data=serde_json::json!({"item_id":"one","canonical_task_ids":["TASK-001"]});
    branch.evidence.push(WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Inspection,"existing output checked"));
    branch.task_coverage=serde_json::from_value(serde_json::json!([{"task_id":"TASK-001","status":"noop","summary":"existing file checked","evidence":[{"kind":"inspection","summary":"existing output checked"}]}])).unwrap();
    let planner=WorkflowV2WritePlanner::new(PathBuf::from("/tmp/noop-plan"));
    let plan=planner.plan(&[WorkflowV2WriteItem::artifact_only("one",WorkflowV2WriteMode::Worktree)]).unwrap();
    let result=result_from_write_fanout(&call,vec![branch],&plan,0,None);
    assert_eq!(result.status,WorkflowV2Status::Noop,"{result:#?}");
}
