use super::*;
use archon_workflow::requirement_state::*;
#[test]
fn decisions_recompute_every_derived_record_and_discard_mangled_host_fields() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, pin) = seed_frozen_acceptance(&temp);
    let raw = serde_json::json!({"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","implements":["AC-X-001"],"source_sections":["0"],"status":"done","requirement_states":"mangled","proven_done":[42]}]});
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        GateMode::Observe,
        serde_json::to_vec(&raw).unwrap(),
    )
    .unwrap();
    let before: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    assert!(before.tasks[0].requirement_states.is_empty());
    assert!(before.tasks[0].proven_done.is_empty());
    assert!(before.tasks[0].status.is_none());
    let mut outcome = DecisionOutcome::default();
    outcome.decisions.insert(
        ("TASK-X-010".into(), "AC-X-001".into()),
        RequirementDecision::new(RequirementState::Undecided, "weak_test"),
    );
    let before_digest = prepared.lock.digest.clone();
    let prepared = prepared.with_decisions(&outcome, Vec::new()).unwrap();
    assert_ne!(prepared.lock.digest, before_digest);
    assert_eq!(
        prepared.lock.digest,
        content_digest(&prepared.skeleton_bytes)
    );
    assert_eq!(
        prepared.pin.skeleton_digest.as_deref(),
        Some(prepared.lock.digest.as_str())
    );
    assert_eq!(
        prepared.pin.skeleton_gate.as_ref(),
        Some(&prepared.lock.gate)
    );
    assert_eq!(prepared.result.skeleton_digest, prepared.lock.digest);
    assert_eq!(prepared.pin.acceptance_digest, pin.acceptance_digest);
    let after: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    assert_eq!(after.tasks[0].status.as_deref(), Some("kept"));
    assert_eq!(
        after.tasks[0].requirement_states["AC-X-001"].reason,
        "weak_test"
    );
    archon_workflow::task_skeleton::validate_skeleton(&after, &pin.acceptance_digest).unwrap();
}

#[test]
fn decisions_preserve_the_preparers_stamp_function() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    fn stamp(
        mode: FreezeGateMode,
        findings: &[GateFinding],
    ) -> archon_workflow::task_set_contract::FreezeGateStamp {
        let mut stamp = gate_stamp(mode, findings);
        stamp.evaluated_at = "fixed metadata".into();
        stamp
    }
    let candidate = serde_json::to_vec(&serde_json::json!({"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"]}]})).unwrap();
    let prepared = super::super::skeleton_freeze::prepare_with_diagnostics(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        GateMode::Observe,
        candidate,
        super::super::link_diagnostics::child,
        stamp,
    )
    .unwrap();
    let prepared = prepared
        .with_decisions(&DecisionOutcome::default(), Vec::new())
        .unwrap();
    assert_eq!(prepared.lock.gate.evaluated_at, "fixed metadata");
    assert_eq!(
        prepared.pin.skeleton_gate.as_ref().unwrap().evaluated_at,
        "fixed metadata"
    );
}

#[test]
fn skeleton_classification_deterministic_fault_is_listed_before_provider_construction() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _) = seed_frozen_acceptance(&temp);
    let candidate = serde_json::to_vec(&serde_json::json!({"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","implements":["AC-X-001"]}]})).unwrap();
    let prepared = prepare_skeleton_freeze_from_candidate(
        temp.path(),
        &tasks,
        &temp.path().join("prds/PRD-X.md"),
        GateMode::Observe,
        candidate,
    )
    .unwrap();
    let reason = prepared.classification().unwrap().error.clone().unwrap();
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = GateMode::Observe;
    let env = archon_core::env_vars::load_env_vars_from(&Default::default());
    let prepared = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(
            crate::command::workflow::workflow_freeze_cli::skeleton_proof::prove_and_decide(
                temp.path(),
                &config,
                &env,
                prepared,
                &crate::command::workflow_freeze_budget::FreezeResume::none(),
            ),
        )
        .unwrap();
    let skeleton: TaskSkeleton = serde_json::from_slice(&prepared.skeleton_bytes).unwrap();
    let decision = &skeleton.tasks[0].requirement_states["AC-X-001"];
    assert_eq!(
        decision.state,
        archon_workflow::requirement_state::RequirementState::Undecided
    );
    assert_eq!(decision.reason, "classification_unavailable");
    assert_eq!(decision.notes, [reason.to_string()]);
    assert_eq!(skeleton.tasks[0].status.as_deref(), Some("kept"));
}
