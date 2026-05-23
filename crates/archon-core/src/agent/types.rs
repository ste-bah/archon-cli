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
    ContextPressureUpdated {
        tokens_used: u64,
        context_window: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        context_name: Option<String>,
        resolution_source: Option<String>,
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
        reason: Option<String>,
    },
    TurnComplete {
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
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
            AgentEvent::ContextPressureUpdated { .. } => "ContextPressureUpdated",
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
    /// Agent identity used only for host-side runtime evidence.
    pub agent_type: String,
    pub agent_version: Option<String>,
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
    /// Context window and auto-compaction settings threaded from config.
    pub context: crate::config::ContextConfig,
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

    pub fn runtime_context_extra(&self) -> serde_json::Value {
        serde_json::json!({
            "archon_runtime": {
                "run_id": self.session_id,
                "agent_type": self.agent_type,
                "agent_version": self.agent_version,
            }
        })
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
            agent_type: "main".to_string(),
            agent_version: None,
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
            context: crate::config::ContextConfig::default(),
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
    /// Cumulative provider input tokens for billing/telemetry only.
    /// Auto-compaction triggers use last_known_context_tokens instead.
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    /// Last API-reported full context size for this turn.
    /// Equals `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`
    /// from the Anthropic usage response — includes system prompt, tool schemas,
    /// memory injections, and messages. Used as the authoritative compaction trigger
    /// source. Falls back to `trigger_tokens(messages)` estimate when zero — on
    /// turn 1, after `/clear`, or transiently after a successful compaction (until
    /// the next API response repopulates it).
    pub last_known_context_tokens: u64,
    pub auto_compact: crate::agent::AutoCompactState,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            mode: AgentMode::Normal,
            total_input_tokens: 0,
            total_output_tokens: 0,
            last_known_context_tokens: 0,
            auto_compact: crate::agent::AutoCompactState::default(),
        }
    }
}

impl ConversationState {
    const INTERRUPTED_TOOL_RESULT: &'static str = "Tool dispatch interrupted before producing a result. \
         The assistant called this tool but no result was recorded, likely due \
         to mid-turn cancellation, dispatch panic, or session crash. Treat as failed.";

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
        let context_output =
            crate::agent::tool_result_context::cap_tool_output_for_context("", content);
        let result = serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": context_output.content,
            "is_error": is_error,
        });
        if let Some(last) = self.messages.last_mut()
            && last.get("role").and_then(|v| v.as_str()) == Some("user")
            && let Some(blocks) = last.get_mut("content").and_then(|v| v.as_array_mut())
            && !blocks.is_empty()
            && blocks
                .iter()
                .all(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
        {
            blocks.push(result);
            return;
        }
        self.messages.push(serde_json::json!({
            "role": "user",
            "content": [result],
        }));
    }

    pub(super) fn fill_missing_tool_results(&mut self, expected_ids: &[String]) -> Vec<String> {
        if expected_ids.is_empty() {
            return Vec::new();
        }
        let recorded_ids: std::collections::HashSet<String> = self
            .messages
            .last()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|block| {
                        block.get("type").and_then(|v| v.as_str()) == Some("tool_result")
                    })
                    .filter_map(|block| {
                        block
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let missing: Vec<String> = expected_ids
            .iter()
            .filter(|id| !recorded_ids.contains(*id))
            .cloned()
            .collect();
        for id in &missing {
            self.add_tool_result(id, Self::INTERRUPTED_TOOL_RESULT, true);
        }
        missing
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_context_extra_carries_agent_identity() {
        let config = AgentConfig {
            session_id: "session-1".to_string(),
            agent_type: "reviewer".to_string(),
            agent_version: Some("1.0.0".to_string()),
            ..AgentConfig::default()
        };

        let extra = config.runtime_context_extra();

        assert_eq!(extra["archon_runtime"]["run_id"], "session-1");
        assert_eq!(extra["archon_runtime"]["agent_type"], "reviewer");
        assert_eq!(extra["archon_runtime"]["agent_version"], "1.0.0");
    }

    #[test]
    fn conversation_state_batches_adjacent_tool_results() {
        let mut state = ConversationState::default();
        state.add_assistant_message(vec![serde_json::json!({
            "type": "tool_use",
            "id": "tool-1",
            "name": "Read",
            "input": {}
        })]);

        state.add_tool_result("tool-1", "one", false);
        state.add_tool_result("tool-2", "two", false);

        assert_eq!(state.messages.len(), 2);
        let blocks = state.messages[1]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["tool_use_id"], "tool-1");
        assert_eq!(blocks[1]["tool_use_id"], "tool-2");
    }

    #[test]
    fn conversation_state_hard_caps_tool_result_text() {
        let mut state = ConversationState::default();
        let huge = "x".repeat(700_000);

        state.add_tool_result("tool-1", &huge, false);

        let content = state.messages[0]["content"][0]["content"]
            .as_str()
            .expect("tool result content");
        assert!(content.len() < 100_000, "content len was {}", content.len());
        assert!(content.contains("tool output trimmed"));
    }
}
