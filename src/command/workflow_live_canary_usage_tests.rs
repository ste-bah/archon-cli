use std::process::Command as CanaryGitCommand;
use std::sync::{Arc, Mutex};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::create_default_registry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_learning::llm_call_usage::{LlmCallUsageScope, UsageAvailability, list_llm_call_usage};
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse as ProviderResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_pipeline::llm_adapter::ProviderLlmAdapter;
use archon_pipeline::runner::LlmClient;
use archon_pipeline::subagent_adapter::SubagentPipelineClient;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_tools::tool::ToolContext;
use archon_workflow::CommandAction;
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use super::{CANARY_ARTIFACT_REL, CanaryAgentClient};
use crate::command::workflow_live::{LiveApprovalMode, run_live_action};

const CANARY_TEST: &str = "command::workflow_live::workflow_live_canary_tests::usage_tests::canary_wf_afae6bee_provider_ledger";
const CANARY_CHILD_ENV: &str = "ARCHON_CANARY_USAGE_CHILD";
const CANARY_USAGE_SCOPE: &str = "wf-afae6bee-measurement";

#[derive(Clone)]
struct ScopedCanaryClient {
    inner: Arc<dyn LlmClient>,
}

impl ScopedCanaryClient {
    fn new(inner: Arc<dyn LlmClient>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LlmClient for ScopedCanaryClient {
    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        let request = archon_pipeline::runner::AgentExecutionRequest {
            pipeline_type: archon_pipeline::runner::PipelineType::Workflow,
            session_id: CANARY_USAGE_SCOPE.into(),
            cwd: None,
            task: "controlled canary planner call".into(),
            ordinal: 0,
            attempt: 1,
            agent: archon_pipeline::runner::AgentInfo {
                key: "planner".into(),
                display_name: "Planner".into(),
                model: model.into(),
                phase: 0,
                critical: true,
                parallelizable: false,
                quality_threshold: 0.0,
                tool_access_level: archon_pipeline::runner::ToolAccessLevel::ReadOnly,
            },
            messages,
            system,
            tools,
            allowed_tools: Vec::new(),
            timeout_secs: None,
            disable_auto_background: true,
            provider_env_resolution: None,
        };
        self.inner.run_agent(request).await
    }

    async fn run_agent(
        &self,
        mut request: archon_pipeline::runner::AgentExecutionRequest,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        request.session_id = CANARY_USAGE_SCOPE.into();
        self.inner.run_agent(request).await
    }
}

struct CanaryProvider {
    script: Arc<CanaryAgentClient>,
    request_bytes: Arc<Mutex<Vec<u64>>>,
}

impl CanaryProvider {
    fn new(script: Arc<CanaryAgentClient>, request_bytes: Arc<Mutex<Vec<u64>>>) -> Self {
        Self {
            script,
            request_bytes,
        }
    }

    fn response_for_request(&self, request: &LlmRequest) -> (String, u64) {
        let mut prompt = String::new();
        for value in request.system.iter().chain(request.messages.iter()) {
            collect_text(value, &mut prompt);
        }
        let content = self.script.respond(&prompt);
        self.script
            .prompts
            .lock()
            .expect("prompt log lock")
            .push(prompt.chars().take(2000).collect());
        let bytes = measured_request_bytes(request);
        self.request_bytes
            .lock()
            .expect("request byte log lock")
            .push(bytes);
        (content, bytes)
    }
}

#[async_trait]
impl LlmProvider for CanaryProvider {
    fn name(&self) -> &str {
        "controlled-canary"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "controlled-canary-model".into(),
            display_name: "Controlled Canary Model".into(),
            context_window: 200_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let (content, request_bytes) = self.response_for_request(&request);
        let input_tokens = request_bytes.div_ceil(4);
        let output_tokens = content.len().div_ceil(4) as u64;
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            for event in provider_events(content, input_tokens, output_tokens) {
                tx.send(event).await.expect("send canary stream event");
            }
        });
        Ok(rx)
    }

    async fn complete(&self, _: LlmRequest) -> Result<ProviderResponse, LlmError> {
        Err(LlmError::Unsupported(
            "controlled canary uses streaming".into(),
        ))
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }
}

fn measured_request_bytes(request: &LlmRequest) -> u64 {
    serde_json::to_vec(&serde_json::json!({
        "system": request.system,
        "messages": request.messages,
        "tools": request.tools,
    }))
    .expect("serialize measured provider request")
    .len() as u64
}

fn provider_events(content: String, input_tokens: u64, output_tokens: u64) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: uuid::Uuid::new_v4().to_string(),
            model: "controlled-canary-model".into(),
            usage: Usage {
                input_tokens,
                input_tokens_available: true,
                cache_creation_input_tokens_available: true,
                cache_read_input_tokens_available: true,
                ..Usage::default()
            },
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: content,
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageDelta {
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                output_tokens,
                output_tokens_available: true,
                ..Usage::default()
            }),
        },
        StreamEvent::MessageStop,
    ]
}

fn collect_text(value: &serde_json::Value, into: &mut String) {
    match value {
        serde_json::Value::String(text) => {
            into.push_str(text);
            into.push('\n');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, into);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_text(item, into);
            }
        }
        _ => {}
    }
}

fn install_canary_executor(provider: Arc<dyn LlmProvider>, root: &std::path::Path) {
    let agent_config = AgentConfig {
        session_id: CANARY_USAGE_SCOPE.into(),
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    let executor = AgentSubagentExecutor::new(
        provider,
        create_default_registry(root.to_path_buf(), None),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
        None,
        None,
        root.to_path_buf(),
        CANARY_USAGE_SCOPE.into(),
        "controlled-canary-model".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(None)),
        Arc::new(agent_config),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            CANARY_USAGE_SCOPE.into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
}

fn assert_canary_usage(path: &std::path::Path, expected_rows: usize, request_bytes: &[u64]) {
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().expect("UTF-8 learning path"),
        "reopen canary learning db",
    )
    .expect("learning db");
    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some(CANARY_USAGE_SCOPE), Some(CANARY_USAGE_SCOPE)),
    )
    .expect("list canary usage");
    assert_eq!(
        rows.len(),
        expected_rows,
        "one durable row per provider call"
    );
    assert_eq!(
        rows.len(),
        request_bytes.len(),
        "ledger and provider call counts differ"
    );
    assert!(
        rows.iter().all(|row| {
            row.provider_id == "controlled-canary"
                && row.model_id == "controlled-canary-model"
                && row.terminal_status == "succeeded"
                && matches!(row.input_tokens, UsageAvailability::Known(_))
                && matches!(row.output_tokens, UsageAvailability::Known(_))
                && matches!(row.cache_creation_input_tokens, UsageAvailability::Known(_))
                && matches!(row.cache_read_input_tokens, UsageAvailability::Known(_))
        }),
        "unexpected controlled canary usage rows: {rows:#?}"
    );
    print_canary_evidence(&rows, request_bytes);
}

fn print_canary_evidence(
    rows: &[archon_learning::llm_call_usage::LlmCallUsageRecord],
    request_bytes: &[u64],
) {
    let totals = rows.iter().fold((0, 0, 0, 0), |totals, row| {
        (
            totals.0 + known_usage(&row.input_tokens),
            totals.1 + known_usage(&row.cache_creation_input_tokens),
            totals.2 + known_usage(&row.cache_read_input_tokens),
            totals.3 + known_usage(&row.output_tokens),
        )
    });
    let evidence = serde_json::json!({
        "fixture": "canary_wf_afae6bee_regression",
        "measurement": "controlled provider-shaped serialized-request-byte/4 estimates",
        "measurement_overlay": true,
        "external_provider_telemetry": false,
        "source_revision": option_env!("GIT_SHA").unwrap_or("unknown"),
        "call_count": rows.len(),
        "request_bytes": request_bytes,
        "usage_totals": {
            "input_tokens": totals.0,
            "cache_creation_input_tokens": totals.1,
            "cache_read_input_tokens": totals.2,
            "output_tokens": totals.3,
        }
    });
    println!("ISSUE75_CANARY_LEDGER_EVIDENCE={evidence}");
}

fn known_usage(usage: &UsageAvailability) -> u64 {
    match usage {
        UsageAvailability::Known(value) => *value,
        UsageAvailability::Unavailable => panic!("controlled provider usage must be available"),
    }
}

fn canary_git(repo: &std::path::Path, args: &[&str]) {
    let output = CanaryGitCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command starts");
    assert!(
        output.status.success(),
        "git {:?} failed: stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn seed_canary_project(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/lib.rs"), "pub fn gap_audit() {}\n").expect("seed source");
    canary_git(&repo, &["init"]);
    canary_git(&repo, &["config", "user.name", "archon-canary"]);
    canary_git(&repo, &["config", "user.email", "canary@example.invalid"]);
    canary_git(&repo, &["add", "."]);
    canary_git(&repo, &["commit", "-m", "initial"]);
    let tasks = root.join("tasks/PRD-CANARY-AFAE6BEE-001");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-TDL-001-data-lake-gap-audit.md"),
        format!(
            "# Data Lake Gap Audit\n\ntask_id: TASK-TDL-001\ndepends_on: []\n\n\
             ## Acceptance Criteria\n\n- Gap audit implemented in the target repository.\n\
             - Artifact evidence written to `{CANARY_ARTIFACT_REL}`.\n\n\
             ## Artifact Requirements\n\n- `{CANARY_ARTIFACT_REL}`\n"
        ),
    )
    .expect("task file");
    (repo, tasks)
}

struct CanaryRunHarness {
    script: Arc<CanaryAgentClient>,
    request_bytes: Arc<Mutex<Vec<u64>>>,
    client: Arc<dyn LlmClient>,
}

async fn build_canary_harness(root: &std::path::Path) -> CanaryRunHarness {
    let script = Arc::new(CanaryAgentClient::new(root.to_path_buf()));
    let request_bytes = Arc::new(Mutex::new(Vec::new()));
    let provider: Arc<dyn LlmProvider> = Arc::new(CanaryProvider::new(
        Arc::clone(&script),
        Arc::clone(&request_bytes),
    ));
    let provider = crate::runtime::provider_observer::observe_llm_provider_with_profile(
        provider,
        "workflow-canary",
        None,
    )
    .await;
    install_canary_executor(Arc::clone(&provider), root);
    let raw: Arc<dyn LlmClient> =
        Arc::new(ProviderLlmAdapter::new(Arc::clone(&provider)).with_origin("workflow-canary"));
    let fallback: Arc<dyn LlmClient> = Arc::new(ScopedCanaryClient::new(raw));
    let client = Arc::new(SubagentPipelineClient::with_provider(
        fallback,
        ToolContext {
            working_dir: root.to_path_buf(),
            ..ToolContext::default()
        },
        provider,
    ));
    CanaryRunHarness {
        script,
        request_bytes,
        client,
    }
}

fn assert_canary_output(output: &str, script: &CanaryAgentClient) -> usize {
    let prompts = script.prompts.lock().expect("prompt log").clone();
    assert!(
        script.artifact_exists(),
        "artifact contract did not reach implementation prompt. Prompts: {}\nOutput:\n{output}",
        prompts.join("\n---\n"),
    );
    assert!(!output.contains("blocked-verification-failed"), "{output}");
    assert!(
        output.contains("Workflow V2 complete:")
            || (output.contains("Workflow V2 needs review:")
                && output.contains("failed_call: blocked-final-readiness")),
        "{output}"
    );
    prompts.len()
}

async fn run_canary() {
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(64);
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let learning_db_path = root.join(".archon").join("learning-state.db");
    // SAFETY: this fixture executes in an isolated child process.
    unsafe {
        std::env::set_var("ARCHON_LEARNING_DB_PATH", &learning_db_path);
    }
    let (repo, tasks) = seed_canary_project(root);
    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        repo.display()
    );
    let harness = build_canary_harness(root).await;
    let output = run_live_action(
        root,
        // `decomposed` was added by v3 (3c547dea) after this test was written.
        // false preserves the behaviour this test was authored against: the
        // script_lifecycle comes from the environment, not forced off.
        CommandAction::Run {
            task,
            decomposed: false,
        },
        harness.client,
        tui_tx,
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("decomposed PRD canary run completes with a final report");
    let prompt_count = assert_canary_output(&output, &harness.script);
    let request_bytes = harness
        .request_bytes
        .lock()
        .expect("request byte log lock")
        .clone();
    assert_canary_usage(&learning_db_path, prompt_count, &request_bytes);
}

fn run_isolated_child() {
    let status =
        std::process::Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg(CANARY_TEST)
            .arg("--nocapture")
            .env(CANARY_CHILD_ENV, "execute")
            .status()
            .expect("run isolated canary child");
    assert!(status.success(), "isolated canary child failed");
}

#[test]
fn canary_wf_afae6bee_provider_ledger() {
    match std::env::var(CANARY_CHILD_ENV) {
        Ok(value) => {
            assert_eq!(value, "execute", "unexpected child marker");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("canary runtime")
                .block_on(run_canary());
        }
        Err(std::env::VarError::NotPresent) => run_isolated_child(),
        Err(error) => panic!("invalid child marker: {error}"),
    }
}
