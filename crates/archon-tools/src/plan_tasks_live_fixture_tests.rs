use super::*;
use crate::plan_tasks::test_plan_approval_authority;
use crate::task_list::TaskListTool;
use crate::task_manager::TASK_MANAGER;
use crate::tool::{Tool, ToolContext};
use archon_completion::models::{CompletionState, VerificationGateResult};
use archon_completion::{CompletionEvidence, EvidenceKind, EvidenceStatus};
use serde_json::json;

fn five_step_plan() -> PlanDocument {
    let mut plan = PlanDocument::new("plan-five", "Five-step approval plan");
    plan.status = PlanStatus::Approved;
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
                vec![archon_completion::RequiredEvidenceKind::Tests]
            } else {
                vec![]
            },
            task_id: None,
        })
        .collect();
    plan
}

fn test_store() -> (cozo::DbInstance, PlanStore) {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = PlanStore::new(&db).unwrap();
    (db, store)
}

#[tokio::test]
#[ignore = "Gate 5 live fixture; execute only in the dedicated smoke gate"]
async fn live_five_step_fixture_advances_first_three_and_leaves_four_blocked() {
    let manager = &TASK_MANAGER;
    let (db, store) = test_store();
    let mut plan = five_step_plan();
    let ids = materialize_plan_tasks(
        manager,
        &store,
        &test_plan_approval_authority(&store, "live"),
        "live",
        &mut plan,
    )
    .unwrap();
    let run_id = format!("live-fixture-{}", uuid::Uuid::new_v4());
    let evidence = CompletionEvidence {
        evidence_id: format!("live-evidence-{}", uuid::Uuid::new_v4()),
        run_id: run_id.clone(),
        evidence_kind: EvidenceKind::TestRun,
        producer: "verified-test-runner".into(),
        command_or_operation: Some("cargo test".into()),
        status: EvidenceStatus::Passed,
        exit_code: Some(0),
        input_hash: Some("input".into()),
        output_hash: Some("output".into()),
        stdout_summary: Some("passed".into()),
        stderr_summary: None,
        artifact_ids: vec![],
        provenance_record_id: "durable-live-fixture".into(),
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    archon_completion::store::insert_gate_result(
        &db,
        &VerificationGateResult {
            gate_id: "live-fixture-gate".into(),
            gate_name: "test-evidence".into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: "live fixture test result verified".into(),
            provenance_record_id: evidence.provenance_record_id.clone(),
        },
        &run_id,
    )
    .unwrap();
    archon_completion::store::insert_completion_evidence(&db, &evidence).unwrap();
    for (index, id) in ids[..3].iter().enumerate() {
        manager
            .set_status_checked_with_evidence_ids(id, TaskStatus::Running, "", &[])
            .unwrap();
        let ids = if index == 2 {
            vec![evidence.evidence_id.clone()]
        } else {
            vec![]
        };
        manager
            .set_status_checked_with_evidence_ids(
                id,
                TaskStatus::Completed,
                if index == 2 { &run_id } else { "" },
                &ids,
            )
            .unwrap();
    }
    manager
        .set_status_checked_with_evidence_ids(&ids[3], TaskStatus::Running, "", &[])
        .unwrap();
    assert_eq!(
        manager.get_task(&ids[3]).unwrap().status,
        TaskStatus::Running
    );
    assert!(matches!(
        manager.set_status_checked_with_evidence_ids(&ids[4], TaskStatus::Running, "", &[]),
        Err(crate::task_manager::TaskTransitionError::BlockedDependency { .. })
    ));
    let tool = TaskListTool;
    let result = tool.execute(json!({}), &ToolContext::default()).await;
    assert!(!result.is_error, "{result:?}");
    let tasks: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let tasks = tasks.as_array().unwrap();
    assert_eq!(tasks.len(), 5);
    let fourth = tasks.iter().find(|task| task["id"] == ids[3]).unwrap();
    assert_eq!(fourth["plan_progress"], json!({"completed": 3, "total": 5}));
    assert_eq!(fourth["blocked_by"], json!([ids[2]]));
}
