use super::*;
use archon_completion::RequiredEvidenceKind;

pub(super) fn five_step_plan() -> PlanDocument {
    let mut plan = PlanDocument::new("plan-five", "Five-step approval plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps = (1..=5)
        .map(|number| archon_session::plan::PlanStep {
            number,
            description: format!("step {number}"),
            affected_files: vec![],
            status: PlanStepStatus::Pending,
            blocked_by: if number == 1 {
                vec![]
            } else {
                vec![number - 1]
            },
            required_evidence: if number == 3 {
                vec![RequiredEvidenceKind::Tests]
            } else {
                vec![]
            },
            task_id: None,
        })
        .collect();
    plan
}

pub(super) fn test_store() -> PlanStore {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    PlanStore::new(&db).unwrap()
}

pub(super) fn approve_plan(_store: &PlanStore, _session_id: &str, plan: &mut PlanDocument) {
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
}

pub(super) fn plan_with_preassigned_task(
    plan_id: &str,
    task_id: &str,
    description: &str,
) -> PlanDocument {
    let mut plan = PlanDocument::new(plan_id, description);
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps = vec![archon_session::plan::PlanStep {
        number: 1,
        description: description.into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
        task_id: Some(task_id.into()),
    }];
    plan
}
