use super::*;
use archon_core::agent::AgentConfig;
use archon_core::dispatch::create_default_registry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_learning::llm_call_usage::{
    LlmCallUsageRecord, LlmCallUsageScope, UsageAvailability, list_llm_call_usage,
};
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmProvider;
use archon_llm::providers::AnthropicProvider;
use archon_llm::types::Secret;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostMethod};
#[path = "workflow_live_v2_wire_server.rs"]
mod wire_server;
use tokio::net::TcpListener;
use wire_server::serve_two_anthropic_requests;

const WIRE_TEST: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_client::wire_tests::consecutive_v2_calls_keep_wire_system_and_tools_stable";
const CHILD_ENV: &str = "ARCHON_WORKFLOW_WIRE_TEST_CHILD";

struct WireHarness {
    _project: tempfile::TempDir,
    learning_db_path: std::path::PathBuf,
    client: LiveV2AgentClient,
    _tui_rx: archon_tui::event_channel::TuiEventReceiver,
    captured: tokio::sync::oneshot::Receiver<Vec<Vec<u8>>>,
}

fn anthropic_provider(url: String) -> Arc<dyn LlmProvider> {
    Arc::new(AnthropicProvider::new(AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(
            IdentityMode::Clean,
            "workflow-wire-test".into(),
            "device-test".into(),
            String::new(),
        ),
        Some(url),
    )))
}

/// Install the executor; returns the agent names it can resolve, for the
/// client (see `fixture_agent_registry`).
fn install_wire_executor(provider: Arc<dyn LlmProvider>, root: &std::path::Path) -> Vec<String> {
    let agent_config = AgentConfig {
        session_id: "workflow-wire-test".into(),
        working_dir: root.to_path_buf(),
        ..AgentConfig::default()
    };
    let (agents, agent_names) =
        crate::command::workflow_live::workflow_live_test_support::fixture_agent_registry(root);
    let executor = AgentSubagentExecutor::new(
        provider,
        create_default_registry(root.to_path_buf(), None),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(agents)),
        None,
        None,
        root.to_path_buf(),
        "workflow-wire-test".into(),
        "claude-sonnet-4-6".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(agent_config),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "workflow-wire-test".into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
    agent_names
}

async fn wire_harness() -> WireHarness {
    let project = tempfile::tempdir().expect("project directory");
    let root = project.path().to_path_buf();
    let learning_db_path = root.join(".archon").join("learning-state.db");
    // SAFETY: this fixture executes in an isolated child process.
    unsafe {
        crate::test_env::set_var("ARCHON_LEARNING_DB_PATH", &learning_db_path);
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (captured_tx, captured) = tokio::sync::oneshot::channel();
    tokio::spawn(serve_two_anthropic_requests(listener, captured_tx));
    let provider = crate::runtime::provider_observer::observe_llm_provider_with_profile(
        anthropic_provider(url),
        "workflow-wire-test",
        None,
    )
    .await;
    let agent_names = install_wire_executor(Arc::clone(&provider), &root);
    let llm = crate::command::pipeline_workflow_llm::subagent_workflow_client_for_test(
        provider,
        "workflow-wire-test",
        root.clone(),
        crate::command::pipeline_workflow_llm::TestClientFallback::Provider,
    );
    let (ui_sink, tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        llm,
        ui_sink,
        agent_names,
        "workflow-wire-test".into(),
        Some(root.display().to_string()),
        Some(30),
    );
    WireHarness {
        _project: project,
        learning_db_path,
        client,
        _tui_rx: tui_rx,
        captured,
    }
}

fn workflow_wire_request(call_id: &str, wave: u64, root: &str) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        role: "researcher".to_string(),
        task: "inspect repository".to_string(),
        constraints: vec!["read only".to_string()],
        input: serde_json::json!({
            "task_universe": {
                "schema_version": "workflow-v2-task-universe-v1",
                "source_roots": ["project-tasks"],
                "tasks": [{"canonical_task_id":"TASK-1","description":"stable task"}]
            },
            "wave": wave
        }),
        repository_root: Some(root.to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

fn run_isolated_child() {
    let executable = std::env::current_exe().expect("current test executable");
    let mut child = std::process::Command::new(executable)
        .arg("--exact")
        .arg(WIRE_TEST)
        .arg("--nocapture")
        .env(CHILD_ENV, "execute-full-wire-test")
        .env(crate::test_env::CHILD, WIRE_TEST)
        .spawn()
        .expect("run isolated workflow wire child");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().expect("kill timed-out child");
            child.wait().expect("reap timed-out child");
            panic!("isolated workflow wire child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.success(), "isolated workflow wire child failed");
}

fn run_wire_child_sync() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("wire test runtime")
        .block_on(run_wire_child());
}

async fn run_wire_child() {
    let harness = wire_harness().await;
    let root = harness._project.path().display().to_string();
    let first = workflow_wire_request("inventory-wave-1", 1, &root);
    let second = workflow_wire_request("inventory-wave-2", 2, &root);
    let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
    for request in [&first, &second] {
        harness
            .client
            .run_agent_request(request, adapter.build_prompt_parts(request).invocation)
            .await
            .expect("workflow wire call");
    }
    let bodies = tokio::time::timeout(std::time::Duration::from_secs(10), harness.captured)
        .await
        .expect("capture server timed out")
        .expect("captured bodies");
    assert_wire_bodies(&bodies);
    let wire_model = bodies
        .first()
        .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
        .and_then(|body| body["model"].as_str().map(str::to_owned))
        .expect("wire model");
    assert_wire_usage(&harness.learning_db_path, &wire_model);
}

#[test]
fn wire_usage_assertion_stays_within_function_size_limit() {
    let source = include_str!("workflow_live_v2_wire_tests.rs");
    let signature = ["fn assert_wire_", "usage(path:"].concat();
    let start = source.find(&signature).expect("usage assertion");
    let function = source[start..]
        .split_once("\nfn assert_wire_usage_metadata")
        .expect("next function")
        .0;

    assert!(
        function.lines().count() < 50,
        "assert_wire_usage spans {} lines",
        function.lines().count()
    );
}

fn assert_wire_usage(path: &std::path::Path, wire_model: &str) {
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().expect("UTF-8 learning path"),
        "reopen workflow wire learning db",
    )
    .expect("learning db");
    let rows = list_llm_call_usage(
        &db,
        &LlmCallUsageScope::new(Some("workflow-wire-test"), Some("workflow-wire-test")),
    )
    .expect("list workflow wire usage");
    assert_wire_usage_metadata(&rows, wire_model);
    assert_eq!(sorted_usage(&rows), expected_usage());
    assert_eq!(usage_totals(&rows), (24, 3, 11, 16));
}

fn assert_wire_usage_metadata(rows: &[LlmCallUsageRecord], wire_model: &str) {
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|row| {
            row.run_id.as_deref() == Some("workflow-wire-test")
                && row.session_id.as_deref() == Some("workflow-wire-test")
                && row.provider_id == "anthropic"
                && row.model_id == wire_model
                && row.terminal_status == "succeeded"
        }),
        "unexpected workflow wire usage rows: {rows:#?}"
    );
}

fn sorted_usage(
    rows: &[LlmCallUsageRecord],
) -> Vec<(
    UsageAvailability,
    UsageAvailability,
    UsageAvailability,
    UsageAvailability,
)> {
    let mut usage = rows
        .iter()
        .map(|row| {
            (
                row.input_tokens.clone(),
                row.cache_creation_input_tokens.clone(),
                row.cache_read_input_tokens.clone(),
                row.output_tokens.clone(),
            )
        })
        .collect::<Vec<_>>();
    usage.sort_by_key(|values| match values.0 {
        UsageAvailability::Known(value) => value,
        UsageAvailability::Unavailable => u64::MAX,
    });
    usage
}

fn expected_usage() -> Vec<(
    UsageAvailability,
    UsageAvailability,
    UsageAvailability,
    UsageAvailability,
)> {
    vec![
        (
            UsageAvailability::Known(11),
            UsageAvailability::Known(3),
            UsageAvailability::Known(0),
            UsageAvailability::Known(7),
        ),
        (
            UsageAvailability::Known(13),
            UsageAvailability::Known(0),
            UsageAvailability::Known(11),
            UsageAvailability::Known(9),
        ),
    ]
}

fn usage_totals(rows: &[LlmCallUsageRecord]) -> (u64, u64, u64, u64) {
    rows.iter().fold((0, 0, 0, 0), |totals, row| {
        (
            totals.0 + known_usage(&row.input_tokens),
            totals.1 + known_usage(&row.cache_creation_input_tokens),
            totals.2 + known_usage(&row.cache_read_input_tokens),
            totals.3 + known_usage(&row.output_tokens),
        )
    })
}

fn known_usage(usage: &UsageAvailability) -> u64 {
    match usage {
        UsageAvailability::Known(value) => *value,
        UsageAvailability::Unavailable => panic!("provider usage must remain available"),
    }
}

fn assert_wire_bodies(raw: &[Vec<u8>]) {
    assert_eq!(raw.len(), 2);
    let bodies = raw
        .iter()
        .map(|body| serde_json::from_slice::<serde_json::Value>(body).expect("request JSON"))
        .collect::<Vec<_>>();
    assert_wire_tools(&bodies);
    assert_wire_system(&bodies);
    assert_wire_messages(&bodies);
}

fn assert_wire_tools(bodies: &[serde_json::Value]) {
    let tools = bodies[0]["tools"].as_array().expect("wire tools array");
    for (name, required_property) in [
        ("Read", "file_path"),
        ("Grep", "pattern"),
        ("Glob", "pattern"),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool.get("name").and_then(serde_json::Value::as_str) == Some(name))
            .unwrap_or_else(|| panic!("missing wire tool {name}"));
        let properties = tool
            .pointer("/input_schema/properties")
            .and_then(serde_json::Value::as_object)
            .unwrap_or_else(|| panic!("missing input schema properties for {name}"));
        assert!(
            properties.contains_key(required_property),
            "{name} schema missing {required_property}"
        );
    }
    assert_eq!(
        serde_json::to_vec(&bodies[0]["tools"]).unwrap(),
        serde_json::to_vec(&bodies[1]["tools"]).unwrap()
    );
}

fn assert_wire_system(bodies: &[serde_json::Value]) {
    let blocks = bodies[0]["system"].as_array().expect("wire system array");
    assert!(!blocks.is_empty(), "wire system is empty");
    let first = bodies[0]["system"].to_string();
    assert!(first.contains("Archon Workflow V2 Stable Context"));
    assert!(first.contains("stable task"));
    assert_eq!(
        serde_json::to_vec(&bodies[0]["system"]).unwrap(),
        serde_json::to_vec(&bodies[1]["system"]).unwrap()
    );
    assert!(
        blocks
            .iter()
            .all(|block| block.get("cache_control").is_none())
    );
    assert!(!first.contains("inventory-wave-1"));
    assert!(!bodies[1]["system"].to_string().contains("inventory-wave-2"));
}

fn assert_wire_messages(bodies: &[serde_json::Value]) {
    let first = bodies[0]["messages"].to_string();
    let second = bodies[1]["messages"].to_string();
    assert!(first.contains("inventory-wave-1"));
    assert!(first.contains("\\\"wave\\\":1"));
    assert!(second.contains("inventory-wave-2"));
    assert!(second.contains("\\\"wave\\\":2"));
}

#[test]
fn consecutive_v2_calls_keep_wire_system_and_tools_stable() {
    match std::env::var(CHILD_ENV) {
        Ok(value) => {
            assert_eq!(value, "execute-full-wire-test", "unexpected child marker");
            run_wire_child_sync();
        }
        Err(std::env::VarError::NotPresent) => run_isolated_child(),
        Err(error) => panic!("invalid child marker: {error}"),
    }
}
