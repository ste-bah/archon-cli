use super::*;

// ---------------------------------------------------------------------------
// Shared session statistics -- updated by the agent, read by slash commands
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SessionStats {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub turn_count: u64,
    pub session_cost: f64,
    pub cache_stats: archon_context::cache::CacheStats,
}

impl Default for SessionStats {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            turn_count: 0,
            session_cost: 0.0,
            cache_stats: archon_context::cache::CacheStats::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Agent events -- emitted to the UI/consumer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AgentEvent {
    UserPromptReady,
    ApiCallStarted {
        model: String,
    },
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStarted {
        name: String,
        id: String,
    },
    ToolCallComplete {
        name: String,
        id: String,
        result: ToolResult,
    },
    PermissionRequired {
        tool: String,
        description: String,
    },
    PermissionGranted {
        tool: String,
    },
    PermissionDenied {
        tool: String,
    },
    TurnComplete {
        input_tokens: u64,
        output_tokens: u64,
    },
    Error(String),
    CompactionTriggered,
    SessionComplete,
    /// Emitted when the agent invokes AskUserQuestion and needs real user input.
    AskUser {
        question: String,
    },
    /// Emitted when SendMessage is invoked to deliver a message to another agent.
    MessageSent {
        target_agent_id: String,
        message: String,
    },
}

impl AgentEvent {
    /// TASK-AGS-108 ERR-ARCH-02: stable event name for WARN logging when
    /// the channel is closed. Returns the variant name as a static string.
    pub fn event_name(&self) -> &'static str {
        match self {
            AgentEvent::UserPromptReady => "UserPromptReady",
            AgentEvent::ApiCallStarted { .. } => "ApiCallStarted",
            AgentEvent::TextDelta(_) => "TextDelta",
            AgentEvent::ThinkingDelta(_) => "ThinkingDelta",
            AgentEvent::ToolCallStarted { .. } => "ToolCallStarted",
            AgentEvent::ToolCallComplete { .. } => "ToolCallComplete",
            AgentEvent::PermissionRequired { .. } => "PermissionRequired",
            AgentEvent::PermissionGranted { .. } => "PermissionGranted",
            AgentEvent::PermissionDenied { .. } => "PermissionDenied",
            AgentEvent::TurnComplete { .. } => "TurnComplete",
            AgentEvent::Error(_) => "Error",
            AgentEvent::CompactionTriggered => "CompactionTriggered",
            AgentEvent::SessionComplete => "SessionComplete",
            AgentEvent::AskUser { .. } => "AskUser",
            AgentEvent::MessageSent { .. } => "MessageSent",
        }
    }
}

/// Wrapper that timestamps when an AgentEvent was sent into the channel.
/// Used to compute send-to-render latency in the drain loop.
#[derive(Debug, Clone)]
pub struct TimestampedEvent {
    pub sent_at: std::time::Instant,
    pub inner: AgentEvent,
}

// ---------------------------------------------------------------------------
// Agent configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_tokens: u32,
    pub thinking_budget: u32,
    pub system_prompt: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub working_dir: std::path::PathBuf,
    pub session_id: String,
    /// Shared atomic flag for fast mode (toggled by /fast slash command).
    pub fast_mode: Arc<AtomicBool>,
    /// Shared effort level (toggled by /effort slash command).
    pub effort_level: Arc<Mutex<EffortLevel>>,
    /// Shared model name (toggled by /model slash command).
    pub model_override: Arc<Mutex<String>>,
    /// Shared permission mode (toggled by /permissions slash command: "auto", "ask", "yolo").
    pub permission_mode: Arc<Mutex<String>>,
    /// Fine-grained permission rules applied before mode-level preflight.
    pub permission_rules: archon_permissions::rules::RuleSet,
    /// Additional working directories added at runtime via `/add-dir`.
    pub extra_dirs: Arc<Mutex<Vec<std::path::PathBuf>>>,
    /// Maximum concurrent tool calls (1 = sequential, from config.tools.max_concurrency).
    pub max_tool_concurrency: usize,
    /// Maximum agentic loop iterations per process_message call (None = unlimited).
    pub max_turns: Option<u32>,
    /// TASK-AGS-107: parent CancellationToken for Ctrl+C propagation.
    /// When set, the agent threads this into ToolContext.cancel_parent so
    /// subagent spawns create child_token() chains. Set by the input
    /// handler spawn in main.rs.
    pub cancel_token: Option<tokio_util::sync::CancellationToken>,
    /// GHOST-006: sandbox enforcement backend. Injected by the TUI session
    /// boot, threaded into ToolContext, and consulted by both tool-execution
    /// dispatch paths. Toggled at runtime via `/sandbox on/off`.
    pub sandbox: Option<std::sync::Arc<dyn archon_permissions::SandboxBackend>>,
    /// Canonical activity event sink shared by parent, subagent, and tool
    /// execution paths.
    pub activity_sink: Option<Arc<dyn AgentActivitySink>>,
}

impl AgentConfig {
    /// Build the structural `LlmRequest` fields that must align between parent
    /// and subagent requests (v0.1.18 fix).
    ///
    /// Returns `(max_tokens, thinking, speed)`. Effort is excluded because
    /// it requires async lock access and has subagent-specific layering
    /// (per-agent-def override vs live /effort).
    pub fn build_base_request_fields(
        &self,
        model: &str,
    ) -> (u32, Option<serde_json::Value>, Option<String>) {
        let speed = if self.fast_mode.load(std::sync::atomic::Ordering::Relaxed) {
            Some("fast".to_string())
        } else {
            None
        };
        let thinking = {
            let mode = archon_llm::thinking::select_thinking_mode(model, self.thinking_budget);
            archon_llm::thinking::thinking_param(&mode)
        };
        (self.max_tokens, thinking, speed)
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8192,
            thinking_budget: 16384,
            system_prompt: Vec::new(),
            tools: Vec::new(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: uuid::Uuid::new_v4().to_string(),
            fast_mode: Arc::new(AtomicBool::new(false)),
            effort_level: Arc::new(Mutex::new(EffortLevel::Medium)),
            model_override: Arc::new(Mutex::new(String::new())),
            permission_mode: Arc::new(Mutex::new("auto".to_string())),
            permission_rules: archon_permissions::rules::RuleSet::empty(),
            extra_dirs: Arc::new(Mutex::new(Vec::new())),
            max_tool_concurrency: archon_tools::concurrency::DEFAULT_MAX_CONCURRENCY,
            max_turns: None,
            cancel_token: None,
            sandbox: None,
            activity_sink: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Conversation state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ConversationState {
    pub messages: Vec<serde_json::Value>,
    pub mode: AgentMode,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            mode: AgentMode::Normal,
            total_input_tokens: 0,
            total_output_tokens: 0,
        }
    }
}

impl ConversationState {
    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": content,
        }));
    }

    pub fn add_assistant_message(&mut self, content: Vec<serde_json::Value>) {
        self.messages.push(serde_json::json!({
            "role": "assistant",
            "content": content,
        }));
    }

    pub fn add_tool_result(&mut self, tool_use_id: &str, content: &str, is_error: bool) {
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content,
                "is_error": is_error,
            }],
        }));
    }

    pub fn first_user_message(&self) -> &str {
        for msg in &self.messages {
            if msg["role"].as_str() == Some("user")
                && let Some(content) = msg["content"].as_str()
            {
                return content;
            }
        }
        ""
    }
}
