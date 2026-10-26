use super::*;

#[tokio::test]
async fn an_orphaned_call_settles_its_stage_and_repeated_starts_are_idempotent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let dead_id = "AC-X-001";
    let live_id = "TASK-X-001";
    let mut state = run.clone();
    for id in [dead_id, live_id] {
        let mut stage = archon_workflow::run::StageState::pending(id);
        stage.status = archon_workflow::StageStatus::Running;
        state.stages.insert(id.to_string(), stage);
    }
    workflow_store.save_state(&state).expect("running stages");

    let host = archon_test_support::live_process::LiveChild::spawn();
    let mut live_marker = marker(live_id);
    live_marker.host_pid = host.pid();
    let inflight = v2_root.join("inflight");
    write_marker(&inflight, &marker(dead_id)).expect("dead host marker");
    write_marker(&inflight, &live_marker).expect("live host marker");

    start_run(workflow_store.clone(), v2_store.clone(), &run.id).await;

    let orphan = v2_store
        .load_call_record(dead_id)
        .expect("lookup")
        .expect("orphan result");
    assert_eq!(orphan.status, WorkflowV2Status::NeedsReview);
    let first_state = workflow_store.load_state(&run.id).expect("state");
    let settled = &first_state.stages[dead_id];
    assert_eq!(settled.status, archon_workflow::StageStatus::NeedsReview);
    assert!(
        settled.completed_at.is_some(),
        "stage completion is recorded"
    );
    assert_eq!(
        first_state.stages[live_id].status,
        archon_workflow::StageStatus::Running
    );
    assert!(
        inflight.join(marker_name(live_id)).exists(),
        "live marker remains"
    );

    start_run(workflow_store.clone(), v2_store.clone(), &run.id).await;

    let second_state = workflow_store.load_state(&run.id).expect("state");
    assert_eq!(second_state.stages[dead_id].status, settled.status);
    assert_eq!(
        second_state.stages[dead_id].completed_at,
        settled.completed_at
    );
    let repeated = v2_store
        .load_call_record(dead_id)
        .expect("lookup")
        .expect("orphan result");
    assert_eq!(repeated, orphan);
    assert!(
        inflight.join(marker_name(live_id)).exists(),
        "live marker remains"
    );
    drop(host);
}

#[tokio::test]
async fn an_answered_orphan_marker_settles_a_running_stage_on_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let call_id = "AC-X-001";

    let mut state = run.clone();
    let mut stage = archon_workflow::run::StageState::pending(call_id);
    stage.status = archon_workflow::StageStatus::Running;
    state.stages.insert(call_id.to_string(), stage);
    workflow_store.save_state(&state).expect("running stage");

    let marker = marker(call_id);
    let answered = WorkflowV2CallRecord::new(
        run.id.clone(),
        marker.call.clone(),
        marker.attempt,
        marker.input_hash.clone(),
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "interrupted call".to_string(),
            ..WorkflowV2Result::default()
        },
        marker.depends_on.clone(),
    );
    v2_store
        .save_call_record(&answered)
        .expect("seed answered record");
    write_marker(&v2_root.join("inflight"), &marker).expect("stale marker");

    start_run(workflow_store.clone(), v2_store, &run.id).await;

    let settled = workflow_store
        .load_state(&run.id)
        .expect("state")
        .stages
        .remove(call_id)
        .expect("stage");
    assert_eq!(settled.status, archon_workflow::StageStatus::NeedsReview);
    assert!(
        settled.completed_at.is_some(),
        "stage completion is recorded"
    );
}

#[tokio::test]
async fn an_accepted_answered_orphan_marker_accepts_a_running_stage_on_restart() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec()).expect("run");
    let v2_root = workflow_store.run_dir(&run.id).join("v2");
    let v2_store = WorkflowV2ResultStore::new(v2_root.clone());
    let call_id = "AC-X-001";

    let mut state = run.clone();
    let mut stage = archon_workflow::run::StageState::pending(call_id);
    stage.status = archon_workflow::StageStatus::Running;
    state.stages.insert(call_id.to_string(), stage);
    workflow_store.save_state(&state).expect("running stage");

    let marker = marker(call_id);
    let answered = WorkflowV2CallRecord::new(
        run.id.clone(),
        marker.call.clone(),
        marker.attempt,
        marker.input_hash.clone(),
        WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: "completed call".to_string(),
            ..WorkflowV2Result::default()
        },
        marker.depends_on.clone(),
    );
    v2_store
        .save_call_record(&answered)
        .expect("seed answered record");
    write_marker(&v2_root.join("inflight"), &marker).expect("stale marker");

    start_run(workflow_store.clone(), v2_store, &run.id).await;

    let settled = workflow_store
        .load_state(&run.id)
        .expect("state")
        .stages
        .remove(call_id)
        .expect("stage");
    assert_eq!(settled.status, archon_workflow::StageStatus::Accepted);
    assert!(
        settled.completed_at.is_some(),
        "stage completion is recorded"
    );
}
