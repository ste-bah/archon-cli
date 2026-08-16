use std::sync::Arc;

use archon_permissions::mode::PermissionMode;
use archon_session::plan::PlanStatus;

use super::*;

async fn approved_plan_agent(
    plan_text: &str,
    session_id: String,
) -> (Agent, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        session_id,
        ..AgentConfig::default()
    };
    *config.permission_mode.lock().await = PermissionMode::Plan.to_string();
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        config,
        event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );
    agent.state.mode = AgentMode::Plan;
    agent.plan_mode_state.lock().await.previous_permission_mode = Some(PermissionMode::Default);
    let authority = plan_store
        .bootstrap_approval_authority_for_test(&agent.config.session_id)
        .unwrap();
    agent.set_plan_store(plan_store, authority).unwrap();
    agent.state.messages.push(serde_json::json!({
        "role": "assistant",
        "content": [{"type": "text", "text": plan_text}]
    }));
    (agent, temp)
}

#[tokio::test]
#[serial_test::serial(plan_task_global_state)]
async fn installation_preparation_failure_leaves_no_approved_plan_or_visible_tasks() {
    let session_id = format!("installation-failure-{}", uuid::Uuid::new_v4());
    let (mut agent, _temp) = approved_plan_agent(
        "# Plan: Failure Plan\n## Steps\n1. Must not materialize",
        session_id.clone(),
    )
    .await;

    archon_tools::task_manager::TASK_MANAGER
        .fail_next_plan_task_installation_for_test(&session_id);
    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(result.is_error, "{result:?}");
    assert!(result.content.contains("Failed to prepare approved plan tasks"));
    let store = agent.plan_store.as_ref().unwrap();
    let plan = store.load_latest_plan(&session_id).unwrap().unwrap();
    assert_eq!(plan.status, PlanStatus::Draft);
    assert!(store
        .load_approval_events(&session_id, &plan.id)
        .unwrap()
        .is_empty());
    assert!(store.load_plan_tasks(&session_id).unwrap().is_empty());
    assert_eq!(
        archon_tools::task_manager::TASK_MANAGER
            .installed_plan_task_count_for_session_for_test(&session_id),
        0
    );
}

#[tokio::test]
#[serial_test::serial(plan_task_global_state)]
async fn approval_collision_leaves_draft_and_existing_visible_task_unchanged() {
    let session_id = format!("installation-collision-{}", uuid::Uuid::new_v4());
    let (mut agent, _temp) = approved_plan_agent(
        "# Plan: Collision Plan\n## Steps\n1. Must not materialize",
        session_id.clone(),
    )
    .await;
    let collision_id = archon_tools::task_manager::TASK_MANAGER.create_task("unrelated task");
    archon_tools::plan_tasks::set_next_plan_task_ids_for_test([collision_id.clone()]);

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(result.is_error, "{result:?}");
    assert!(result.content.contains("task ID collision"));
    let store = agent.plan_store.as_ref().unwrap();
    let plan = store.load_latest_plan(&session_id).unwrap().unwrap();
    assert_eq!(plan.status, PlanStatus::Draft);
    assert!(store.load_approval_events(&session_id, &plan.id).unwrap().is_empty());
    assert!(store.load_plan_tasks(&session_id).unwrap().is_empty());
    assert_eq!(
        archon_tools::task_manager::TASK_MANAGER
            .get_task(&collision_id)
            .unwrap()
            .description,
        "unrelated task"
    );
    assert_eq!(
        archon_tools::task_manager::TASK_MANAGER
            .installed_plan_task_count_for_session_for_test(&session_id),
        0
    );
}

#[tokio::test]
#[serial_test::serial(plan_task_global_state)]
async fn approval_task_collision_rolls_back_terminal_plan_and_ledger() {
    use archon_session::plan::PersistedPlanTask;

    let session_id = format!("approval-durable-collision-{}", uuid::Uuid::new_v4());
    let (mut agent, _temp) = approved_plan_agent(
        "# Plan: Durable Collision Plan\n## Steps\n1. Must not overwrite durable task",
        session_id.clone(),
    )
    .await;
    {
        let store = agent.plan_store.as_ref().unwrap();
        let collision_id = "durable-collision-task";
        store
            .save_plan_task_fixture(
                &session_id,
                &PersistedPlanTask {
                    task_id: collision_id.into(),
                    plan_id: "unrelated-plan".into(),
                    plan_step: 9,
                    description: "existing durable task".into(),
                    status: "Pending".into(),
                    blocked_by: vec![],
                    required_evidence: vec![],
                    updated_at: "2026-08-15T00:00:00Z".into(),
                },
            )
            .unwrap();
    }
    let collision_id = "durable-collision-task";
    archon_tools::plan_tasks::set_next_plan_task_ids_for_test([collision_id.into()]);

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(result.is_error, "{result:?}");
    assert!(result.content.contains("task ID collision"));
    let store = agent.plan_store.as_ref().unwrap();
    let stored_plan = store.load_latest_plan(&session_id).unwrap().unwrap();
    assert_eq!(stored_plan.status, PlanStatus::Draft);
    assert!(store
        .load_approval_events(&session_id, &stored_plan.id)
        .unwrap()
        .is_empty());
    let tasks = store.load_plan_tasks(&session_id).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, collision_id);
    assert_eq!(tasks[0].description, "existing durable task");
    assert_eq!(
        archon_tools::task_manager::TASK_MANAGER
            .installed_plan_task_count_for_session_for_test(&session_id),
        0
    );
}

#[tokio::test]
#[serial_test::serial(plan_task_global_state)]
async fn approved_plan_installation_matches_durable_and_visible_task_sets() {
    let session_id = format!("installation-coherence-{}", uuid::Uuid::new_v4());
    let (mut agent, _temp) = approved_plan_agent(
        "# Plan: Coherent Plan\n## Steps\n1. First\n2. Second",
        session_id.clone(),
    )
    .await;

    let result = agent
        .handle_exit_plan_mode_approval(ToolResult::success("exit accepted"))
        .await;

    assert!(!result.is_error, "{result:?}");
    let store = agent.plan_store.as_ref().unwrap();
    let durable_tasks = store.load_plan_tasks(&session_id).unwrap();
    assert_eq!(durable_tasks.len(), 2);
    assert_eq!(
        archon_tools::task_manager::TASK_MANAGER
            .installed_plan_task_count_for_session_for_test(&session_id),
        durable_tasks.len()
    );
    for durable in durable_tasks {
        let visible = archon_tools::task_manager::TASK_MANAGER
            .get_task(&durable.task_id)
            .expect("durably materialized task must be visible");
        assert_eq!(visible.description, durable.description);
        assert_eq!(visible.metadata.unwrap().session_id, session_id);
    }
}
