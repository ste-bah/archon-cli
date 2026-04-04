use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::effort::EffortLevel;
use archon_llm::streaming::StreamEvent;
use archon_memory::extraction::{
    should_extract, store_extracted, build_extraction_prompt, parse_extraction_response,
    ExtractionConfig, ExtractionState,
};
use archon_memory::injection::MemoryInjector;
use archon_memory::MemoryGraph;
use archon_permissions::auto::{AutoDecision, AutoModeEvaluator};
use archon_session::checkpoint::CheckpointStore;
use archon_tools::agent_tool::SubagentRequest;
use archon_tools::tool::{AgentMode, ToolContext, ToolResult};
use tokio::sync::Mutex;

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
    ApiCallStarted { model: String },
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStarted { name: String, id: String },
    ToolCallComplete { name: String, id: String, result: ToolResult },
    PermissionRequired { tool: String, description: String },
    PermissionGranted { tool: String },
    PermissionDenied { tool: String },
    TurnComplete { input_tokens: u64, output_tokens: u64 },
    Error(String),
    CompactionTriggered,
    SessionComplete,
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
            if msg["role"].as_str() == Some("user") {
                if let Some(content) = msg["content"].as_str() {
                    return content;
                }
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
    client: AnthropicClient,
    registry: ToolRegistry,
    config: AgentConfig,
    state: ConversationState,
    event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    checkpoint_store: Option<Arc<Mutex<CheckpointStore>>>,
    turn_number: u64,
    // GAP 5/7: Memory graph + injector for per-turn injection and auto-extraction
    memory_graph: Option<Arc<MemoryGraph>>,
    memory_injector: MemoryInjector,
    extraction_config: ExtractionConfig,
    extraction_state: ExtractionState,
    // GAP 6: Auto-mode permission evaluator
    auto_evaluator: Option<AutoModeEvaluator>,
    // GAP 8: Subagent manager
    subagent_manager: SubagentManager,
    /// Shared flag: whether /thinking display is on (used to potentially skip thinking in future)
    pub show_thinking: Arc<AtomicBool>,
    /// Shared session statistics for /status and /cost slash commands.
    pub session_stats: Arc<Mutex<SessionStats>>,
    /// Hook system dispatcher for pre/post tool execution hooks.
    hook_dispatcher: Option<crate::hooks::HookDispatcher>,
    /// Channel for permission prompt responses from the TUI.
    /// Agent sends PermissionRequired event, then waits on this for y/n.
    pub permission_response_rx: Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<bool>>>>,
}

impl Agent {
    pub fn new(
        client: AnthropicClient,
        registry: ToolRegistry,
        config: AgentConfig,
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Self {
        Self {
            client,
            registry,
            config,
            state: ConversationState::default(),
            event_tx,
            checkpoint_store: None,
            turn_number: 0,
            memory_graph: None,
            memory_injector: MemoryInjector::new(),
            extraction_config: ExtractionConfig::default(),
            extraction_state: ExtractionState::default(),
            auto_evaluator: None,
            subagent_manager: SubagentManager::default(),
            show_thinking: Arc::new(AtomicBool::new(true)),
            session_stats: Arc::new(Mutex::new(SessionStats::default())),
            hook_dispatcher: None,
            permission_response_rx: None,
        }
    }

    /// Close the event channel so receivers know the agent is done.
    /// Used by print mode to unblock the event consumer task.
    pub fn close_event_channel(&mut self) {
        // Replace the sender with a closed one by dropping it
        let (tx, _) = tokio::sync::mpsc::channel(1);
        self.event_tx = tx;
        // The old sender is dropped, closing the channel
    }

    /// Set the hook dispatcher for pre/post tool execution hooks.
    pub fn set_hook_dispatcher(&mut self, dispatcher: crate::hooks::HookDispatcher) {
        self.hook_dispatcher = Some(dispatcher);
    }

    /// Fire a hook by type with a JSON payload. No-op if no dispatcher is set.
    pub async fn fire_hook(&self, hook_type: crate::hooks::HookType, payload: serde_json::Value) {
        if let Some(ref dispatcher) = self.hook_dispatcher {
            dispatcher.fire(hook_type, payload).await;
        }
    }

    /// Set the checkpoint store for file snapshots before Write/Edit operations.
    pub fn set_checkpoint_store(&mut self, store: CheckpointStore) {
        self.checkpoint_store = Some(Arc::new(Mutex::new(store)));
    }

    /// Set the memory graph for per-turn injection (GAP 7) and extraction (GAP 5).
    pub fn set_memory_graph(&mut self, graph: Arc<MemoryGraph>) {
        self.memory_graph = Some(graph);
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

        loop {
            // GAP 7: Inject recalled memories into system prompt
            let system_with_memories = self.inject_memories();

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
            let request = MessageRequest {
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
            };

            self.send_event(AgentEvent::ApiCallStarted {
                model: active_model.clone(),
            })
            .await;

            // Send request and get streaming events
            let mut rx = self
                .client
                .stream_message(request)
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

                    StreamEvent::InputJsonDelta {
                        partial_json,
                        ..
                    } => {
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
                        self.send_event(AgentEvent::Error(format!(
                            "{error_type}: {message}"
                        )))
                        .await;
                        return Err(AgentLoopError::ApiError(format!(
                            "{error_type}: {message}"
                        )));
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
                let ctx = ToolContext {
                    working_dir: self.config.working_dir.clone(),
                    session_id: self.config.session_id.clone(),
                    mode: effective_mode,
                };

                for tool in &pending_tools {
                    let input: serde_json::Value =
                        serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));

                    // GAP 6: Permission classification before tool execution
                    // Read the shared permission mode to determine behavior
                    let perm_mode = {
                        let mode = self.config.permission_mode.lock().await;
                        mode.clone()
                    };
                    let tool_allowed = match perm_mode.as_str() {
                        "bypassPermissions" | "yolo" | "dontAsk" => {
                            tracing::debug!(tool = %tool.name, "bypass-mode: allowed");
                            true
                        }
                        "acceptEdits" => {
                            // Allow read-only + Write/Edit, but prompt for Bash/PowerShell
                            match tool.name.as_str() {
                                "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                                | "TodoWrite" | "Sleep" | "Write" | "Edit" | "Config"
                                | "EnterPlanMode" | "ExitPlanMode" | "NotebookEdit" => true,
                                _ => {
                                    self.send_event(AgentEvent::PermissionRequired {
                                        tool: tool.name.clone(),
                                        description: format!("Permission required for {}", tool.name),
                                    }).await;
                                    // In acceptEdits, deny non-edit tools (Bash, etc.)
                                    self.send_event(AgentEvent::PermissionDenied {
                                        tool: tool.name.clone(),
                                    }).await;
                                    false
                                }
                            }
                        }
                        "default" | "ask" => {
                            match tool.name.as_str() {
                                "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                                | "TodoWrite" | "Sleep" | "EnterPlanMode" | "ExitPlanMode" => {
                                    tracing::debug!(tool = %tool.name, "default-mode: safe, allowed");
                                    true
                                }
                                _ => {
                                    // Send permission request to TUI and wait for response
                                    self.send_event(AgentEvent::PermissionRequired {
                                        tool: tool.name.clone(),
                                        description: format!("{} wants to use {}", tool.name, tool.name),
                                    }).await;

                                    // Wait for user response via permission channel
                                    if let Some(ref rx) = self.permission_response_rx {
                                        let mut rx = rx.lock().await;
                                        match tokio::time::timeout(
                                            std::time::Duration::from_secs(120),
                                            rx.recv(),
                                        ).await {
                                            Ok(Some(true)) => {
                                                self.send_event(AgentEvent::PermissionGranted {
                                                    tool: tool.name.clone(),
                                                }).await;
                                                tracing::info!(tool = %tool.name, "default-mode: user approved");
                                                true
                                            }
                                            _ => {
                                                self.send_event(AgentEvent::PermissionDenied {
                                                    tool: tool.name.clone(),
                                                }).await;
                                                tracing::info!(tool = %tool.name, "default-mode: user denied or timeout");
                                                false
                                            }
                                        }
                                    } else {
                                        // No permission channel — auto-approve
                                        tracing::info!(tool = %tool.name, "default-mode: no permission channel, auto-approved");
                                        true
                                    }
                                }
                            }
                        }
                        _ => {
                            // "auto" mode -- use AutoModeEvaluator
                            if let Some(ref evaluator) = self.auto_evaluator {
                                let decision = match tool.name.as_str() {
                                    "Bash" | "PowerShell" => {
                                        let cmd = input.get("command")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        evaluator.evaluate_command(cmd)
                                    }
                                    "Write" | "Edit" => {
                                        let path = input.get("file_path")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        evaluator.evaluate_file_write(Path::new(path))
                                    }
                                    "Read" | "Glob" | "Grep" | "ToolSearch" | "AskUserQuestion"
                                    | "TodoWrite" | "Sleep" => {
                                        AutoDecision::Allow
                                    }
                                    "Config" => {
                                        let action = input.get("action")
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
                                        self.send_event(AgentEvent::PermissionDenied {
                                            tool: tool.name.clone(),
                                        }).await;
                                        false
                                    }
                                    AutoDecision::PromptWithWarning(msg) => {
                                        tracing::warn!(tool = %tool.name, warning = %msg, "auto-mode: dangerous, denied");
                                        self.send_event(AgentEvent::PermissionDenied {
                                            tool: tool.name.clone(),
                                        }).await;
                                        false
                                    }
                                }
                            } else {
                                true // no evaluator = allow
                            }
                        }
                    };

                    if !tool_allowed {
                        let denied_result = ToolResult::error(format!(
                            "Permission denied for tool '{}'. Current mode: {}. Use /permissions yolo to allow all operations.",
                            tool.name, perm_mode
                        ));
                        self.send_event(AgentEvent::ToolCallComplete {
                            name: tool.name.clone(),
                            id: tool.id.clone(),
                            result: denied_result.clone(),
                        }).await;
                        self.state.add_tool_result(&tool.id, &denied_result.content, true);
                        continue;
                    }

                    // Phase 2 (CLI-116): Checkpoint file before Write/Edit
                    if matches!(tool.name.as_str(), "Write" | "Edit") {
                        if let Some(ref store) = self.checkpoint_store {
                            if let Some(file_path) = input.get("file_path").and_then(|v| v.as_str()) {
                                let store = store.lock().await;
                                if let Err(e) = store.snapshot(
                                    &self.config.session_id,
                                    file_path,
                                    self.turn_number as i64,
                                    &tool.name,
                                ) {
                                    tracing::warn!(
                                        "checkpoint snapshot failed for {file_path}: {e}"
                                    );
                                }
                            }
                        }
                    }

                    // Pre-tool-use hook: check if any hook blocks this tool
                    if let Some(ref dispatcher) = self.hook_dispatcher {
                        if let Some(hook_result) = dispatcher.fire_pre_tool_use(&tool.name, &input).await {
                            if hook_result.allow == Some(false) {
                                let reason = hook_result.reason.unwrap_or_else(|| "blocked by pre_tool_use hook".into());
                                let result = ToolResult::error(format!("Hook blocked: {reason}"));
                                self.send_event(AgentEvent::ToolCallComplete {
                                    name: tool.name.clone(),
                                    id: tool.id.clone(),
                                    result: result.clone(),
                                }).await;
                                self.state.add_tool_result(&tool.id, &result.content, result.is_error);
                                continue;
                            }
                        }
                    }

                    let result = self.registry.dispatch(&tool.name, input, &ctx).await;

                    // GAP 8: Detect SubagentRequest and execute one-shot.
                    // AgentTool returns a bare SubagentRequest as the full content.
                    // TaskCreate returns {"task_id":"...","subagent_request":{...}}.
                    let result = if !result.is_error && (tool.name == "Agent" || tool.name == "TaskCreate") {
                        self.handle_subagent_result(&result, tool.name == "TaskCreate").await
                    } else {
                        result
                    };

                    self.send_event(AgentEvent::ToolCallComplete {
                        name: tool.name.clone(),
                        id: tool.id.clone(),
                        result: result.clone(),
                    })
                    .await;

                    self.state
                        .add_tool_result(&tool.id, &result.content, result.is_error);
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
                stats.cache_stats.update(
                    turn_cache_creation,
                    turn_cache_read,
                    turn_input_tokens,
                );
            }

            self.send_event(AgentEvent::TurnComplete {
                input_tokens: turn_input_tokens,
                output_tokens: turn_output_tokens,
            })
            .await;

            // GAP 5: Auto-memory extraction check
            self.extraction_state.record_turn();
            if should_extract(&self.extraction_config, &self.extraction_state, self.turn_number as usize) {
                self.trigger_memory_extraction();
            }

            break;
        }

        Ok(())
    }

    async fn send_event(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event).await;
    }

    /// Get the auth provider for spawning parallel API calls (e.g. /btw).
    pub fn auth_provider(&self) -> &archon_llm::auth::AuthProvider {
        self.client.auth()
    }

    /// Get the identity provider for spawning parallel API calls.
    pub fn identity_provider(&self) -> &archon_llm::identity::IdentityProvider {
        self.client.identity()
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
    pub async fn compact(&mut self) -> String {
        use archon_context::messages::ContextMessage;
        use crate::commands::handle_compact;
        use crate::hooks::HookType;

        // Convert JSON messages to ContextMessages
        let context_msgs: Vec<ContextMessage> = self.state.messages.iter().map(|m| {
            let role = m["role"].as_str().unwrap_or("user").to_string();
            let content = m["content"].clone();
            let text_len = match &content {
                serde_json::Value::String(s) => s.len(),
                serde_json::Value::Array(arr) => arr.iter().map(|v| {
                    v.get("text").and_then(|t| t.as_str()).map_or(0, |s| s.len())
                }).sum(),
                _ => 0,
            };
            ContextMessage {
                role,
                content,
                estimated_tokens: (text_len as f64 / 4.0).ceil() as u64,
            }
        }).collect();

        if context_msgs.len() < 5 {
            return "Nothing to compact (fewer than 5 messages).".into();
        }

        let message_count = context_msgs.len();
        let before_tokens: u64 = context_msgs.iter().map(|m| m.estimated_tokens).sum();

        // Fire PreCompact hook
        if let Some(ref dispatcher) = self.hook_dispatcher {
            let payload = serde_json::json!({
                "hook_type": "pre_compact",
                "message_count": message_count,
                "token_count": before_tokens,
            });
            dispatcher.fire(HookType::PreCompact, payload).await;
        }

        // Build a summary from the conversation for compaction
        let summary = self.state.first_user_message().to_string();
        let output = handle_compact(&context_msgs, &summary);

        if output.mutated {
            // Replace the conversation messages with the compacted version
            self.state.messages = output.messages.iter().map(|cm| {
                serde_json::json!({
                    "role": cm.role,
                    "content": cm.content,
                })
            }).collect();
            // Invalidate memory cache since context changed
            self.memory_injector.invalidate_cache();
        }

        // Compute post-compaction token count
        let after_tokens: u64 = if output.mutated {
            output.messages.iter().map(|m| m.estimated_tokens).sum()
        } else {
            before_tokens
        };
        let tokens_removed = before_tokens.saturating_sub(after_tokens);

        // Determine strategy label
        let strategy = if !output.mutated {
            "none"
        } else if message_count <= 10 {
            "micro"
        } else if before_tokens > 100_000 {
            "snip"
        } else {
            "auto"
        };

        // Fire PostCompact hook
        if let Some(ref dispatcher) = self.hook_dispatcher {
            let payload = serde_json::json!({
                "hook_type": "post_compact",
                "strategy": strategy,
                "tokens_removed": tokens_removed,
                "tokens_remaining": after_tokens,
            });
            dispatcher.fire(HookType::PostCompact, payload).await;
        }

        // Return detailed summary
        if output.mutated {
            let before_k = before_tokens as f64 / 1000.0;
            let after_k = after_tokens as f64 / 1000.0;
            let removed_k = tokens_removed as f64 / 1000.0;
            format!(
                "Compacted conversation ({strategy}): {before_k:.1}k → {after_k:.1}k tokens ({removed_k:.1}k removed, {message_count} messages)"
            )
        } else {
            output.message
        }
    }

    /// GAP 7: Inject recalled memories into the system prompt for this turn.
    fn inject_memories(&mut self) -> Vec<serde_json::Value> {
        let mut system = self.config.system_prompt.clone();

        let graph = match self.memory_graph {
            Some(ref g) => g,
            None => return system,
        };

        // Collect recent user messages as context for recall
        let context: Vec<String> = self.state.messages.iter()
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

        system
    }

    /// GAP 5: Trigger memory extraction in the background.
    fn trigger_memory_extraction(&mut self) {
        let graph = match self.memory_graph {
            Some(ref g) => Arc::clone(g),
            None => return,
        };

        // Collect last N messages for extraction
        let messages: Vec<String> = self.state.messages.iter()
            .rev()
            .take(10)
            .filter_map(|m| {
                let role = m["role"].as_str().unwrap_or("unknown");
                let content = m["content"].as_str().unwrap_or("");
                if content.is_empty() { return None; }
                Some(format!("{role}: {content}"))
            })
            .collect();

        if messages.is_empty() {
            return;
        }

        let session_id = self.config.session_id.clone();
        let turn = self.turn_number as usize;
        let client = self.client.clone();
        let model = self.config.model.clone();

        // Record extraction so we don't fire again immediately
        self.extraction_state.record_extraction(turn);

        // Run extraction in background via a real LLM call
        tokio::spawn(async move {
            let prompt = build_extraction_prompt(&messages);

            let request = MessageRequest {
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
            };

            match client.stream_message(request).await {
                Ok(mut rx) => {
                    let mut response_text = String::new();
                    while let Some(event) = rx.recv().await {
                        if let StreamEvent::TextDelta { text, .. } = event {
                            response_text.push_str(&text);
                        }
                    }

                    let extracted = parse_extraction_response(&response_text).unwrap_or_default();
                    if !extracted.is_empty() {
                        match store_extracted(&graph, &extracted, &session_id) {
                            Ok(count) => tracing::info!("auto-extracted {count} memories at turn {turn}"),
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
    async fn handle_subagent_result(&mut self, tool_result: &ToolResult, nested: bool) -> ToolResult {
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
        let subagent_id = match self.subagent_manager.register(request.clone()) {
            Ok(id) => id,
            Err(e) => return ToolResult::error(format!("Failed to register subagent: {e}")),
        };

        tracing::info!(subagent_id = %subagent_id, prompt_len = request.prompt.len(), "spawning one-shot subagent");

        // One-shot subagent: make a single API call with the subagent's prompt
        let model = request.model.as_deref().unwrap_or(&self.config.model);
        let sub_request = MessageRequest {
            model: model.to_string(),
            max_tokens: self.config.max_tokens,
            system: vec![serde_json::json!({
                "type": "text",
                "text": "You are a subagent. Complete the task described in the user message. Be thorough and precise.",
            })],
            messages: vec![serde_json::json!({
                "role": "user",
                "content": request.prompt,
            })],
            tools: Vec::new(), // subagent has no tools for one-shot
            thinking: None,
            speed: None,
            effort: None,
        };

        match self.client.stream_message(sub_request).await {
            Ok(mut rx) => {
                let mut response_text = String::new();
                while let Some(event) = rx.recv().await {
                    if let StreamEvent::TextDelta { text, .. } = event {
                        response_text.push_str(&text);
                    }
                }

                if let Err(e) = self.subagent_manager.complete(&subagent_id, response_text.clone()) {
                    tracing::warn!("failed to mark subagent complete: {e}");
                }

                ToolResult::success(response_text)
            }
            Err(e) => {
                let reason = format!("Subagent API call failed: {e}");
                let _ = self.subagent_manager.mark_failed(&subagent_id, reason.clone());
                ToolResult::error(reason)
            }
        }
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
        let blocks = assistant_msg["content"].as_array().expect("content is array");
        assert_eq!(blocks[0]["signature"], "sig123");
    }
}
