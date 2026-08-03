use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::ContentBlockType;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use archon_workflow::task_universe::WorkflowV2TaskUniverseTask;
use archon_workflow::v2::lifecycle_driver::{
    LifecycleDriver, LifecycleLimits, OrchestrationLedger,
};
use archon_workflow::v2::orchestrator_actions::OrchestratorAction;
use archon_workflow::{WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter, WorkflowV2ResultStore};
use tokio::sync::mpsc;

use super::*;

const TEST_NAME: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_script::workflow_live_v3_compaction_tests::workflow_v3_document_ingestion_uses_real_subagent_recovery";
const AGGREGATE_TEST_NAME: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_script::workflow_live_v3_compaction_tests::workflow_v3_aggregate_document_burst_recovers_on_emergency_projection";
const OPENING_TEST_NAME: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_script::workflow_live_v3_compaction_tests::workflow_v3_opening_prompt_overflow_is_classified_independently";
const CHILD_ENV: &str = "ARCHON_V3_COMPACTION_TEST_CHILD";

#[derive(Clone, Copy)]
enum FixtureScenario {
    ToolField,
    AggregateContext,
    OpeningPrompt,
}

struct DocumentProvider {
    scenario: FixtureScenario,
    calls: AtomicUsize,
    requests: Mutex<Vec<LlmRequest>>,
}

impl DocumentProvider {
    fn new(scenario: FixtureScenario) -> Self {
        Self {
            scenario,
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for DocumentProvider {
    fn name(&self) -> &str {
        "anthropic"
    }
    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(&self, request: LlmRequest) -> Result<mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            return Ok(events(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Compacted document history.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }
        self.requests.lock().unwrap().push(request.clone());
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if matches!(self.scenario, FixtureScenario::OpeningPrompt) {
            return Err(LlmError::ContextWindowExceeded {
                provider_message: "maximum context length exceeded".into(),
                provider: Some("anthropic".into()),
                model: Some("claude-sonnet-4-6".into()),
            });
        }
        let tool_name = "Read";
        if call == 0 {
            let tool_calls = match self.scenario {
                FixtureScenario::ToolField => vec![tool_call_events(0, "doc-1", tool_name)],
                FixtureScenario::AggregateContext => (0..5)
                    .map(|index| tool_call_events(index, &format!("doc-{index}"), tool_name))
                    .collect(),
                FixtureScenario::OpeningPrompt => unreachable!(),
            };
            return Ok(events(
                tool_calls
                    .into_iter()
                    .flatten()
                    .chain([StreamEvent::MessageStop])
                    .collect(),
            )
            .await);
        }
        let pressure = match self.scenario {
            FixtureScenario::ToolField => largest_tool_result(&request) > 64_000,
            FixtureScenario::AggregateContext => {
                largest_tool_result(&request) <= 200_000 && request_body_bytes(&request) > 500_000
            }
            FixtureScenario::OpeningPrompt => unreachable!(),
        };
        if pressure {
            return Err(match self.scenario {
                FixtureScenario::ToolField => LlmError::Http(
                    "messages.4.content.0.tool_result.content: String should have at most 64000 bytes".into(),
                ),
                FixtureScenario::AggregateContext => LlmError::ContextWindowExceeded {
                    provider_message: "maximum context length exceeded".into(),
                    provider: Some("anthropic".into()),
                    model: Some("claude-sonnet-4-6".into()),
                },
                FixtureScenario::OpeningPrompt => unreachable!(),
            });
        }
        Ok(events(vec![
            StreamEvent::TextDelta {
                index: 0,
                text: accepted_result_text(),
            },
            StreamEvent::MessageStop,
        ])
        .await)
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("fixture streams")
    }
}

struct FixtureDocumentTool {
    name: &'static str,
    content: String,
}

#[async_trait::async_trait]
impl Tool for FixtureDocumentTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "Return deterministic oversized document text"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        ToolResult::success(self.content.clone())
    }
    fn permission_level(&self, _: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[test]
fn workflow_v3_document_ingestion_uses_real_subagent_recovery() {
    run_isolated_fixture(TEST_NAME, FixtureScenario::ToolField);
}

#[test]
fn workflow_v3_aggregate_document_burst_recovers_on_emergency_projection() {
    run_isolated_fixture(AGGREGATE_TEST_NAME, FixtureScenario::AggregateContext);
}

#[test]
fn workflow_v3_opening_prompt_overflow_is_classified_independently() {
    run_isolated_fixture(OPENING_TEST_NAME, FixtureScenario::OpeningPrompt);
}

fn run_isolated_fixture(test_name: &str, scenario: FixtureScenario) {
    if std::env::var(CHILD_ENV).ok().as_deref() != Some("run") {
        run_child(test_name);
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run_fixture(scenario));
}

fn run_child(test_name: &str) {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(test_name)
        .arg("--nocapture")
        .env(CHILD_ENV, "run")
        .status()
        .expect("run isolated V3 fixture");
    assert!(status.success(), "isolated V3 fixture failed");
}

async fn run_fixture(scenario: FixtureScenario) {
    let temp = tempfile::tempdir().unwrap();
    let provider = Arc::new(DocumentProvider::new(scenario));
    let activity = Arc::new(archon_observability::InMemoryActivitySink::new());
    let mut registry = ToolRegistry::new();
    match scenario {
        FixtureScenario::ToolField => registry.register(Box::new(FixtureDocumentTool {
            name: "Read",
            content: format!("HEAD{}TAIL", "x".repeat(180_000)),
        })),
        FixtureScenario::AggregateContext => registry.register(Box::new(FixtureDocumentTool {
            name: "Read",
            content: format!("HEAD{}TAIL", "x".repeat(150 * 1024)),
        })),
        FixtureScenario::OpeningPrompt => {}
    }
    let config = AgentConfig {
        session_id: "v3-compaction".into(),
        working_dir: temp.path().to_path_buf(),
        activity_sink: Some(activity.clone()),
        context: archon_core::config::ContextConfig {
            preserve_recent_turns: 2,
            max_tool_result_bytes: 256_000,
            context_window_override: Some(1_000_000),
            ..Default::default()
        },
        ..Default::default()
    };
    let executor = AgentSubagentExecutor::new(
        provider.clone(),
        registry,
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(temp.path()))),
        None,
        None,
        temp.path().to_path_buf(),
        "v3-compaction".into(),
        "claude-sonnet-4-6".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".into())),
        Arc::new(tokio::sync::Mutex::new(None)),
        Arc::new(config),
        Arc::new(test_identity()),
    );
    install_subagent_executor(Arc::new(executor));

    let llm = crate::command::pipeline_workflow_llm::subagent_workflow_client_for_test(
        provider.clone(),
        "v3-compaction",
        temp.path().to_path_buf(),
        crate::command::pipeline_workflow_llm::TestClientFallback::Forbidden,
    );
    let (ui_sink, mut tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    tokio::spawn(async move { while tui_rx.recv().await.is_some() {} });
    let client = LiveV2AgentClient::new(
        llm,
        ui_sink,
        Vec::new(),
        "v3-compaction".into(),
        Some(temp.path().display().to_string()),
        Some(30),
    );
    let universe = task_universe();
    let spec = WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "v3-compaction".into(),
        task: "Ingest document through a real V3 explorer subagent.".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: BTreeMap::new(),
        learning_hooks: Vec::new(),
    };
    let store = WorkflowStore::new(temp.path().join(".archon/workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let generated = archon_core::config::GeneratedWorkflowConfig::default();
    let runner = WorkflowV2ScriptRunner::new(
        spec.task,
        WorkflowV2ScriptRuntime {
            target_repository_root: None,
            generated_config: generated.clone(),
        },
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        store,
        run.id,
        true,
        Some(universe.clone()),
        None,
    );
    let host = Arc::new(WorkflowScriptHost {
        scaffold_hash: workflow_scaffold_hash("# v3 compaction fixture"),
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(WorkflowScriptAccumulator::default())),
    });
    let driver = LifecycleDriver::new(
        host,
        universe,
        None,
        Some(temp.path().display().to_string()),
        serde_json::json!([]),
        Default::default(),
        LifecycleLimits {
            max_repair_iterations: generated.max_repair_iterations,
            max_investigation_iterations: generated.max_investigation_iterations,
            implementation_wave_max_parallelism: generated.implementation_wave_max_parallelism,
        },
    );
    let mut ledger = OrchestrationLedger::for_universe(&driver.universe);
    let outcome = driver
        .dispatch_orchestrator_action(
            0,
            &OrchestratorAction::SpawnExplorer {
                question: "Read the fixture document and report accepted evidence.".into(),
            },
            &mut ledger,
        )
        .await
        .expect("V3 explorer action");

    let requests = provider.requests.lock().unwrap();
    let request_sizes: Vec<usize> = requests.iter().map(largest_tool_result).collect();
    let recovery: Vec<serde_json::Value> = activity
        .events()
        .into_iter()
        .filter_map(|event| serde_json::from_str(&event.message).ok())
        .collect();
    match scenario {
        FixtureScenario::ToolField => {
            assert_eq!(outcome.status, "ok", "outcome={outcome:#?}");
            assert_eq!(
                requests.len(),
                3,
                "tool call, rejected document request, emergency completion; sizes={request_sizes:?}"
            );
            assert_eq!(request_sizes[0], 0);
            assert!(request_sizes[1] > 64_000);
            assert!(request_sizes[2] <= 64_000);
            assert_eq!(recovery.len(), 1);
            assert_eq!(recovery[0]["classification"], "tool_result_field");
            assert_eq!(recovery[0]["tier"], "emergency_projection");
            assert_eq!(recovery[0]["reduced"], true);
        }
        FixtureScenario::AggregateContext => {
            assert_eq!(outcome.status, "ok", "outcome={outcome:#?}");
            assert_eq!(
                requests.len(),
                3,
                "tool burst, aggregate rejection, emergency completion; sizes={request_sizes:?}"
            );
            assert!(request_body_bytes(&requests[1]) > 500_000);
            assert!(request_body_bytes(&requests[2]) < 500_000);
            assert_eq!(recovery.len(), 1);
            assert_eq!(recovery[0]["classification"], "aggregate_context");
            assert_eq!(recovery[0]["tier"], "emergency_projection");
            assert_eq!(recovery[0]["reduced"], true);
        }
        FixtureScenario::OpeningPrompt => {
            assert_eq!(outcome.status, "gate_rejected");
            assert_eq!(
                requests.len(),
                2,
                "opening prompt and unchanged emergency attempt"
            );
            assert!(
                requests
                    .iter()
                    .all(|request| largest_tool_result(request) == 0)
            );
            assert_eq!(recovery.len(), 1);
            assert_eq!(recovery[0]["classification"], "opening_prompt");
            assert_eq!(recovery[0]["tier"], "emergency_projection");
            assert_eq!(recovery[0]["reduced"], false);
        }
    }
}

fn tool_call_events(index: u32, tool_use_id: &str, tool_name: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ContentBlockStart {
            index,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some(tool_use_id.into()),
            tool_name: Some(tool_name.into()),
        },
        StreamEvent::InputJsonDelta {
            index,
            partial_json: "{}".into(),
        },
        StreamEvent::ContentBlockStop { index },
    ]
}

fn request_body_bytes(request: &LlmRequest) -> usize {
    serde_json::to_vec(&serde_json::json!({
        "model": &request.model,
        "max_tokens": request.max_tokens,
        "system": &request.system,
        "messages": &request.messages,
        "tools": &request.tools,
        "thinking": &request.thinking,
        "speed": &request.speed,
        "effort": &request.effort,
        "extra": &request.extra,
        "request_origin": &request.request_origin,
        "reasoning_encrypted": &request.reasoning_encrypted,
    }))
    .expect("serialize fixture request")
    .len()
}

async fn events(events: Vec<StreamEvent>) -> mpsc::Receiver<StreamEvent> {
    let (tx, rx) = mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.unwrap();
    }
    rx
}

fn accepted_result_text() -> String {
    serde_json::json!({
        "status":"accepted",
        "summary":"real V3 document ingestion recovered",
        "evidence":[{"kind":"inspection","summary":"document read"}],
        "artifacts":[], "commands_run":[], "files_read":[], "files_changed":[],
        "task_coverage":[], "residual_gaps":[], "data":{}
    })
    .to_string()
}

fn largest_tool_result(request: &LlmRequest) -> usize {
    request
        .messages
        .iter()
        .filter_map(|message| message.get("content").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|block| {
            block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
        })
        .filter_map(|block| block.get("content").and_then(serde_json::Value::as_str))
        .map(|content| {
            serde_json::to_vec(&serde_json::Value::String(content.into()))
                .unwrap()
                .len()
        })
        .max()
        .unwrap_or(0)
}

fn task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-V3-DOC".into(),
            source_path: "tasks/TASK-V3-DOC.md".into(),
            acceptance_criteria: vec![
                "Document ingestion completes through real subagent recovery.".into(),
            ],
            ..Default::default()
        }],
    }
}

fn test_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Clean,
        "v3-compaction".into(),
        "device".into(),
        String::new(),
    )
}
