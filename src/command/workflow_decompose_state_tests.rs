use std::collections::BTreeMap;

use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, FixedRunIdentityV1, HostCommandRequest,
    SubjectDisposition, WorkflowRunKind, WorkflowStore, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
};

use super::workflow_decompose_state::{
    FIXED_STATE_PATH, FixedCallProjectionKind, project_fixed_call,
};

fn seed_state(store: &WorkflowStore, run_id: &str, log_path: &std::path::Path) {
    let state = FixedDecompositionStateV1 {
        schema_version: 1,
        run_kind: WorkflowRunKind::FixedDecompositionV1,
        identity: FixedRunIdentityV1 {
            template_version: "fixed-decomposition-v1".into(),
            starting_binary_revision: "rev".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: "/project".into(),
            prd_identity: "/project/PRD.md".into(),
            task_root_identity: "/project/tasks".into(),
        },
        phase: DecompositionPhase::Identity,
        attempts: BTreeMap::new(),
        dispositions: BTreeMap::new(),
        log_path: log_path.to_string_lossy().into_owned(),
    };
    store
        .write_run_json(run_id, FIXED_STATE_PATH, &state)
        .unwrap();
}

fn host_record(run_id: &str) -> WorkflowV2CallRecord {
    let outcome = archon_workflow::HostCommandResult {
        exit_code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
        stdout_bytes: 0,
        stderr_bytes: 0,
        timed_out: false,
        interrupted: false,
        stdout_truncated: false,
        stderr_truncated: false,
        gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::json!("accepted"),
            policy_findings: Vec::new(),
            operational_error: None,
        }),
        publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
            schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
            call_id: "call-1".into(),
            command_id: "freeze-acceptance".into(),
            entries: Vec::new(),
            committed_at: "now".into(),
        }),
        subjects: Vec::new(),
        postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
            satisfied: true,
            summary: "accepted".into(),
        }),
    };
    let mut result = WorkflowV2Result::accepted("accepted");
    result.data = serde_json::to_value(outcome).unwrap();
    WorkflowV2CallRecord::new(
        run_id,
        WorkflowV2HostCall {
            id: "call-1".into(),
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

#[test]
fn fixed_call_projection_persists_phase_disposition_event_then_log() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-fixed";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    seed_state(&store, run_id, &log);

    project_fixed_call(
        &store,
        run_id,
        &host_record(run_id),
        FixedCallProjectionKind::Executed,
    )
    .unwrap();

    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap(),
    )
    .unwrap();
    assert_eq!(state.phase, DecompositionPhase::Acceptance);
    assert_eq!(
        state.dispositions.get("acceptance"),
        Some(&SubjectDisposition::Accepted)
    );
    assert!(store.events_path(run_id).metadata().unwrap().len() > 0);
    let line = std::fs::read_to_string(log).unwrap();
    assert!(line.contains("phase=acceptance"), "{line}");
    assert!(line.contains("disposition=accepted"), "{line}");
}

#[test]
fn legacy_call_projection_is_byte_silent_without_fixed_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-legacy";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "before\n").unwrap();

    project_fixed_call(
        &store,
        run_id,
        &host_record(run_id),
        FixedCallProjectionKind::Executed,
    )
    .unwrap();

    assert_eq!(
        std::fs::read(store.events_path(run_id)).unwrap(),
        b"before\n"
    );
    assert!(!store.run_dir(run_id).join(FIXED_STATE_PATH).exists());
}

#[test]
fn fixed_status_renders_identity_phase_attempts_dispositions_and_log() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-status";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    seed_state(&store, run_id, &log);
    let mut state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap(),
    )
    .unwrap();
    state.phase = DecompositionPhase::Bodies;
    state.attempts.insert(
        "TASK-X-010".into(),
        archon_workflow::DecompositionAttemptStateV1 {
            logical_attempt: 3,
            interrupted: false,
            last_error: Some("repair exact finding".into()),
        },
    );
    state.dispositions.insert(
        "acceptance".into(),
        SubjectDisposition::AcceptedWithShadowFindings,
    );
    store
        .write_run_json(run_id, FIXED_STATE_PATH, &state)
        .unwrap();
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let mut checkpoint = archon_workflow::WorkflowV2Checkpoint::default();
    checkpoint.mark_completed("host-call-1");
    checkpoint.mark_completed("host-call-2");
    v2.save_checkpoint(&checkpoint).unwrap();

    let status = super::workflow_decompose_status::render(&store, run_id)
        .unwrap()
        .unwrap();

    assert!(
        status.contains("run_kind: fixed_decomposition_v1"),
        "{status}"
    );
    assert!(
        status.contains("template_version: fixed-decomposition-v1"),
        "{status}"
    );
    assert!(status.contains("phase: bodies"), "{status}");
    assert!(status.contains("TASK-X-010 attempt=3"), "{status}");
    assert!(
        status.contains("acceptance=accepted_with_shadow_findings"),
        "{status}"
    );
    assert!(status.contains("resume_eligible_calls: 2"), "{status}");
    assert!(
        status.contains(&log.to_string_lossy().to_string()),
        "{status}"
    );
}

#[test]
fn legacy_status_extension_is_absent_without_fixed_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    std::fs::create_dir_all(store.run_dir("wf-legacy")).unwrap();
    assert!(
        super::workflow_decompose_status::render(&store, "wf-legacy")
            .unwrap()
            .is_none()
    );
}
