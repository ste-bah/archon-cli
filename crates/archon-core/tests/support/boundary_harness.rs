//! Shared by the #236 workspace-boundary tests: a provider that asks for a
//! fixed list of tool calls, one per turn, and records the result of each
//! call. The real executor and the real file tools run the calls.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
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
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::subagent_executor::{ExecutorError, SubagentExecutor};
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::subagent_session::{CompletedHistory, SubagentSession};
use archon_tools::tool::ToolContext;

/// One tool call: the tool's name and its input.
pub type Call = (&'static str, serde_json::Value);

/// Not a call: the provider answers with text on this turn, so the run ends.
/// The calls after it are made by the next run on the same [`Host`].
pub const STOP: Call = ("", serde_json::Value::Null);

/// What one tool call returned.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub is_error: bool,
    pub text: String,
}

struct ScriptedCalls {
    calls: Vec<Call>,
    turn: AtomicU32,
    last_messages: Mutex<Vec<serde_json::Value>>,
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedCalls {
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
        *self.last_messages.lock().unwrap() = request.messages.clone();
        let turn = self.turn.fetch_add(1, Ordering::SeqCst) as usize;
        let mut events = vec![StreamEvent::MessageStart {
            id: format!("msg-{turn}"),
            model: "mock".into(),
            usage: Usage::default(),
        }];
        if let Some((tool, input)) = self.calls.get(turn).filter(|(tool, _)| !tool.is_empty()) {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some(format!("tool-{turn}")),
                    tool_name: Some((*tool).into()),
                },
                StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: input.to_string(),
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

impl ScriptedCalls {
    /// The result of call `index`, read from the conversation the provider
    /// was last sent.
    fn outcome(&self, index: usize) -> Outcome {
        let id = format!("tool-{index}");
        let messages = self.last_messages.lock().unwrap();
        let block = messages
            .iter()
            .filter_map(|message| message["content"].as_array())
            .flatten()
            .find(|block| block["type"] == "tool_result" && block["tool_use_id"] == id.as_str())
            .unwrap_or_else(|| panic!("no tool result for {id} in {messages:?}"))
            .clone();
        let text = match &block["content"] {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        Outcome {
            is_error: block["is_error"].as_bool().unwrap_or(false),
            text,
        }
    }
}

/// A child spawn: where it runs, what it asks for, and the calls it makes.
pub struct Spawn<'a> {
    pub parent: ToolContext,
    pub cwd: &'a Path,
    pub isolation: Option<&'a str>,
    pub read_roots: Vec<String>,
    pub calls: Vec<Call>,
}

/// One executor and one scripted provider, kept across runs, so a second run
/// can continue the agent the first one ran, as a workflow's validation
/// repair does.
pub struct Host {
    provider: Arc<ScriptedCalls>,
    pub executor: Arc<AgentSubagentExecutor>,
    pub manager: Arc<tokio::sync::Mutex<SubagentManager>>,
    pub session: String,
    pub contexts: Arc<Mutex<Vec<ToolContext>>>,
    /// Each agent's completed history, as a workflow keeps it for a repair.
    pub histories: Mutex<HashMap<String, CompletedHistory>>,
}

impl Host {
    pub fn new(executor_dir: &Path, session: &str, calls: Vec<Call>) -> Self {
        Self::with_config(executor_dir, session, calls, AgentConfig::default())
    }

    /// As [`Host::new`], with the executor's isolation policy taken from
    /// `config`.
    pub fn with_config(
        executor_dir: &Path,
        session: &str,
        calls: Vec<Call>,
        config: AgentConfig,
    ) -> Self {
        Self::with_manager(
            executor_dir,
            session,
            calls,
            config,
            Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        )
    }

    pub fn with_manager(
        executor_dir: &Path,
        session: &str,
        calls: Vec<Call>,
        config: AgentConfig,
        manager: Arc<tokio::sync::Mutex<SubagentManager>>,
    ) -> Self {
        let provider = Arc::new(ScriptedCalls {
            calls,
            turn: AtomicU32::new(0),
            last_messages: Mutex::new(Vec::new()),
        });
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(archon_tools::file_write::WriteTool));
        tools.register(Box::new(archon_tools::file_read::ReadTool));
        let contexts = Arc::new(Mutex::new(Vec::new()));
        tools.register(Box::new(ContextProbe(contexts.clone())));
        tools.register(Box::new(archon_tools::bash::BashTool::default()));
        let executor = Arc::new(AgentSubagentExecutor::new(
            provider.clone(),
            tools,
            Arc::clone(&manager),
            Arc::new(std::sync::RwLock::new(AgentRegistry::load(executor_dir))),
            None,
            None,
            executor_dir.to_path_buf(),
            session.into(),
            "mock-model".into(),
            vec![],
            Arc::new(tokio::sync::Mutex::new("bypassPermissions".to_string())),
            Arc::new(config),
            Arc::new(IdentityProvider::new(
                IdentityMode::Clean,
                session.into(),
                String::new(),
                String::new(),
            )),
        ));
        Self {
            provider,
            executor,
            manager,
            session: session.into(),
            contexts,
            histories: Mutex::new(HashMap::new()),
        }
    }

    /// Run `request` as agent `agent_id` under `parent`, as a workflow's
    /// first call of it: a new history, kept for a repair.
    pub async fn spawn(
        &self,
        agent_id: &str,
        request: SubagentRequest,
        parent: ToolContext,
    ) -> Result<String, ExecutorError> {
        let history = CompletedHistory::default();
        self.histories
            .lock()
            .unwrap()
            .insert(agent_id.into(), history.clone());
        self.call(agent_id, request, parent, history, false).await
    }

    /// Continue `agent_id` as a workflow's validation repair does: the same
    /// id, the call's history, and the workflow's request and context again.
    pub async fn repair(
        &self,
        agent_id: &str,
        request: SubagentRequest,
        parent: ToolContext,
    ) -> Result<String, ExecutorError> {
        let history = self
            .histories
            .lock()
            .unwrap()
            .get(agent_id)
            .cloned()
            .unwrap_or_default();
        self.call(agent_id, request, parent, history, true).await
    }

    async fn call(
        &self,
        agent_id: &str,
        request: SubagentRequest,
        parent: ToolContext,
        history: CompletedHistory,
        continuing: bool,
    ) -> Result<String, ExecutorError> {
        let session = SubagentSession {
            agent_id: agent_id.into(),
            history,
            continuing,
        };
        archon_tools::subagent_session::scope(
            session,
            self.executor.run_to_completion(
                agent_id.into(),
                request,
                ToolContext {
                    session_id: self.session.clone(),
                    ..parent
                },
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await
    }

    /// The result of call `index`, counted over every run on this host.
    pub fn outcome(&self, index: usize) -> Outcome {
        self.provider.outcome(index)
    }

    /// The conversation the provider was last sent.
    pub fn last_messages(&self) -> Vec<serde_json::Value> {
        self.provider.last_messages.lock().unwrap().clone()
    }

    /// How many provider requests every run on this host has made.
    pub fn turns(&self) -> u32 {
        self.provider.turn.load(Ordering::SeqCst)
    }
}

/// The request the confinement tests spawn with.
pub fn request(cwd: &Path, isolation: Option<&str>, read_roots: Vec<String>) -> SubagentRequest {
    SubagentRequest {
        prompt: "run the calls".into(),
        model: None,
        allowed_tools: vec!["Write".into(), "Read".into()],
        max_turns: 16,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: Some(cwd.display().to_string()),
        isolation: isolation.map(str::to_string),
        read_roots,
        write_roots: Vec::new(),
        provider_env: None,
    }
}

/// Run `spawn` through the real executor. `Ok` holds each call's outcome
/// in order. `Err` is the spawn's own failure.
pub async fn run(spawn: Spawn<'_>) -> Result<Vec<Outcome>, ExecutorError> {
    let count = spawn.calls.len();
    let host = Host::new(
        &spawn.parent.working_dir,
        "subagent-workspace-boundary-session",
        spawn.calls,
    );
    host.spawn(
        &uuid::Uuid::new_v4().to_string(),
        request(spawn.cwd, spawn.isolation, spawn.read_roots),
        spawn.parent,
    )
    .await?;
    Ok((0..count).map(|index| host.outcome(index)).collect())
}

pub fn write(path: &Path, content: &str) -> Call {
    (
        "Write",
        serde_json::json!({"file_path": path.display().to_string(), "content": content}),
    )
}

pub fn read(path: &Path) -> Call {
    (
        "Read",
        serde_json::json!({"file_path": path.display().to_string()}),
    )
}

/// A parent whose working directory and extra directories are `dirs`, as a
/// workflow run's client context is.
pub fn parent(working_dir: &Path, extra_dirs: &[&Path]) -> ToolContext {
    ToolContext {
        working_dir: working_dir.to_path_buf(),
        extra_dirs: extra_dirs.iter().map(|dir| dir.to_path_buf()).collect(),
        ..ToolContext::default()
    }
}

pub fn real_temp() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    // The plain (non-verbatim) spelling: git refuses Windows `\\?\` paths.
    let root = archon_shell::paths::canonicalize(temp.path()).expect("real temp");
    (temp, root)
}

pub fn dir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

struct ContextProbe(Arc<Mutex<Vec<ToolContext>>>);
#[async_trait::async_trait]
impl archon_tools::tool::Tool for ContextProbe {
    fn name(&self) -> &str {
        "ContextProbe"
    }
    fn description(&self) -> &str {
        "Capture the effective context for this test."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn execute(
        &self,
        _: serde_json::Value,
        ctx: &ToolContext,
    ) -> archon_tools::tool::ToolResult {
        self.0.lock().unwrap().push(ctx.clone());
        archon_tools::tool::ToolResult::success("captured")
    }
    fn permission_level(&self, _: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::FILE_READ
    }
}
