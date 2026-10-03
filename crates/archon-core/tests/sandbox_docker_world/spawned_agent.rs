//! #201 Phase 4: a spawned agent in the same container world. A module of
//! `sandbox_docker_world.rs`, so `--ignored` still runs the whole docker
//! suite as one test binary.

use std::path::Path;

use archon_core::sandbox::{DockerFs, DockerSandboxBackend};
use archon_permissions::sandbox::SandboxScope;
use archon_tools::filesystem::FileSystem;

use super::docker_config;

/// Drives a subagent's tool round the way a provider would.
///
/// One `Bash` call, then a text turn that ends the run. Keeps every request it
/// was handed: the second one carries the first turn's `tool_result` blocks,
/// which is where the container's own answer arrives.
struct BashThenText {
    command: String,
    calls: std::sync::atomic::AtomicU32,
    requests: std::sync::Arc<std::sync::Mutex<Vec<archon_llm::provider::LlmRequest>>>,
}

#[async_trait::async_trait]
impl archon_llm::provider::LlmProvider for BashThenText {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<archon_llm::provider::ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: archon_llm::provider::ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        request: archon_llm::provider::LlmRequest,
    ) -> Result<
        tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>,
        archon_llm::provider::LlmError,
    > {
        use archon_llm::streaming::StreamEvent;
        use std::sync::atomic::Ordering;

        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        self.requests
            .lock()
            .expect("requests mutex poisoned")
            .push(request);

        let mut events = vec![StreamEvent::MessageStart {
            id: "msg-1".into(),
            model: "mock".into(),
            usage: archon_llm::types::Usage::default(),
        }];
        if first {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: archon_llm::types::ContentBlockType::ToolUse,
                    tool_use_id: Some("tool-1".into()),
                    tool_name: Some("Bash".into()),
                },
                StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: serde_json::json!({ "command": self.command }).to_string(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
            ]);
        } else {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: archon_llm::types::ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                },
                StreamEvent::TextDelta {
                    index: 0,
                    text: "done".into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
            ]);
        }
        events.push(StreamEvent::MessageStop);

        let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(
        &self,
        _request: archon_llm::provider::LlmRequest,
    ) -> Result<archon_llm::provider::LlmResponse, archon_llm::provider::LlmError> {
        unimplemented!()
    }
}

/// The workflow primitive, end to end, under `backend = "docker"`.
///
/// `w.agent()`, `w.agents()`, `w.parallel()` and `w.pipeline()` all bottom out
/// in one subagent spawned from a sandboxed parent context — `run_subagent`
/// into `AgentSubagentExecutor::run_to_completion`, which is what runs here.
/// Above that sits only the script layer, which decides *which* calls to make
/// and has no opinion about the world they run in.
///
/// Two facts prove the container answered rather than the host: the shell sees
/// exactly the loopback interface, which is what `--network none` leaves and no
/// host has; and the bytes it wrote to `/workspace` arrive on the host through
/// the bind mount, at the path the docker filesystem translates that container
/// path to. Which filesystem object the child holds is
/// `subagent_sandbox_inheritance` — here the child's working directory is the
/// parent's, so the two are the same allocation by construction.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_spawned_agent_runs_its_bash_in_the_parents_container() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().expect("tempdir");
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));

    let mut tool_registry = archon_core::dispatch::ToolRegistry::new();
    tool_registry.register(Box::new(archon_tools::bash::BashTool::default()));

    let executor = archon_core::subagent_executor::AgentSubagentExecutor::new(
        Arc::new(BashThenText {
            command: "printf 'written by the subagent\\n' > /workspace/from_subagent.txt; \
                      printf 'NETS=%s\\n' \"$(ls /sys/class/net | tr '\\n' '+')\""
                .into(),
            calls: std::sync::atomic::AtomicU32::new(0),
            requests: Arc::clone(&requests),
        }),
        tool_registry,
        Arc::new(tokio::sync::Mutex::new(
            archon_core::subagent::SubagentManager::new(4),
        )),
        Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::load(dir.path()),
        )),
        None,
        None,
        dir.path().to_path_buf(),
        "docker-world-session".into(),
        "mock-model".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(archon_core::agent::AgentConfig::default()),
        Arc::new(archon_llm::identity::IdentityProvider::new(
            archon_llm::identity::IdentityMode::Clean,
            "docker-world-session".into(),
            String::new(),
            String::new(),
        )),
    );

    let parent_fs: Arc<dyn FileSystem> = Arc::new(DockerFs::new(dir.path()));
    let parent_ctx = archon_tools::tool::ToolContext {
        working_dir: dir.path().to_path_buf(),
        session_id: "docker-world-session".into(),
        sandbox: Some(Arc::new(DockerSandboxBackend::new(
            docker_config(),
            "rw",
            SandboxScope::Session,
        ))),
        fs: Some(Arc::clone(&parent_fs)),
        ..archon_tools::tool::ToolContext::default()
    };

    archon_tools::subagent_executor::SubagentExecutor::run_to_completion(
        &executor,
        uuid::Uuid::new_v4().to_string(),
        archon_tools::subagent_request::SubagentRequest {
            prompt: "run one command in your world".into(),
            model: None,
            allowed_tools: vec!["Bash".into()],
            max_turns: 4,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
            read_roots: Vec::new(),
            write_roots: Vec::new(),
            provider_env: None,
        },
        parent_ctx,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("the subagent run completes");

    let transcript = requests
        .lock()
        .expect("requests mutex poisoned")
        .iter()
        .map(|request: &archon_llm::provider::LlmRequest| {
            serde_json::Value::Array(request.messages.clone()).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        transcript.contains("NETS=") && !transcript.contains("eth0"),
        "the subagent's shell can see a real network interface, so it ran on the \
         host rather than in the parent's --network none container: {transcript}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("from_subagent.txt"))
            .expect("the subagent's write reached the host through the mount"),
        "written by the subagent\n"
    );
    assert_eq!(
        parent_fs
            .read_to_string(Path::new("/workspace/from_subagent.txt"))
            .await
            .expect("the path the subagent's own container named"),
        "written by the subagent\n"
    );
}
