use std::collections::BTreeMap;

use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, FixedRunIdentityV1, HostCommandRequest,
    SubjectDisposition, WorkflowRunKind, WorkflowStore, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2Status,
};

use super::workflow_decompose_state::{
    FIXED_STATE_PATH, FixedCallProjectionKind, project_fixed_call,
};

fn seed_state(store: &WorkflowStore, run_id: &str, log_path: &std::path::Path) {
    let task_root = log_path.parent().unwrap();
    std::fs::create_dir_all(task_root).unwrap();
    let task_root = task_root.canonicalize().unwrap();
    let project_root = task_root.parent().unwrap();
    let state = FixedDecompositionStateV1 {
        schema_version: 1,
        run_kind: WorkflowRunKind::FixedDecompositionV1,
        identity: FixedRunIdentityV1 {
            template_version: "fixed-decomposition-v1".into(),
            starting_binary_revision: "rev".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: project_root.to_string_lossy().into_owned(),
            prd_identity: project_root.join("PRD.md").to_string_lossy().into_owned(),
            task_root_identity: task_root.to_string_lossy().into_owned(),
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
fn fixed_body_subject_is_digest_only_in_decomposition_log() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-opaque-subject";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    seed_state(&store, run_id, &log);
    let mut record = host_record(run_id);
    record.call.options.host_command =
        Some(HostCommandRequest::new("land-task-body", Some("candidate".into())).unwrap());
    let mut outcome: archon_workflow::HostCommandResult =
        serde_json::from_value(record.result.data.clone()).unwrap();
    outcome.publication_receipt.as_mut().unwrap().command_id = "land-task-body".into();
    outcome.subjects = vec![archon_workflow::HostCommandSubject {
        task_id: "TASK-534543524554-010".into(),
        file_name: "task.md".into(),
    }];
    record.result.data = serde_json::to_value(outcome).unwrap();

    project_fixed_call(&store, run_id, &record, FixedCallProjectionKind::Executed).unwrap();

    let line = std::fs::read_to_string(log).unwrap();
    assert!(!line.contains("TASK-534543524554-010"), "{line}");
    assert!(line.contains("subject_digest="), "{line}");
}

#[test]
fn fixed_call_projection_refuses_forged_persisted_log_destination() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-forged-log";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let canonical_log = temp.path().join("tasks/.decompose.log");
    seed_state(&store, run_id, &canonical_log);
    let mut state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap(),
    )
    .unwrap();
    let forged = temp.path().join("outside.log");
    std::fs::write(&forged, "sentinel\n").unwrap();
    state.log_path = forged.to_string_lossy().into_owned();
    store
        .write_run_json(run_id, FIXED_STATE_PATH, &state)
        .unwrap();

    let error = project_fixed_call(
        &store,
        run_id,
        &host_record(run_id),
        FixedCallProjectionKind::Executed,
    )
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("differs from canonical task-root log")
    );
    assert_eq!(std::fs::read_to_string(forged).unwrap(), "sentinel\n");
}

#[cfg(unix)]
#[test]
fn fixed_call_projection_refuses_symlinked_canonical_log() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-symlink-log";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let canonical_log = temp.path().join("tasks/.decompose.log");
    seed_state(&store, run_id, &canonical_log);
    let outside = temp.path().join("outside.log");
    std::fs::write(&outside, "sentinel\n").unwrap();
    std::os::unix::fs::symlink(&outside, &canonical_log).unwrap();

    let error = project_fixed_call(
        &store,
        run_id,
        &host_record(run_id),
        FixedCallProjectionKind::Executed,
    )
    .unwrap_err();

    assert!(error.to_string().contains("not a regular non-symlink file"));
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "sentinel\n");
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

#[test]
fn fixed_status_renders_sanitized_route_call_shadow_and_active_detail() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-detailed-status";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    seed_state(&store, run_id, &log);
    store
        .write_run_json(
            run_id,
            crate::command::workflow_decompose::FIXED_PROVIDER_ROUTE_PATH,
            &crate::command::workflow_provider_route::TrustedProviderRouteSnapshot {
                origin: "trusted_config".into(),
                endpoint: Some("https://private.invalid/messages".into()),
                endpoint_digest: Some("route-digest".into()),
            },
        )
        .unwrap();
    let v2 = archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let mut host = host_record(run_id);
    let mut outcome: archon_workflow::HostCommandResult =
        serde_json::from_value(host.result.data.clone()).unwrap();
    outcome.gate_envelope.as_mut().unwrap().policy_findings = vec![
        archon_workflow::GatePolicyFinding {
            text: "first finding".into(),
            subject: "acceptance".into(),
            source_path: None,
            remediation_scope: archon_workflow::RemediationScope::CandidateArtifact,
        },
        archon_workflow::GatePolicyFinding {
            text: "second finding".into(),
            subject: "acceptance".into(),
            source_path: None,
            remediation_scope: archon_workflow::RemediationScope::CandidateArtifact,
        },
    ];
    host.result.data = serde_json::to_value(outcome).unwrap();
    v2.save_call_record(&host).unwrap();

    let mut running = WorkflowV2Result::default();
    running.status = archon_workflow::WorkflowV2Status::Running;
    running.summary = "author in flight".into();
    let active = WorkflowV2CallRecord::new(
        run_id,
        WorkflowV2HostCall {
            id: "body-TASK-X-010-author-2".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        2,
        "active-input".into(),
        running,
        Vec::new(),
    );
    v2.save_call_record(&active).unwrap();

    let status = super::workflow_decompose_status::render(&store, run_id)
        .unwrap()
        .unwrap();

    assert!(
        status.contains("provider_route: trusted_config digest=route-digest"),
        "{status}"
    );
    assert!(
        status.contains("calls: total=2 authors=1 bodies=1 host_commands=1"),
        "{status}"
    );
    assert!(status.contains("shadow_findings: 2"), "{status}");
    assert!(
        status.contains("active_call: body-TASK-X-010-author-2 method=agent attempt=2"),
        "{status}"
    );
    assert!(!status.contains("private.invalid"), "{status}");
    assert!(!status.contains("first finding"), "{status}");
}

#[cfg(unix)]
#[test]
fn fixed_log_append_opens_the_actual_descriptor_with_nofollow() {
    let writer = include_str!("workflow_decompose_log.rs");
    let projection = include_str!("workflow_decompose_state.rs");
    assert!(
        writer.contains("custom_flags(libc::O_NOFOLLOW)"),
        "{writer}"
    );
    assert!(writer.contains("file.metadata()"), "{writer}");
    assert!(
        projection.contains("append_nofollow_line(path"),
        "{projection}"
    );
    assert!(!projection.contains("OpenOptions::new"), "{projection}");
}

#[test]
fn failed_host_command_projects_the_phase_of_the_command_that_failed() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-failed-host";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    seed_state(&store, run_id, &log);

    // A host command that fails outright carries `{error: ...}` where a
    // HostCommandResult would be. The phase still belongs to the command that
    // failed - an operator reading the log must see where the run stopped.
    let mut record = host_record(run_id);
    record.status = WorkflowV2Status::Failed;
    record.result.status = WorkflowV2Status::Failed;
    record.result.data = serde_json::json!({ "error": "host command failed" });

    project_fixed_call(&store, run_id, &record, FixedCallProjectionKind::Executed).unwrap();

    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join(FIXED_STATE_PATH)).unwrap(),
    )
    .unwrap();
    assert_eq!(state.phase, DecompositionPhase::Acceptance);
    let line = std::fs::read_to_string(log).unwrap();
    assert!(line.contains("phase=acceptance"), "{line}");
    assert!(line.contains("disposition=failed"), "{line}");
}
