use cozo::DbInstance;

use super::*;

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", "").expect("in-memory db")
}

fn test_authority(store: &PlanStore, session_id: &str) -> PlanApprovalAuthority {
    store
        .bootstrap_approval_authority_for_test(session_id)
        .expect("test authority")
}

fn claimed_generation() -> (PlanDocument, PersistedPlanTask) {
    let mut plan = PlanDocument::new("claimed-plan", "Claimed plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-15T00:00:00Z".into(),
        user_edited: false,
    });
    plan.steps.push(PlanStep {
        task_id: Some("claimed-task".into()),
        ..step(1, "canonical", PlanStepStatus::Pending)
    });
    let task = PersistedPlanTask {
        task_id: "claimed-task".into(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "canonical".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-15T00:00:00Z".into(),
    };
    (plan, task)
}

fn persist_approval(store: &PlanStore, session_id: &str, plan: &PlanDocument) {
    let record = PlanApprovalRecord {
        plan_id: plan.id.clone(),
        session_id: session_id.into(),
        approval: plan.approval.clone().expect("approved plan approval"),
    };
    store
        .save_terminal_plan_with_approval(
            &test_authority(store, session_id),
            session_id,
            plan,
            &record,
        )
        .expect("persist approved canonical plan");
}
#[test]
fn claimed_plan_rejects_legacy_step_status_update() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "claimed-legacy-step-status";
    let (plan, task) = claimed_generation();
    persist_approval(&store, session_id, &plan);
    store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &plan,
            std::slice::from_ref(&task),
        )
        .expect("seed claimed generation");

    let error = store
        .update_step_status(session_id, &plan.id, 1, PlanStepStatus::Complete)
        .expect_err("legacy step-only update would break rehydration");
    assert!(error.to_string().contains("materialized canonical plan"));
    assert_eq!(
        store
            .load_plan(session_id, &plan.id)
            .unwrap()
            .unwrap()
            .steps[0]
            .status,
        PlanStepStatus::Pending
    );
    assert_eq!(store.load_plan_tasks(session_id).unwrap(), vec![task]);
}

#[test]
fn claimed_plan_rejects_legacy_terminal_approval_overwrite() {
    let db = test_db();
    let store = PlanStore::new(&db).expect("init");
    let session_id = "claimed-terminal-approval";
    let (canonical, task) = claimed_generation();
    persist_approval(&store, session_id, &canonical);
    store
        .save_plan_with_tasks(
            &test_authority(&store, session_id),
            session_id,
            &canonical,
            &[task],
        )
        .expect("seed claimed generation");
    let mut replacement = canonical.clone();
    replacement.title = "Must not overwrite".into();
    let record = PlanApprovalRecord {
        plan_id: replacement.id.clone(),
        session_id: session_id.into(),
        approval: PlanApproval {
            decision: PlanApprovalDecision::Approve,
            source: PlanApprovalSource::Interactive,
            decided_at: "2026-08-15T00:00:01Z".into(),
            user_edited: false,
        },
    };
    replacement.approval = Some(record.approval.clone());

    let error = store
        .save_terminal_plan_with_approval(
            &test_authority(&store, session_id),
            session_id,
            &replacement,
            &record,
        )
        .expect_err("claimed plan must reject legacy terminal overwrite");
    assert!(error.to_string().contains("materialized canonical plan"));
    assert_eq!(
        store
            .load_plan(session_id, &canonical.id)
            .unwrap()
            .unwrap()
            .to_json(),
        canonical.to_json()
    );
    assert_eq!(
        store
            .load_approval_events(session_id, &canonical.id)
            .unwrap(),
        vec![PlanApprovalRecord {
            plan_id: canonical.id.clone(),
            session_id: session_id.into(),
            approval: canonical.approval.clone().expect("canonical approval"),
        }]
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
