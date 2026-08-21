//! The acceptance criterion for #201: one world, not two.
//!
//! > `Read` after a `Bash` write returns the written bytes under every backend
//! > and workspace mode.
//!
//! Everything else in the issue is machinery in service of that sentence. It is
//! also the one property a unit test cannot establish, because the failure it
//! guards against is precisely that the host and the container are different
//! filesystems — a fake world proves the plumbing, not the agreement.
//!
//! These are `#[ignore]` because they need a working Docker daemon and the
//! `ubuntu:24.04` image present locally (the backend runs with `--pull never`
//! on purpose, so it can never reach the network mid-session). Run them with:
//!
//! ```text
//! docker pull ubuntu:24.04
//! cargo test -p archon-core --test sandbox_docker_world -- --ignored --nocapture
//! ```
//!
//! Marked ignored rather than silently skipped when Docker is absent: a test
//! that quietly passes on a machine that could not run it reports coverage
//! nobody has.

use std::path::{Path, PathBuf};

use archon_core::sandbox::{DockerConfig, DockerFs, DockerSandboxBackend};
use archon_permissions::sandbox::{
    SandboxBackend, SandboxCommandRequest, SandboxTerminal, SandboxTerminalRequest,
};
use archon_tools::filesystem::FileSystem;

fn docker_config() -> DockerConfig {
    DockerConfig {
        enabled: true,
        ..DockerConfig::default()
    }
}

fn request(working_dir: &Path, command: &str) -> SandboxCommandRequest {
    SandboxCommandRequest {
        command: command.to_string(),
        working_dir: working_dir.to_path_buf(),
        timeout_ms: 120_000,
        max_output_bytes: 64 * 1024,
        env: Vec::new(),
    }
}

/// The headline case: the container writes, the agent reads, same bytes.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn read_returns_the_bytes_bash_wrote_in_the_container() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    let result = backend
        .execute_bash(request(
            dir.path(),
            "printf 'written inside the container\\n' > /workspace/from_bash.txt",
        ))
        .await
        .expect("the docker backend executes bash");
    assert!(
        !result.is_error,
        "the container write itself failed: {}",
        result.content
    );

    let seen = fs
        .read_to_string(Path::new("/workspace/from_bash.txt"))
        .await
        .expect("the agent reads the path the container named");

    assert_eq!(seen, "written inside the container\n");
}

/// And the other direction, which is the one that actually bites: the agent
/// writes, then a command inside the container has to see it. A `Write` that
/// landed on the host while `Bash` ran in a container would pass the read-back
/// above and fail here.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn bash_sees_the_bytes_the_agent_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    fs.write(
        Path::new("/workspace/from_agent.txt"),
        b"written by the agent\n",
    )
    .await
    .expect("agent write");

    let result = backend
        .execute_bash(request(dir.path(), "cat /workspace/from_agent.txt"))
        .await
        .expect("the docker backend executes bash");

    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.contains("written by the agent"),
        "the container could not see the agent's write: {}",
        result.content
    );
}

/// A path taken verbatim from container output must resolve, including one
/// several directories deep — the shape a compiler error or a `find` prints.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_nested_path_printed_by_the_container_resolves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");
    let fs = DockerFs::new(dir.path());

    let result = backend
        .execute_bash(request(
            dir.path(),
            "mkdir -p /workspace/src/deep && printf 'fn main() {}' > /workspace/src/deep/main.rs \
             && find /workspace -name main.rs",
        ))
        .await
        .expect("the docker backend executes bash");
    assert!(!result.is_error, "{}", result.content);

    let printed = result
        .content
        .lines()
        .map(str::trim)
        .find(|line| line.ends_with("main.rs"))
        .expect("find printed the path")
        .to_string();
    assert_eq!(
        printed, "/workspace/src/deep/main.rs",
        "the container names paths under the mount point"
    );

    let seen = fs
        .read_to_string(&PathBuf::from(&printed))
        .await
        .expect("the exact path the container printed");

    assert_eq!(seen, "fn main() {}");
}

/// #201 Phase 6, end to end: a terminal opens *inside* the container.
///
/// The unit tests pin the `docker run` arguments; only this one proves the
/// shell that comes up is in the container. The two facts it asks the shell for
/// are the container's, not the image's: uid 0, because the test process is
/// not root and a host shell would inherit its uid; and one network interface,
/// because the backend runs with `--network none` while any host has more.
/// Comparing distributions would prove nothing on a machine whose host happens
/// to be the same one as the image.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_terminal_opens_inside_the_container() {
    // No chmod: the container runs as the host uid, so a 0700 tempdir owned by
    // the invoking user is writable from inside. Opening the directory up would
    // hide a regression in exactly that.
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "rw");

    let SandboxTerminal::Open(command) = backend.terminal(&SandboxTerminalRequest {
        shell: None,
        workspace: dir.path().to_path_buf(),
        cwd: dir.path().to_path_buf(),
    }) else {
        panic!("the docker backend must open a terminal in the container");
    };
    assert_eq!(command.shell, "bash");

    let mut builder = archon_pty::CommandBuilder::new(&command.program);
    builder.args(&command.args);
    let session = archon_pty::PtySession::spawn_headless(
        builder,
        archon_pty::PtySize {
            rows: 50,
            cols: 240,
            pixel_width: 0,
            pixel_height: 0,
        },
    )
    .expect("the docker terminal spawns");
    let (control, mut output) = session.split();

    // The network namespace is the discriminator, not the uid: the container
    // runs as the host user by design (see `host_identity_args`), so `id -u`
    // now matches the host and proves nothing. `--network none` leaves exactly
    // one interface, where a host shell sees eth0 and friends.
    control.send_input(
        b"printf 'from the terminal\\n' > /workspace/from_terminal.txt; \
          printf 'NETS=%s\\n' \"$(ls /sys/class/net | tr '\\n' '+')\"\n"
            .to_vec(),
    );

    // Waiting on the expanded text, never on anything the command line itself
    // contains: the PTY echoes what was typed, so a marker present in both
    // would end the wait before the shell had answered.
    let mut seen = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while std::time::Instant::now() < deadline && !seen.contains("NETS=lo") {
        match tokio::time::timeout(std::time::Duration::from_secs(5), output.recv()).await {
            Ok(Some(chunk)) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    control.kill();

    assert!(
        seen.contains("NETS=lo+\r") || seen.contains("NETS=lo+\n"),
        "the shell sees more than the loopback interface, so it is not in the \
         --network none container — it is a host shell: {seen}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("from_terminal.txt"))
            .expect("the terminal's write reached the host through the mount"),
        "from the terminal\n"
    );
}

/// A read-only workspace must actually be read-only in the container. If this
/// ever passes, `workspace_access = "ro"` is decoration.
#[tokio::test]
#[ignore = "requires a Docker daemon and the ubuntu:24.04 image"]
async fn a_readonly_workspace_refuses_a_container_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = DockerSandboxBackend::new(docker_config(), "ro");

    let result = backend
        .execute_bash(request(
            dir.path(),
            "printf 'should not land' > /workspace/nope.txt",
        ))
        .await
        .expect("the docker backend executes bash");

    assert!(
        result.is_error || result.exit_code != Some(0),
        "a read-only mount accepted a write: {}",
        result.content
    );
    assert!(
        !dir.path().join("nope.txt").exists(),
        "the host file was created despite a read-only workspace"
    );
}

// ---------------------------------------------------------------------------
// #201 Phase 4 — a spawned agent in the same container world
// ---------------------------------------------------------------------------

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
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
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
        sandbox: Some(Arc::new(DockerSandboxBackend::new(docker_config(), "rw"))),
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
        transcript.contains("NETS=lo+"),
        "the subagent's shell sees more than the loopback interface, so it ran on \
         the host rather than in the parent's --network none container: {transcript}"
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
