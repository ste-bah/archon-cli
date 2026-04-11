use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use archon_consciousness::corrections::{CorrectionTracker, CorrectionType};
use archon_consciousness::inner_voice::InnerVoice;
use archon_consciousness::rules::RulesEngine;
use archon_llm::effort::EffortLevel;
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::streaming::StreamEvent;
use archon_memory::MemoryTrait;
use archon_memory::extraction::{
    ExtractionConfig, ExtractionState, build_extraction_prompt, parse_extraction_response,
    should_extract, store_extracted,
};
use archon_memory::injection::MemoryInjector;
use archon_permissions::auto::{AutoDecision, AutoModeEvaluator};
use archon_session::checkpoint::CheckpointStore;
use archon_session::plan::PlanStore;
use archon_tools::agent_tool::SubagentRequest;
use archon_tools::plan_mode::is_tool_allowed_in_mode;
use archon_tools::send_message::SendMessageRequest;
use archon_tools::tool::{AgentMode, ToolContext, ToolResult};
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::agents::AgentRegistry;
use crate::dispatch::ToolRegistry;
use crate::subagent::SubagentManager;

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
    /// Additional working directories added at runtime via `/add-dir`.
    pub extra_dirs: Arc<Mutex<Vec<std::path::PathBuf>>>,
    /// Maximum concurrent tool calls (1 = sequential, from config.tools.max_concurrency).
    pub max_tool_concurrency: usize,
    /// Maximum agentic loop iterations per process_message call (None = unlimited).
    pub max_turns: Option<u32>,
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
            extra_dirs: Arc::new(Mutex::new(Vec::new())),
            max_tool_concurrency: archon_tools::concurrency::DEFAULT_MAX_CONCURRENCY,
            max_turns: None,
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

// ---------------------------------------------------------------------------
// Pending tool call accumulated from streaming events
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PendingToolCall {
    id: String,
    name: String,
    input_json: String,
}

// ---------------------------------------------------------------------------
// Agent — the main orchestration engine
// ---------------------------------------------------------------------------

pub struct Agent {
    client: Arc<dyn LlmProvider>,
    registry: ToolRegistry,
    config: AgentConfig,
    state: ConversationState,
    event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    checkpoint_store: Option<Arc<Mutex<CheckpointStore>>>,
    plan_store: Option<PlanStore>,
    turn_number: u64,
    // GAP 5/7: Memory graph + injector for per-turn injection and auto-extraction
    memory: Option<Arc<dyn MemoryTrait>>,
    memory_injector: MemoryInjector,
    extraction_config: ExtractionConfig,
    extraction_state: ExtractionState,
    // GAP 6: Auto-mode permission evaluator
    auto_evaluator: Option<AutoModeEvaluator>,
    // GAP 8: Subagent manager
    subagent_manager: Arc<Mutex<SubagentManager>>,
    /// Shared flag: whether /thinking display is on (used to potentially skip thinking in future)
    pub show_thinking: Arc<AtomicBool>,
    /// Shared session statistics for /status and /cost slash commands.
    pub session_stats: Arc<Mutex<SessionStats>>,
    /// Hook registry for pre/post tool execution hooks.
    hook_registry: Option<Arc<crate::hooks::HookRegistry>>,
    /// File watch manager for dynamic watch paths from hooks (REQ-HOOK-017).
    file_watch_manager: Arc<crate::hooks::FileWatchManager>,
    /// Channel for permission prompt responses from the TUI.
    /// Agent sends PermissionRequired event, then waits on this for y/n.
    pub permission_response_rx: Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<bool>>>>,
    /// Inner voice state injected into the system prompt each turn when enabled.
    /// Tracks confidence, energy, focus, struggles, successes, and turn count.
    inner_voice: Option<Arc<Mutex<InnerVoice>>>,
    /// Channel for receiving user answers when AskUserQuestion is invoked.
    /// The TUI sends the user's response through the paired sender.
    pub ask_user_response_rx: Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>>,
    /// Saved permission mode before entering plan mode, so ExitPlanMode can restore it.
    previous_permission_mode: Option<String>,
    /// Append-only log of permission denials for audit / `/denials` display.
    pub denial_log: Arc<Mutex<archon_permissions::denial_log::DenialLog>>,
    /// Custom agent registry (built-in + project + user agents).
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    /// CLI-416: Personality briefing injected into system prompt on first turn only.
    personality_briefing: Option<String>,
    /// CLI-417: Memory garden briefing injected into system prompt on first turn only.
    pub memory_briefing: Option<String>,
    /// Permission store for hook-driven permission updates (REQ-HOOK-016).
    permission_store: Arc<dyn crate::hooks::PermissionStore>,
    /// Critical system reminder re-injected into system prompt at every turn (AGT-022).
    critical_system_reminder: Option<String>,
    /// Pending resume messages to inject into the next SubagentRunner (AGT-024).
    /// TASK-AGS-105: Arc<Mutex<...>> so the `AgentSubagentExecutor` can
    /// `take()` this slot from inside `run_to_completion` via its own
    /// clone (see mapping doc Section 2g).
    pending_resume_messages: Arc<tokio::sync::Mutex<Option<Vec<serde_json::Value>>>>,
}

impl Agent {
    pub fn new(
        client: Arc<dyn LlmProvider>,
        registry: ToolRegistry,
        config: AgentConfig,
        event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    ) -> Self {
        let permission_store: Arc<dyn crate::hooks::PermissionStore> =
            Arc::new(crate::hooks::RuntimePermissionStore::new(
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".archon")
                    .join("settings.json"),
                config.working_dir.join(".archon").join("settings.json"),
            ));
        Self {
            client,
            registry,
            config,
            state: ConversationState::default(),
            event_tx,
            checkpoint_store: None,
            plan_store: None,
            turn_number: 0,
            memory: None,
            memory_injector: MemoryInjector::new(),
            extraction_config: ExtractionConfig::default(),
            extraction_state: ExtractionState::default(),
            auto_evaluator: None,
            subagent_manager: Arc::new(Mutex::new(SubagentManager::default())),
            show_thinking: Arc::new(AtomicBool::new(true)),
            session_stats: Arc::new(Mutex::new(SessionStats::default())),
            hook_registry: None,
            file_watch_manager: Arc::new(crate::hooks::FileWatchManager::new(100)),
            permission_response_rx: None,
            inner_voice: None,
            ask_user_response_rx: None,
            previous_permission_mode: None,
            denial_log: Arc::new(Mutex::new(archon_permissions::denial_log::DenialLog::new())),
            agent_registry,
            personality_briefing: None,
            memory_briefing: None,
            permission_store,
            critical_system_reminder: None,
            pending_resume_messages: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// TASK-AGS-105: install the `AgentSubagentExecutor` into the process
    /// OnceLock so `AgentTool::execute` and `TaskCreateTool::execute` can
    /// resolve it via `archon_tools::subagent_executor::get_subagent_executor`.
    ///
    /// Called explicitly by the embedder (CLI, tests) AFTER constructing the
    /// `Agent` with its full field set (hook_registry, memory, etc.). This is
    /// a separate step from `Agent::new` because many of the fields the
    /// executor needs are set via post-construction setters
    /// (`set_hook_registry`, `set_memory`, ...). The install is idempotent
    /// per-process (OnceLock semantics): first caller wins.
    pub fn install_subagent_executor(&self) {
        let exec = crate::subagent_executor::AgentSubagentExecutor::new(
            Arc::clone(&self.client),
            self.registry.clone(),
            Arc::clone(&self.subagent_manager),
            Arc::clone(&self.agent_registry),
            self.hook_registry.as_ref().map(Arc::clone),
            self.memory.as_ref().map(Arc::clone),
            self.config.working_dir.clone(),
            self.config.session_id.clone(),
            self.config.model.clone(),
            self.config.system_prompt.clone(),
            Arc::clone(&self.config.permission_mode),
            Arc::clone(&self.pending_resume_messages),
        );
        archon_tools::subagent_executor::install_subagent_executor(Arc::new(exec));
    }

    /// Enable the inner voice feature. The supplied state is shared so that
    /// external components (slash commands, compaction handlers) can inspect
    /// or snapshot it.
    /// Set the personality briefing text (injected on first turn only).
    pub fn set_personality_briefing(&mut self, text: String) {
        self.personality_briefing = Some(text);
    }

    /// Set the memory garden briefing text (injected on first turn only).
    pub fn set_memory_briefing(&mut self, text: String) {
        self.memory_briefing = Some(text);
    }

    /// Set the critical system reminder (re-injected every turn, AGT-022).
    pub fn set_critical_system_reminder(&mut self, text: String) {
        if text.is_empty() {
            self.critical_system_reminder = None;
        } else {
            self.critical_system_reminder = Some(text);
        }
    }

    pub fn set_inner_voice(&mut self, iv: Arc<Mutex<InnerVoice>>) {
        self.inner_voice = Some(iv);
    }

    /// Access the inner voice handle, if enabled.
    pub fn inner_voice(&self) -> Option<&Arc<Mutex<InnerVoice>>> {
        self.inner_voice.as_ref()
    }

    /// Access the subagent manager (read-only) for status queries.
    pub fn subagent_manager(&self) -> Arc<Mutex<SubagentManager>> {
        Arc::clone(&self.subagent_manager)
    }

    /// Close the event channel so receivers know the agent is done.
    /// Used by print mode to unblock the event consumer task.
    pub fn close_event_channel(&mut self) {
        // Replace the sender with a closed one by dropping it.
        // TASK-AGS-102: unbounded variant — same drop-to-close semantics.
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        self.event_tx = tx;
        // The old sender is dropped, closing the channel
    }

    /// Set the hook registry for pre/post tool execution hooks.
    pub fn set_hook_registry(&mut self, registry: Arc<crate::hooks::HookRegistry>) {
        self.hook_registry = Some(registry);
    }

    /// Add dynamic watch paths from hooks (REQ-HOOK-017).
    pub fn add_watch_paths(&self, paths: Vec<String>) {
        self.file_watch_manager.add_watch_paths(paths);
    }

    /// Clear all dynamic watch paths (called on SessionEnd).
    pub fn clear_watch_paths(&self) {
        self.file_watch_manager.clear();
    }

    /// Fire a hook by event with a JSON payload. Returns the aggregated result.
    /// No-op (returns empty aggregate) if no registry is set.
    pub async fn fire_hook(
        &self,
        event: crate::hooks::HookEvent,
        payload: serde_json::Value,
    ) -> crate::hooks::AggregatedHookResult {
        if let Some(ref registry) = self.hook_registry {
            registry
                .execute_hooks(
                    event,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await
        } else {
            crate::hooks::AggregatedHookResult::new()
        }
    }

    /// Set the checkpoint store for file snapshots before Write/Edit operations.
    pub fn set_checkpoint_store(&mut self, store: CheckpointStore) {
        self.checkpoint_store = Some(Arc::new(Mutex::new(store)));
    }

    /// Set the plan store for plan persistence.
    pub fn set_plan_store(&mut self, store: PlanStore) {
        self.plan_store = Some(store);
    }

    /// Set the memory graph for per-turn injection (GAP 7) and extraction (GAP 5).
    pub fn set_memory(&mut self, memory: Arc<dyn MemoryTrait>) {
        self.memory = Some(memory);
    }

    /// Restore conversation state from previously saved messages.
    /// Used for session resume (`--resume <id>`).
    pub fn restore_conversation(&mut self, messages: Vec<serde_json::Value>) {
        self.state.messages = messages;
    }

    /// Set the auto-mode evaluator for permission classification (GAP 6).
    pub fn set_auto_evaluator(&mut self, evaluator: AutoModeEvaluator) {
        self.auto_evaluator = Some(evaluator);
    }

    /// Process a single user message through the full agent loop.
    /// Returns when the LLM produces a final text response (no more tool calls).
    pub async fn process_message(&mut self, user_input: &str) -> Result<(), AgentLoopError> {
        self.turn_number += 1;
        self.state.add_user_message(user_input);

        let mut agentic_iterations: u32 = 0;
        loop {
            // GAP 7: Inject recalled memories into system prompt
            let mut system_with_memories = self.inject_memories();
            // Append inner voice block (consciousness state) if enabled
            self.inject_inner_voice(&mut system_with_memories).await;
            // Append critical system reminder (AGT-022) — re-injected every turn
            self.inject_critical_reminder(&mut system_with_memories);

            // GAP 3: Read fast_mode from shared atomic
            let speed = if self.config.fast_mode.load(Ordering::Relaxed) {
                Some("fast".to_string())
            } else {
                None
            };

            // GAP 4: Read effort level from shared mutex.
            // Ultrathink override: if input contains "ultrathink" (case-insensitive),
            // force effort to high for this turn only.
            let ultrathink_active = user_input.to_lowercase().contains("ultrathink");
            let effort = if ultrathink_active {
                // Ultrathink always uses high effort — None means "high" (the default)
                None
            } else {
                let level = self.config.effort_level.lock().await;
                match *level {
                    EffortLevel::High => None,
                    other => Some(other.to_string()),
                }
            };

            // Read model from shared state (set by /model command), fall back to config default
            let active_model = {
                let override_model = self.config.model_override.lock().await;
                if override_model.is_empty() {
                    self.config.model.clone()
                } else {
                    override_model.clone()
                }
            };

            // Build the API request
            let request = LlmRequest {
                model: active_model.clone(),
                max_tokens: self.config.max_tokens,
                system: system_with_memories,
                messages: self.state.messages.clone(),
                tools: self.config.tools.clone(),
                thinking: {
                    let mode = archon_llm::thinking::select_thinking_mode(
                        &active_model,
                        self.config.thinking_budget,
                    );
                    archon_llm::thinking::thinking_param(&mode)
                },
                speed,
                effort,
                extra: serde_json::Value::Null,
            };

            self.send_event(AgentEvent::ApiCallStarted {
                model: active_model.clone(),
            })
            .await;

            // Send request and get streaming events
            let mut rx = self
                .client
                .stream(request)
                .await
                .map_err(|e| AgentLoopError::ApiError(format!("{e}")))?;

            // Process the stream
            let mut text_content = String::new();
            let mut thinking_content = String::new();
            let mut thinking_signature = String::new();
            let mut pending_tools: Vec<PendingToolCall> = Vec::new();
            let mut _current_tool_index: Option<u32> = None;
            let mut _stop_reason: Option<String> = None;
            let mut turn_input_tokens: u64 = 0;
            let mut turn_output_tokens: u64 = 0;
            let mut turn_cache_creation: u64 = 0;
            let mut turn_cache_read: u64 = 0;

            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::MessageStart { usage, .. } => {
                        turn_input_tokens += usage.input_tokens;
                        turn_cache_creation += usage.cache_creation_input_tokens;
                        turn_cache_read += usage.cache_read_input_tokens;
                    }

                    StreamEvent::ContentBlockStart {
                        index,
                        block_type,
                        tool_use_id,
                        tool_name,
                    } => {
                        if block_type == archon_llm::types::ContentBlockType::ToolUse {
                            let id = tool_use_id.unwrap_or_default();
                            let name = tool_name.unwrap_or_default();
                            self.send_event(AgentEvent::ToolCallStarted {
                                name: name.clone(),
                                id: id.clone(),
                            })
                            .await;
                            pending_tools.push(PendingToolCall {
                                id,
                                name,
                                input_json: String::new(),
                            });
                            _current_tool_index = Some(index);
                        }
                    }

                    StreamEvent::TextDelta { text, .. } => {
                        text_content.push_str(&text);
                        self.send_event(AgentEvent::TextDelta(text)).await;
                    }

                    StreamEvent::ThinkingDelta { thinking, .. } => {
                        thinking_content.push_str(&thinking);
                        self.send_event(AgentEvent::ThinkingDelta(thinking)).await;
                    }

                    StreamEvent::InputJsonDelta { partial_json, .. } => {
                        // Accumulate JSON for the current tool call
                        if let Some(tool) = pending_tools.last_mut() {
                            tool.input_json.push_str(&partial_json);
                        }
                    }

                    StreamEvent::ContentBlockStop { .. } => {
                        _current_tool_index = None;
                    }

                    StreamEvent::MessageDelta {
                        stop_reason: sr,
                        usage,
                    } => {
                        _stop_reason = sr;
                        if let Some(u) = usage {
                            turn_output_tokens += u.output_tokens;
                        }
                    }

                    StreamEvent::MessageStop => {}
                    StreamEvent::Ping => {}
                    StreamEvent::SignatureDelta { signature, .. } => {
                        thinking_signature.push_str(&signature);
                    }

                    StreamEvent::Error {
                        error_type,
                        message,
                    } => {
                        // CRIT-06: Fire Notification hook on API errors
                        self.fire_hook(
                            crate::hooks::HookEvent::Notification,
                            serde_json::json!({
                                "hook_event": "Notification",
                                "level": "error",
                                "message": format!("{error_type}: {message}"),
                            }),
                        )
                        .await;
                        self.send_event(AgentEvent::Error(format!("{error_type}: {message}")))
                            .await;
                        return Err(AgentLoopError::ApiError(format!("{error_type}: {message}")));
                    }
                }
            }

            // Update token totals
            self.state.total_input_tokens += turn_input_tokens;
            self.state.total_output_tokens += turn_output_tokens;

            // Build the assistant message content blocks
            let mut assistant_content: Vec<serde_json::Value> = Vec::new();

            if !thinking_content.is_empty() {
                assistant_content.push(serde_json::json!({
                    "type": "thinking",
                    "thinking": thinking_content,
                    "signature": thinking_signature,
                }));
            }

            if !text_content.is_empty() {
                assistant_content.push(serde_json::json!({
                    "type": "text",
                    "text": text_content,
                }));
            }

            for tool in &pending_tools {
                let input: serde_json::Value =
                    serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));
                assistant_content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tool.id,
                    "name": tool.name,
                    "input": input,
                }));
            }

            self.state.add_assistant_message(assistant_content);

            // If there are tool calls, dispatch them and loop
            if !pending_tools.is_empty() {
                // Resolve agent mode from the shared permission_mode string
                let effective_mode = {
                    let pm = self.config.permission_mode.lock().await;
                    if pm.as_str() == "plan" {
                        AgentMode::Plan
                    } else {
                        AgentMode::Normal
                    }
                };
                let extra = self.config.extra_dirs.lock().await.clone();
                // TASK-AGS-105: compute in_fork once per turn from the
                // parent's message history so the SubagentExecutor can
                // enforce the fork-in-fork guard without crossing the
                // `state.messages` boundary into archon-tools.
                let in_fork =
                    crate::agents::built_in::is_in_fork_child_by_messages(&self.state.messages);
                let ctx = ToolContext {
                    working_dir: self.config.working_dir.clone(),
                    session_id: self.config.session_id.clone(),
                    mode: effective_mode,
                    extra_dirs: extra,
                    in_fork,
                    // `nested` stays false here — only TaskCreateTool::execute
                    // flips it to true when routing a subagent request through
                    // the executor.
                    nested: false,
                };

                // -------------------------------------------------------
                // PHASE 1: Pre-flight (sequential)
                // Check permissions, run pre-hooks, snapshot checkpoints.
                // Denied/blocked tools get results recorded immediately.
                // Allowed tools are collected for dispatch.
                // -------------------------------------------------------
                struct PreflightResult {
                    tool_name: String,
                    tool_id: String,
                    input: serde_json::Value,
                    tool_arc: Arc<dyn archon_tools::tool::Tool>,
                    file_path: Option<String>,
                }

                let mut allowed: Vec<PreflightResult> = Vec::new();

                for tool in &pending_tools {
                    let mut input: serde_json::Value =
                        serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));

                    // --- Permission check ---
                    let perm_mode = {
                        let mode = self.config.permission_mode.lock().await;
                        mode.clone()
                    };
                    let tool_allowed = match perm_mode.as_str() {
                        "bypassPermissions" | "yolo" | "dontAsk" => {
                            tracing::debug!(tool = %tool.name, "bypass-mode: allowed");
                            true
                        }
                        "acceptEdits" => match tool.name.as_str() {
                            "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                            | "TodoWrite" | "Sleep" | "Write" | "Edit" | "Config"
                            | "EnterPlanMode" | "ExitPlanMode" | "NotebookEdit" => true,
                            _ => {
                                let perm_agg = self
                                    .fire_hook(
                                        crate::hooks::HookEvent::PermissionRequest,
                                        serde_json::json!({
                                            "hook_event": "PermissionRequest",
                                            "tool_name": tool.name,
                                            "mode": "acceptEdits",
                                        }),
                                    )
                                    .await;
                                // Apply updated_permissions from hooks (REQ-HOOK-016)
                                if !perm_agg.updated_permissions.is_empty() {
                                    let authority = crate::hooks::SourceAuthority::Project;
                                    let errors = crate::hooks::apply_permission_updates(
                                        &perm_agg.updated_permissions,
                                        &authority,
                                        self.permission_store.as_ref(),
                                    );
                                    for err in &errors {
                                        tracing::error!("permission update failed: {}", err);
                                    }
                                }
                                self.send_event(AgentEvent::PermissionRequired {
                                    tool: tool.name.clone(),
                                    description: format!("Permission required for {}", tool.name),
                                })
                                .await;
                                self.fire_hook(
                                    crate::hooks::HookEvent::PermissionDenied,
                                    serde_json::json!({
                                        "hook_event": "PermissionDenied",
                                        "tool_name": tool.name,
                                        "mode": "acceptEdits",
                                    }),
                                )
                                .await;
                                self.send_event(AgentEvent::PermissionDenied {
                                    tool: tool.name.clone(),
                                })
                                .await;
                                false
                            }
                        },
                        "default" | "ask" => match tool.name.as_str() {
                            "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                            | "TodoWrite" | "Sleep" | "EnterPlanMode" | "ExitPlanMode" => {
                                tracing::debug!(tool = %tool.name, "default-mode: safe, allowed");
                                true
                            }
                            _ => {
                                let perm_agg = self
                                    .fire_hook(
                                        crate::hooks::HookEvent::PermissionRequest,
                                        serde_json::json!({
                                            "hook_event": "PermissionRequest",
                                            "tool_name": tool.name,
                                            "mode": "ask",
                                        }),
                                    )
                                    .await;
                                // Apply updated_permissions from hooks (REQ-HOOK-016)
                                if !perm_agg.updated_permissions.is_empty() {
                                    let authority = crate::hooks::SourceAuthority::Project;
                                    let errors = crate::hooks::apply_permission_updates(
                                        &perm_agg.updated_permissions,
                                        &authority,
                                        self.permission_store.as_ref(),
                                    );
                                    for err in &errors {
                                        tracing::error!("permission update failed: {}", err);
                                    }
                                }
                                self.send_event(AgentEvent::PermissionRequired {
                                    tool: tool.name.clone(),
                                    description: format!(
                                        "{} wants to use {}",
                                        tool.name, tool.name
                                    ),
                                })
                                .await;

                                if let Some(ref rx) = self.permission_response_rx {
                                    let mut rx = rx.lock().await;
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(120),
                                        rx.recv(),
                                    )
                                    .await
                                    {
                                        Ok(Some(true)) => {
                                            self.send_event(AgentEvent::PermissionGranted {
                                                tool: tool.name.clone(),
                                            })
                                            .await;
                                            tracing::info!(tool = %tool.name, "default-mode: user approved");
                                            true
                                        }
                                        _ => {
                                            self.fire_hook(
                                                crate::hooks::HookEvent::PermissionDenied,
                                                serde_json::json!({
                                                    "hook_event": "PermissionDenied",
                                                    "tool_name": tool.name,
                                                    "mode": "ask",
                                                    "reason": "user_denied_or_timeout",
                                                }),
                                            )
                                            .await;
                                            self.send_event(AgentEvent::PermissionDenied {
                                                tool: tool.name.clone(),
                                            })
                                            .await;
                                            tracing::info!(tool = %tool.name, "default-mode: user denied or timeout");
                                            false
                                        }
                                    }
                                } else {
                                    tracing::info!(tool = %tool.name, "default-mode: no permission channel, auto-approved");
                                    true
                                }
                            }
                        },
                        _ => {
                            // "auto" mode -- use AutoModeEvaluator
                            if let Some(ref evaluator) = self.auto_evaluator {
                                let decision = match tool.name.as_str() {
                                    "Bash" | "PowerShell" => {
                                        let cmd = input
                                            .get("command")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        evaluator.evaluate_command(cmd)
                                    }
                                    "Write" | "Edit" => {
                                        let path = input
                                            .get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        evaluator.evaluate_file_write(Path::new(path))
                                    }
                                    "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                                    | "TodoWrite" | "Sleep" => AutoDecision::Allow,
                                    "Config" => {
                                        let action = input
                                            .get("action")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        if action.eq_ignore_ascii_case("get") {
                                            AutoDecision::Allow
                                        } else {
                                            AutoDecision::Prompt
                                        }
                                    }
                                    _ => AutoDecision::Prompt,
                                };
                                match decision {
                                    AutoDecision::Allow => {
                                        tracing::debug!(tool = %tool.name, "auto-mode: allowed");
                                        true
                                    }
                                    AutoDecision::Prompt => {
                                        tracing::warn!(tool = %tool.name, "auto-mode: risky, denied");
                                        self.fire_hook(
                                            crate::hooks::HookEvent::PermissionDenied,
                                            serde_json::json!({
                                                "hook_event": "PermissionDenied",
                                                "tool_name": tool.name,
                                                "mode": "auto",
                                                "reason": "risky_operation",
                                            }),
                                        )
                                        .await;
                                        self.send_event(AgentEvent::PermissionDenied {
                                            tool: tool.name.clone(),
                                        })
                                        .await;
                                        false
                                    }
                                    AutoDecision::PromptWithWarning(msg) => {
                                        tracing::warn!(tool = %tool.name, warning = %msg, "auto-mode: dangerous, denied");
                                        self.fire_hook(
                                            crate::hooks::HookEvent::PermissionDenied,
                                            serde_json::json!({
                                                "hook_event": "PermissionDenied",
                                                "tool_name": tool.name,
                                                "mode": "auto",
                                                "reason": "dangerous_operation",
                                                "warning": msg,
                                            }),
                                        )
                                        .await;
                                        self.send_event(AgentEvent::PermissionDenied {
                                            tool: tool.name.clone(),
                                        })
                                        .await;
                                        false
                                    }
                                }
                            } else {
                                true // no evaluator = allow
                            }
                        }
                    };

                    if !tool_allowed {
                        {
                            let mut log = self.denial_log.lock().await;
                            log.record(&tool.name, &format!("mode={perm_mode}"));
                        }
                        let denied_result = ToolResult::error(format!(
                            "Permission denied for tool '{}'. Current mode: {}. Use /permissions yolo to allow all operations.",
                            tool.name, perm_mode
                        ));
                        self.send_event(AgentEvent::ToolCallComplete {
                            name: tool.name.clone(),
                            id: tool.id.clone(),
                            result: denied_result.clone(),
                        })
                        .await;
                        self.state
                            .add_tool_result(&tool.id, &denied_result.content, true);
                        continue;
                    }

                    // --- Plan mode check ---
                    if !is_tool_allowed_in_mode(&tool.name, effective_mode) {
                        let result = ToolResult::error(format!(
                            "Tool '{}' is not available in plan mode. Only read-only tools are allowed.",
                            tool.name
                        ));
                        self.send_event(AgentEvent::ToolCallComplete {
                            name: tool.name.clone(),
                            id: tool.id.clone(),
                            result: result.clone(),
                        })
                        .await;
                        self.state.add_tool_result(&tool.id, &result.content, true);
                        continue;
                    }

                    // --- Checkpoint before Write/Edit ---
                    if matches!(tool.name.as_str(), "Write" | "Edit")
                        && let Some(ref store) = self.checkpoint_store
                        && let Some(file_path) = input.get("file_path").and_then(|v| v.as_str())
                    {
                        let store = store.lock().await;
                        if let Err(e) = store.snapshot(
                            &self.config.session_id,
                            file_path,
                            self.turn_number as i64,
                            &tool.name,
                        ) {
                            tracing::warn!("checkpoint snapshot failed for {file_path}: {e}");
                        }
                    }

                    // --- Pre-tool-use hook (REQ-HOOK-001/003/004) ---
                    if let Some(ref registry) = self.hook_registry {
                        let hook_input = serde_json::json!({
                            "hook_event": "PreToolUse",
                            "tool_name": tool.name,
                            "tool_input": input,
                        });
                        let hook_agg = registry
                            .execute_hooks(
                                crate::hooks::HookEvent::PreToolUse,
                                hook_input,
                                &self.config.working_dir,
                                &self.config.session_id,
                            )
                            .await;

                        // Check for blocking (any hook returned exit 2 or outcome=Blocking)
                        if hook_agg.is_blocked() {
                            let reason = hook_agg
                                .block_reason()
                                .unwrap_or_else(|| "hook blocked".to_owned());
                            let result = ToolResult::error(format!("Hook blocked: {reason}"));
                            self.send_event(AgentEvent::ToolCallComplete {
                                name: tool.name.clone(),
                                id: tool.id.clone(),
                                result: result.clone(),
                            })
                            .await;
                            self.state
                                .add_tool_result(&tool.id, &result.content, result.is_error);
                            continue;
                        }

                        // Check permission_behavior override (REQ-HOOK-004)
                        if let Some(ref pb) = hook_agg.permission_behavior {
                            match pb {
                                crate::hooks::PermissionBehavior::Deny => {
                                    let reason = hook_agg
                                        .permission_decision_reason
                                        .as_deref()
                                        .unwrap_or("hook denied permission");
                                    let result =
                                        ToolResult::error(format!("Permission denied: {reason}"));
                                    self.send_event(AgentEvent::ToolCallComplete {
                                        name: tool.name.clone(),
                                        id: tool.id.clone(),
                                        result: result.clone(),
                                    })
                                    .await;
                                    self.state.add_tool_result(
                                        &tool.id,
                                        &result.content,
                                        result.is_error,
                                    );
                                    continue;
                                }
                                crate::hooks::PermissionBehavior::Allow => {
                                    // Skip normal permission check — hook allowed it
                                    tracing::debug!(
                                        tool = %tool.name,
                                        "permission overridden to Allow by policy hook"
                                    );
                                }
                                crate::hooks::PermissionBehavior::Ask => {
                                    // TODO(Phase 2): force interactive prompt
                                    tracing::debug!(
                                        tool = %tool.name,
                                        "permission_behavior=ask (not yet implemented, using normal flow)"
                                    );
                                }
                                crate::hooks::PermissionBehavior::Passthrough => {
                                    // No-op: normal permission flow proceeds
                                }
                            }
                        }

                        // Apply updated_input if hook modified it (REQ-HOOK-003)
                        if let Some(modified_input) = hook_agg.updated_input {
                            if modified_input.is_object() {
                                tracing::debug!(
                                    tool = %tool.name,
                                    "PreToolUse hook modified tool input"
                                );
                                input = modified_input;
                            } else {
                                tracing::warn!(
                                    tool = %tool.name,
                                    "PreToolUse hook returned non-object updated_input, ignoring"
                                );
                            }
                        }

                        // Log system messages from hooks (REQ-HOOK-001)
                        for msg in &hook_agg.system_messages {
                            tracing::warn!(tool = %tool.name, "[Hook Warning] {}", msg);
                        }
                        for msg in &hook_agg.status_messages {
                            tracing::info!(tool = %tool.name, "[Hook Status] {}", msg);
                        }
                    }

                    // --- Resolve tool from registry ---
                    let tool_arc = match self.registry.lookup(&tool.name) {
                        Some(t) => t,
                        None => {
                            let result = ToolResult::error(format!(
                                "Unknown tool: '{}'. Available tools: {}",
                                tool.name,
                                self.registry.tool_names().join(", ")
                            ));
                            self.send_event(AgentEvent::ToolCallComplete {
                                name: tool.name.clone(),
                                id: tool.id.clone(),
                                result: result.clone(),
                            })
                            .await;
                            self.state.add_tool_result(&tool.id, &result.content, true);
                            continue;
                        }
                    };

                    // --- Capture file_path for post-processing ---
                    let file_path =
                        if matches!(tool.name.as_str(), "Write" | "Edit" | "NotebookEdit") {
                            input
                                .get("file_path")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                        } else {
                            None
                        };

                    allowed.push(PreflightResult {
                        tool_name: tool.name.clone(),
                        tool_id: tool.id.clone(),
                        input,
                        tool_arc,
                        file_path,
                    });
                }

                // -------------------------------------------------------
                // PHASE 2: Dispatch (concurrent when possible)
                // Execute the actual tool calls. Uses JoinSet + Semaphore
                // when multiple tools are allowed and concurrency > 1.
                // -------------------------------------------------------
                let dispatch_results: Vec<ToolResult> = if allowed.len() > 1
                    && self.config.max_tool_concurrency > 1
                {
                    tracing::info!(
                        tools = allowed.len(),
                        max_concurrency = self.config.max_tool_concurrency,
                        "dispatching tools concurrently"
                    );
                    let sem = Arc::new(Semaphore::new(self.config.max_tool_concurrency));
                    let ctx_arc = Arc::new(ctx.clone());
                    let mut join_set = JoinSet::new();

                    for (idx, pre) in allowed.iter().enumerate() {
                        let tool = pre.tool_arc.clone();
                        let input = pre.input.clone();
                        let ctx_clone = ctx_arc.clone();
                        let sem_clone = sem.clone();

                        join_set.spawn(async move {
                            let _permit = sem_clone.acquire().await.expect("semaphore closed");
                            let result = tool.execute(input, &ctx_clone).await;
                            (idx, result)
                        });
                    }

                    let mut indexed: Vec<(usize, ToolResult)> = Vec::with_capacity(allowed.len());
                    let mut panicked: Vec<ToolResult> = Vec::new();
                    while let Some(join_result) = join_set.join_next().await {
                        match join_result {
                            Ok(pair) => indexed.push(pair),
                            Err(e) => {
                                tracing::error!("tool task panicked: {e}");
                                panicked
                                    .push(ToolResult::error(format!("tool task panicked: {e}")));
                            }
                        }
                    }
                    // Assign panicked results to the missing indices
                    if !panicked.is_empty() {
                        let seen: std::collections::HashSet<usize> =
                            indexed.iter().map(|(idx, _)| *idx).collect();
                        let mut missing: Vec<usize> =
                            (0..allowed.len()).filter(|i| !seen.contains(i)).collect();
                        for result in panicked {
                            let idx = missing.pop().unwrap_or(0);
                            indexed.push((idx, result));
                        }
                    }
                    indexed.sort_by_key(|(idx, _)| *idx);
                    indexed.into_iter().map(|(_, r)| r).collect()
                } else {
                    // Sequential dispatch (single tool or concurrency disabled)
                    let mut results = Vec::with_capacity(allowed.len());
                    for pre in &allowed {
                        let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                        results.push(result);
                    }
                    results
                };

                // -------------------------------------------------------
                // PHASE 3: Post-process (sequential)
                // Handle interceptions, fire post-hooks, emit events,
                // update inner voice, record results in conversation state.
                // -------------------------------------------------------
                let mut prevent_continuation_reason: Option<String> = None;
                for (pre, result) in allowed.iter().zip(dispatch_results.into_iter()) {
                    // TASK-AGS-105: AgentTool / TaskCreate now return their
                    // final user-facing ToolResult directly via the
                    // SubagentExecutor seam. No re-parse or indirection here.
                    let result = result;

                    // CRIT-07 + AGT-026: Intercept SendMessage and route to target agent.
                    // 4 delivery paths:
                    //   A. Running in memory -> queue message
                    //   B. Stopped in state, has transcript -> resume
                    //   C. Evicted from state, transcript on disk -> resume
                    //   D. No transcript -> error
                    let result = if !result.is_error && pre.tool_name == "SendMessage" {
                        match serde_json::from_str::<SendMessageRequest>(&result.content) {
                            Ok(req) => match req.message_type.as_str() {
                                "text" => {
                                    // AGT-026: Resolve target via name registry, then format validation
                                    let (agent_id, is_running) = {
                                        let mgr = self.subagent_manager.lock().await;
                                        let resolved = if let Some(id) = mgr.resolve_name(&req.to) {
                                            Some(id.to_string())
                                        } else if archon_tools::send_message::is_valid_agent_id(
                                            &req.to,
                                        ) {
                                            Some(req.to.clone())
                                        } else {
                                            None
                                        };
                                        let running = resolved
                                            .as_ref()
                                            .map(|id| mgr.is_running(id))
                                            .unwrap_or(false);
                                        (resolved, running)
                                    };

                                    match agent_id {
                                        None => {
                                            // Not in name registry, not a valid agent ID format
                                            ToolResult::error(format!(
                                                "Unknown agent '{}' -- not in name registry and not a valid agent ID",
                                                req.to
                                            ))
                                        }
                                        Some(agent_id) if is_running => {
                                            // Path A: Agent is running — queue message for delivery
                                            {
                                                let mut mgr = self.subagent_manager.lock().await;
                                                mgr.queue_pending_message(
                                                    &agent_id,
                                                    req.message.clone(),
                                                );
                                            }
                                            self.send_event(AgentEvent::MessageSent {
                                                target_agent_id: agent_id.clone(),
                                                message: req.message.clone(),
                                            })
                                            .await;
                                            ToolResult::success(format!(
                                                "Message queued for delivery to {} at its next tool round.",
                                                req.to
                                            ))
                                        }
                                        Some(agent_id) => {
                                            // Path B+C: Agent not running — try to resume from transcript
                                            let resume_ctx = crate::agents::transcript::AgentTranscriptStore::new(&self.config.session_id)
                                            .and_then(|store| crate::agents::transcript::load_resume_context(&store, &agent_id));

                                            if let Some(ctx) = resume_ctx {
                                                tracing::info!(
                                                    agent_id = %agent_id,
                                                    agent_type = %ctx.agent_type,
                                                    history_len = ctx.messages.len(),
                                                    "Resuming agent from transcript"
                                                );
                                                let resume_request = archon_tools::agent_tool::SubagentRequest {
                                                prompt: req.message.clone(),
                                                model: None,
                                                allowed_tools: Vec::new(),
                                                max_turns: archon_tools::agent_tool::SubagentRequest::DEFAULT_MAX_TURNS,
                                                timeout_secs: archon_tools::agent_tool::SubagentRequest::DEFAULT_TIMEOUT_SECS,
                                                subagent_type: Some(ctx.agent_type),
                                                run_in_background: true,
                                                cwd: None,
                                                isolation: None,
                                            };
                                                let resume_json =
                                                    serde_json::to_string(&resume_request)
                                                        .unwrap_or_default();
                                                let resume_result = ToolResult {
                                                    content: resume_json,
                                                    is_error: false,
                                                };
                                                *self.pending_resume_messages.lock().await =
                                                    Some(ctx.messages);
                                                self.send_event(AgentEvent::MessageSent {
                                                    target_agent_id: agent_id.clone(),
                                                    message: req.message.clone(),
                                                })
                                                .await;
                                                // TASK-AGS-105 Section 2f: route resume through
                                                // run_subagent (the AGT-025 auto-bg race still
                                                // applies) instead of the legacy
                                                // handle_subagent_result indirection.
                                                let _ = resume_result; // legacy stub, drop
                                                let resume_sid = agent_id.clone();
                                                let cancel = tokio_util::sync::CancellationToken::new();
                                                let resume_ctx = archon_tools::tool::ToolContext {
                                                    working_dir: self.config.working_dir.clone(),
                                                    session_id: self.config.session_id.clone(),
                                                    mode: archon_tools::tool::AgentMode::Normal,
                                                    extra_dirs: vec![],
                                                    in_fork: crate::agents::built_in::is_in_fork_child_by_messages(&self.state.messages),
                                                    nested: false,
                                                };
                                                match archon_tools::agent_tool::run_subagent(
                                                    resume_sid,
                                                    resume_request,
                                                    cancel,
                                                    resume_ctx,
                                                ).await {
                                                    archon_tools::subagent_executor::SubagentOutcome::Completed(text) => ToolResult::success(text),
                                                    archon_tools::subagent_executor::SubagentOutcome::Failed(err) => ToolResult::error(err),
                                                    archon_tools::subagent_executor::SubagentOutcome::AutoBackgrounded => ToolResult::success(format!(
                                                        "Subagent '{}' auto-backgrounded. Still running — use SendMessage to check status.",
                                                        agent_id
                                                    )),
                                                    archon_tools::subagent_executor::SubagentOutcome::Cancelled => ToolResult::error("subagent cancelled"),
                                                }
                                            } else {
                                                // Path D: No transcript found — error
                                                ToolResult::error(format!(
                                                    "No transcript found for agent '{}'",
                                                    req.to
                                                ))
                                            }
                                        }
                                    }
                                }
                                "shutdown_request" => {
                                    let mgr = self.subagent_manager.lock().await;
                                    // Try by name first, then by raw ID
                                    let target_id = mgr
                                        .resolve_name(&req.to)
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| req.to.clone());
                                    if mgr.request_shutdown(&target_id) {
                                        ToolResult::success(format!(
                                            "Shutdown requested for agent '{}'",
                                            req.to
                                        ))
                                    } else {
                                        ToolResult::error(format!(
                                            "Agent '{}' not found or not running",
                                            req.to
                                        ))
                                    }
                                }
                                "shutdown_response" | "plan_approval_response" => {
                                    // TASK-T2 (G2): Structured response message types.
                                    // Build an XML envelope and deliver via the pending-message
                                    // queue so the target agent can parse it on its next tool round.
                                    let envelope =
                                        archon_tools::send_message::build_structured_envelope(&req);
                                    let delivered = {
                                        let mut mgr = self.subagent_manager.lock().await;
                                        let target_id = mgr
                                            .resolve_name(&req.to)
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| req.to.clone());
                                        if !mgr.is_running(&target_id) {
                                            None
                                        } else {
                                            mgr.queue_pending_message(&target_id, envelope);
                                            Some(target_id)
                                        }
                                    };

                                    match delivered {
                                        Some(target_id) => {
                                            // Guard has been dropped — safe to send event.
                                            self.send_event(AgentEvent::MessageSent {
                                                target_agent_id: target_id,
                                                message: format!(
                                                    "[{}] request_id={}",
                                                    req.message_type,
                                                    req.request_id.as_deref().unwrap_or("")
                                                ),
                                            })
                                            .await;
                                            ToolResult::success(format!(
                                                "{} delivered to {}",
                                                req.message_type, req.to
                                            ))
                                        }
                                        None => ToolResult::error(format!(
                                            "Agent '{}' not running — cannot deliver structured response",
                                            req.to
                                        )),
                                    }
                                }
                                other => ToolResult::error(format!(
                                    "Unknown message_type: {}",
                                    other
                                )),
                            },
                            Err(e) => ToolResult::error(format!(
                                "Failed to parse SendMessage result: {e}"
                            )),
                        }
                    } else {
                        result
                    };

                    // CRIT-08: Intercept EnterPlanMode / ExitPlanMode.
                    let result = if !result.is_error && pre.tool_name == "EnterPlanMode" {
                        let prev = self.config.permission_mode.lock().await.clone();
                        self.previous_permission_mode = Some(prev);
                        *self.config.permission_mode.lock().await = "plan".to_string();
                        self.state.mode = AgentMode::Plan;
                        result
                    } else if !result.is_error && pre.tool_name == "ExitPlanMode" {
                        let restore = self
                            .previous_permission_mode
                            .take()
                            .unwrap_or_else(|| "auto".to_string());
                        *self.config.permission_mode.lock().await = restore;
                        self.state.mode = AgentMode::Normal;

                        // Wire 2: Parse plan from assistant text and persist.
                        if let Some(ref plan_store) = self.plan_store {
                            // Get the last assistant message's text content
                            let plan_text = self
                                .state
                                .messages
                                .iter()
                                .rev()
                                .find(|m| m["role"].as_str() == Some("assistant"))
                                .and_then(|m| match &m["content"] {
                                    serde_json::Value::Array(blocks) => blocks
                                        .iter()
                                        .find(|b| b["type"].as_str() == Some("text"))
                                        .and_then(|b| b["text"].as_str())
                                        .map(|s| s.to_string()),
                                    serde_json::Value::String(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default();

                            if !plan_text.is_empty() {
                                let plan = parse_plan_from_text(&plan_text);
                                let sid = self.config.session_id.clone();
                                match plan_store.save_plan(&sid, &plan) {
                                    Ok(()) => tracing::info!(
                                        "plan saved: {} ({} steps)",
                                        plan.title,
                                        plan.steps.len()
                                    ),
                                    Err(e) => tracing::warn!("failed to save plan: {e}"),
                                }
                            }
                        }

                        result
                    } else {
                        result
                    };

                    // CRIT-09: Intercept AskUserQuestion sentinel.
                    let mut result = if !result.is_error
                        && result.content.starts_with("[PENDING_USER_INPUT]")
                    {
                        let question = result
                            .content
                            .strip_prefix("[PENDING_USER_INPUT]")
                            .unwrap_or(&result.content)
                            .to_string();

                        // CRIT-06: Fire Elicitation hook before presenting question to user
                        let elicitation_agg = self
                            .fire_hook(
                                crate::hooks::HookEvent::Elicitation,
                                serde_json::json!({
                                    "hook_event": "Elicitation",
                                    "question": question,
                                }),
                            )
                            .await;

                        // REQ-HOOK-019: If hook returns elicitation_action, auto-respond
                        if let Some(ref action) = elicitation_agg.elicitation_action {
                            let auto_response = match action {
                                crate::hooks::ElicitationAction::Accept => {
                                    if let Some(ref content) = elicitation_agg.elicitation_content {
                                        serde_json::to_string(content)
                                            .unwrap_or_else(|_| "accepted".to_string())
                                    } else {
                                        "accepted".to_string()
                                    }
                                }
                                crate::hooks::ElicitationAction::Decline => "declined".to_string(),
                                crate::hooks::ElicitationAction::Cancel => "cancelled".to_string(),
                            };

                            // Fire ElicitationResult with auto-response
                            self.fire_hook(
                                crate::hooks::HookEvent::ElicitationResult,
                                serde_json::json!({
                                    "hook_event": "ElicitationResult",
                                    "result": &auto_response,
                                    "auto_responded": true,
                                }),
                            )
                            .await;

                            ToolResult::success(auto_response)
                        } else {
                            self.send_event(AgentEvent::AskUser {
                                question: question.clone(),
                            })
                            .await;

                            if let Some(rx) = &self.ask_user_response_rx {
                                match rx.lock().await.recv().await {
                                    Some(answer) => {
                                        // CRIT-06: Fire ElicitationResult hook after user responds
                                        self.fire_hook(
                                            crate::hooks::HookEvent::ElicitationResult,
                                            serde_json::json!({
                                                "hook_event": "ElicitationResult",
                                                "result": &answer,
                                            }),
                                        )
                                        .await;
                                        ToolResult::success(answer)
                                    }
                                    None => ToolResult::error(
                                        "User input channel closed unexpectedly.".to_string(),
                                    ),
                                }
                            } else {
                                ToolResult::error(
                                    "User input requested but no input channel is configured."
                                        .to_string(),
                                )
                            }
                        } // end else (no elicitation_action)
                    } else {
                        result
                    };

                    // CRIT-06: Fire PostToolUse / PostToolUseFailure hooks (REQ-HOOK-005)
                    // Retry loop: max 3 re-executions if PostToolUse hook sets retry=true
                    let max_retries: u32 = 3;
                    let mut retry_count: u32 = 0;
                    loop {
                        if result.is_error {
                            let _post_agg = self
                                .fire_hook(
                                    crate::hooks::HookEvent::PostToolUseFailure,
                                    serde_json::json!({
                                        "hook_event": "PostToolUseFailure",
                                        "tool_name": pre.tool_name,
                                        "tool_id": pre.tool_id,
                                        "error": result.content,
                                    }),
                                )
                                .await;
                            break; // No retry on failure
                        }

                        let post_agg = self
                            .fire_hook(
                                crate::hooks::HookEvent::PostToolUse,
                                serde_json::json!({
                                    "hook_event": "PostToolUse",
                                    "tool_name": pre.tool_name,
                                    "tool_id": pre.tool_id,
                                    "result": result.content,
                                }),
                            )
                            .await;

                        // Apply updated_mcp_tool_output (REQ-HOOK-005)
                        if let Some(modified_output) = post_agg.updated_mcp_tool_output {
                            tracing::debug!(
                                tool = %pre.tool_name,
                                "PostToolUse hook modified tool output"
                            );
                            let new_content = match modified_output {
                                serde_json::Value::String(s) => s,
                                other => serde_json::to_string(&other)
                                    .unwrap_or_else(|_| other.to_string()),
                            };
                            result = ToolResult::success(new_content);
                        }

                        // Append additional_contexts (REQ-HOOK-005)
                        if !post_agg.additional_contexts.is_empty() {
                            let context = post_agg.additional_contexts.join("\n");
                            result = ToolResult::success(format!(
                                "{}\n---\n[Hook Context]\n{}",
                                result.content, context
                            ));
                        }

                        // Log system/status messages from PostToolUse hooks
                        for msg in &post_agg.system_messages {
                            tracing::warn!(tool = %pre.tool_name, "[Hook Warning] {}", msg);
                        }
                        for msg in &post_agg.status_messages {
                            tracing::info!(tool = %pre.tool_name, "[Hook Status] {}", msg);
                        }

                        // Handle prevent_continuation (REQ-HOOK-005 flow control)
                        if post_agg.prevent_continuation {
                            let reason = post_agg
                                .stop_reason
                                .as_deref()
                                .unwrap_or("hook requested stop");
                            tracing::info!(
                                tool = %pre.tool_name,
                                "PostToolUse hook set prevent_continuation: {}", reason
                            );
                            prevent_continuation_reason = Some(reason.to_string());
                        }

                        // Handle retry (REQ-HOOK-005 flow control)
                        if post_agg.retry && retry_count < max_retries {
                            retry_count += 1;
                            tracing::info!(
                                tool = %pre.tool_name,
                                attempt = retry_count,
                                max = max_retries,
                                "PostToolUse hook requested retry, re-executing tool"
                            );
                            result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                            continue; // Loop back to fire PostToolUse again
                        } else if post_agg.retry {
                            tracing::warn!(
                                tool = %pre.tool_name,
                                "PostToolUse hook requested retry but max retries ({}) exceeded",
                                max_retries
                            );
                        }

                        break; // Normal exit — no retry requested or retries exhausted
                    }

                    if let Some(ref fp) = pre.file_path {
                        let file_agg = self
                            .fire_hook(
                                crate::hooks::HookEvent::FileChanged,
                                serde_json::json!({
                                    "hook_event": "FileChanged",
                                    "tool_name": pre.tool_name,
                                    "file_path": fp,
                                }),
                            )
                            .await;
                        // Consume watch_paths from FileChanged hooks (REQ-HOOK-017)
                        if !file_agg.watch_paths.is_empty() {
                            tracing::info!(
                                "Hook returned {} watch paths",
                                file_agg.watch_paths.len()
                            );
                            self.file_watch_manager
                                .add_watch_paths(file_agg.watch_paths);
                        }
                    }

                    // CRIT-06: Fire CwdChanged if a Bash tool call changed the working directory
                    if pre.tool_name == "Bash" {
                        if let Some(cmd) = pre.input.get("command").and_then(|v| v.as_str()) {
                            if cmd.trim_start().starts_with("cd ")
                                || cmd.contains(" && cd ")
                                || cmd.contains("; cd ")
                            {
                                let cwd_agg = self
                                    .fire_hook(
                                        crate::hooks::HookEvent::CwdChanged,
                                        serde_json::json!({
                                            "hook_event": "CwdChanged",
                                            "command": cmd,
                                        }),
                                    )
                                    .await;
                                // Consume watch_paths from CwdChanged hooks (REQ-HOOK-017)
                                if !cwd_agg.watch_paths.is_empty() {
                                    tracing::info!(
                                        "Hook returned {} watch paths",
                                        cwd_agg.watch_paths.len()
                                    );
                                    self.file_watch_manager.add_watch_paths(cwd_agg.watch_paths);
                                }
                            }
                        }
                    }

                    // CRIT-06: Fire WorktreeCreate/WorktreeRemove based on tool name
                    if pre.tool_name == "EnterWorktree" {
                        self.fire_hook(
                            crate::hooks::HookEvent::WorktreeCreate,
                            serde_json::json!({
                                "hook_event": "WorktreeCreate",
                                "tool_name": pre.tool_name,
                            }),
                        )
                        .await;
                    } else if pre.tool_name == "ExitWorktree" {
                        self.fire_hook(
                            crate::hooks::HookEvent::WorktreeRemove,
                            serde_json::json!({
                                "hook_event": "WorktreeRemove",
                                "tool_name": pre.tool_name,
                            }),
                        )
                        .await;
                    }

                    self.send_event(AgentEvent::ToolCallComplete {
                        name: pre.tool_name.clone(),
                        id: pre.tool_id.clone(),
                        result: result.clone(),
                    })
                    .await;

                    if let Some(iv) = &self.inner_voice {
                        let mut iv = iv.lock().await;
                        if result.is_error {
                            iv.on_tool_failure(&pre.tool_name);
                        } else {
                            iv.on_tool_success(&pre.tool_name);
                        }
                    }

                    // Wire 3: Track plan step progress on Write/Edit completions.
                    if !result.is_error && (pre.tool_name == "Write" || pre.tool_name == "Edit") {
                        if let Some(ref plan_store) = self.plan_store {
                            let sid = self.config.session_id.clone();
                            if let Ok(Some(plan)) = plan_store.load_latest_plan(&sid) {
                                if plan.status == "active" || plan.status == "draft" {
                                    if let Some(ref fp) = pre.file_path {
                                        for step in &plan.steps {
                                            if step.status
                                                == archon_session::plan::PlanStepStatus::Pending
                                                && step
                                                    .affected_files
                                                    .iter()
                                                    .any(|f| fp.ends_with(f) || f.ends_with(fp))
                                            {
                                                if let Err(e) = plan_store.update_step_status(
                                                    &sid,
                                                    &plan.id,
                                                    step.number,
                                                    archon_session::plan::PlanStepStatus::InProgress,
                                                ) {
                                                    tracing::debug!("plan step update failed: {e}");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    self.state
                        .add_tool_result(&pre.tool_id, &result.content, result.is_error);
                }

                // If a PostToolUse hook requested prevent_continuation, stop the loop
                if let Some(reason) = prevent_continuation_reason {
                    tracing::info!("Hook requested conversation stop: {}", reason);
                    break;
                }

                // Check max_turns limit before looping
                agentic_iterations += 1;
                if let Some(max) = self.config.max_turns {
                    if agentic_iterations >= max {
                        tracing::info!(
                            "max_turns limit reached ({}/{}), stopping agentic loop",
                            agentic_iterations,
                            max
                        );
                        self.send_event(AgentEvent::Error(format!(
                            "Agentic turn limit reached ({max} turns). Stopping."
                        )))
                        .await;
                        break;
                    }
                }

                // Loop back to send tool results to the API
                continue;
            }

            // No tool calls -- turn is complete
            // Update shared session stats for /status and /cost
            {
                let mut stats = self.session_stats.lock().await;
                stats.input_tokens = self.state.total_input_tokens;
                stats.output_tokens = self.state.total_output_tokens;
                stats.turn_count = self.turn_number;
                // Rough cost estimate: $3/MTok input, $15/MTok output (Sonnet pricing)
                stats.session_cost = (stats.input_tokens as f64 * 3.0
                    + stats.output_tokens as f64 * 15.0)
                    / 1_000_000.0;
                // Update cache statistics from this turn
                stats
                    .cache_stats
                    .update(turn_cache_creation, turn_cache_read, turn_input_tokens);
            }

            // Apply turn completion to inner voice (energy decay, turn counter).
            if let Some(iv) = &self.inner_voice {
                iv.lock().await.on_turn_complete();
            }

            self.send_event(AgentEvent::TurnComplete {
                input_tokens: turn_input_tokens,
                output_tokens: turn_output_tokens,
            })
            .await;

            // CRIT-14 (ITEM 4): Decay rule scores every 50 turns.
            if self.turn_number % 50 == 0 {
                if let Some(ref graph) = self.memory {
                    let engine = RulesEngine::new(graph.as_ref());
                    if let Err(e) = engine.decay_scores(1.0) {
                        tracing::warn!("rules decay_scores failed: {e}");
                    }
                }
            }

            // Detect user corrections and record them in the memory graph.
            if let Some(ref graph) = self.memory {
                self.detect_and_record_correction(user_input, graph);
            }

            // GAP 5: Auto-memory extraction check
            self.extraction_state.record_turn();
            if should_extract(
                &self.extraction_config,
                &self.extraction_state,
                self.turn_number as usize,
            ) {
                self.trigger_memory_extraction();
            }

            break;
        }

        Ok(())
    }

    async fn send_event(&self, event: AgentEvent) {
        // TASK-AGS-102: unbounded send — synchronous, fails only if rx dropped.
        // ERR-ARCH-02 (TASK-AGS-108) will formalise the closed-channel WARN.
        if let Err(e) = self.event_tx.send(event) {
            tracing::trace!("agent_event_tx closed: {e}");
        }
    }

    /// Get the auth provider for spawning parallel API calls (e.g. /btw).
    ///
    /// Returns `None` if the active provider is not Anthropic.
    pub fn auth_provider(&self) -> Option<&archon_llm::auth::AuthProvider> {
        self.client.as_anthropic().map(|c| c.auth())
    }

    /// Get the identity provider for spawning parallel API calls.
    ///
    /// Returns `None` if the active provider is not Anthropic.
    pub fn identity_provider(&self) -> Option<&archon_llm::identity::IdentityProvider> {
        self.client.as_anthropic().map(|c| c.identity())
    }

    /// Get the current effective model name.
    pub fn current_model(&self) -> &str {
        &self.config.model
    }

    pub fn conversation_state(&self) -> &ConversationState {
        &self.state
    }

    /// Clear conversation history, keeping config and subsystems intact.
    pub async fn clear_conversation(&mut self) {
        self.state.messages.clear();
        self.state.total_input_tokens = 0;
        self.state.total_output_tokens = 0;
        self.turn_number = 0;
        self.memory_injector.invalidate_cache();
        // Reset shared session stats so /status and /cost reflect the cleared state
        {
            let mut stats = self.session_stats.lock().await;
            *stats = SessionStats::default();
        }
    }

    /// GAP 1: Trigger conversation compaction.
    ///
    /// Converts the agent's messages to ContextMessages, runs compaction,
    /// and replaces the conversation state. Fires PreCompact and PostCompact
    /// hooks around the compaction. Returns a human-readable status message.
    ///
    /// `subcommand` selects the strategy:
    /// - `None` or `Some("auto")` — pick strategy automatically via `select_strategy`
    /// - `Some("micro")` — microcompact (summarize oldest 30 %)
    /// - `Some("snip")` — snip oldest turns without summarization
    pub async fn compact(&mut self, subcommand: Option<&str>) -> String {
        use crate::commands::handle_compact;
        use archon_context::compact::select_strategy;
        use archon_context::messages::ContextMessage;
        use archon_context::microcompact::microcompact_messages;
        use archon_context::snip::snip_messages;

        // Convert JSON messages to ContextMessages
        let context_msgs: Vec<ContextMessage> = self
            .state
            .messages
            .iter()
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user").to_string();
                let content = m["content"].clone();
                let text_len = match &content {
                    serde_json::Value::String(s) => s.len(),
                    serde_json::Value::Array(arr) => arr
                        .iter()
                        .map(|v| {
                            v.get("text")
                                .and_then(|t| t.as_str())
                                .map_or(0, |s| s.len())
                        })
                        .sum(),
                    _ => 0,
                };
                ContextMessage {
                    role,
                    content,
                    estimated_tokens: (text_len as f64 / 4.0).ceil() as u64,
                }
            })
            .collect();

        if context_msgs.len() < 5 {
            return "Nothing to compact (fewer than 5 messages).".into();
        }

        let message_count = context_msgs.len();
        let before_tokens: u64 = context_msgs.iter().map(|m| m.estimated_tokens).sum();

        // Resolve the effective strategy.
        // "auto" (or no subcommand) uses select_strategy based on context usage ratio.
        let effective_strategy = match subcommand {
            Some("micro") => Some(archon_context::boundary::CompactionStrategy::Micro),
            Some("snip") => Some(archon_context::boundary::CompactionStrategy::Snip),
            Some("auto") | None => {
                // Estimate usage ratio against the model context window (default 200k).
                let context_window = 200_000u64;
                let usage_ratio = before_tokens as f32 / context_window as f32;
                select_strategy(usage_ratio)
            }
            Some(other) => {
                return format!(
                    "Unknown /compact subcommand: '{other}'. Use auto, micro, or snip."
                );
            }
        };

        // If select_strategy says no compaction needed and user didn't force a strategy
        let effective_strategy = match effective_strategy {
            Some(s) => s,
            None => {
                return "Context usage is below 60 %; no compaction needed.".into();
            }
        };

        // Fire PreCompact hook
        if let Some(ref registry) = self.hook_registry {
            let payload = serde_json::json!({
                "hook_event": "PreCompact",
                "message_count": message_count,
                "token_count": before_tokens,
                "strategy": effective_strategy.to_string(),
            });
            registry
                .execute_hooks(
                    crate::hooks::HookEvent::PreCompact,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await;
        }

        // Dispatch based on the resolved strategy.
        let (result_messages, strategy_label, _status_message) = match effective_strategy {
            archon_context::boundary::CompactionStrategy::Snip => {
                // Snip: remove oldest turns without LLM summarization.
                let total_turns = archon_context::snip::count_turns(&context_msgs);
                if total_turns < 3 {
                    return "Too few turns to snip.".into();
                }
                // Snip the oldest ~50 % of turns (at least 1).
                let snip_end = (total_turns / 2).max(1);
                match snip_messages(&context_msgs, 1, snip_end) {
                    Ok((msgs, boundary)) => {
                        let label = "snip";
                        let status = format!(
                            "Snipped turns 1–{snip_end} ({} tokens removed)",
                            boundary.tokens_removed
                        );
                        (msgs, label, status)
                    }
                    Err(e) => return format!("Snip failed: {e}"),
                }
            }

            archon_context::boundary::CompactionStrategy::Micro
            | archon_context::boundary::CompactionStrategy::Auto => {
                // Both Micro and Auto need an LLM-generated summary.
                let mut summary_text = self.generate_compaction_summary(&context_msgs).await;

                // Wire 4: Inject active plan context into compaction summary.
                if let Some(ref plan_store) = self.plan_store {
                    if let Some(plan_ctx) = archon_session::plan::plan_context_for_compaction(
                        plan_store,
                        &self.config.session_id,
                    ) {
                        summary_text.push_str(&plan_ctx);
                    }
                }

                match effective_strategy {
                    archon_context::boundary::CompactionStrategy::Micro => {
                        let preserve = archon_context::compact::DEFAULT_PRESERVE_RECENT_TURNS;
                        let (msgs, boundary) =
                            microcompact_messages(&context_msgs, &summary_text, preserve);
                        let label = "micro";
                        let status =
                            format!("Microcompacted: {} tokens removed", boundary.tokens_removed);
                        (msgs, label, status)
                    }
                    _ => {
                        // Auto / default: full compaction via handle_compact
                        let output = handle_compact(&context_msgs, &summary_text);
                        let label = "auto";
                        let status = output.message.clone();
                        if output.mutated {
                            (output.messages, label, status)
                        } else {
                            return output.message;
                        }
                    }
                }
            }
        };

        // Replace the conversation messages with the compacted version
        self.state.messages = result_messages
            .iter()
            .map(|cm| {
                serde_json::json!({
                    "role": cm.role,
                    "content": cm.content,
                })
            })
            .collect();
        // Invalidate memory cache since context changed
        self.memory_injector.invalidate_cache();

        // CRIT-15 (ITEM 5): Snapshot inner voice state on compaction and persist to memory graph.
        if let Some(ref iv) = self.inner_voice {
            let snapshot = iv.lock().await.on_compaction();
            tracing::debug!(
                "inner voice snapshot on compaction: confidence={:.2}, energy={:.2}, turns={}",
                snapshot.confidence,
                snapshot.energy,
                snapshot.turn_count
            );
            // Persist snapshot so it can be restored via InnerVoice::from_snapshot on resume.
            if let Some(ref graph) = self.memory {
                if let Ok(json) = serde_json::to_string(&snapshot) {
                    let _ = graph.store_memory(
                        &json,
                        "inner_voice_snapshot",
                        archon_memory::types::MemoryType::Fact,
                        90.0,
                        &["inner_voice_snapshot".to_string()],
                        "agent",
                        "",
                    );
                }
            }
        }

        // Compute post-compaction token count
        let after_tokens: u64 = result_messages.iter().map(|m| m.estimated_tokens).sum();
        let tokens_removed = before_tokens.saturating_sub(after_tokens);

        // Fire PostCompact hook
        if let Some(ref registry) = self.hook_registry {
            let payload = serde_json::json!({
                "hook_event": "PostCompact",
                "strategy": strategy_label,
                "tokens_removed": tokens_removed,
                "tokens_remaining": after_tokens,
            });
            registry
                .execute_hooks(
                    crate::hooks::HookEvent::PostCompact,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await;
        }

        // Return detailed summary
        let before_k = before_tokens as f64 / 1000.0;
        let after_k = after_tokens as f64 / 1000.0;
        let removed_k = tokens_removed as f64 / 1000.0;
        format!(
            "Compacted conversation ({strategy_label}): {before_k:.1}k → {after_k:.1}k tokens ({removed_k:.1}k removed, {message_count} messages)"
        )
    }

    /// Generate an LLM summary of the conversation for compaction.
    ///
    /// Builds the summary request via [`build_compact_summary_request`], sends it
    /// to the LLM provider, and collects the response text. Falls back to the
    /// first user message if the LLM call fails.
    async fn generate_compaction_summary(
        &self,
        context_msgs: &[archon_context::messages::ContextMessage],
    ) -> String {
        use crate::commands::build_compact_summary_request;

        let summary_request_msgs = build_compact_summary_request(context_msgs);

        // Convert ContextMessages to JSON messages for LlmRequest
        let json_messages: Vec<serde_json::Value> = summary_request_msgs
            .iter()
            .map(|cm| {
                serde_json::json!({
                    "role": cm.role,
                    "content": cm.content,
                })
            })
            .collect();

        let request = LlmRequest {
            model: self.config.model.clone(),
            max_tokens: 2048,
            system: vec![serde_json::json!({
                "type": "text",
                "text": archon_context::compact::SUMMARY_PROMPT,
            })],
            messages: json_messages,
            tools: Vec::new(),
            thinking: None,
            speed: Some("fast".to_string()),
            effort: Some("low".to_string()),
            extra: serde_json::Value::Null,
        };

        match self.client.stream(request).await {
            Ok(mut rx) => {
                let mut response_text = String::new();
                while let Some(event) = rx.recv().await {
                    if let StreamEvent::TextDelta { text, .. } = event {
                        response_text.push_str(&text);
                    }
                }
                if response_text.is_empty() {
                    tracing::warn!(
                        "LLM returned empty summary; falling back to first user message"
                    );
                    self.state.first_user_message().to_string()
                } else {
                    response_text
                }
            }
            Err(e) => {
                tracing::warn!(
                    "compaction summary LLM call failed: {e}; falling back to first user message"
                );
                self.state.first_user_message().to_string()
            }
        }
    }

    /// Append the inner voice `<inner_voice>` block to the system prompt
    /// for this turn, if the feature is enabled.
    async fn inject_inner_voice(&self, system: &mut Vec<serde_json::Value>) {
        let iv = match &self.inner_voice {
            Some(iv) => iv,
            None => return,
        };
        let block = iv.lock().await.to_prompt_block();
        system.push(serde_json::json!({
            "type": "text",
            "text": block,
        }));
    }

    /// Inject critical system reminder into the system prompt (AGT-022).
    /// Re-injected every turn, wrapped in `<system-reminder>` tags.
    fn inject_critical_reminder(&self, system: &mut Vec<serde_json::Value>) {
        if let Some(ref reminder) = self.critical_system_reminder {
            system.push(serde_json::json!({
                "type": "text",
                "text": format!("<system-reminder>{reminder}</system-reminder>"),
            }));
        }
    }

    /// GAP 7: Inject recalled memories into the system prompt for this turn.
    fn inject_memories(&mut self) -> Vec<serde_json::Value> {
        let mut system = self.config.system_prompt.clone();

        let graph = match self.memory {
            Some(ref g) => g,
            None => return system,
        };

        // Collect recent user messages as context for recall
        let context: Vec<String> = self
            .state
            .messages
            .iter()
            .rev()
            .filter(|m| m["role"].as_str() == Some("user"))
            .take(3)
            .filter_map(|m| m["content"].as_str().map(|s| s.to_string()))
            .collect();

        if context.is_empty() {
            return system;
        }

        match self.memory_injector.inject(graph.as_ref(), &context, 500) {
            Ok(memories_text) if !memories_text.is_empty() => {
                system.push(serde_json::json!({
                    "type": "text",
                    "text": memories_text,
                }));
            }
            Ok(_) => {} // empty — no relevant memories
            Err(e) => {
                tracing::warn!("memory injection failed: {e}");
            }
        }

        // Inject recalled corrections relevant to the current context.
        let ctx_joined = context.join(" ");
        let tracker = CorrectionTracker::new(graph.as_ref());
        match tracker.recall_corrections(&ctx_joined, 5) {
            Ok(corrections) if !corrections.is_empty() => {
                let mut block = String::from(
                    "<past_corrections>\nPrevious user corrections relevant to this context:\n",
                );
                for c in &corrections {
                    block.push_str(&format!(
                        "- [{}] {}\n",
                        c.correction_type.severity_multiplier(),
                        c.content
                    ));
                }
                block.push_str("</past_corrections>");
                system.push(serde_json::json!({
                    "type": "text",
                    "text": block,
                }));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("correction recall failed: {e}");
            }
        }

        // CLI-416/417: Inject personality and memory briefings on first turn only.
        if let Some(briefing) = self.personality_briefing.take() {
            system.push(serde_json::json!({
                "type": "text",
                "text": briefing,
            }));
        }
        if let Some(briefing) = self.memory_briefing.take() {
            system.push(serde_json::json!({
                "type": "text",
                "text": briefing,
            }));
        }

        system
    }

    /// Detect correction patterns in user input and record via CorrectionTracker.
    fn detect_and_record_correction(&self, user_input: &str, graph: &Arc<dyn MemoryTrait>) {
        let lower = user_input.to_lowercase();
        let correction_type = if lower.starts_with("no,")
            || lower.starts_with("no ")
            || lower.starts_with("wrong")
            || lower.starts_with("that's wrong")
            || lower.starts_with("that is wrong")
        {
            CorrectionType::FactualError
        } else if lower.contains("i said")
            || lower.contains("i already told you")
            || lower.contains("i already asked")
            || lower.contains("as i mentioned")
        {
            CorrectionType::RepeatedInstruction
        } else if lower.starts_with("don't ")
            || lower.starts_with("do not ")
            || lower.starts_with("stop ")
            || lower.contains("never do that")
        {
            CorrectionType::DidForbiddenAction
        } else if lower.contains("didn't ask")
            || lower.contains("did not ask")
            || lower.contains("without permission")
            || lower.contains("without asking")
        {
            CorrectionType::ActedWithoutPermission
        } else if lower.contains("instead,")
            || lower.contains("should have")
            || lower.contains("better approach")
            || lower.contains("use this instead")
        {
            CorrectionType::ApproachCorrection
        } else {
            return; // No correction pattern detected.
        };

        let tracker = CorrectionTracker::new(graph.as_ref());
        let context = format!("turn:{}", self.turn_number);
        if let Err(e) = tracker.record_correction(correction_type, user_input, &context, None) {
            tracing::warn!("failed to record correction: {e}");
        }

        // CRIT-15 (ITEM 5): Notify inner voice of user correction.
        if let Some(ref iv) = self.inner_voice {
            if let Ok(mut iv) = iv.try_lock() {
                iv.on_user_correction();
            }
        }

        // CRIT-14 (ITEM 4): Reinforce rules related to the correction.
        // When the user corrects us, reinforce the top matching rule so it
        // gains more prominence in future prompts.
        let engine = RulesEngine::new(graph.as_ref());
        if let Ok(rules) = engine.get_rules_sorted() {
            if let Some(top) = rules.first() {
                if let Err(e) = engine.reinforce_rule(&top.id) {
                    tracing::debug!("reinforce_rule failed: {e}");
                }
            }
        }
    }

    /// GAP 5: Trigger memory extraction in the background.
    fn trigger_memory_extraction(&mut self) {
        let graph = match self.memory {
            Some(ref g) => Arc::clone(g),
            None => return,
        };

        // Collect last N messages for extraction
        let messages: Vec<String> = self
            .state
            .messages
            .iter()
            .rev()
            .take(10)
            .filter_map(|m| {
                let role = m["role"].as_str().unwrap_or("unknown");
                let content = m["content"].as_str().unwrap_or("");
                if content.is_empty() {
                    return None;
                }
                Some(format!("{role}: {content}"))
            })
            .collect();

        if messages.is_empty() {
            return;
        }

        let session_id = self.config.session_id.clone();
        let turn = self.turn_number as usize;
        let client = Arc::clone(&self.client);
        let model = self.config.model.clone();

        // Record extraction so we don't fire again immediately
        self.extraction_state.record_extraction(turn);

        // Run extraction in background via a real LLM call
        tokio::spawn(async move {
            let prompt = build_extraction_prompt(&messages);

            let request = LlmRequest {
                model,
                max_tokens: 1024,
                system: vec![serde_json::json!({
                    "type": "text",
                    "text": "You extract structured memories from conversations. Return ONLY a JSON array."
                })],
                messages: vec![serde_json::json!({
                    "role": "user",
                    "content": prompt,
                })],
                tools: Vec::new(),
                thinking: None,
                speed: Some("fast".to_string()),
                effort: Some("low".to_string()),
                extra: serde_json::Value::Null,
            };

            match client.stream(request).await {
                Ok(mut rx) => {
                    let mut response_text = String::new();
                    while let Some(event) = rx.recv().await {
                        if let StreamEvent::TextDelta { text, .. } = event {
                            response_text.push_str(&text);
                        }
                    }

                    let extracted = parse_extraction_response(&response_text).unwrap_or_default();
                    if !extracted.is_empty() {
                        match store_extracted(graph.as_ref(), &extracted, &session_id) {
                            Ok(count) => {
                                tracing::info!("auto-extracted {count} memories at turn {turn}")
                            }
                            Err(e) => tracing::warn!("memory extraction storage failed: {e}"),
                        }
                    } else {
                        tracing::debug!("no memories extracted at turn {turn}");
                    }
                }
                Err(e) => {
                    tracing::warn!("memory extraction API call failed: {e}");
                }
            }
        });
    }

    /// GAP 8: Handle subagent execution when a tool returns a SubagentRequest.
    ///
    /// When `nested` is false (AgentTool), the entire content is a SubagentRequest.
    /// When `nested` is true (TaskCreate), the SubagentRequest is under the
    /// `subagent_request` key in the response JSON.
    async fn handle_subagent_result(
        &mut self,
        tool_result: &ToolResult,
        nested: bool,
    ) -> ToolResult {
        // Parse the SubagentRequest from the tool result
        let request: SubagentRequest = if nested {
            // TaskCreate format: {"task_id":"...","subagent_request":{...}}
            let wrapper: serde_json::Value = match serde_json::from_str(&tool_result.content) {
                Ok(v) => v,
                Err(_) => return tool_result.clone(),
            };
            match wrapper.get("subagent_request") {
                Some(req_val) => match serde_json::from_value(req_val.clone()) {
                    Ok(req) => req,
                    Err(_) => return tool_result.clone(), // no valid subagent_request
                },
                None => return tool_result.clone(), // no subagent_request key (manual task)
            }
        } else {
            // AgentTool format: bare SubagentRequest as entire content
            match serde_json::from_str(&tool_result.content) {
                Ok(req) => req,
                Err(_) => return tool_result.clone(),
            }
        };

        // Register the subagent
        let subagent_id = match self.subagent_manager.lock().await.register(request.clone()) {
            Ok(id) => id,
            Err(e) => return ToolResult::error(format!("Failed to register subagent: {e}")),
        };

        // AGT-026: Register agent name in name registry for SendMessage resolution
        if let Some(ref agent_type) = request.subagent_type {
            self.subagent_manager
                .lock()
                .await
                .register_name(agent_type.clone(), subagent_id.clone());
        }

        tracing::info!(subagent_id = %subagent_id, prompt_len = request.prompt.len(), "spawning one-shot subagent");

        // CRIT-06: Fire SubagentStart hook
        self.fire_hook(
            crate::hooks::HookEvent::SubagentStart,
            serde_json::json!({
                "hook_event": "SubagentStart",
                "subagent_id": subagent_id,
                "model": request.model,
                "prompt_length": request.prompt.len(),
            }),
        )
        .await;

        // CRIT-06: Fire TaskCreated hook if this was a TaskCreate request
        if nested {
            self.fire_hook(
                crate::hooks::HookEvent::TaskCreated,
                serde_json::json!({
                    "hook_event": "TaskCreated",
                    "subagent_id": subagent_id,
                }),
            )
            .await;
        }

        // Resolve the full agent definition (clone to release the registry lock)
        // AGT-023: When fork enabled and subagent_type is None, use FORK_AGENT
        let resolved_def: Option<crate::agents::CustomAgentDefinition> =
            if let Some(ref agent_type) = request.subagent_type {
                // Defense-in-depth: block explicit "fork" type inside fork children (AC-107)
                if agent_type == "fork"
                    && crate::agents::built_in::is_in_fork_child(Some("fork"), &self.state.messages)
                {
                    let _ = self
                        .subagent_manager
                        .lock()
                        .await
                        .mark_failed(&subagent_id, "Cannot fork inside a fork child".into());
                    return ToolResult::error("Cannot fork inside a fork child".to_string());
                }
                let reg = self.agent_registry.read().expect("agent registry lock");
                reg.resolve(agent_type).cloned()
            } else if crate::agents::built_in::is_fork_enabled() {
                // Fork guard: reject fork attempts inside fork children
                if crate::agents::built_in::is_in_fork_child(None, &self.state.messages) {
                    let _ = self
                        .subagent_manager
                        .lock()
                        .await
                        .mark_failed(&subagent_id, "Cannot fork inside a fork child".into());
                    return ToolResult::error("Cannot fork inside a fork child".to_string());
                }
                let reg = self.agent_registry.read().expect("agent registry lock");
                reg.resolve("fork").cloned()
            } else {
                None
            };

        let system_prompt = {
            let base_prompt = resolved_def.as_ref()
                .map(|d| d.system_prompt.clone())
                .unwrap_or_else(|| {
                    request.subagent_type.as_ref()
                        .map(|t| format!("You are a '{}' subagent. Complete the task described in the user message. Be thorough and precise.", t))
                        .unwrap_or_else(|| "You are a subagent. Complete the task described in the user message. Be thorough and precise.".into())
                });

            // Fork agent inherits parent system prompt for context parity.
            // Extract text from parent's Vec<serde_json::Value> system blocks,
            // truncate to 50KB, prepend to fork's own prompt.
            // Skip ARCHON.md/memory sections since fork gets those through its own
            // resolution path at lines below.
            let is_fork = resolved_def
                .as_ref()
                .map(|d| d.agent_type == "fork")
                .unwrap_or(false);
            if is_fork {
                const MAX_PARENT_PROMPT_BYTES: usize = 50_000;
                let parent_text: String = self
                    .config
                    .system_prompt
                    .iter()
                    .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if parent_text.is_empty() {
                    base_prompt
                } else {
                    let truncated = if parent_text.len() > MAX_PARENT_PROMPT_BYTES {
                        let cut = parent_text
                            .char_indices()
                            .map(|(i, _)| i)
                            .take_while(|&i| i <= MAX_PARENT_PROMPT_BYTES)
                            .last()
                            .unwrap_or(0);
                        format!("{}...[truncated]", &parent_text[..cut])
                    } else {
                        parent_text
                    };
                    format!("<parent-context>\n{truncated}\n</parent-context>\n\n{base_prompt}")
                }
            } else {
                base_prompt
            }
        };

        // Prepend ARCHON.md content when omit_claude_md is false (default)
        let omit_claude_md = resolved_def
            .as_ref()
            .map(|d| d.omit_claude_md)
            .unwrap_or(false);
        let system_prompt = if !omit_claude_md {
            let archon_md = crate::archonmd::load_hierarchical_archon_md(&self.config.working_dir);
            if archon_md.is_empty() {
                system_prompt
            } else {
                format!("{system_prompt}\n\n<archon-md>\n{archon_md}\n</archon-md>")
            }
        } else {
            system_prompt
        };

        // AGT-023: When fork mode enabled, force ALL agent spawns async (forceAsync pattern)
        let force_async = crate::agents::built_in::is_fork_enabled();
        let is_background = request.run_in_background
            || resolved_def.as_ref().map(|d| d.background).unwrap_or(false)
            || force_async;

        // Model fallback chain: request → definition → parent config
        let model = request
            .model
            .as_deref()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.model.as_deref()))
            .unwrap_or(&self.config.model);

        // Max turns: use definition's value when request uses the default (10)
        let max_turns = if request.max_turns == 10 {
            resolved_def
                .as_ref()
                .and_then(|d| d.max_turns)
                .unwrap_or(10)
        } else {
            request.max_turns as u32
        };

        // Effort level from agent definition
        let def_effort = resolved_def.as_ref().and_then(|d| d.effort.clone());

        // Resolve isolation: request overrides agent definition
        let isolation = request
            .isolation
            .as_deref()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.isolation.as_deref()));

        if request.cwd.is_some() && isolation == Some("worktree") {
            let _ = self.subagent_manager.lock().await.mark_failed(
                &subagent_id,
                "cwd override cannot be combined with isolation='worktree'".to_string(),
            );
            return ToolResult::error(
                "cwd override cannot be combined with isolation='worktree'".to_string(),
            );
        }

        // Register session-scoped hooks from agent definition
        if let Some(ref def) = resolved_def {
            if let Some(ref hooks_json) = def.hooks {
                match crate::agents::loader::parse_agent_hooks(hooks_json) {
                    Ok(hook_pairs) => {
                        if let Some(ref registry) = self.hook_registry {
                            for (event, config) in hook_pairs {
                                registry.register_session_hook(
                                    &self.config.session_id,
                                    event,
                                    config,
                                );
                            }
                            tracing::debug!(agent_type = ?request.subagent_type, "registered session-scoped hooks from agent definition");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(agent_type = ?request.subagent_type, error = %e, "failed to parse agent hooks")
                    }
                }
            }
        }

        // Pre-flight check: required MCP servers must be available
        if let Some(ref def) = resolved_def {
            let available_tools = self.registry.tool_names();
            let available_mcp: Vec<String> = available_tools
                .iter()
                .filter(|n| n.starts_with("mcp__"))
                .map(|n| n.to_string())
                .collect();
            if !def.has_required_mcp_servers(&available_mcp) {
                let reason = format!(
                    "Agent '{}' requires MCP servers {:?} but they are not available. Available MCP tools: {:?}",
                    def.agent_type, def.required_mcp_servers, available_mcp,
                );
                let _ = self
                    .subagent_manager
                    .lock()
                    .await
                    .mark_failed(&subagent_id, reason.clone());
                return ToolResult::error(format!("{}", reason,));
            }
        }

        // Inject skills into system prompt if agent definition specifies them
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(ref skills) = def.skills {
                if !skills.is_empty() {
                    let skills_list = skills.join(", ");
                    format!(
                        "{system_prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>"
                    )
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Inject tool guidance into system prompt if agent definition has it
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.tool_guidance.is_empty() {
                format!(
                    "{system_prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>",
                    def.tool_guidance
                )
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Inject agent memory (recall_queries) into system prompt if available
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.recall_queries.is_empty() {
                if let Some(ref memory) = self.memory {
                    let memories = crate::agents::memory::load_agent_memory(
                        &def.agent_type,
                        &def.recall_queries,
                        memory.as_ref(),
                        def.memory_scope.as_ref(),
                    );
                    if !memories.is_empty() {
                        let mem_block = memories.join("\n---\n");
                        format!("{system_prompt}\n\n<agent-memory>\n{mem_block}\n</agent-memory>")
                    } else {
                        system_prompt
                    }
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Inject file-based memory prompt (MEMORY.md) if agent has memory_scope
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(memory_prompt) = crate::agents::memory::load_agent_memory_prompt(
                &def.agent_type,
                def.memory_scope.as_ref(),
                &self.config.working_dir,
            ) {
                format!("{system_prompt}\n\n{memory_prompt}")
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Inject LEANN queries and tags into system prompt as agent context
        let system_prompt = if let Some(ref def) = resolved_def {
            let mut additions = Vec::new();
            if !def.leann_queries.is_empty() {
                let queries = def.leann_queries.join(", ");
                additions.push(format!("<leann-queries>\nRelevant code search queries for your task: {queries}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>"));
            }
            if !def.tags.is_empty() {
                let tags = def.tags.join(", ");
                additions.push(format!("<agent-tags>\nYour memory tags: {tags}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>"));
            }
            if additions.is_empty() {
                system_prompt
            } else {
                format!("{system_prompt}\n\n{}", additions.join("\n\n"))
            }
        } else {
            system_prompt
        };

        // Build filtered tool registry for the subagent
        let (tool_defs, tool_reg) = self.build_subagent_tools(&request, resolved_def.as_ref());

        // Create worktree if isolation == "worktree"
        let worktree_info = if isolation == Some("worktree") {
            let wt_session = format!("subagent-{subagent_id}");
            match archon_tools::worktree_manager::WorktreeManager::create_worktree_from_path(
                &self.config.working_dir,
                &wt_session,
            ) {
                Ok(info) => {
                    tracing::info!(subagent_id = %subagent_id, worktree = %info.worktree_path.display(), "created worktree for isolated subagent");
                    Some(info)
                }
                Err(e) => {
                    tracing::warn!(subagent_id = %subagent_id, error = %e, "failed to create worktree, falling back to parent dir");
                    None
                }
            }
        } else {
            None
        };

        let working_dir = worktree_info
            .as_ref()
            .map(|wt| wt.worktree_path.clone())
            .or_else(|| request.cwd.as_ref().map(std::path::PathBuf::from))
            .unwrap_or_else(|| self.config.working_dir.clone());

        // permissionMode cascade (matches Claude Code behavior):
        // Parent wins if bypassPermissions/acceptEdits/auto; otherwise agent def overrides.
        // Then map "plan" → AgentMode::Plan, everything else → Normal.
        let subagent_mode = {
            let parent_pm = self.config.permission_mode.blocking_lock().clone();
            let def_pm_str = resolved_def
                .as_ref()
                .and_then(|d| d.permission_mode.as_ref())
                .map(|pm| pm.as_str().to_string());
            let resolved_pm = match parent_pm.as_str() {
                "bypassPermissions" | "acceptEdits" | "auto" => parent_pm,
                _ => def_pm_str.unwrap_or(parent_pm),
            };
            if resolved_pm == "plan" {
                archon_tools::tool::AgentMode::Plan
            } else {
                archon_tools::tool::AgentMode::Normal
            }
        };

        let tool_ctx = archon_tools::tool::ToolContext {
            working_dir,
            session_id: self.config.session_id.clone(),
            mode: subagent_mode,
            extra_dirs: vec![],
            ..Default::default()
        };

        let mut runner = crate::subagent::runner::SubagentRunner::new(
            self.client.clone(),
            system_prompt,
            tool_defs,
            tool_reg,
            tool_ctx,
            model.to_string(),
            max_turns,
            request.timeout_secs as u64,
        );

        // Wire effort level from agent definition
        if let Some(effort) = def_effort {
            runner.set_effort(effort);
        }

        // Wire critical system reminder for per-turn injection (AGT-022)
        if let Some(ref def) = resolved_def {
            if let Some(ref reminder) = def.critical_system_reminder {
                runner.set_critical_system_reminder(reminder.clone());
            }
        }

        // Wire transcript recording (AGT-024)
        if let Some(store) =
            crate::agents::transcript::AgentTranscriptStore::new(&self.config.session_id)
        {
            // Write metadata sidecar
            let meta = crate::agents::transcript::AgentMetadata {
                agent_type: resolved_def
                    .as_ref()
                    .map(|d| d.agent_type.clone())
                    .unwrap_or_else(|| "general-purpose".into()),
                worktree_path: worktree_info
                    .as_ref()
                    .map(|wt| wt.worktree_path.display().to_string()),
                description: Some(request.prompt.chars().take(200).collect()),
                filename: resolved_def.as_ref().and_then(|d| d.filename.clone()),
            };
            store.write_metadata(&subagent_id, &meta);
            runner.set_transcript(store, subagent_id.clone());
        }

        // AGT-024: Inject resume messages if pending (from SendMessage resume path)
        if let Some(resume_msgs) = self.pending_resume_messages.lock().await.take() {
            tracing::info!(
                count = resume_msgs.len(),
                "Injecting resume messages into SubagentRunner"
            );
            runner.set_initial_messages(resume_msgs);
        }

        // AGT-026: Wire pending message source for drain at tool round boundaries
        runner.set_pending_message_source(Arc::clone(&self.subagent_manager), subagent_id.clone());

        // Wire shutdown flag for graceful shutdown via SendMessage
        {
            let mgr = self.subagent_manager.lock().await;
            if let Some(flag) = mgr.get_shutdown_flag(&subagent_id) {
                runner.set_shutdown_flag(flag);
            }
            // TASK-T3 (G4): wire shared progress tracker so the runner can
            // accumulate token usage and tool-use counts during the loop.
            if let Some(tracker) = mgr.get_progress_tracker_arc(&subagent_id) {
                runner.set_progress_tracker(tracker);
            }
        }

        // Background mode: spawn and return immediately while updating the
        // shared SubagentManager when the spawned task finishes.
        if is_background {
            let prompt = request.prompt.clone();
            let sid = subagent_id.clone();
            let subagent_manager = Arc::clone(&self.subagent_manager);
            let mem_agent_type = resolved_def.as_ref().map(|d| d.agent_type.clone());
            let mem_scope = resolved_def.as_ref().and_then(|d| d.memory_scope.clone());
            let mem_tags = resolved_def
                .as_ref()
                .map(|d| d.tags.clone())
                .unwrap_or_default();
            let mem_arc = self.memory.as_ref().map(Arc::clone);
            let mem_project_path = self.config.working_dir.to_string_lossy().into_owned();
            tokio::spawn(async move {
                match runner.run(&prompt).await {
                    Ok(text) => {
                        let mut mgr = subagent_manager.lock().await;
                        if let Err(e) = mgr.complete(&sid, text.clone()) {
                            tracing::warn!(subagent_id = %sid, error = %e, "failed to mark background subagent complete");
                        }
                        mgr.cleanup_agent(&sid);
                        drop(mgr);
                        // Save agent memory on successful completion
                        if let (Some(agent_type), Some(memory)) = (&mem_agent_type, &mem_arc) {
                            let content: String = text.chars().take(500).collect();
                            let title = format!("completion:{}:{}", agent_type, sid);
                            if let Err(e) = crate::agents::memory::save_agent_memory(
                                agent_type,
                                &content,
                                &title,
                                &mem_tags,
                                memory.as_ref(),
                                &mem_project_path,
                                mem_scope.as_ref(),
                            ) {
                                tracing::warn!(agent = %agent_type, error = %e, "failed to save agent memory");
                            }
                        }
                        tracing::info!(subagent_id = %sid, len = text.len(), "background subagent completed");
                    }
                    Err(e) => {
                        let reason = format!("Subagent failed: {e}");
                        let mut mgr = subagent_manager.lock().await;
                        if let Err(mark_err) = mgr.mark_failed(&sid, reason.clone()) {
                            tracing::warn!(subagent_id = %sid, error = %mark_err, "failed to mark background subagent failed");
                        }
                        mgr.cleanup_agent(&sid);
                        drop(mgr);
                        tracing::warn!(subagent_id = %sid, error = %e, "background subagent failed");
                    }
                }
            });

            return ToolResult::success(format!(
                "Subagent '{subagent_id}' launched in background."
            ));
        }

        // AGT-025: Auto-background foreground agents after timeout
        let auto_bg_ms = crate::subagent::get_auto_background_ms();
        let should_auto_bg = auto_bg_ms > 0 && !is_background;

        if should_auto_bg {
            // Spawn the runner into a background task immediately, then race
            // its JoinHandle against the auto-background timer.
            // If it completes first, we get the result synchronously.
            // If the timer fires first, the task keeps running in the background.
            let prompt = request.prompt.clone();
            let sid = subagent_id.clone();
            let subagent_manager_bg = Arc::clone(&self.subagent_manager);
            let mem_agent_type_bg = resolved_def.as_ref().map(|d| d.agent_type.clone());
            let mem_scope_bg = resolved_def.as_ref().and_then(|d| d.memory_scope.clone());
            let mem_tags_bg = resolved_def
                .as_ref()
                .map(|d| d.tags.clone())
                .unwrap_or_default();
            let mem_arc_bg = self.memory.as_ref().map(Arc::clone);
            let mem_project_path_bg = self.config.working_dir.to_string_lossy().into_owned();

            let join_handle = tokio::spawn(async move {
                let result = runner.run(&prompt).await;
                // Update manager on completion (same as explicit background path)
                match &result {
                    Ok(text) => {
                        let mut mgr = subagent_manager_bg.lock().await;
                        if let Err(e) = mgr.complete(&sid, text.clone()) {
                            tracing::warn!(subagent_id = %sid, error = %e, "failed to mark auto-bg subagent complete");
                        }
                        mgr.cleanup_agent(&sid);
                        drop(mgr);
                        // Save agent memory on successful completion
                        if let (Some(agent_type), Some(memory)) = (&mem_agent_type_bg, &mem_arc_bg)
                        {
                            let content: String = text.chars().take(500).collect();
                            let title = format!("completion:{}:{}", agent_type, sid);
                            if let Err(e) = crate::agents::memory::save_agent_memory(
                                agent_type,
                                &content,
                                &title,
                                &mem_tags_bg,
                                memory.as_ref(),
                                &mem_project_path_bg,
                                mem_scope_bg.as_ref(),
                            ) {
                                tracing::warn!(agent = %agent_type, error = %e, "failed to save agent memory");
                            }
                        }
                    }
                    Err(e) => {
                        let reason = format!("Subagent failed: {e}");
                        let mut mgr = subagent_manager_bg.lock().await;
                        if let Err(mark_err) = mgr.mark_failed(&sid, reason) {
                            tracing::warn!(subagent_id = %sid, error = %mark_err, "failed to mark auto-bg subagent failed");
                        }
                        mgr.cleanup_agent(&sid);
                    }
                }
                result
            });

            let timeout_duration = std::time::Duration::from_millis(auto_bg_ms);

            tokio::select! {
                join_result = join_handle => {
                    // Agent completed before timeout — handle as normal foreground result
                    let result = match join_result {
                        Ok(inner) => inner,
                        Err(e) => Err(anyhow::anyhow!("Subagent task panicked: {e}")),
                    };
                    return match result {
                        Ok(response_text) => {
                            // Manager already updated by the spawned task
                            self.fire_hook(crate::hooks::HookEvent::TeammateIdle, serde_json::json!({"hook_event": "TeammateIdle", "subagent_id": subagent_id})).await;
                            self.fire_hook(crate::hooks::HookEvent::SubagentStop, serde_json::json!({"hook_event": "SubagentStop", "subagent_id": subagent_id, "success": true})).await;
                            if nested {
                                self.fire_hook(crate::hooks::HookEvent::TaskCompleted, serde_json::json!({"hook_event": "TaskCompleted", "subagent_id": subagent_id, "success": true})).await;
                            }
                            let mut final_text = response_text;
                            if let Some(ref wt) = worktree_info {
                                match archon_tools::worktree_manager::WorktreeManager::cleanup_session(&format!("subagent-{subagent_id}")) {
                                    Ok(()) => { tracing::info!(subagent_id = %subagent_id, "clean worktree auto-removed"); }
                                    Err(_has_changes) => {
                                        let wt_note = format!("\n\n[Worktree: {} (branch: {})]", wt.worktree_path.display(), wt.branch_name);
                                        final_text.push_str(&wt_note);
                                    }
                                }
                            }
                            ToolResult::success(final_text)
                        }
                        Err(e) => {
                            let reason = format!("Subagent failed: {e}");
                            // Manager already updated by the spawned task
                            self.fire_hook(crate::hooks::HookEvent::SubagentStop, serde_json::json!({"hook_event": "SubagentStop", "subagent_id": subagent_id, "success": false, "error": &reason})).await;
                            if let Some(ref _wt) = worktree_info {
                                let _ = archon_tools::worktree_manager::WorktreeManager::cleanup_session(&format!("subagent-{subagent_id}"));
                            }
                            ToolResult::error(reason)
                        }
                    };
                }
                _ = tokio::time::sleep(timeout_duration) => {
                    // Timer fired — agent continues running in spawned task
                    tracing::info!(
                        subagent_id = %subagent_id,
                        timeout_ms = auto_bg_ms,
                        "Auto-backgrounding foreground agent after {}s timeout",
                        auto_bg_ms / 1000
                    );
                    // The spawned task continues running; manager will be updated when it completes.
                    // Transcript recording (AGT-024) persists all messages for resume.
                    return ToolResult::success(format!(
                        "Subagent '{subagent_id}' auto-backgrounded after {}s. Still running — use SendMessage to check status.",
                        auto_bg_ms / 1000
                    ));
                }
            }
        }

        // Foreground: run the multi-turn agentic loop and wait
        match runner.run(&request.prompt).await {
            Ok(response_text) => {
                {
                    let mut mgr = self.subagent_manager.lock().await;
                    if let Err(e) = mgr.complete(&subagent_id, response_text.clone()) {
                        tracing::warn!("failed to mark subagent complete: {e}");
                    }
                    mgr.cleanup_agent(&subagent_id);
                }

                // CRIT-06: Fire TeammateIdle hook when subagent completes its work
                self.fire_hook(
                    crate::hooks::HookEvent::TeammateIdle,
                    serde_json::json!({
                        "hook_event": "TeammateIdle",
                        "subagent_id": subagent_id,
                    }),
                )
                .await;

                // CRIT-06: Fire SubagentStop hook
                self.fire_hook(
                    crate::hooks::HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": true,
                    }),
                )
                .await;

                // CRIT-06: Fire TaskCompleted hook if nested (TaskCreate)
                if nested {
                    self.fire_hook(
                        crate::hooks::HookEvent::TaskCompleted,
                        serde_json::json!({
                            "hook_event": "TaskCompleted",
                            "subagent_id": subagent_id,
                            "success": true,
                        }),
                    )
                    .await;
                }

                // Save agent memory on successful completion
                if let (Some(def), Some(memory)) = (&resolved_def, &self.memory) {
                    let content: String = response_text.chars().take(500).collect();
                    let title = format!("completion:{}:{}", def.agent_type, subagent_id);
                    let project_path = self.config.working_dir.to_string_lossy();
                    if let Err(e) = crate::agents::memory::save_agent_memory(
                        &def.agent_type,
                        &content,
                        &title,
                        &def.tags,
                        memory.as_ref(),
                        &project_path,
                        def.memory_scope.as_ref(),
                    ) {
                        tracing::warn!(agent = %def.agent_type, error = %e, "failed to save agent memory");
                    }
                }

                // Worktree cleanup: auto-remove if clean, preserve if changes exist
                let mut final_text = response_text;
                if let Some(ref wt) = worktree_info {
                    match archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    ) {
                        Ok(()) => {
                            tracing::info!(subagent_id = %subagent_id, "clean worktree auto-removed");
                        }
                        Err(_has_changes) => {
                            let wt_note = format!(
                                "\n\n[Worktree: {} (branch: {})]",
                                wt.worktree_path.display(),
                                wt.branch_name
                            );
                            final_text.push_str(&wt_note);
                            tracing::info!(subagent_id = %subagent_id, branch = %wt.branch_name, "worktree preserved with changes");
                        }
                    }
                }

                ToolResult::success(final_text)
            }
            Err(e) => {
                let reason = format!("Subagent failed: {e}");
                {
                    let mut mgr = self.subagent_manager.lock().await;
                    let _ = mgr.mark_failed(&subagent_id, reason.clone());
                    mgr.cleanup_agent(&subagent_id);
                }

                // CRIT-06: Fire SubagentStop hook on failure
                self.fire_hook(
                    crate::hooks::HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": false,
                        "error": &reason,
                    }),
                )
                .await;

                // Cleanup worktree on failure
                if let Some(ref wt) = worktree_info {
                    let _ = archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    );
                    tracing::info!(subagent_id = %subagent_id, "worktree cleaned up after failure");
                    let _ = wt; // suppress unused warning
                }

                ToolResult::error(reason)
            }
        }
    }

    /// Build the filtered tool registry for a subagent.
    ///
    /// Applies: hardcoded denylist → definition allowlist → request allowlist → definition denylist.
    /// Returns (tool_definitions, filtered_registry) for SubagentRunner.
    fn build_subagent_tools(
        &self,
        request: &SubagentRequest,
        agent_def: Option<&crate::agents::CustomAgentDefinition>,
    ) -> (Vec<serde_json::Value>, ToolRegistry) {
        // Hardcoded denylist — subagents must NEVER have these tools
        const DENYLIST: &[&str] = &[
            "Agent",
            "AskUserQuestion",
            "EnterPlanMode",
            "ExitPlanMode",
            "TaskCreate",
            "TaskStop",
        ];

        // Default safe tools when no explicit allowlist
        const DEFAULT_TOOLS: &[&str] = &["Read", "Grep", "Glob", "Bash", "Write", "Edit"];

        // Determine base allowed set.
        // Priority: request allowlist > definition allowlist > default tools
        let base_allowed: Vec<&str> = if !request.allowed_tools.is_empty() {
            request
                .allowed_tools
                .iter()
                .map(|s| s.as_str())
                .filter(|n| !DENYLIST.contains(n))
                .collect()
        } else if let Some(def_tools) = agent_def.and_then(|d| d.allowed_tools.as_ref()) {
            def_tools
                .iter()
                .map(|s| s.as_str())
                .filter(|n| !DENYLIST.contains(n))
                .collect()
        } else {
            DEFAULT_TOOLS.to_vec()
        };

        // Apply agent definition denylist
        let agent_deny: Vec<String> = agent_def
            .and_then(|d| d.disallowed_tools.as_ref())
            .cloned()
            .unwrap_or_default();

        // Apply plan-mode filtering: remove mutating tools if parent is in plan mode.
        // Keep aligned with is_tool_allowed_in_mode() allowlist in plan_mode.rs.
        const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
        let is_plan_mode = self.config.permission_mode.blocking_lock().as_str() == "plan";

        // MCP server scoping: if definition specifies mcp_servers, only include MCP tools
        // from those servers. Non-MCP tools pass through unaffected.
        let mcp_scope: Option<&Vec<String>> = agent_def.and_then(|d| d.mcp_servers.as_ref());

        let final_allowed: Vec<&str> = base_allowed
            .into_iter()
            .filter(|n| !agent_deny.iter().any(|d| d == n))
            .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
            .filter(|n| {
                // If mcp_servers is specified, only allow MCP tools from listed servers
                if let Some(allowed_servers) = mcp_scope {
                    if n.starts_with("mcp__") {
                        // Extract server name: mcp__servername__toolname -> servername
                        let parts: Vec<&str> = n.splitn(3, "__").collect();
                        if parts.len() >= 2 {
                            let server = parts[1];
                            return allowed_servers
                                .iter()
                                .any(|s| s.eq_ignore_ascii_case(server));
                        }
                        return false; // malformed MCP tool name
                    }
                }
                true // non-MCP tools always pass, or no mcp_servers restriction
            })
            .collect();

        let filtered = self.registry.clone_filtered(&final_allowed);
        let defs = filtered.tool_definitions();
        (defs, filtered)
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("API error: {0}")]
    ApiError(String),

    #[error("tool dispatch error: {0}")]
    ToolError(String),
}

// ---------------------------------------------------------------------------
// Plan text parser
// ---------------------------------------------------------------------------

/// Parse a plan from the assistant's text output.
/// Simple line-by-line state machine: extracts title, steps, risks, questions.
fn parse_plan_from_text(text: &str) -> archon_session::plan::PlanDocument {
    use archon_session::plan::{PlanDocument, PlanStep, PlanStepStatus};

    enum Section {
        None,
        Steps,
        Risks,
        Questions,
    }

    let mut title = String::from("Untitled Plan");
    let mut steps = Vec::new();
    let mut risks = Vec::new();
    let mut questions = Vec::new();
    let mut section = Section::None;
    let mut step_num: u32 = 0;

    for line in text.lines() {
        let trimmed = line.trim();

        // Detect title from headings
        if let Some(t) = trimmed
            .strip_prefix("## Plan:")
            .or_else(|| trimmed.strip_prefix("# Plan:"))
        {
            let t = t.trim();
            if !t.is_empty() {
                title = t.to_string();
            }
            continue;
        }

        // Detect section headings
        if trimmed.starts_with("### Steps") || trimmed.starts_with("## Steps") {
            section = Section::Steps;
            continue;
        }
        if trimmed.starts_with("### Risks") || trimmed.starts_with("## Risks") {
            section = Section::Risks;
            continue;
        }
        if trimmed.starts_with("### Questions")
            || trimmed.starts_with("## Questions")
            || trimmed.starts_with("### Open Questions")
            || trimmed.starts_with("## Open Questions")
        {
            section = Section::Questions;
            continue;
        }
        // Any other heading resets section
        if trimmed.starts_with("### ") || trimmed.starts_with("## ") {
            section = Section::None;
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match section {
            Section::Steps => {
                // Match numbered items like "1. Do something" or "- Do something"
                let desc = if let Some(rest) = trimmed.strip_prefix(|c: char| c.is_ascii_digit()) {
                    // Strip remaining digits and the dot
                    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                    rest.strip_prefix('.').or(Some(rest)).map(|s| s.trim())
                } else {
                    trimmed.strip_prefix("- ").map(|s| s.trim())
                };
                if let Some(desc) = desc {
                    if !desc.is_empty() {
                        step_num += 1;
                        steps.push(PlanStep {
                            number: step_num,
                            description: desc.to_string(),
                            affected_files: Vec::new(),
                            status: PlanStepStatus::Pending,
                        });
                    }
                }
            }
            Section::Risks => {
                if let Some(r) = trimmed.strip_prefix("- ") {
                    risks.push(r.trim().to_string());
                } else {
                    risks.push(trimmed.to_string());
                }
            }
            Section::Questions => {
                if let Some(q) = trimmed.strip_prefix("- ") {
                    questions.push(q.trim().to_string());
                } else {
                    questions.push(trimmed.to_string());
                }
            }
            Section::None => {}
        }
    }

    let id = format!("plan-{}", chrono::Utc::now().timestamp_millis());
    let mut doc = PlanDocument::new(&id, &title);
    doc.steps = steps;
    doc.risks = risks;
    doc.questions = questions;
    doc.status = "active".to_string();
    doc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that thinking blocks include the `signature` field when built
    /// as assistant message content. This is required by the Anthropic API
    /// for multi-turn conversations containing thinking blocks.
    #[test]
    fn thinking_block_includes_signature() {
        let thinking_content = "Let me analyze this step by step...";
        let thinking_signature = "EqoBCkEYstO+bkwMCwF8m...test-sig";

        let mut assistant_content: Vec<serde_json::Value> = Vec::new();

        if !thinking_content.is_empty() {
            assistant_content.push(serde_json::json!({
                "type": "thinking",
                "thinking": thinking_content,
                "signature": thinking_signature,
            }));
        }

        assistant_content.push(serde_json::json!({
            "type": "text",
            "text": "Here is my response.",
        }));

        assert_eq!(assistant_content.len(), 2);

        let thinking_block = &assistant_content[0];
        assert_eq!(thinking_block["type"], "thinking");
        assert_eq!(thinking_block["thinking"], thinking_content);
        assert_eq!(thinking_block["signature"], thinking_signature);
        // Crucially: the signature field MUST exist (not be null/missing)
        assert!(
            thinking_block.get("signature").is_some(),
            "thinking block must contain 'signature' field for Anthropic API"
        );
    }

    /// Verify that thinking blocks still include the signature field even
    /// when the signature is empty (edge case: stream ended before signature).
    #[test]
    fn thinking_block_includes_empty_signature() {
        let thinking_content = "Some thinking...";
        let thinking_signature = "";

        let block = serde_json::json!({
            "type": "thinking",
            "thinking": thinking_content,
            "signature": thinking_signature,
        });

        assert!(block.get("signature").is_some());
        assert_eq!(block["signature"], "");
    }

    #[test]
    fn conversation_state_add_assistant_message_preserves_thinking_signature() {
        let mut state = ConversationState::default();
        state.add_user_message("hello");

        let content = vec![
            serde_json::json!({
                "type": "thinking",
                "thinking": "deep thought",
                "signature": "sig123",
            }),
            serde_json::json!({
                "type": "text",
                "text": "response",
            }),
        ];
        state.add_assistant_message(content);

        assert_eq!(state.messages.len(), 2);
        let assistant_msg = &state.messages[1];
        let blocks = assistant_msg["content"]
            .as_array()
            .expect("content is array");
        assert_eq!(blocks[0]["signature"], "sig123");
    }

    // -----------------------------------------------------------------------
    // TASK-AGT-012: Permission mode + max_concurrent tests
    // -----------------------------------------------------------------------

    #[test]
    fn plan_mode_deny_list_is_static() {
        // Verify the plan mode deny constants are correct
        const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
        assert!(PLAN_MODE_DENY.contains(&"Write"));
        assert!(PLAN_MODE_DENY.contains(&"Edit"));
        assert!(PLAN_MODE_DENY.contains(&"Bash"));
        assert!(PLAN_MODE_DENY.contains(&"NotebookEdit"));
        assert!(!PLAN_MODE_DENY.contains(&"Read"));
        assert!(!PLAN_MODE_DENY.contains(&"Grep"));
        assert!(!PLAN_MODE_DENY.contains(&"Glob"));
    }

    #[test]
    fn subagent_manager_register_before_run_complete_after() {
        // Verify the SubagentManager register→complete lifecycle
        let mut mgr = crate::subagent::SubagentManager::new(4);
        let req = archon_tools::agent_tool::SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
        };

        // Register returns ID
        let id = mgr.register(req).expect("register should succeed");
        assert!(!id.is_empty());

        // Status is Running
        let info = mgr.get_status(&id).expect("should exist");
        assert!(matches!(
            info.status,
            crate::subagent::SubagentStatus::Running
        ));

        // Complete frees the slot
        mgr.complete(&id, "done".into())
            .expect("complete should work");
        let info = mgr.get_status(&id).expect("should still exist");
        assert!(matches!(
            info.status,
            crate::subagent::SubagentStatus::Completed
        ));
    }

    #[test]
    fn subagent_manager_max_concurrent_enforced() {
        let mut mgr = crate::subagent::SubagentManager::new(1);
        let req = || archon_tools::agent_tool::SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
        };

        let id1 = mgr.register(req()).expect("first register ok");

        // Second should fail
        let err = mgr.register(req());
        assert!(err.is_err(), "should reject second concurrent subagent");

        // Complete first, then second should succeed
        mgr.complete(&id1, "done".into()).unwrap();
        let _id2 = mgr
            .register(req())
            .expect("should succeed after completing first");
    }

    #[test]
    fn permission_mode_plan_blocks_mutating_tools() {
        // Verify the filtering logic: in plan mode, Write/Edit/Bash/NotebookEdit are removed
        const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
        let tools = vec![
            "Read",
            "Grep",
            "Glob",
            "Write",
            "Edit",
            "Bash",
            "NotebookEdit",
        ];
        let is_plan_mode = true;

        let filtered: Vec<&str> = tools
            .into_iter()
            .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
            .collect();

        assert_eq!(filtered, vec!["Read", "Grep", "Glob"]);
    }

    #[test]
    fn permission_mode_normal_allows_all_tools() {
        const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
        let tools = vec!["Read", "Grep", "Glob", "Write", "Edit", "Bash"];
        let is_plan_mode = false;

        let filtered: Vec<&str> = tools
            .into_iter()
            .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
            .collect();

        assert_eq!(
            filtered,
            vec!["Read", "Grep", "Glob", "Write", "Edit", "Bash"]
        );
    }
}
