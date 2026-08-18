use super::handle_slash_command;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use archon_core::agent::{Agent, AgentConfig, AgentEvent, AskUserPromptKind, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_permissions::mode::PermissionMode;
use archon_tools::tool::AgentMode;
use async_trait::async_trait;

struct IntegratedPlanProvider {
    streams: Mutex<VecDeque<(&'static str, &'static str, &'static str)>>,
    task_update_payloads: Arc<Mutex<VecDeque<String>>>,
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

impl IntegratedPlanProvider {
    fn new(
        requests: Arc<Mutex<Vec<LlmRequest>>>,
        task_update_payloads: Arc<Mutex<VecDeque<String>>>,
    ) -> Self {
        Self {
            streams: Mutex::new(VecDeque::from([
                (
                    "integrated-exit-rejected",
                    "ExitPlanMode",
                    "# Plan: Initial integrated plan\n## Steps\n1. Add the verified test\n2. Update the lifecycle reference",
                ),
                (
                    "integrated-write-blocked",
                    "Write",
                    r#"{"file_path":"src/blocked.rs","content":"must not exist"}"#,
                ),
                (
                    "integrated-exit-approved",
                    "ExitPlanMode",
                    "# Plan: Revised integrated plan\n## Steps\n1. Add the verified test\n2. Update the lifecycle reference",
                ),
                (
                    "integrated-test-run",
                    "Bash",
                    concat!(
                        r#"{"command":"cargo test --manifest-path "#,
                        env!("CARGO_MANIFEST_DIR"),
                        r#"/crates/archon-completion/Cargo.toml schema::tests::test_ensure_schema_idempotent --lib"}"#
                    ),
                ),
                (
                    "integrated-task-running",
                    "TaskUpdate",
                    "__FIRST_TASK_RUNNING__",
                ),
                (
                    "integrated-task-completed",
                    "TaskUpdate",
                    "__FIRST_TASK_COMPLETED__",
                ),
                (
                    "integrated-write-unplanned",
                    "Write",
                    r#"{"file_path":"src/unplanned-integrated.rs","content":"unplanned fixture mutation\n"}"#,
                ),
            ])),
            task_update_payloads,
            requests,
        }
    }
}

#[async_trait]
impl LlmProvider for IntegratedPlanProvider {
    fn name(&self) -> &str {
        "integrated-plan-provider"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        self.requests.lock().unwrap().push(request);
        let (tool_use_id, tool_name, payload) = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .expect("fixture supplied enough model responses");
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        if tool_name == "ExitPlanMode" {
            tx.send(StreamEvent::TextDelta {
                index: 0,
                text: payload.into(),
            })
            .await
            .unwrap();
        }
        tx.send(StreamEvent::ContentBlockStart {
            index: 1,
            block_type: archon_llm::types::ContentBlockType::ToolUse,
            tool_use_id: Some(tool_use_id.into()),
            tool_name: Some(tool_name.into()),
        })
        .await
        .unwrap();
        tx.send(StreamEvent::InputJsonDelta {
            index: 1,
            partial_json: if tool_name == "ExitPlanMode" {
                "{}".into()
            } else if payload.starts_with("__FIRST_TASK_") {
                self.task_update_payloads
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("fixture installed TaskUpdate payload")
            } else {
                payload.into()
            },
        })
        .await
        .unwrap();
        tx.send(StreamEvent::MessageStop).await.unwrap();
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("integrated smoke uses streaming only")
    }
}

#[tokio::test]
#[ignore = "Gate 5 deterministic integrated Plan Mode lifecycle fixture"]
#[serial_test::serial(plan_task_global_state)]
async fn plan_mode_integrated_live_smoke() {
    use archon_session::plan::{PlanReconciliationStatus, PlanStatus};
    use archon_tools::task_manager::{TASK_MANAGER, TaskManager};

    let mut fixture = crate::command::context::slash_ctx_test_fixture::build_test_slash_context(
        &format!("plan-mode-integrated-live-smoke-{}", uuid::Uuid::new_v4()),
        PermissionMode::Default.as_str(),
        None,
        None,
    );
    let session_id = fixture.ctx.session_id.clone();
    let working_dir = fixture.ctx.working_dir.clone();
    let session_store = Arc::clone(&fixture.ctx.session_store);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let task_update_payloads = Arc::new(Mutex::new(VecDeque::new()));
    let (agent_event_tx, mut agent_event_rx) = tokio::sync::mpsc::channel(1024);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::plan_mode::ExitPlanModeTool));
    registry.register(Box::new(archon_tools::file_write::WriteTool));
    registry.register(Box::new(archon_tools::bash::BashTool::default()));
    registry.register(Box::new(archon_tools::task_update::TaskUpdateTool));
    let config = AgentConfig {
        session_id: session_id.clone(),
        model: "session-model".into(),
        working_dir: working_dir.clone(),
        permission_mode: Arc::clone(&fixture.ctx.permission_mode),
        max_turns: Some(1),
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(
        Arc::new(IntegratedPlanProvider::new(
            Arc::clone(&requests),
            Arc::clone(&task_update_payloads),
        )),
        registry,
        config,
        agent_event_tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(&working_dir))),
    );
    agent.set_plan_mode_state(Arc::clone(&fixture.ctx.plan_mode_state));
    agent.set_session_store(Arc::clone(&session_store));
    let (approval_tx, approval_rx) = tokio::sync::mpsc::channel(2);
    agent.ask_user_response_rx = Some(Arc::new(tokio::sync::Mutex::new(approval_rx)));
    let plan_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let authority = plan_store
        .bootstrap_approval_authority(&session_id, [0xA5; 32])
        .unwrap();
    let planned_task_ids = [
        uuid::Uuid::new_v4().simple().to_string(),
        uuid::Uuid::new_v4().simple().to_string(),
    ];
    archon_tools::plan_tasks::set_next_plan_task_ids_for_test(planned_task_ids.clone());
    let global_task_cleanup =
        TASK_MANAGER.scoped_plan_task_cleanup_for_test(&session_id, planned_task_ids.clone());
    agent.set_plan_store(plan_store, authority).unwrap();

    let mut fast_mode = archon_llm::fast_mode::FastModeState::new_with(false);
    let mut effort_state = archon_llm::effort::EffortState::new();
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    assert!(
        handle_slash_command(
            "/plan",
            &mut fast_mode,
            &mut effort_state,
            &tui_tx,
            &mut fixture.ctx,
        )
        .await,
        "/plan must travel through handle_slash_command → build_command_context → dispatcher → apply_effect",
    );
    assert_eq!(
        *agent.permission_mode_handle().lock().await,
        PermissionMode::Plan.as_str(),
        "Agent and slash runtime must share the production permission handle",
    );
    assert_eq!(
        agent.plan_mode_state().lock().await.entered_via,
        Some(archon_core::agent::plan_mode_state::PlanEntryPath::SlashCommand),
    );

    approval_tx
        .send("reject: test evidence missing".into())
        .await
        .unwrap();
    approval_tx.send("approve".into()).await.unwrap();
    agent
        .process_message("submit the first two-step plan")
        .await
        .unwrap();
    let initial = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(initial.status, PlanStatus::Abandoned);
    assert_eq!(*agent.permission_mode_handle().lock().await, "plan");

    agent
        .process_message("try the blocked mutation")
        .await
        .unwrap();
    assert!(
        !working_dir.join("src/blocked.rs").exists(),
        "real Write preflight must block the model-provided mutation while rejection keeps Plan Mode active",
    );

    let audit_path = archon_core::plan_file::plan_audit_path(&working_dir, &session_id).unwrap();
    let audit = std::fs::read_to_string(audit_path).unwrap();
    assert!(audit.contains("Write (intercepted in Plan Mode)"));
    assert!(audit.contains("src/blocked.rs"));
    assert!(audit.contains("must not exist"));
    assert!(
        audit.contains("intercepted in Plan Mode"),
        "audit must record the Plan Mode rejection reason"
    );

    agent
        .process_message("submit the revised two-step plan")
        .await
        .unwrap();
    assert_eq!(*agent.permission_mode_handle().lock().await, "default");
    let approved = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan(&session_id)
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, PlanStatus::Approved);
    assert_eq!(approved.steps.len(), 2);
    let linked_tasks = TASK_MANAGER
        .list_tasks()
        .into_iter()
        .filter(|task| {
            task.metadata
                .as_ref()
                .is_some_and(|metadata| metadata.session_id == session_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(linked_tasks.len(), 2);

    let manual_manager = TaskManager::new();
    let manual_task_id = manual_manager.create_task("manual task is process scoped");
    let first_task = linked_tasks
        .iter()
        .find(|task| {
            task.metadata
                .as_ref()
                .is_some_and(|metadata| metadata.plan_step == 1)
        })
        .unwrap();

    agent
        .process_message("execute the approved verification command")
        .await
        .unwrap();
    let test_evidence = archon_completion::store::get_evidence_by_run(
        session_store.db(),
        &format!("{}:integrated-test-run", session_id),
    )
    .unwrap();
    assert_eq!(test_evidence.len(), 1);
    assert_eq!(test_evidence[0].producer, "authoritative-bash-execution");
    assert_eq!(test_evidence[0].exit_code, Some(0));
    task_update_payloads.lock().unwrap().extend([
        serde_json::json!({"task_id": first_task.id, "status": "Running"}).to_string(),
        serde_json::json!({
            "task_id": first_task.id,
            "status": "Completed",
            "evidence_run_id": test_evidence[0].run_id,
            "evidence_ids": [test_evidence[0].evidence_id],
        })
        .to_string(),
    ]);
    agent
        .process_message("mark the verified plan task running")
        .await
        .unwrap();
    agent
        .process_message("complete the verified plan task")
        .await
        .unwrap();

    agent
        .process_message("perform the approved unplanned mutation")
        .await
        .unwrap();
    assert!(working_dir.join("src/unplanned-integrated.rs").is_file());
    agent.clear_conversation().await;

    let reconciled = archon_session::plan::PlanStore::new(session_store.db())
        .unwrap()
        .load_latest_plan(&session_id)
        .unwrap()
        .unwrap();
    assert!(reconciled.reconciliation.iter().any(|entry| {
        entry.step == Some(1) && entry.status == PlanReconciliationStatus::Completed
    }));
    assert!(reconciled.reconciliation.iter().any(|entry| {
        entry.step == Some(2) && entry.status == PlanReconciliationStatus::Omitted
    }));
    assert!(reconciled.reconciliation.iter().any(|entry| {
        entry.step.is_none()
            && entry.status == PlanReconciliationStatus::UnplannedExtra
            && entry.detail.contains("src/unplanned-integrated.rs")
    }));

    let fresh_manager = TaskManager::new();
    let rehydration_store = archon_session::plan::PlanStore::new(session_store.db()).unwrap();
    let rehydration_authority = Arc::new(
        rehydration_store
            .bootstrap_approval_authority(&session_id, [0xA5; 32])
            .unwrap(),
    );
    let installed = archon_tools::plan_tasks::rehydrate_plan_tasks(
        &fresh_manager,
        &rehydration_store,
        &rehydration_authority,
        &session_id,
    )
    .unwrap();
    assert_eq!(
        installed, 2,
        "fresh manager must rehydrate durable plan-linked tasks"
    );
    assert!(
        fresh_manager.get_task(&manual_task_id).is_none(),
        "manual task must be absent from the fresh manager because it has no durable plan-task row",
    );

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 7);
    assert!(requests.iter().take(3).all(|request| {
        request.request_origin.as_deref() == Some("plan_mode")
            && request
                .system
                .iter()
                .filter_map(|block| block["text"].as_str())
                .any(|text| text.contains("<system-reminder>") && text.contains("ExitPlanMode"))
    }));
    drop(requests);
    let events =
        std::iter::from_fn(|| agent_event_rx.try_recv().ok()).collect::<Vec<TimestampedEvent>>();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event.inner,
                AgentEvent::AskUser {
                    kind: AskUserPromptKind::PlanApproval,
                    ..
                }
            ))
            .count(),
        2,
    );
    assert_eq!(agent.conversation_state().mode, AgentMode::Normal);

    drop(global_task_cleanup);
    assert!(
        planned_task_ids
            .iter()
            .all(|task_id| TASK_MANAGER.get_task(task_id).is_none()),
        "cleanup must remove only smoke-created global plan tasks"
    );
    assert!(
        !TASK_MANAGER.has_plan_store_attachment_for_test(&session_id),
        "cleanup must detach the smoke session's PlanStore attachment"
    );
}
