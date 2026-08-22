//! A tool running inside a subagent must be able to name the subagent that
//! called it. `ToolContext::session_id` cannot do that — it is the parent's,
//! copied verbatim into every child — so `ToolContext::subagent_id` carries
//! the run's registration id instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::hooks::{
    HookCallback, HookCallbackEntry, HookContext, HookEvent, HookRegistry, HookResult,
    SourceAuthority,
};
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::subagent_executor::SubagentExecutor;
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Recording tool — captures the `ToolContext` it was handed
// ---------------------------------------------------------------------------

/// What the tool saw: its own `ToolContext`, plus the ids the
/// `SubagentManager` had registered as running at that instant. Sampling the
/// manager from inside the tool call keeps the comparison deterministic —
/// polling it from the outside would race the run's own registration and
/// cleanup.
type Captured = Arc<std::sync::Mutex<Vec<(ToolContext, Vec<String>)>>>;

struct RecordIdentityTool {
    captured: Captured,
    manager: Arc<tokio::sync::Mutex<SubagentManager>>,
}

#[async_trait::async_trait]
impl Tool for RecordIdentityTool {
    fn name(&self) -> &str {
        "RecordIdentity"
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "Records the ToolContext it receives."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let registered: Vec<String> = self
            .manager
            .lock()
            .await
            .list_active()
            .iter()
            .map(|info| info.id.clone())
            .collect();
        self.captured
            .lock()
            .expect("captured contexts mutex poisoned")
            .push((ctx.clone(), registered));
        ToolResult::success("recorded")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// Mock provider — one tool_use turn, then a final text turn
// ---------------------------------------------------------------------------

struct ToolThenTextProvider {
    calls: AtomicU32,
}

#[async_trait::async_trait]
impl LlmProvider for ToolThenTextProvider {
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
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
        let mut events = vec![StreamEvent::MessageStart {
            id: "msg-1".into(),
            model: "mock".into(),
            usage: Usage::default(),
        }];
        if first {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some("tool-1".into()),
                    tool_name: Some("RecordIdentity".into()),
                },
                StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: "{}".into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
            ]);
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
// Fixtures
// ---------------------------------------------------------------------------

const PARENT_SESSION_ID: &str = "subagent-identity-parent-session";

struct Fixture {
    executor: Arc<AgentSubagentExecutor>,
    captured: Captured,
}

fn fixture(hook_registry: Option<Arc<HookRegistry>>) -> Fixture {
    fixture_with(
        Arc::new(ToolThenTextProvider {
            calls: AtomicU32::new(0),
        }),
        hook_registry,
    )
}

/// As [`fixture`], but with the provider chosen by the caller.
///
/// Split out for #189 Phase 5, so this same loop can be driven by a replay
/// provider reading cassettes instead of by the mock below.
fn fixture_with(
    provider: Arc<dyn LlmProvider>,
    hook_registry: Option<Arc<HookRegistry>>,
) -> Fixture {
    let captured: Captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let manager = Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4)));
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Box::new(RecordIdentityTool {
        captured: Arc::clone(&captured),
        manager: Arc::clone(&manager),
    }));

    let project_dir = std::env::temp_dir();
    let executor = Arc::new(AgentSubagentExecutor::new(
        provider,
        tool_registry,
        Arc::clone(&manager),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(&project_dir))),
        hook_registry,
        None,
        project_dir,
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

    Fixture { executor, captured }
}

fn request() -> SubagentRequest {
    SubagentRequest {
        prompt: "record your identity".into(),
        model: None,
        allowed_tools: vec!["RecordIdentity".into()],
        max_turns: 4,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_inside_subagent_sees_the_registered_subagent_id() {
    let fixture = fixture(None);
    let subagent_id = uuid::Uuid::new_v4().to_string();

    // The parent-side context carries no subagent identity; anything the tool
    // observes must have been stamped on by the executor.
    let parent_ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: PARENT_SESSION_ID.into(),
        ..ToolContext::default()
    };

    fixture
        .executor
        .run_to_completion(
            subagent_id.clone(),
            request(),
            parent_ctx,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("subagent run should complete");

    let contexts = fixture
        .captured
        .lock()
        .expect("captured contexts mutex poisoned");
    let (ctx, registered) = contexts
        .first()
        .expect("RecordIdentity should have been dispatched inside the subagent");

    assert_eq!(
        registered.as_slice(),
        std::slice::from_ref(&subagent_id),
        "exactly one subagent should have been registered while the tool ran"
    );
    assert_eq!(
        ctx.subagent_id.as_deref(),
        Some(subagent_id.as_str()),
        "tool context must carry the id the subagent was registered under"
    );
    assert_eq!(
        ctx.session_id, PARENT_SESSION_ID,
        "the parent session id must survive alongside the subagent id"
    );
    assert_ne!(
        ctx.subagent_id.as_deref(),
        Some(ctx.session_id.as_str()),
        "subagent id and session id must not collapse into one value"
    );
}

/// The same loop, offline (#189 Phase 5).
///
/// This is the acceptance criterion for cassettes: an agent-loop test that
/// already existed, driven end to end with no provider reachable. The first
/// pass records the mock's two turns; the second replays them, with the mock
/// left behind entirely — `ReplayProvider::replaying` holds no provider at all,
/// so a cassette miss cannot quietly become a live call.
///
/// It exercises the loop, not just the transport: turn one is a `tool_use` that
/// has to dispatch `RecordIdentity`, and turn two is the text that ends the run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_subagent_loop_runs_offline_from_cassettes() {
    let cassettes = tempfile::tempdir().expect("tempdir");
    let dir = cassettes.path().to_path_buf();

    let live = Arc::new(ToolThenTextProvider {
        calls: AtomicU32::new(0),
    });
    let recorded = run_once(Arc::new(archon_llm::replay::ReplayProvider::recording(
        Arc::clone(&live) as Arc<dyn LlmProvider>,
        dir.clone(),
    )))
    .await;
    assert_eq!(
        live.calls.load(Ordering::SeqCst),
        2,
        "the recording pass should have driven both turns of the loop"
    );

    let replayed = run_once(Arc::new(archon_llm::replay::ReplayProvider::replaying(dir))).await;

    assert_eq!(
        live.calls.load(Ordering::SeqCst),
        2,
        "the replay pass reached the live provider"
    );
    assert_eq!(
        replayed, recorded,
        "the replayed run dispatched a different tool sequence than the recorded one"
    );
    assert_eq!(
        replayed,
        vec!["RecordIdentity".to_string()],
        "the loop did not dispatch the recorded tool call"
    );
}

/// Run the subagent once and report which tools it dispatched.
async fn run_once(provider: Arc<dyn LlmProvider>) -> Vec<String> {
    let fixture = fixture_with(provider, None);
    let parent_ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: PARENT_SESSION_ID.into(),
        ..ToolContext::default()
    };
    fixture
        .executor
        .run_to_completion(
            uuid::Uuid::new_v4().to_string(),
            request(),
            parent_ctx,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .expect("subagent run should complete");

    let captured = fixture
        .captured
        .lock()
        .expect("captured contexts mutex poisoned");
    // Only the tool's own name is compared: the ids inside the context are
    // fresh per run by design, and asserting on them is what
    // `tool_inside_subagent_sees_the_registered_subagent_id` above is for.
    captured
        .iter()
        .map(|_| "RecordIdentity".to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_stop_callback_observes_the_stopping_subagent() {
    let hooks = Arc::new(HookRegistry::new());
    let seen: Arc<std::sync::Mutex<Vec<Option<String>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_in_cb = Arc::clone(&seen);
    let callback: HookCallback = Arc::new(move |ctx: &HookContext| {
        seen_in_cb
            .lock()
            .expect("seen mutex poisoned")
            .push(ctx.agent_id.clone());
        HookResult::allow()
    });
    hooks.register_callback(
        HookEvent::SubagentStop,
        HookCallbackEntry {
            name: "record-stopping-subagent".to_string(),
            callback,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    let fixture = fixture(Some(Arc::clone(&hooks)));
    let subagent_id = uuid::Uuid::new_v4().to_string();

    fixture
        .executor
        .on_visible_complete(subagent_id.clone(), Ok("finished".to_string()), false)
        .await;

    let seen = seen.lock().expect("seen mutex poisoned");
    assert_eq!(
        seen.as_slice(),
        &[Some(subagent_id)],
        "SubagentStop callback must be told which subagent stopped"
    );
}
