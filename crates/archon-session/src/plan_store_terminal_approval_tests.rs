use cozo::DbInstance;

use super::*;

fn test_authority(store: &PlanStore, session_id: &str) -> PlanApprovalAuthority {
    store
        .bootstrap_approval_authority_for_test(session_id)
        .expect("test authority")
}

#[test]
fn rejected_approval_cannot_persist_a_terminal_task_generation() {
    let db = DbInstance::new("mem", "", "").expect("in-memory db");
    let store = PlanStore::new(&db).expect("init");
    let session_id = "rejected-terminal-tasks";
    let mut plan = PlanDocument::new("rejected-terminal-plan", "Rejected terminal plan");
    plan.status = PlanStatus::Approved;
    plan.steps.push(PlanStep {
        number: 1,
        description: "must not materialize".into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
        task_id: Some("rejected-terminal-task".into()),
    });
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Reject {
                reason: "not approved".into(),
            },
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-16T00:00:00Z".into(),
            user_edited: false,
        },
    };
    plan.approval = Some(record.approval.clone());
    let task = PersistedPlanTask {
        task_id: "rejected-terminal-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "must not materialize".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-16T00:00:00Z".into(),
    };

    let error = store
        .save_terminal_plan_with_approval_and_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            &record,
            &[task],
        )
        .expect_err("rejected approvals must not materialize executable tasks");

    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("approving plan decision"));
    assert!(store.load_plan(session_id, &plan.id).unwrap().is_none());
    assert!(
        store
            .load_approval_events(session_id, &plan.id)
            .unwrap()
            .is_empty()
    );
    assert!(store.load_plan_tasks(session_id).unwrap().is_empty());
    assert!(
        !store
            .materialization_claim_exists_for_test(session_id, &plan.id)
            .unwrap()
    );
}

#[test]
fn rejected_approval_remains_auditable_without_tasks() {
    let db = DbInstance::new("mem", "", "").expect("in-memory db");
    let store = PlanStore::new(&db).expect("init");
    let session_id = "rejected-terminal-audit";
    let mut plan = PlanDocument::new("rejected-audit-plan", "Rejected audit plan");
    plan.status = PlanStatus::Approved;
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Reject {
                reason: "not approved".into(),
            },
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-16T00:00:00Z".into(),
            user_edited: false,
        },
    };
    plan.approval = Some(record.approval.clone());

    store
        .save_terminal_plan_with_approval(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            &record,
        )
        .expect("rejected decision remains an audit artifact");

    assert_eq!(
        store
            .load_plan(session_id, &plan.id)
            .unwrap()
            .expect("rejected audit plan")
            .to_json(),
        plan.to_json()
    );
    assert_eq!(
        store
            .load_approval_events(session_id, &record.plan_id)
            .unwrap(),
        vec![record]
    );
    assert!(store.load_plan_tasks(session_id).unwrap().is_empty());
}
