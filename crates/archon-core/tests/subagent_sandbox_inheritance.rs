//! #201 Phase 4: a subagent runs in its parent's world, not beside it.
//!
//! The gate opened `ControlPlane` on the strength of one claim: a spawned child
//! carries the parent's sandbox backend and the parent's filesystem, and every
//! tool it calls is decided by that backend before it runs. These drive the
//! real spawn path — `AgentSubagentExecutor::run_to_completion`, the same entry
//! `run_subagent` and therefore every workflow primitive reaches — so cutting
//! the wiring in `build_child_tool_context` fails here instead of passing
//! against a reimplementation of it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::sandbox::{DockerConfig, DockerFs, DockerSandboxBackend};
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_permissions::{SandboxBackend, SandboxScope, ToolCapability};
use archon_tools::subagent_executor::SubagentExecutor;
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const PARENT_SESSION_ID: &str = "subagent-sandbox-parent-session";

/// The file both trees hold, with different bytes in each. Which of the two a
/// read through the child's filesystem returns is the whole question.
const MARKER: &str = "marker.txt";

// ---------------------------------------------------------------------------
// Probe tools
// ---------------------------------------------------------------------------

/// Captures the `ToolContext` the child handed it.
struct ReportWorld {
    captured: Arc<std::sync::Mutex<Vec<ToolContext>>>,
}

#[async_trait::async_trait]
impl Tool for ReportWorld {
    fn name(&self) -> &str {
        "ReportWorld"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "Records the ToolContext it receives."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        self.captured
            .lock()
            .expect("captured contexts mutex poisoned")
            .push(ctx.clone());
        ToolResult::success("REPORT_WORLD_RAN")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// A tool of one declared class that records whether it was ever entered.
///
/// The class is what the backend decides on, so two of these — one `Egress`,
/// one `ControlPlane` — are enough to show that the child's calls are routed
/// through the same table the parent's are, and that the table's Phase 4 answer
/// is what the child gets.
struct ClassProbe {
    name: &'static str,
    capability: ToolCapability,
    ran: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Tool for ClassProbe {
    fn name(&self) -> &str {
        self.name
    }

    fn capability(&self) -> ToolCapability {
        self.capability
    }

    fn description(&self) -> &str {
        "Records that it was entered."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.ran.store(true, Ordering::SeqCst);
        ToolResult::success(format!("{}_RAN", self.name))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// Mock provider — one tool_use turn over all three probes, then a text turn
// ---------------------------------------------------------------------------

/// Keeps every request it was handed.
///
/// The second one carries the first turn's `tool_result` blocks, which is the
/// only place a *denial* is observable: a refused tool never runs, so its own
/// flag cannot tell a refusal apart from a call that was never made.
struct ThreeProbesThenText {
    calls: AtomicU32,
    requests: Arc<std::sync::Mutex<Vec<LlmRequest>>>,
}

#[async_trait::async_trait]
impl LlmProvider for ThreeProbesThenText {
    fn name(&self) -> &str {
        "mock"
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
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        self.requests
            .lock()
            .expect("requests mutex poisoned")
            .push(request);

        let mut events = vec![StreamEvent::MessageStart {
            id: "msg-1".into(),
            model: "mock".into(),
            usage: Usage::default(),
        }];
        if first {
            for (index, tool) in [
                (0u32, "ReportWorld"),
                (1, "LeaveTheMachine"),
                (2, "ScheduleWork"),
            ] {
                events.extend([
                    StreamEvent::ContentBlockStart {
                        index,
                        block_type: ContentBlockType::ToolUse,
                        tool_use_id: Some(format!("tool-{index}")),
                        tool_name: Some(tool.into()),
                    },
                    StreamEvent::InputJsonDelta {
                        index,
                        partial_json: "{}".into(),
                    },
                    StreamEvent::ContentBlockStop { index },
                ]);
            }
        } else {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
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

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Spawned {
    child_contexts: Vec<ToolContext>,
    /// Every request the provider saw, as JSON, so a `tool_result` can be read
    /// back without reaching into the runner's private history.
    transcript: String,
    egress_ran: bool,
    control_plane_ran: bool,
    parent_backend: Arc<dyn SandboxBackend>,
}

/// Spawn one subagent whose parent context is sandboxed, and report what the
/// child saw.
///
/// `parent_dir` is the executor's working directory and `child_dir` is the cwd
/// the request asks for; keeping them different is what makes the filesystem
/// assertion able to tell "inherited" apart from "inherited and rerooted".
async fn spawn_sandboxed_child(
    parent_dir: &std::path::Path,
    child_dir: &std::path::Path,
) -> Spawned {
    let captured: Arc<std::sync::Mutex<Vec<ToolContext>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests: Arc<std::sync::Mutex<Vec<LlmRequest>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let egress_ran = Arc::new(AtomicBool::new(false));
    let control_plane_ran = Arc::new(AtomicBool::new(false));

    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Box::new(ReportWorld {
        captured: Arc::clone(&captured),
    }));
    tool_registry.register(Box::new(ClassProbe {
        name: "LeaveTheMachine",
        capability: ToolCapability::Egress,
        ran: Arc::clone(&egress_ran),
    }));
    tool_registry.register(Box::new(ClassProbe {
        name: "ScheduleWork",
        capability: ToolCapability::ControlPlane,
        ran: Arc::clone(&control_plane_ran),
    }));

    let executor = Arc::new(AgentSubagentExecutor::new(
        Arc::new(ThreeProbesThenText {
            calls: AtomicU32::new(0),
            requests: Arc::clone(&requests),
        }),
        tool_registry,
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(parent_dir))),
        None,
        None,
        parent_dir.to_path_buf(),
        PARENT_SESSION_ID.into(),
        "mock-model".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            PARENT_SESSION_ID.into(),
            String::new(),
            String::new(),
        )),
    ));

    // The real docker backend, not a stand-in: the class table it applies is
    // the thing under test, and a fake one would only prove that the child
    // consults *something*.
    let parent_backend: Arc<dyn SandboxBackend> = Arc::new(DockerSandboxBackend::new(
        DockerConfig {
            enabled: true,
            ..DockerConfig::default()
        },
        "rw",
        SandboxScope::Session,
    ));
    let parent_ctx = ToolContext {
        working_dir: parent_dir.to_path_buf(),
        session_id: PARENT_SESSION_ID.into(),
        sandbox: Some(Arc::clone(&parent_backend)),
        fs: Some(Arc::new(DockerFs::new(parent_dir))),
        ..ToolContext::default()
    };

    executor
        .run_to_completion(
            uuid::Uuid::new_v4().to_string(),
            SubagentRequest {
                prompt: "report the world you are in".into(),
                model: None,
                allowed_tools: vec![
                    "ReportWorld".into(),
                    "LeaveTheMachine".into(),
                    "ScheduleWork".into(),
                ],
                max_turns: 4,
                timeout_secs: 60,
                subagent_type: None,
                run_in_background: false,
                cwd: Some(child_dir.display().to_string()),
                isolation: None,
                provider_env: None,
            },
            parent_ctx,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("subagent run should complete");

    let transcript = requests
        .lock()
        .expect("requests mutex poisoned")
        .iter()
        .map(|request| serde_json::Value::Array(request.messages.clone()).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    Spawned {
        child_contexts: captured
            .lock()
            .expect("captured contexts mutex poisoned")
            .clone(),
        transcript,
        egress_ran: egress_ran.load(Ordering::SeqCst),
        control_plane_ran: control_plane_ran.load(Ordering::SeqCst),
        parent_backend,
    }
}

/// Two trees holding the same filename with different bytes.
fn two_trees() -> (tempfile::TempDir, tempfile::TempDir) {
    let parent = tempfile::tempdir().expect("tempdir");
    let child = tempfile::tempdir().expect("tempdir");
    std::fs::write(parent.path().join(MARKER), b"parent tree").expect("seed parent tree");
    std::fs::write(child.path().join(MARKER), b"child tree").expect("seed child tree");
    (parent, child)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The backend arrives by identity, not by shape. `Arc::ptr_eq` is the
/// assertion that cannot be satisfied by a child that built a backend of its
/// own from the same config and would therefore miss a `/sandbox off`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_tool_call_runs_under_the_parents_own_backend() {
    let (parent_dir, child_dir) = two_trees();

    let spawned = spawn_sandboxed_child(parent_dir.path(), child_dir.path()).await;

    let ctx = spawned
        .child_contexts
        .first()
        .expect("ReportWorld should have been dispatched inside the subagent");
    let child_backend = ctx
        .sandbox
        .as_ref()
        .expect("the child ran with no sandbox backend at all");

    assert!(
        Arc::ptr_eq(child_backend, &spawned.parent_backend),
        "the child holds a different backend than the parent installed"
    );
}

/// The filesystem has to arrive *and* be rerooted. `Bash` mounts
/// `ctx.working_dir`, so a child in its own cwd whose filesystem still pointed
/// at the parent's tree would read one tree and execute against another —
/// which is the split the whole issue exists to close, reintroduced at the one
/// place Archon runs the most agents at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_childs_filesystem_is_the_parents_world_rooted_at_the_childs_tree() {
    let (parent_dir, child_dir) = two_trees();

    let spawned = spawn_sandboxed_child(parent_dir.path(), child_dir.path()).await;

    let ctx = spawned
        .child_contexts
        .first()
        .expect("ReportWorld should have been dispatched inside the subagent");
    assert_eq!(
        ctx.working_dir,
        child_dir.path(),
        "the child did not run in the cwd it was given"
    );
    assert!(
        ctx.fs.is_some(),
        "the child fell back to the host filesystem while its shell runs in a container"
    );

    // A container path, because that is the only vocabulary the child's `Bash`
    // output speaks. The host filesystem cannot answer it at all, and a
    // filesystem left rooted at the parent answers it with the wrong file.
    let seen = ctx
        .fs()
        .read_to_string(std::path::Path::new("/workspace/marker.txt"))
        .await
        .expect("the child resolves a path its own container would print");

    assert_eq!(seen, "child tree");
}

/// The child is another caller of the gate, not a hole in it. `Egress` is still
/// refused inside a subagent, and refused by the backend rather than by a tool
/// that declined to run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_backend_still_refuses_egress_inside_the_child() {
    let (parent_dir, child_dir) = two_trees();

    let spawned = spawn_sandboxed_child(parent_dir.path(), child_dir.path()).await;

    assert!(
        !spawned.egress_ran,
        "an egress tool executed inside a sandboxed subagent"
    );
    assert!(
        spawned.transcript.contains("leaves the machine"),
        "the child's egress call was not refused by the backend: {}",
        spawned.transcript
    );
}

/// Phase 4's own arm, seen from inside a child: `ControlPlane` now runs, and
/// the tool that ran is the one the model asked for rather than an error string
/// shaped like a result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_plane_work_runs_inside_a_sandboxed_child() {
    let (parent_dir, child_dir) = two_trees();

    let spawned = spawn_sandboxed_child(parent_dir.path(), child_dir.path()).await;

    assert!(
        spawned.control_plane_ran,
        "a control-plane tool was refused inside a sandboxed subagent"
    );
    assert!(
        spawned.transcript.contains("ScheduleWork_RAN"),
        "the control-plane result never reached the child's history: {}",
        spawned.transcript
    );
}
