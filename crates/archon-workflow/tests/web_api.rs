use archon_workflow::{
    HeuristicWorkflowPlanner, WorkflowEventKind, WorkflowEventLog, WorkflowPlanner, WorkflowStore,
    web_api,
};
use serde_json::json;

#[test]
fn summary_and_detail_expose_workflow_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Audit codebase").unwrap();
    let mut run = store.create_run(spec).unwrap();
    run.stage_mut("discover").unwrap().status = archon_workflow::StageStatus::Failed;
    store.save_state(&run).unwrap();

    let summary = web_api::summary(&store, 10).unwrap();
    assert_eq!(summary.runs.len(), 1);
    assert_eq!(summary.runs[0].failed_count, 1);

    let detail = web_api::detail(&store, &run.id).unwrap();
    assert!(
        detail
            .stages
            .iter()
            .any(|stage| stage.status == archon_workflow::StageStatus::Failed)
    );
}

#[test]
fn event_previews_hide_tool_noise_and_private_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Research topic").unwrap();
    let run = store.create_run(spec).unwrap();
    let log = WorkflowEventLog::new(store.clone());
    log.emit(
        &run.id,
        1,
        WorkflowEventKind::StageStarted,
        json!({"stage": "discover", "thinking": "secret"}),
    )
    .unwrap();
    log.emit(
        &run.id,
        2,
        WorkflowEventKind::StageCompleted,
        json!({"stage": "tool", "raw_tool_output": "spam"}),
    )
    .unwrap();

    let events = web_api::event_previews(&store, &run.id, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].summary, "discover");
    let raw = serde_json::to_string(&events).unwrap();
    assert!(!raw.contains("secret"));
    assert!(!raw.contains("spam"));
}
