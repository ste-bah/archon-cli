use super::*;
use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::create_default_registry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::anthropic::AnthropicClient;
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmProvider;
use archon_llm::providers::AnthropicProvider;
use archon_llm::types::Secret;
use archon_pipeline::llm_adapter::ProviderLlmAdapter;
use archon_pipeline::subagent_adapter::SubagentPipelineClient;
use archon_tools::subagent_executor::install_subagent_executor;
use archon_tools::tool::ToolContext;
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostMethod};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const WIRE_TEST: &str = "command::workflow_live::workflow_live_v2::workflow_live_v2_client::wire_tests::consecutive_v2_calls_keep_wire_system_and_tools_stable";
const CHILD_ENV: &str = "ARCHON_WORKFLOW_WIRE_TEST_CHILD";

struct WireHarness {
    _project: tempfile::TempDir,
    client: LiveV2AgentClient,
    captured: tokio::sync::oneshot::Receiver<Vec<Vec<u8>>>,
}

async fn accept_raw_request(listener: &TcpListener) -> (tokio::net::TcpStream, Vec<u8>) {
    let (mut socket, _) = listener.accept().await.expect("accept request");
    let mut request = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.expect("read request");
        assert!(read > 0, "connection closed before headers completed");
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let content_length = content_length(&request[..header_end]);
    while request.len() - header_end < content_length {
        let mut buffer = [0; 1024];
        let read = socket.read(&mut buffer).await.expect("read body");
        assert!(read > 0, "connection closed before body completed");
        request.extend_from_slice(&buffer[..read]);
    }
    (
        socket,
        request[header_end..header_end + content_length].to_vec(),
    )
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse().ok())
        .expect("content length")
}

async fn serve_two_anthropic_requests(
    listener: TcpListener,
    captured: tokio::sync::oneshot::Sender<Vec<Vec<u8>>>,
) {
    let mut bodies = Vec::new();
    for index in 0..2 {
        let (mut socket, body) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            accept_raw_request(&listener),
        )
        .await
        .expect("request capture timed out");
        bodies.push(body);
        write_anthropic_response(&mut socket, index).await;
    }
    captured.send(bodies).expect("send captured bodies");
}

async fn write_anthropic_response(socket: &mut tokio::net::TcpStream, index: usize) {
    let response = format!(
        concat!(
            "event: message_start\n",
            "data: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-{}\",\"model\":\"claude-sonnet-4-6\",\"usage\":{{\"input_tokens\":11,\"output_tokens\":0}}}}}}\n\n",
            "event: content_block_start\n",
            "data: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
            "event: content_block_delta\n",
            "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"recorded\"}}}}\n\n",
            "event: content_block_stop\n",
            "data: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n",
            "event: message_stop\n",
            "data: {{\"type\":\"message_stop\"}}\n\n"
        ),
        index
    );
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
        response.len()
    );
    socket
        .write_all(headers.as_bytes())
        .await
        .expect("write headers");
    socket
        .write_all(response.as_bytes())
        .await
        .expect("write body");
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

fn install_wire_executor(provider: Arc<dyn LlmProvider>, root: &std::path::Path) {
    let executor = AgentSubagentExecutor::new(
        provider,
        create_default_registry(root.to_path_buf(), None),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(1))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(root))),
        None,
        None,
        root.to_path_buf(),
        "workflow-wire-test".into(),
        "claude-sonnet-4-6".into(),
        Vec::new(),
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(None)),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "workflow-wire-test".into(),
            String::new(),
            String::new(),
        )),
    );
    install_subagent_executor(Arc::new(executor));
}

async fn wire_harness() -> WireHarness {
    let project = tempfile::tempdir().expect("project directory");
    let root = project.path().to_path_buf();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let (captured_tx, captured) = tokio::sync::oneshot::channel();
    tokio::spawn(serve_two_anthropic_requests(listener, captured_tx));
    let provider = anthropic_provider(url);
    install_wire_executor(Arc::clone(&provider), &root);
    let raw: Arc<dyn LlmClient> =
        Arc::new(ProviderLlmAdapter::new(Arc::clone(&provider)).with_origin("workflow-wire-test"));
    let llm = SubagentPipelineClient::with_provider(
        raw,
        ToolContext {
            working_dir: root.clone(),
            ..ToolContext::default()
        },
        provider,
    );
    let (tui_tx, _tui_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(llm),
        tui_tx,
        Vec::new(),
        "workflow-wire-test".into(),
        Some(root.display().to_string()),
        Some(30),
    );
    WireHarness {
        _project: project,
        client,
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
