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
    /// Approximate token size of the most recent request body actually put on
    /// the wire, from the preflight [`AgentEvent::ContextPressureUpdated`].
    ///
    /// Distinct from `input_tokens`, which is the provider's *billed* count
    /// summed over the session and is reported only after a turn succeeds. A
    /// request that gets rate-limited never bills anything, so on the failure
    /// this exists to diagnose — TPM pressure, issue #37 — `input_tokens` is
    /// exactly the number that stays silent. This one is recorded before the
    /// request is sent, so it survives the failure.
    ///
    /// Zero until the first request of the session is prepared.
    pub last_request_body_tokens: u64,
}

impl Default for SessionStats {
    fn default() -> Self {
        Self {
            input_tokens: 0,
            output_tokens: 0,
            turn_count: 0,
            session_cost: 0.0,
            cache_stats: archon_context::cache::CacheStats::default(),
            last_request_body_tokens: 0,
        }
    }
}

/// Semantic purpose of an AskUser prompt. Consumers must use this metadata
/// rather than inferring approval behavior from untrusted prompt text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskUserPromptKind {
    Ordinary,
    PlanApproval,
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
        /// Calibrated tokens attributed to the single largest message (#189
        /// Phase 3), so the status line can say what is filling the window and
        /// not only how full it is. Zero when nothing is attributed yet.
        heaviest_message_tokens: u64,
        /// The heaviest messages, biggest first, as `(message_index, tokens)`
        /// (#192 scope B).
        ///
        /// `heaviest_message_tokens` answers "how bad is the worst one"; this
        /// answers "which ones, and by how much", which is the question
        /// `top_contributors` was written for and had no caller for. Capped at
        /// [`TOP_CONTRIBUTOR_LIMIT`] because this rides on every turn and the
        /// tail of the ranking is not actionable.
        top_contributors: Vec<(usize, u64)>,
        /// Attributed tokens across *every* message, not only the ones listed.
        ///
        /// Without it a share cannot be computed from a truncated list, and
        /// "42k" alone does not say whether that is most of the window or a
        /// rounding error.
        attributed_total: u64,
    },
    TextDelta(String),
    ThinkingDelta(String),
    /// Unapproved reasoning preview for interactive display only.
    TransientThinkingDelta(String),
    /// Approve the active transient preview for normal completion/history.
    CommitThinkingPreview,
    /// Remove the active transient preview without retaining history.
    DiscardThinkingPreview,
    ToolCallStarted {
        name: String,
        id: String,
    },
    ToolCallComplete {
        name: String,
        id: String,
        result: ToolResult,
        transcript_summary: Option<String>,
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
        kind: AskUserPromptKind,
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
            AgentEvent::TransientThinkingDelta(_) => "TransientThinkingDelta",
            AgentEvent::CommitThinkingPreview => "CommitThinkingPreview",
            AgentEvent::DiscardThinkingPreview => "DiscardThinkingPreview",
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
    /// Whether the operator explicitly authorized bypassPermissions through
    /// the dangerous CLI flag. Plan Mode may restore bypass only when true.
    pub allow_bypass_permissions: bool,
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
    /// #201 Phase 1: the filesystem of the execution world.
    ///
    /// One field, read both by `build_tool_context` and by the read-before-edit
    /// guard, so a backend cannot end up enforcing freshness against the host
    /// while the tools write somewhere else. `None` is the host.
    pub fs: Option<std::sync::Arc<dyn archon_tools::filesystem::FileSystem>>,
    /// Canonical activity event sink shared by parent, subagent, and tool
    /// execution paths.
    pub activity_sink: Option<Arc<dyn AgentActivitySink>>,
    /// Context window and auto-compaction settings threaded from config.
    pub context: crate::config::ContextConfig,
    /// Authoritative maximum concurrent subagents, threaded from
    /// `config.subagent.max_concurrent`. Used to construct the session
    /// [`crate::subagent::SubagentManager`] so the live fan-out cap is
    /// configurable rather than a hardcoded constant.
    pub max_subagent_concurrency: usize,
    /// Seconds a subagent's LLM stream may go silent before the round is
    /// abandoned, threaded from `config.subagent.stream_idle_timeout_secs`.
    ///
    /// Threaded the same way as `max_subagent_concurrency` rather than passed
    /// to the runner constructor, which several call sites share.
    pub subagent_stream_idle_timeout_secs: u64,
    /// `config.subagent.auto_isolation` — when to isolate an agent that did not
    /// ask to be isolated (#184 M3).
    pub subagent_auto_isolation: archon_tools::isolation::AutoIsolation,
    /// `config.subagent.isolation_max_tier` — the most isolation any agent may
    /// have, however it was requested.
    pub subagent_isolation_max_tier: archon_tools::isolation::IsolationTier,
    /// `[filesystem]` — whether a write must be backed by a read of the same
    /// bytes (#193 Phase A).
    pub filesystem: crate::config::FilesystemConfig,
    /// Which subagent this agent is, if it is one (#193 Phase A).
    ///
    /// `session_id` is copied verbatim from parent to child, so on its own it
    /// cannot tell one agent from another inside a session — which matters for
    /// the read-before-write registry, where a parent's reading must not count
    /// as evidence for a child that never opened the file. `None` means the
    /// top-level agent, which is an answer rather than missing data.
    pub subagent_id: Option<String>,
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
        self.build_base_request_fields_with(model, false)
    }

    /// As [`Self::build_base_request_fields`], but able to escalate thinking
    /// for an `ultrathink` turn (#123).
    ///
    /// Split rather than added as a parameter to the existing method so the
    /// dozen-odd call sites that have no notion of turn input keep compiling
    /// unchanged; only the two request-building paths pass `true`.
    ///
    /// Note that on adaptive models (Opus/Sonnet) this returns the same
    /// `{"type": "adaptive"}` either way — adaptive thinking has no depth
    /// knob. The `ultrathink` escalation on those models is carried by effort
    /// instead, which `turn_effort` raises to `Max` for the same turn.
    pub fn build_base_request_fields_with(
        &self,
        model: &str,
        ultrathink: bool,
    ) -> (u32, Option<serde_json::Value>, Option<String>) {
        let speed = if self.fast_mode.load(std::sync::atomic::Ordering::Relaxed) {
            Some("fast".to_string())
        } else {
            None
        };
        let thinking = {
            let mode = archon_llm::thinking::select_thinking_mode(model, self.thinking_budget);
            let mode = if ultrathink {
                archon_llm::thinking::escalated_for_ultrathink(mode, self.max_tokens)
            } else {
                mode
            };
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
            agent_type: "main".to_string(),
            agent_version: None,
            fast_mode: Arc::new(AtomicBool::new(false)),
            effort_level: Arc::new(Mutex::new(EffortLevel::Medium)),
            model_override: Arc::new(Mutex::new(String::new())),
            permission_mode: Arc::new(Mutex::new("auto".to_string())),
            allow_bypass_permissions: false,
            permission_rules: archon_permissions::rules::RuleSet::empty(),
            extra_dirs: Arc::new(Mutex::new(Vec::new())),
            max_tool_concurrency: archon_tools::concurrency::DEFAULT_MAX_CONCURRENCY,
            max_turns: None,
            cancel_token: None,
            sandbox: None,
            fs: None,
            activity_sink: None,
            context: crate::config::ContextConfig::default(),
            max_subagent_concurrency: crate::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT,
            subagent_stream_idle_timeout_secs: crate::config::DEFAULT_STREAM_IDLE_TIMEOUT_SECS,
            subagent_auto_isolation: archon_tools::isolation::AutoIsolation::Overlap,
            subagent_isolation_max_tier: archon_tools::isolation::IsolationTier::Worktree,
            filesystem: crate::config::FilesystemConfig::default(),
            subagent_id: None,
        }
    }
}

#[path = "types_conversation_state.rs"]
mod conversation_state;
pub use conversation_state::{ConversationState, SpillContext};

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
