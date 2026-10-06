//! Issue-258: a host call that run control interrupted is shown as
//! interrupted, with the resume instruction, never as a failed subject. The
//! call record itself stays `NeedsReview` (not reusable), so a resume re-runs
//! the call; only the projection and the rendering read it differently.

use archon_workflow::{
    FixedDecompositionStateV1, HostCommandRequest, RunStatus, SubjectDisposition, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status,
};

use super::workflow_decompose_state::{
    FIXED_STATE_PATH, FixedCallProjectionKind, project_fixed_call,
};
use super::workflow_decompose_state_tests::{host_record, seed_state};

/// The record `save_interrupted_call_record` writes when a pause stops a
/// freeze command mid-flight.
fn paused_record(run_id: &str, command_id: &str) -> WorkflowV2CallRecord {
    let summary = "workflow v2 call 'hostCommand#3' was paused after 13959s in flight and produced no result: workflow paused by run control: generation 2 observed before/after V2 call 'hostCommand#3'";
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: summary.to_string(),
        ..WorkflowV2Result::default()
    };
    result.data = serde_json::json!({
        "call_id": "hostCommand#3",
        "interrupted": "paused",
        "elapsed_seconds": 13959,
        "error": "workflow paused by run control: generation 2 observed before/after V2 call 'hostCommand#3'",
    });
    WorkflowV2CallRecord::new(
        run_id,
        WorkflowV2HostCall {
            id: "hostCommand#3".into(),
            method: WorkflowV2HostMethod::HostCommand,
            write_mode: None,
            options: WorkflowV2HostOptions {
                host_command: Some(
                    HostCommandRequest::new(command_id, Some("candidate".into())).unwrap(),
                ),
                ..WorkflowV2HostOptions::default()
            },
        },
        1,
        "input".to_string(),
        result,
        Vec::new(),
    )
}

/// A fixed-decomposition run whose stored status is `status`, with its
/// projection state seeded and no call projected yet.
fn fixed_run(temp: &tempfile::TempDir, status: RunStatus) -> (WorkflowStore, String) {
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "fixed-interrupt-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: std::collections::BTreeMap::new(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = status;
    store.save_state(&run).unwrap();
    if !store.events_path(&run.id).exists() {
        std::fs::write(store.events_path(&run.id), "").unwrap();
    }
    let log = temp.path().join("tasks/.decompose.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    seed_state(&store, &run.id, &log);
    (store, run.id)
}

fn save(store: &WorkflowStore, run_id: &str, record: &WorkflowV2CallRecord) {
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .save_call_record(record)
        .unwrap();
}

fn project(
    store: &WorkflowStore,
    run_id: &str,
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
) {
    project_fixed_call(store, run_id, record, kind).unwrap();
}

fn state(store: &WorkflowStore, run_id: &str) -> FixedDecompositionStateV1 {
    serde_json::from_slice(&std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap())
        .unwrap()
}

fn status(store: &WorkflowStore, run_id: &str) -> String {
    super::workflow_decompose_status::render(store, run_id)
        .unwrap()
        .unwrap()
}

fn hint(run_id: &str) -> String {
    format!("interrupted calls re-run on resume: archon workflow resume --live --yes {run_id}")
}

/// A paused freeze, saved and projected as `save_interrupted_call_record`
/// does it.
fn paused_freeze_run(temp: &tempfile::TempDir, status: RunStatus) -> (WorkflowStore, String) {
    let (store, run_id) = fixed_run(temp, status);
    let record = paused_record(&run_id, "freeze-acceptance");
    save(&store, &run_id, &record);
    project(
        &store,
        &run_id,
        &record,
        FixedCallProjectionKind::Interrupted,
    );
    (store, run_id)
}

#[test]
fn an_interrupted_host_command_projects_as_interrupted_not_failed() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = paused_freeze_run(&temp, RunStatus::Paused);

    assert_eq!(
        state(&store, &run_id).dispositions.get("acceptance"),
        Some(&SubjectDisposition::Interrupted),
        "a paused freeze is interrupted, not failed"
    );
    let log = std::fs::read_to_string(temp.path().join("tasks/.decompose.log")).unwrap();
    assert!(log.contains("disposition=interrupted"), "{log}");
    assert!(!log.contains("disposition=failed"), "{log}");
    let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
    assert!(events.contains("host_command_interrupted"), "{events}");
}

#[test]
fn status_shows_an_interrupted_host_call_with_the_resume_hint() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = paused_freeze_run(&temp, RunStatus::Paused);

    let status = status(&store, &run_id);

    assert!(status.contains("- acceptance=interrupted"), "{status}");
    assert!(!status.contains("=failed"), "{status}");
    assert!(
        status.contains("call_status: accepted=0 interrupted=1 failed=0"),
        "{status}"
    );
    assert!(
        status.contains("interrupted_call: hostCommand#3 capability=freeze-acceptance reason=paused elapsed_secs=13959"),
        "{status}"
    );
    assert!(status.contains(&hint(&run_id)), "{status}");
}

/// `workflow decompose --resume` accepts a cancelled run as well as a paused
/// one, so both get the hint; a run still running gets none.
#[test]
fn the_resume_hint_follows_what_resume_accepts() {
    for (stored, shown) in [
        (RunStatus::Paused, true),
        (RunStatus::Cancelled, true),
        (RunStatus::Running, false),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (store, run_id) = paused_freeze_run(&temp, stored.clone());
        let status = status(&store, &run_id);
        assert_eq!(
            status.contains(&hint(&run_id)),
            shown,
            "{stored:?}: {status}"
        );
    }
}

/// A run paused under the old projection already persisted
/// `freeze-acceptance=failed` for its interrupted call. Status reads the call
/// record, not that stale entry.
#[test]
fn status_overrides_a_stale_failed_entry_for_an_interrupted_call() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = fixed_run(&temp, RunStatus::Paused);
    let mut stale = state(&store, &run_id);
    stale
        .dispositions
        .insert("acceptance".into(), SubjectDisposition::Pending);
    stale
        .dispositions
        .insert("freeze-acceptance".into(), SubjectDisposition::Failed);
    store
        .write_run_json(&run_id, FIXED_STATE_PATH, &stale)
        .unwrap();
    save(
        &store,
        &run_id,
        &paused_record(&run_id, "freeze-acceptance"),
    );

    let status = status(&store, &run_id);

    assert!(status.contains("- acceptance=interrupted"), "{status}");
    assert!(!status.contains("freeze-acceptance=failed"), "{status}");
    assert!(status.contains(&hint(&run_id)), "{status}");
}

/// The command completing after the resume replaces the stale entry the old
/// projection keyed by command id.
#[test]
fn a_completed_command_removes_its_stale_command_keyed_entry() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = fixed_run(&temp, RunStatus::Running);
    let mut stale = state(&store, &run_id);
    stale
        .dispositions
        .insert("freeze-acceptance".into(), SubjectDisposition::Failed);
    store
        .write_run_json(&run_id, FIXED_STATE_PATH, &stale)
        .unwrap();

    project(
        &store,
        &run_id,
        &host_record(&run_id),
        FixedCallProjectionKind::Executed,
    );

    let dispositions = state(&store, &run_id).dispositions;
    assert_eq!(
        dispositions.get("acceptance"),
        Some(&SubjectDisposition::Accepted)
    );
    assert_eq!(
        dispositions.get("freeze-acceptance"),
        None,
        "{dispositions:?}"
    );
}

/// A landing names its task only in its outcome. Starting or interrupting
/// one must not leave a generic `body` entry that its completion, keyed by
/// the real task id, never clears.
#[test]
fn a_task_body_landing_leaves_no_generic_body_entry() {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = fixed_run(&temp, RunStatus::Running);
    let interrupted = paused_record(&run_id, "land-task-body");
    project(
        &store,
        &run_id,
        &interrupted,
        FixedCallProjectionKind::Started,
    );
    project(
        &store,
        &run_id,
        &interrupted,
        FixedCallProjectionKind::Interrupted,
    );

    let mut landed = host_record(&run_id);
    landed.call = interrupted.call.clone();
    landed.attempt = 2;
    landed.result.data["subjects"] =
        serde_json::json!([{ "taskId": "TASK-X-010", "fileName": "TASK-X-010.md" }]);
    project(&store, &run_id, &landed, FixedCallProjectionKind::Executed);

    let dispositions = state(&store, &run_id).dispositions;
    assert_eq!(
        dispositions.get("TASK-X-010"),
        Some(&SubjectDisposition::Accepted),
        "{dispositions:?}"
    );
    assert_eq!(dispositions.get("body"), None, "{dispositions:?}");
}

#[test]
fn round3_successful_body_resume_clears_legacy_failure() {
    successful_body_resume(false);
}

#[test]
fn round3_accepted_body_record_reconciles_legacy_failure_read_only() {
    successful_body_resume(true);
}

fn successful_body_resume(recover_projection: bool) {
    let temp = tempfile::tempdir().unwrap();
    let (store, run_id) = fixed_run(&temp, RunStatus::Paused);
    let mut legacy = state(&store, &run_id);
    legacy
        .dispositions
        .insert("body".into(), SubjectDisposition::Pending);
    legacy
        .dispositions
        .insert("land-task-body".into(), SubjectDisposition::Failed);
    store
        .write_run_json(&run_id, FIXED_STATE_PATH, &legacy)
        .unwrap();
    let interrupted = paused_record(&run_id, "land-task-body");
    save(&store, &run_id, &interrupted);
    let before = std::fs::read(store.run_dir(&run_id).join(FIXED_STATE_PATH)).unwrap();
    assert!(!status(&store, &run_id).contains("land-task-body=failed"));
    assert_eq!(
        std::fs::read(store.run_dir(&run_id).join(FIXED_STATE_PATH)).unwrap(),
        before
    );

    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let mut landed = host_record(&run_id);
    landed.call = interrupted.call;
    landed.attempt = 2;
    landed.result.data["subjects"] =
        serde_json::json!([{ "taskId": "TASK-X-010", "fileName": "TASK-X-010.md" }]);
    save(&store, &run_id, &landed);
    // Status must also be correct if the process exits between saving the
    // accepted call record and updating the projection. It remains read-only.
    if recover_projection {
        let saved_projection =
            std::fs::read(store.run_dir(&run_id).join(FIXED_STATE_PATH)).unwrap();
        let recovered = status(&store, &run_id);
        assert!(!recovered.contains("land-task-body=failed"), "{recovered}");
        assert_eq!(
            std::fs::read(store.run_dir(&run_id).join(FIXED_STATE_PATH)).unwrap(),
            saved_projection
        );
    }
    project(&store, &run_id, &landed, FixedCallProjectionKind::Executed);
    let rendered = status(&store, &run_id);
    assert!(rendered.contains("TASK-X-010=accepted"), "{rendered}");
    assert!(!rendered.contains("land-task-body=failed"), "{rendered}");
    let dispositions = state(&store, &run_id).dispositions;
    assert!(!dispositions.contains_key("body"), "{dispositions:?}");
    assert!(
        !dispositions.contains_key("land-task-body"),
        "{dispositions:?}"
    );

    // A different landing's genuine failure must remain visible, even after
    // the successful resumed call projects its named task again.
    let mut failed = landed.clone();
    failed.call.id = "another-landing".into();
    failed.status = WorkflowV2Status::Failed;
    failed.result.status = WorkflowV2Status::Failed;
    let mut unbound: archon_workflow::HostCommandResult =
        serde_json::from_value(landed.result.data.clone()).unwrap();
    unbound.subjects.clear();
    unbound.exit_code = Some(1);
    unbound.publication_receipt = None;
    unbound.postcondition = None;
    for data in [
        serde_json::Value::Null,
        serde_json::to_value(unbound).unwrap(),
    ] {
        failed.result.data = data;
        save(&store, &run_id, &failed);
        project(&store, &run_id, &failed, FixedCallProjectionKind::Executed);
        project(&store, &run_id, &landed, FixedCallProjectionKind::Executed);
        let rendered = status(&store, &run_id);
        assert!(rendered.contains("land-task-body=failed"), "{rendered}");
        assert_eq!(
            state(&store, &run_id).dispositions.get("land-task-body"),
            Some(&SubjectDisposition::Failed),
            "durable projection retains another call's failure"
        );
    }
}

#[test]
fn issue291_projection_keeps_other_landings_failure_on_execution_reuse_and_interrupt() {
    for kind in [
        FixedCallProjectionKind::Executed,
        FixedCallProjectionKind::Reused,
        FixedCallProjectionKind::Interrupted,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (store, run_id) = fixed_run(&temp, RunStatus::Running);
        let mut failed = host_record(&run_id);
        failed.call.id = "failed-unbound-landing".into();
        failed.call.options.host_command =
            Some(HostCommandRequest::new("land-task-body", Some("candidate-a".into())).unwrap());
        failed.status = WorkflowV2Status::Failed;
        failed.result.status = WorkflowV2Status::Failed;
        failed.result.data = serde_json::Value::Null;
        save(&store, &run_id, &failed);
        project(&store, &run_id, &failed, FixedCallProjectionKind::Executed);
        let mut other = host_record(&run_id);
        other.call.id = "different-landing".into();
        other.call.options.host_command =
            Some(HostCommandRequest::new("land-task-body", Some("candidate-b".into())).unwrap());
        other.result.data["subjects"] =
            serde_json::json!([{ "taskId": "TASK-X-010", "fileName": "TASK-X-010.md" }]);
        if kind == FixedCallProjectionKind::Interrupted {
            other.status = WorkflowV2Status::NeedsReview;
            other.result.status = WorkflowV2Status::NeedsReview;
            other.result.data = serde_json::json!({"interrupted":"paused"});
        }
        save(&store, &run_id, &other);
        project(&store, &run_id, &other, kind);
        assert_eq!(
            state(&store, &run_id).dispositions.get("land-task-body"),
            Some(&SubjectDisposition::Failed),
            "{kind:?}"
        );
        // Only a replacement of the failed call itself clears that failure.
        failed.status = WorkflowV2Status::Accepted;
        failed.result = host_record(&run_id).result;
        failed.result.data["subjects"] =
            serde_json::json!([{ "taskId": "TASK-X-011", "fileName": "TASK-X-011.md" }]);
        save(&store, &run_id, &failed);
        project(&store, &run_id, &failed, FixedCallProjectionKind::Executed);
        assert!(
            !state(&store, &run_id)
                .dispositions
                .contains_key("land-task-body")
        );
    }
}
