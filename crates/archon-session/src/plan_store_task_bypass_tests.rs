use cozo::DbInstance;

use super::*;

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", "").expect("in-memory db")
}

fn claimed_generation() -> (PlanDocument, PersistedPlanTask) {
    let mut plan = PlanDocument::new("task-bypass-plan", "Task bypass plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps.push(PlanStep {
        task_id: Some("task-bypass-id".into()),
        required_evidence: vec![archon_completion::RequiredEvidenceKind::Tests],
        ..step(1, "canonical description", PlanStepStatus::Pending)
    });
    let task = PersistedPlanTask {
        task_id: "task-bypass-id".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "canonical description".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![archon_completion::RequiredEvidenceKind::Tests],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    (plan, task)
}

fn persist_approval(
    store: &PlanStore,
    authority: &PlanApprovalAuthority,
    session_id: &str,
    plan: &PlanDocument,
) {
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: plan.approval.clone().expect("approved plan approval"),
    };
    store
        .save_terminal_plan_with_approval(authority, session_id, plan, &record)
        .expect("persist approved canonical plan");
}
#[test]
fn public_task_snapshot_write_cannot_replace_claimed_task_metadata() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "task-snapshot-bypass";
    let (plan, task) = claimed_generation();
    let authority = store
        .bootstrap_approval_authority(session_id, [1; 32])
        .expect("authority");
    persist_approval(&store, &authority, session_id, &plan);
    store
        .save_plan_with_tasks(&authority, session_id, &plan, std::slice::from_ref(&task))
        .expect("seed claimed generation");
    let replacement = PersistedPlanTask {
        description: "forged description".into(),
        status: "Completed".into(),
        required_evidence: vec![],
        completion_evidence: vec![],
        ..task.clone()
    };

    let error = store
        .save_plan_task_with_step_status(session_id, &replacement, PlanStepStatus::Complete)
        .expect_err("public persistence must not authorize metadata replacement");
    assert!(error.to_string().contains("not publicly writable"));
    assert_eq!(store.load_plan_tasks(session_id).unwrap(), vec![task]);
    assert_eq!(
        store
            .load_plan(session_id, "task-bypass-plan")
            .unwrap()
            .unwrap()
            .to_json(),
        plan.to_json()
    );
}

#[test]
fn public_task_status_write_cannot_complete_without_evidence() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "task-status-bypass";
    let (plan, task) = claimed_generation();
    let authority = store
        .bootstrap_approval_authority(session_id, [1; 32])
        .expect("authority");
    persist_approval(&store, &authority, session_id, &plan);
    store
        .save_plan_with_tasks(&authority, session_id, &plan, std::slice::from_ref(&task))
        .expect("seed claimed generation");

    let error = store
        .update_plan_task_status(session_id, "task-bypass-id", "Completed")
        .expect_err("public persistence must not bypass evidence validation");
    assert!(error.to_string().contains("not publicly writable"));
    assert_eq!(store.load_plan_tasks(session_id).unwrap(), vec![task]);
    assert_eq!(
        store
            .load_plan(session_id, "task-bypass-plan")
            .unwrap()
            .unwrap()
            .to_json(),
        plan.to_json()
    );
}

fn step(number: u32, description: &str, status: PlanStepStatus) -> PlanStep {
    PlanStep {
        number,
        description: description.into(),
        affected_files: Vec::new(),
        status,
        blocked_by: Vec::new(),
        required_evidence: Vec::new(),
        task_id: None,
    }
}
