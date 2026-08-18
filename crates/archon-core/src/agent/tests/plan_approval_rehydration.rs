use std::sync::Arc;

use super::*;

#[test]
#[serial_test::serial(plan_task_global_state)]
fn plan_store_attachment_rehydrates_existing_materialized_tasks() {
    use archon_session::plan::{
        PersistedPlanTask, PlanApproval, PlanApprovalDecision, PlanApprovalRecord,
        PlanApprovalSource, PlanDocument, PlanStatus, PlanStep, PlanStepStatus,
    };

    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = archon_session::plan::PlanStore::new(&db).unwrap();
    let session_id = format!("noninteractive-rehydrate-{}", uuid::Uuid::new_v4());
    let task_id = format!("rehydrate-task-{}", uuid::Uuid::new_v4());
    let approval = PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: "2026-08-16T00:00:00Z".into(),
        user_edited: false,
    };
    let mut plan = PlanDocument::new("rehydrate-plan", "Rehydrate plan");
    plan.status = PlanStatus::Approved;
    plan.approval = Some(approval.clone());
    plan.steps = vec![PlanStep {
        number: 1,
        description: "restore durable task".into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
        task_id: Some(task_id.clone()),
    }];
    let durable_task = PersistedPlanTask {
        task_id: task_id.clone(),
        plan_id: plan.id.clone(),
        plan_step: 1,
        description: "restore durable task".into(),
        status: "Pending".into(),
        blocked_by: vec![],
        required_evidence: vec![],
        completion_evidence: vec![],
        updated_at: "2026-08-16T00:00:00Z".into(),
    };
    let authority = store
        .bootstrap_approval_authority_for_test(&session_id)
        .unwrap();
    store
        .save_terminal_plan_with_approval_and_tasks(
            &authority,
            &session_id,
            &plan,
            &PlanApprovalRecord {
                plan_id: plan.id.clone(),
                session_id: session_id.clone(),
                approval,
            },
            &[durable_task],
        )
        .unwrap();

    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        AgentConfig {
            session_id: session_id.clone(),
            ..AgentConfig::default()
        },
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );

    agent.set_plan_store(store, authority).unwrap();

    let visible = archon_tools::task_manager::TASK_MANAGER
        .get_task(&task_id)
        .expect("durable plan task must be rehydrated");
    assert_eq!(visible.description, "restore durable task");
    assert_eq!(visible.metadata.unwrap().session_id, session_id);
}

#[test]
fn plan_store_attachment_rejects_corrupt_durable_task_state() {
    use archon_session::plan::{
        PersistedPlanTask, PlanDocument, PlanStatus, PlanStep, PlanStepStatus,
    };

    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let store = archon_session::plan::PlanStore::new(&db).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id: format!("corrupt-rehydrate-{}", uuid::Uuid::new_v4()),
        ..AgentConfig::default()
    };
    let session_id = config.session_id.clone();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    let plan_id = "corrupt-plan";
    let task_id = format!("corrupt-{}", uuid::Uuid::new_v4());
    let mut plan = PlanDocument::new(plan_id, "Corrupt plan");
    plan.status = PlanStatus::Approved;
    plan.steps = vec![PlanStep {
        number: 1,
        description: "corrupt durable task".into(),
        affected_files: vec![],
        status: PlanStepStatus::Pending,
        blocked_by: vec![],
        required_evidence: vec![],
        task_id: Some(task_id.clone()),
    }];
    store.save_plan(&session_id, &plan).unwrap();
    store
        .save_plan_task_fixture(
            &session_id,
            &PersistedPlanTask {
                task_id,
                plan_id: plan_id.into(),
                plan_step: 1,
                description: "corrupt durable task".into(),
                status: "Corrupt".into(),
                blocked_by: vec![],
                required_evidence: vec![],
                completion_evidence: vec![],
                updated_at: "2026-08-15T00:00:00Z".into(),
            },
        )
        .unwrap();

    let authority = store
        .bootstrap_approval_authority_for_test(&session_id)
        .unwrap();
    let error = agent.set_plan_store(store, authority).unwrap_err();
    assert!(error.contains("unknown persisted task status: Corrupt"));
    assert!(agent.plan_store.is_none());
}
