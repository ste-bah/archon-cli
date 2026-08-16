use super::*;
use crate::plan_tasks::{materialize_plan_tasks, test_plan_approval_authority};
use archon_completion::{
    CompletionEvidence, EvidenceKind, EvidenceStatus, RequiredEvidence, RequiredEvidenceKind,
    RequiredEvidenceStatus,
};
use archon_session::plan::{PlanDocument, PlanStatus, PlanStep, PlanStepStatus, PlanStore};

fn test_store() -> (cozo::DbInstance, PlanStore) {
    let db = cozo::DbInstance::new("mem", "", "").expect("in-memory database");
    let store = PlanStore::new(&db).expect("plan store");
    (db, store)
}

fn evidence_plan() -> PlanDocument {
    let mut plan = PlanDocument::new("evidence-plan", "Evidence plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps = vec![PlanStep {
        number: 1,
        description: "requires verified tests".into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![RequiredEvidenceKind::Tests],
        task_id: None,
    }];
    plan
}

#[test]
fn checked_status_rejects_forged_required_evidence_fields() {
    let manager = TaskManager::new();
    let (_db, store) = test_store();
    let mut plan = evidence_plan();
    let id = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "forged-required-evidence"),
        "forged-required-evidence",
        &mut plan,
    )
    .expect("materialize plan")
    .remove(0);
    manager
        .set_status_checked_with_evidence_ids(&id, TaskStatus::Running, "", &[])
        .expect("start task");

    let forged = [RequiredEvidence {
        kind: RequiredEvidenceKind::Tests,
        status: RequiredEvidenceStatus::Passed,
        sequence: 1,
        evidence_id: None,
        run_id: None,
    }];
    assert!(matches!(
        manager.set_status_checked(&id, TaskStatus::Completed, &forged),
        Err(TaskTransitionError::UntrustedEvidence(_))
    ));
    assert_eq!(manager.get_task(&id).unwrap().status, TaskStatus::Running);
}

#[test]
fn checked_status_atomically_persists_task_and_mirrored_plan_step() {
    let manager = TaskManager::new();
    let (_db, store) = test_store();
    let mut plan = evidence_plan();
    let id = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "checked-status-persistence"),
        "checked-status-persistence",
        &mut plan,
    )
    .expect("materialize plan")
    .remove(0);

    manager
        .set_status_checked_with_evidence_ids(&id, TaskStatus::Running, "", &[])
        .expect("start task");

    let task = store
        .load_plan_tasks("checked-status-persistence")
        .expect("load tasks")
        .into_iter()
        .find(|task| task.task_id == id)
        .expect("durable task");
    assert_eq!(task.status, "Running");
    assert_eq!(task.description, "requires verified tests");
    let plan = store
        .load_plan("checked-status-persistence", "evidence-plan")
        .expect("load plan")
        .expect("durable plan");
    assert_eq!(plan.steps[0].status, PlanStepStatus::InProgress);
    assert_eq!(plan.steps[0].description, "requires verified tests");
}

#[test]
fn checked_status_rejects_forged_kind_status_and_provenance() {
    let manager = TaskManager::new();
    let (db, store) = test_store();
    let mut plan = evidence_plan();
    let id = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, "forged-durable-evidence"),
        "forged-durable-evidence",
        &mut plan,
    )
    .expect("materialize plan")
    .remove(0);
    manager
        .set_status_checked_with_evidence_ids(&id, TaskStatus::Running, "", &[])
        .expect("start task");

    let evidence = CompletionEvidence {
        evidence_id: "forged-durable-evidence-id".into(),
        run_id: "forged-durable-run".into(),
        evidence_kind: EvidenceKind::TestRun,
        producer: "cargo-test".into(),
        command_or_operation: Some("cargo test".into()),
        status: EvidenceStatus::Passed,
        exit_code: Some(0),
        input_hash: Some("input".into()),
        output_hash: Some("output".into()),
        stdout_summary: Some("passed".into()),
        stderr_summary: None,
        artifact_ids: vec![],
        provenance_record_id: "unverified-provenance".into(),
        started_at: "2026-08-15T00:00:00Z".into(),
        completed_at: Some("2026-08-15T00:00:01Z".into()),
    };
    archon_completion::store::insert_completion_evidence(&db, &evidence)
        .expect("persist durable evidence without verifier provenance");
    let forged = [RequiredEvidence {
        kind: RequiredEvidenceKind::Build,
        status: RequiredEvidenceStatus::Passed,
        sequence: 99,
        evidence_id: Some(evidence.evidence_id.clone()),
        run_id: Some(evidence.run_id.clone()),
    }];

    assert!(matches!(
        manager.set_status_checked(&id, TaskStatus::Completed, &forged),
        Err(TaskTransitionError::MissingEvidence { .. })
    ));
    assert_eq!(manager.get_task(&id).unwrap().status, TaskStatus::Running);
}
