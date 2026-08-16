use super::tool_postprocess_steps::PostprocessFlow;
use super::tool_types::PreflightResult;
use super::*;
use crate::dispatch::ToolRegistry;
use archon_tools::bash::BashTool;
use archon_tools::team_create::TeamCreateTool;
use archon_tools::tool::{PermissionLevel, Tool, WorkingTreeEffect};
use std::sync::atomic::{AtomicUsize, Ordering};

struct RetryTestTool {
    executions: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for RetryTestTool {
    fn name(&self) -> &str {
        "RetryTest"
    }

    fn description(&self) -> &str {
        "retry test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions.fetch_add(1, Ordering::SeqCst);
        ToolResult::success("executed")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[tokio::test]
async fn write_outside_approved_path_does_not_mark_plan_step_in_progress() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "exact-step-path-session";
    let store = durable_plan_store(temp.path(), session_id);
    let mut agent = bash_agent();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pre = PreflightResult {
        tool_name: "Write".into(),
        tool_id: "unplanned-write".into(),
        input: serde_json::json!({"file_path": "unplanned/src/planned.rs"}),
        tool_arc: Arc::new(RetryTestTool {
            executions: Arc::new(AtomicUsize::new(0)),
        }),
        file_path: Some("unplanned/src/planned.rs".into()),
        filesystem_effect: WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };

    agent
        .postprocess_single_tool(
            &pre,
            ToolResult::success("wrote"),
            &ctx,
            "test",
            &mut PostprocessFlow::default(),
        )
        .await;

    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert_eq!(
        plan.steps[0].status,
        archon_session::plan::PlanStepStatus::Pending
    );
}

#[tokio::test]
async fn successful_bash_mutation_is_durably_reconciled_as_unplanned_extra() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "bash-mutation-session";
    let store = durable_plan_store(temp.path(), session_id);
    let mut agent = bash_agent();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "bash-write".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"mkdir -p src && printf extra > src/extra.rs"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(
        plan.execution_evidence
            .touched_files
            .contains("src/extra.rs")
    );
    assert!(plan.reconciliation.iter().any(|entry| {
        entry.status == archon_session::plan::PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains("src/extra.rs")
    }));
}

#[tokio::test]
async fn failed_bash_mutation_is_durably_reconciled_as_unplanned_extra() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "failed-bash-mutation-session";
    let store = durable_plan_store(temp.path(), session_id);
    let mut agent = bash_agent();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "bash-failed-write".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"printf x > extra; false"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
    assert!(result.is_error, "fixture command must fail after writing");

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(plan.execution_evidence.touched_files.contains("extra"));
    assert!(plan.reconciliation.iter().any(|entry| {
        entry.status == archon_session::plan::PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains("extra")
    }));
}

#[tokio::test]
async fn successful_team_create_is_durably_reconciled_as_unplanned_extra() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "team-create-session";
    let store = durable_plan_store(temp.path(), session_id);
    let mut agent = team_create_agent(temp.path());
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "team-create".into(),
        name: "TeamCreate".into(),
        input_json: r#"{"name":"test","members":[{"role":"reviewer","system_prompt":"review"}]}"#
            .into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("TeamCreate preflight must capture a filesystem baseline");
    assert_eq!(pre.filesystem_effect, WorkingTreeEffect::Arbitrary);
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(
        plan.execution_evidence
            .touched_files
            .iter()
            .any(|path| { path.starts_with(".archon/teams/") && path.ends_with("/team.json") })
    );
    assert!(plan.reconciliation.iter().any(|entry| {
        entry.status == archon_session::plan::PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains(".archon/teams/")
    }));
}

#[tokio::test]
async fn directory_and_symlink_mutations_are_durably_reconciled() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "directory-symlink-mutation-session";
    let store = durable_plan_store(temp.path(), session_id);
    let mut agent = bash_agent();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    let pending = PendingToolCall {
        id: "bash-directory-symlink".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"mkdir created && ln -s created linked"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let plan = store.load_latest_plan(session_id).unwrap().unwrap();
    assert!(plan.execution_evidence.touched_files.contains("created"));
    assert!(plan.execution_evidence.touched_files.contains("linked"));
}

#[tokio::test]
async fn mutation_persistence_failure_turns_tool_result_into_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let session_id = "mutation-persistence-failure-session";
    let store = durable_plan_store(temp.path(), session_id);
    store.fail_next_mutation_persistence();
    let (mut agent, mut events) = bash_agent_with_events();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store);
    let pending = PendingToolCall {
        id: "mutation-persistence-failure".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"printf x > extra"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .expect("Bash preflight must capture a filesystem baseline");
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;

    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;

    let result =
        std::iter::from_fn(|| events.try_recv().ok()).find_map(|event| match event.inner {
            AgentEvent::ToolCallComplete { name, result, .. } if name == "Bash" => Some(result),
            _ => None,
        });
    let result = result.expect("Bash completion");
    assert!(result.is_error);
    assert!(
        result
            .content
            .contains("injected mutation persistence failure")
    );
}

#[path = "tool_postprocess_filesystem_tests.rs"]
mod filesystem;

#[path = "tool_postprocess_guard_tests.rs"]
mod guards;

fn bash_agent() -> Agent {
    bash_agent_with_events().0
}

fn team_create_agent(project_dir: &std::path::Path) -> Agent {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(TeamCreateTool::new(project_dir.into())));
    Agent::new(
        Arc::new(super::tests::MockLlmProvider),
        registry,
        AgentConfig::default(),
        tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY).0,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

fn bash_agent_with_events() -> (Agent, tokio::sync::mpsc::Receiver<TimestampedEvent>) {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(BashTool::default()));
    let (tx, rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let config = AgentConfig {
        permission_mode: Arc::new(tokio::sync::Mutex::new("bypassPermissions".into())),
        ..AgentConfig::default()
    };
    (
        Agent::new(
            Arc::new(super::tests::MockLlmProvider),
            registry,
            config,
            tx,
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(
                &std::env::temp_dir(),
            ))),
        ),
        rx,
    )
}

fn durable_plan_store(path: &std::path::Path, session_id: &str) -> archon_session::plan::PlanStore {
    use archon_session::plan::{PlanDocument, PlanStatus, PlanStep, PlanStepStatus};

    let session_store =
        archon_session::storage::SessionStore::open(&path.join("session.db")).unwrap();
    let store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let mut plan = PlanDocument::new("bash-mutation-plan", "Bash mutation plan");
    plan.status = PlanStatus::Executing;
    plan.steps.push(PlanStep {
        number: 1,
        description: "change planned file".into(),
        affected_files: vec!["src/planned.rs".into()],
        status: PlanStepStatus::Pending,
        blocked_by: Vec::new(),
        required_evidence: Vec::new(),
        task_id: None,
    });
    store.save_plan(session_id, &plan).unwrap();
    store
}

#[tokio::test]
#[ignore = "Gate 5 executable durable-plan lifecycle fixture"]
async fn durable_plan_lifecycle_blocks_completion_after_real_bash_and_context_clear() {
    use archon_tools::plan_tasks::{materialize_plan_tasks, test_plan_approval_authority};
    use archon_tools::task_manager::{TaskManager, TaskStatus};

    let temp = tempfile::tempdir().unwrap();
    let session_id = "durable-bash-lifecycle";
    let session_store =
        archon_session::storage::SessionStore::open(&temp.path().join("session.db")).unwrap();
    let store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let mut plan = lifecycle_plan(session_id);
    let manager = TaskManager::new();
    let task_ids = materialize_plan_tasks(
        &manager,
        &store,
        &test_plan_approval_authority(&store, session_id),
        session_id,
        &mut plan,
    )
    .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&task_ids[0], TaskStatus::Running, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&task_ids[0], TaskStatus::Completed, "", &[])
        .unwrap();
    manager
        .set_status_checked_with_evidence_ids(&task_ids[2], TaskStatus::Failed, "", &[])
        .unwrap();

    let mut agent = bash_agent();
    agent.config.session_id = session_id.into();
    agent.config.working_dir = temp.path().to_path_buf();
    agent.plan_store = Some(store.clone());
    execute_extra_bash_file(&mut agent, temp.path()).await;
    agent.state.add_user_message("transient state");
    agent.clear_conversation().await;

    let stored = store.load_latest_plan(session_id).unwrap().unwrap();
    assert_lifecycle_statuses(&stored.reconciliation);
    assert!(agent.plan_completion_block().is_some());
    assert!(agent.conversation_state().messages.is_empty());
}

async fn execute_extra_bash_file(agent: &mut Agent, root: &std::path::Path) {
    let pending = PendingToolCall {
        id: "bash-extra".into(),
        name: "Bash".into(),
        input_json: r#"{"command":"mkdir -p src && printf extra > src/extra.rs"}"#.into(),
    };
    let pre = agent
        .preflight_single_tool(&pending, AgentMode::Normal)
        .await
        .unwrap();
    let ctx = ToolContext {
        working_dir: root.to_path_buf(),
        ..ToolContext::default()
    };
    let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
    agent
        .postprocess_single_tool(&pre, result, &ctx, "test", &mut PostprocessFlow::default())
        .await;
}

fn lifecycle_plan(session_id: &str) -> archon_session::plan::PlanDocument {
    use archon_session::plan::{
        PlanApproval, PlanApprovalDecision, PlanApprovalSource, PlanStatus, PlanStep,
        PlanStepStatus,
    };

    let mut plan =
        archon_session::plan::PlanDocument::new("durable-bash-plan", "Durable Bash plan");
    plan.session_id = Some(session_id.into());
    plan.status = PlanStatus::Approved;
    plan.approval = Some(PlanApproval {
        decision: PlanApprovalDecision::Approve,
        source: PlanApprovalSource::NonInteractive,
        decided_at: chrono::Utc::now().to_rfc3339(),
        user_edited: false,
    });
    plan.steps = ["src/a.rs", "src/b.rs", "src/c.rs"]
        .into_iter()
        .enumerate()
        .map(|(index, file)| PlanStep {
            number: index as u32 + 1,
            description: format!("change {file}"),
            affected_files: vec![file.into()],
            status: PlanStepStatus::Pending,
            blocked_by: Vec::new(),
            required_evidence: Vec::new(),
            task_id: None,
        })
        .collect();
    plan
}

fn assert_lifecycle_statuses(entries: &[archon_session::plan::PlanStepReconciliation]) {
    use archon_session::plan::PlanReconciliationStatus::{
        Completed, Deviated, Omitted, UnplannedExtra,
    };

    assert!(
        entries
            .iter()
            .any(|entry| entry.step == Some(1) && entry.status == Completed)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.step == Some(2) && entry.status == Omitted)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.step == Some(3) && entry.status == Deviated)
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.status == UnplannedExtra && entry.detail.contains("src/extra.rs"))
    );
}
