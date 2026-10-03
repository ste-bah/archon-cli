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

#[test]
fn event_previews_after_returns_incremental_sanitized_events() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Research topic").unwrap();
    let run = store.create_run(spec).unwrap();
    let log = WorkflowEventLog::new(store.clone());
    log.emit(
        &run.id,
        1,
        WorkflowEventKind::StageStarted,
        json!({"stage": "discover"}),
    )
    .unwrap();
    log.emit(
        &run.id,
        2,
        WorkflowEventKind::StageCompleted,
        json!({"stage": "discover", "thinking": "private"}),
    )
    .unwrap();

    let events = web_api::event_previews_after(&store, &run.id, 1, 10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 2);
    assert_eq!(events[0].summary, "discover");
    assert!(!serde_json::to_string(&events).unwrap().contains("private"));
}

/// Issue-245: v2 results and branches are stored unredacted; the web views
/// keep every row but never show a secret-shaped word or a forbidden key.
#[test]
fn v2_views_redact_unredacted_disk_records() {
    use archon_workflow::{
        WorkflowV2BranchOutcome, WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
        WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    };
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let spec = HeuristicWorkflowPlanner.plan("Audit codebase").unwrap();
    let run = store.create_run(spec).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let secrets = "token=x sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 Authorization: Bearer opaque-bearer-credential-123";
    let mut result = WorkflowV2Result::accepted(secrets);
    result.data = json!({"api_key": "k", "reasoning": "r", "raw_text": "t"});
    let call = WorkflowV2HostCall {
        id: "wave-1".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &run.id,
        call,
        1,
        "hash".into(),
        result.clone(),
        Vec::new(),
    ))
    .unwrap();
    let outcome = WorkflowV2BranchOutcome {
        item_id: "item-1".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: Some(secrets.into()),
        failure_kind: None,
        item_input_hash: Some("hash-1".into()),
        completion_evidence: Vec::new(),
    };
    v2.save_branch_outcome("wave-1", &outcome).unwrap();
    let on_disk = std::fs::read_to_string(v2.result_path("wave-1")).unwrap();
    assert!(on_disk.contains("token=x") && on_disk.contains("api_key"));

    let detail = web_api::detail(&store, &run.id).unwrap();
    assert_eq!(detail.v2_results.len(), 1, "the row is kept");
    assert_eq!(detail.v2_branches.len(), 1, "the row is kept");
    let shown = serde_json::to_string(&(&detail.v2_results, &detail.v2_branches)).unwrap();
    for hidden in [
        "token=x",
        "sk-ant-",
        "opaque-bearer-credential-123",
        "api_key",
        "reasoning",
        "raw_text",
    ] {
        assert!(!shown.contains(hidden), "{hidden} shown in {shown}");
    }
    assert!(shown.contains("<redacted>"), "{shown}");
}
