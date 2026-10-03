//! Issue-258: a host call that run control interrupted is shown as
//! interrupted, with the resume instruction, never as a failed subject. The
//! call record itself stays `NeedsReview` (not reusable), so a resume re-runs
//! the call; only the projection and the rendering read it differently.

use archon_workflow::{
    FixedDecompositionStateV1, HostCommandRequest, SubjectDisposition, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2Status,
};

use super::workflow_decompose_state::{
    FIXED_STATE_PATH, FixedCallProjectionKind, project_fixed_call,
};
use super::workflow_decompose_state_tests::seed_state;

/// The record `save_interrupted_call_record` writes when a pause stops a
/// freeze command mid-flight.
fn paused_freeze_record(run_id: &str) -> WorkflowV2CallRecord {
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
                    HostCommandRequest::new("freeze-acceptance", Some("candidate".into())).unwrap(),
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

fn project_paused_freeze(temp: &tempfile::TempDir, run_id: &str) -> (WorkflowStore, String) {
    let store = WorkflowStore::new(temp.path().join("workflows"));
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    seed_state(&store, run_id, &log);
    let record = paused_freeze_record(run_id);
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .save_call_record(&record)
        .unwrap();
    project_fixed_call(
        &store,
        run_id,
        &record,
        FixedCallProjectionKind::Interrupted,
    )
    .unwrap();
    (store, std::fs::read_to_string(&log).unwrap())
}

#[test]
fn an_interrupted_host_command_projects_as_interrupted_not_failed() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "wf-paused";
    let (store, log) = project_paused_freeze(&temp, run_id);

    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state.dispositions.get("acceptance"),
        Some(&SubjectDisposition::Interrupted),
        "a paused freeze is interrupted, not failed"
    );
    assert!(log.contains("disposition=interrupted"), "{log}");
    assert!(!log.contains("disposition=failed"), "{log}");
    let events = std::fs::read_to_string(store.events_path(run_id)).unwrap();
    assert!(events.contains("host_command_interrupted"), "{events}");
}

#[test]
fn status_shows_an_interrupted_host_call_with_the_resume_hint() {
    let temp = tempfile::tempdir().unwrap();
    let run_id = "wf-paused";
    let (store, _) = project_paused_freeze(&temp, run_id);

    let status = super::workflow_decompose_status::render(&store, run_id)
        .unwrap()
        .unwrap();

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
    assert!(
        status.contains(&format!(
            "interrupted calls re-run on resume: archon workflow resume --live --yes {run_id}"
        )),
        "{status}"
    );
}
