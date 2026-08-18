use std::collections::BTreeSet;

use super::*;
use archon_completion::{RequiredEvidence, RequiredEvidenceKind, RequiredEvidenceStatus};
use archon_session::plan::{
    PersistedPlanTask, PlanDocument, PlanReconciliationStatus, PlanStatus, PlanStep,
    PlanStepReconciliation, reconcile_durable_plan,
};
use archon_tools::task_manager::{TaskInfo, TaskStatus};
use chrono::Utc;

#[test]
fn reconciliation_detects_completed_omitted_deviated_and_unplanned_work() {
    let plan = approved_plan();
    let tasks = vec![
        task("task-1", TaskStatus::Completed),
        task("task-2", TaskStatus::Pending),
    ];
    let evidence = PlanExecutionEvidence {
        touched_files: BTreeSet::from(["src/a.rs".into(), "src/extra.rs".into()]),
        completion: vec![passed(RequiredEvidenceKind::Tests)],
    };

    let mut persisted = durable_tasks(&plan, &tasks);
    persisted[0].status = "Completed".into();
    let mut observed_plan = plan.clone();
    observed_plan.execution_evidence.touched_files = evidence.touched_files.clone();
    let reconciliation = reconcile_durable_plan(&observed_plan, &persisted);

    assert_status(
        &reconciliation,
        Some(1),
        PlanReconciliationStatus::Completed,
    );
    assert_status(&reconciliation, Some(2), PlanReconciliationStatus::Omitted);
    assert!(reconciliation.iter().any(|entry| {
        entry.step.is_none()
            && entry.status == PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains("src/extra.rs")
    }));
}

#[test]
fn reconciliation_requires_exact_normalized_worktree_path_matches() {
    let mut plan = approved_plan();
    plan.execution_evidence
        .touched_files
        .insert("unplanned/src/a.rs".into());

    let reconciliation = reconcile_durable_plan(&plan, &durable_tasks(&plan, &[]));

    assert!(reconciliation.iter().any(|entry| {
        entry.step.is_none()
            && entry.status == PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains("unplanned/src/a.rs")
    }));
}

#[test]
fn completed_plan_step_without_durable_required_evidence_is_deviated() {
    let plan = approved_plan();
    let tasks = vec![
        task("task-1", TaskStatus::Completed),
        task("task-2", TaskStatus::Pending),
    ];

    let mut persisted = durable_tasks(&plan, &tasks);
    persisted[0].completion_evidence.clear();
    let reconciliation = reconcile_durable_plan(&plan, &persisted);

    assert_status(&reconciliation, Some(1), PlanReconciliationStatus::Deviated);
}

#[test]
fn durable_completed_task_reconciles_as_completed() {
    let plan = approved_plan();
    let tasks = vec![
        task("task-1", TaskStatus::Completed),
        task("task-2", TaskStatus::Pending),
    ];

    let persisted = durable_tasks(&plan, &tasks);
    let reconciliation = reconcile_durable_plan(&plan, &persisted);

    assert_status(
        &reconciliation,
        Some(1),
        PlanReconciliationStatus::Completed,
    );
}

#[test]
fn completed_plan_step_without_a_completed_canonical_task_is_omitted() {
    let mut plan = approved_plan();
    plan.steps[0].status = archon_session::plan::PlanStepStatus::Complete;
    let tasks = vec![
        task("task-1", TaskStatus::Pending),
        task("task-2", TaskStatus::Pending),
    ];
    let persisted = durable_tasks(&plan, &tasks);
    let reconciliation = reconcile_durable_plan(&plan, &persisted);

    assert_status(&reconciliation, Some(1), PlanReconciliationStatus::Omitted);
}

#[tokio::test]
async fn materialized_lifecycle_reconciliation_survives_context_clear() {
    let mut agent = cleared_materialized_reconciliation_agent().await;

    assert!(agent.conversation_state().messages.is_empty());
    assert!(agent.plan_completion_block().is_some());
    let stored = agent
        .plan_store
        .as_ref()
        .unwrap()
        .load_latest_plan(&agent.config.session_id)
        .unwrap()
        .unwrap();
    assert_status(
        &stored.reconciliation,
        Some(1),
        PlanReconciliationStatus::Completed,
    );
    assert_status(
        &stored.reconciliation,
        Some(2),
        PlanReconciliationStatus::Omitted,
    );
    assert_status(
        &stored.reconciliation,
        Some(3),
        PlanReconciliationStatus::Deviated,
    );
    assert!(stored.reconciliation.iter().any(|entry| {
        entry.step.is_none() && entry.status == PlanReconciliationStatus::UnplannedExtra
    }));
}

async fn cleared_materialized_reconciliation_agent() -> Agent {
    use archon_tools::plan_tasks::{materialize_plan_tasks, test_plan_approval_authority};
    use archon_tools::task_manager::TaskManager;

    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let plan_store = archon_session::plan::PlanStore::new(&db).unwrap();
    let session_id = format!("reconciliation-clear-{}", uuid::Uuid::new_v4());
    let mut plan = lifecycle_plan(&session_id);
    let manager = TaskManager::new();
    let authority = test_plan_approval_authority(&plan_store, &session_id);
    let task_ids =
        materialize_plan_tasks(&manager, &plan_store, &authority, &session_id, &mut plan).unwrap();
    set_materialized_task_outcomes(&plan_store, &authority, &session_id, &manager, &task_ids);

    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = session_id;
    agent.plan_store = Some(plan_store);
    agent.plan_approval_authority = Some(authority);
    agent.record_plan_file_mutation("src/a.rs").unwrap();
    agent.record_plan_file_mutation("src/extra.rs").unwrap();
    agent.state.add_user_message("transient conversation state");
    agent.clear_conversation().await;
    agent
}

fn set_materialized_task_outcomes(
    store: &archon_session::plan::PlanStore,
    authority: &archon_session::plan::PlanApprovalAuthority,
    session_id: &str,
    manager: &archon_tools::task_manager::TaskManager,
    task_ids: &[String],
) {
    let evidence = store
        .record_authoritative_test_execution(
            authority,
            session_id,
            "reconciliation-tool",
            0,
            "cargo test fixture",
            "test result: ok. 1 passed; 0 failed",
            0,
        )
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&task_ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(
            &task_ids[0],
            TaskStatus::Completed,
            &evidence.run_id,
            &[evidence.evidence_id],
        )
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&task_ids[2], TaskStatus::Failed, "", &[])
        .unwrap();
}

#[tokio::test]
async fn reconciliation_survives_context_clear_and_blocks_completion_claim() {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let session_id = format!("reconciliation-clear-{}", uuid::Uuid::new_v4());
    let mut plan = approved_plan();
    plan.session_id = Some(session_id.clone());
    plan_store.save_plan(&session_id, &plan).unwrap();
    let mut agent = super::super::tests::test_agent();
    agent.config.session_id = session_id.clone();
    agent.plan_store = Some(plan_store);
    agent.record_plan_file_mutation("src/a.rs").unwrap();
    agent.record_plan_file_mutation("src/extra.rs").unwrap();
    agent.state.add_user_message("transient conversation state");

    agent.clear_conversation().await;

    assert!(agent.conversation_state().messages.is_empty());
    assert_eq!(agent.plan_execution_evidence.touched_files.len(), 2);
    let block = agent.plan_completion_block();
    assert!(block.is_some());
    let stored = agent
        .plan_store
        .as_ref()
        .unwrap()
        .load_latest_plan(&session_id)
        .unwrap()
        .unwrap();
    assert!(
        stored
            .reconciliation
            .iter()
            .any(|entry| entry.status == PlanReconciliationStatus::Omitted)
    );
    assert!(
        stored
            .reconciliation
            .iter()
            .any(|entry| entry.status == PlanReconciliationStatus::UnplannedExtra)
    );
}

fn lifecycle_plan(session_id: &str) -> PlanDocument {
    let mut plan = PlanDocument::new(
        &format!("lifecycle-plan-{}", uuid::Uuid::new_v4()),
        "Materialized lifecycle plan",
    );
    plan.session_id = Some(session_id.to_string());
    plan.status = PlanStatus::Approved;
    plan.approval = Some(archon_session::plan::PlanApproval {
        decision: archon_session::plan::PlanApprovalDecision::Approve,
        source: archon_session::plan::PlanApprovalSource::NonInteractive,
        decided_at: chrono::Utc::now().to_rfc3339(),
        user_edited: false,
    });
    plan.steps = vec![
        step(1, "src/a.rs", None, vec![RequiredEvidenceKind::Tests]),
        step(2, "src/b.rs", None, Vec::new()),
        step(3, "src/c.rs", None, Vec::new()),
    ];
    for step in &mut plan.steps {
        step.status = archon_session::plan::PlanStepStatus::Pending;
    }
    plan
}

fn assert_status(
    reconciliation: &[PlanStepReconciliation],
    step: Option<u32>,
    status: PlanReconciliationStatus,
) {
    assert!(
        reconciliation
            .iter()
            .any(|entry| entry.step == step && entry.status == status)
    );
}

pub(super) fn approved_plan() -> PlanDocument {
    PlanDocument {
        id: "plan-1".into(),
        title: "Approved plan".into(),
        steps: vec![
            step(
                1,
                "src/a.rs",
                Some("task-1"),
                vec![RequiredEvidenceKind::Tests],
            ),
            step(2, "src/b.rs", Some("task-2"), Vec::new()),
        ],
        risks: Vec::new(),
        questions: Vec::new(),
        status: PlanStatus::Executing,
        approval: None,
        reconciliation: Vec::new(),
        execution_evidence: archon_session::plan::PlanExecutionEvidence::default(),
        session_id: None,
        branch: None,
        commits: Vec::new(),
        user_edited: false,
    }
}

fn step(
    number: u32,
    file: &str,
    task_id: Option<&str>,
    required_evidence: Vec<RequiredEvidenceKind>,
) -> PlanStep {
    PlanStep {
        number,
        description: format!("Change {file}"),
        affected_files: vec![file.into()],
        status: archon_session::plan::PlanStepStatus::Complete,
        blocked_by: Vec::new(),
        required_evidence,
        task_id: task_id.map(str::to_owned),
    }
}

fn durable_tasks(plan: &PlanDocument, tasks: &[TaskInfo]) -> Vec<PersistedPlanTask> {
    plan.steps
        .iter()
        .filter_map(|step| {
            let task_id = step.task_id.as_ref()?;
            let task = tasks.iter().find(|task| task.id == *task_id)?;
            Some(PersistedPlanTask {
                task_id: task.id.clone(),
                plan_id: plan.id.clone(),
                plan_step: step.number,
                description: step.description.clone(),
                status: match task.status {
                    TaskStatus::Completed => "Completed",
                    TaskStatus::Failed => "Failed",
                    TaskStatus::Stopped => "Stopped",
                    TaskStatus::Running => "Running",
                    _ => "Pending",
                }
                .into(),
                blocked_by: Vec::new(),
                required_evidence: step.required_evidence.clone(),
                completion_evidence: if task.status == TaskStatus::Completed {
                    step.required_evidence.iter().copied().map(passed).collect()
                } else {
                    Vec::new()
                },
                updated_at: Utc::now().to_rfc3339(),
            })
        })
        .collect()
}

fn task(id: &str, status: TaskStatus) -> TaskInfo {
    TaskInfo {
        id: id.into(),
        description: id.into(),
        status,
        created_at: Utc::now(),
        completed_at: None,
        output: String::new(),
        cost: 0.0,
        agent_id: None,
        board_item_id: None,
        metadata: None,
    }
}

fn passed(kind: RequiredEvidenceKind) -> RequiredEvidence {
    RequiredEvidence {
        kind,
        status: RequiredEvidenceStatus::Passed,
        sequence: 0,
        evidence_id: Some("evidence".into()),
        run_id: Some("run".into()),
    }
}
