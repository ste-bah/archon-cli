use std::collections::HashMap;

use archon_tools::agent_tool::SubagentRequest;
use chrono::{DateTime, Utc};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Auto-background configuration (AGT-025)
// ---------------------------------------------------------------------------

/// Default auto-background timeout in milliseconds (120 seconds).
pub const AUTO_BACKGROUND_MS: u64 = 120_000;

/// Check if auto-backgrounding is enabled via environment variable.
///
/// When enabled, foreground sync agents that run longer than `AUTO_BACKGROUND_MS`
/// are automatically converted to background agents. The agent continues running
/// but the parent stops waiting synchronously.
pub fn is_auto_background_enabled() -> bool {
    std::env::var("ARCHON_AUTO_BACKGROUND_TASKS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Returns the auto-background timeout in milliseconds, or 0 if disabled.
pub fn get_auto_background_ms() -> u64 {
    if is_auto_background_enabled() {
        AUTO_BACKGROUND_MS
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Subagent status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Completed,
    TimedOut,
    Failed(String),
}

// ---------------------------------------------------------------------------
// Progress tracking (TASK-T3 / G4) — per-agent live progress counters
// ---------------------------------------------------------------------------

/// A single tool invocation recorded on the recent-activity ring.
#[derive(Debug, Clone)]
pub struct ToolActivity {
    pub tool_name: String,
    pub timestamp: DateTime<Utc>,
}

/// Mutable per-agent progress state shared between the runner (writer) and
/// observers (readers).  Wrapped in `std::sync::Mutex` so the runner can
/// update it from inside its synchronous stream-event match arms without
/// holding any lock across an `.await`.
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    pub tool_use_count: u32,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cache_creation_tokens: u64,
    pub cumulative_cache_read_tokens: u64,
    /// Bounded ring of the most recent tool dispatches (cap 5).
    pub recent_activities: std::collections::VecDeque<ToolActivity>,
    pub last_update: DateTime<Utc>,
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self {
            tool_use_count: 0,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cache_creation_tokens: 0,
            cumulative_cache_read_tokens: 0,
            recent_activities: std::collections::VecDeque::new(),
            last_update: Utc::now(),
        }
    }
}

/// Read-only snapshot returned by `SubagentManager::get_progress`.
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub tool_use_count: u32,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cache_creation_tokens: u64,
    pub cumulative_cache_read_tokens: u64,
    pub recent_activities: Vec<ToolActivity>,
    pub last_update: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Subagent info — tracks a single subagent's lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubagentInfo {
    pub id: String,
    pub request: SubagentRequest,
    pub status: SubagentStatus,
    pub created_at: DateTime<Utc>,
    pub result: Option<String>,
    /// Flag for graceful shutdown (set by SendMessage shutdown_request).
    pub shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// TASK-T3 (G4): live progress counters shared with the runner.
    pub progress: std::sync::Arc<std::sync::Mutex<ProgressTracker>>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("subagent not found: {0}")]
    NotFound(String),

    #[error("max concurrent subagents reached ({0})")]
    MaxConcurrent(usize),

    #[error("subagent {0} is not in Running state")]
    NotRunning(String),
}

// ---------------------------------------------------------------------------
// SubagentManager — manages subagent lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SubagentManager {
    agents: HashMap<String, SubagentInfo>,
    max_concurrent: usize,
    /// Name registry: maps agent display names to agent IDs.
    /// Populated when Agent tool spawns a named agent, cleared on completion.
    name_registry: HashMap<String, String>,
    /// Pending messages queued for delivery at next tool round boundary.
    /// Key: agent_id, Value: queued messages (FIFO order).
    pending_messages: HashMap<String, Vec<String>>,
}

impl SubagentManager {
    /// Default maximum concurrent subagents.
    pub const DEFAULT_MAX_CONCURRENT: usize = 4;

    pub fn new(max_concurrent: usize) -> Self {
        Self {
            agents: HashMap::new(),
            max_concurrent,
            name_registry: HashMap::new(),
            pending_messages: HashMap::new(),
        }
    }

    /// Register a new subagent request.  Returns the UUID assigned.
    pub fn register(&mut self, request: SubagentRequest) -> Result<String, SubagentError> {
        let active = self
            .agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .count();

        if active >= self.max_concurrent {
            return Err(SubagentError::MaxConcurrent(self.max_concurrent));
        }

        let id = Uuid::new_v4().to_string();
        let info = SubagentInfo {
            id: id.clone(),
            request,
            status: SubagentStatus::Running,
            created_at: Utc::now(),
            result: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            progress: std::sync::Arc::new(std::sync::Mutex::new(ProgressTracker::default())),
        };
        self.agents.insert(id.clone(), info);
        Ok(id)
    }

    /// Get the status of a subagent by id.
    pub fn get_status(&self, id: &str) -> Option<&SubagentInfo> {
        self.agents.get(id)
    }

    /// List all currently-running subagents.
    pub fn list_active(&self) -> Vec<&SubagentInfo> {
        self.agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .collect()
    }

    /// Mark a subagent as completed with the given result.
    pub fn complete(&mut self, id: &str, result: String) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::Completed;
        info.result = Some(result);
        Ok(())
    }

    /// Mark a subagent as timed out.
    pub fn mark_timed_out(&mut self, id: &str) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::TimedOut;
        Ok(())
    }

    /// Mark a subagent as failed with a reason.
    pub fn mark_failed(&mut self, id: &str, reason: String) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::Failed(reason);
        Ok(())
    }

    // -------------------------------------------------------------------
    // Agent name registry (AGT-026)
    // -------------------------------------------------------------------

    /// Register a human-readable name for an agent ID.
    /// Called when Agent tool spawns a named agent.
    pub fn register_name(&mut self, name: String, agent_id: String) {
        self.name_registry.insert(name, agent_id);
    }

    /// Remove a name from the registry. Called when agent completes.
    pub fn unregister_name(&mut self, name: &str) {
        self.name_registry.remove(name);
    }

    /// Resolve an agent name to its ID, if registered.
    pub fn resolve_name(&self, name: &str) -> Option<&str> {
        self.name_registry.get(name).map(|s| s.as_str())
    }

    /// Check if an agent ID is currently running.
    pub fn is_running(&self, agent_id: &str) -> bool {
        self.agents
            .get(agent_id)
            .map(|info| info.status == SubagentStatus::Running)
            .unwrap_or(false)
    }

    /// Check if an agent ID exists in state (running or stopped).
    pub fn has_agent(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    // -------------------------------------------------------------------
    // Pending messages (AGT-026)
    // -------------------------------------------------------------------

    /// Queue a message for delivery to a running agent.
    /// Messages are delivered at the next tool round boundary.
    pub fn queue_pending_message(&mut self, agent_id: &str, message: String) {
        self.pending_messages
            .entry(agent_id.to_string())
            .or_default()
            .push(message);
    }

    /// Drain all pending messages for an agent (take + clear).
    /// Returns empty vec if no messages queued.
    pub fn drain_pending_messages(&mut self, agent_id: &str) -> Vec<String> {
        self.pending_messages.remove(agent_id).unwrap_or_default()
    }

    /// Request graceful shutdown of a running agent.
    pub fn request_shutdown(&self, agent_id: &str) -> bool {
        if let Some(info) = self.agents.get(agent_id) {
            info.shutdown_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Get a clone of the shutdown flag for a registered agent.
    pub fn get_shutdown_flag(
        &self,
        agent_id: &str,
    ) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        self.agents
            .get(agent_id)
            .map(|info| std::sync::Arc::clone(&info.shutdown_flag))
    }

    // -------------------------------------------------------------------
    // Progress tracking (TASK-T3 / G4)
    // -------------------------------------------------------------------

    /// Clone the progress tracker `Arc` for an agent so the runner can write
    /// to it directly.  Returns `None` if the agent is unknown.
    pub fn get_progress_tracker_arc(
        &self,
        id: &str,
    ) -> Option<std::sync::Arc<std::sync::Mutex<ProgressTracker>>> {
        self.agents.get(id).map(|info| info.progress.clone())
    }

    /// Take a read-only snapshot of an agent's current progress.
    /// Returns `None` if the agent is unknown or the inner mutex is poisoned.
    pub fn get_progress(&self, id: &str) -> Option<ProgressSnapshot> {
        let info = self.agents.get(id)?;
        let guard = info.progress.lock().ok()?;
        Some(ProgressSnapshot {
            tool_use_count: guard.tool_use_count,
            cumulative_input_tokens: guard.cumulative_input_tokens,
            cumulative_output_tokens: guard.cumulative_output_tokens,
            cumulative_cache_creation_tokens: guard.cumulative_cache_creation_tokens,
            cumulative_cache_read_tokens: guard.cumulative_cache_read_tokens,
            recent_activities: guard.recent_activities.iter().cloned().collect(),
            last_update: guard.last_update,
        })
    }

    /// Clean up all state for an agent on completion:
    /// drops pending messages, removes from name registry if name matches.
    pub fn cleanup_agent(&mut self, agent_id: &str) {
        self.pending_messages.remove(agent_id);
        // Remove from name registry if this ID is registered
        self.name_registry.retain(|_, id| id != agent_id);
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CONCURRENT)
    }
}

// ---------------------------------------------------------------------------
// SubagentRunner — multi-turn agentic loop with tool dispatch (AGT-009)
// ---------------------------------------------------------------------------

pub mod runner {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use archon_llm::identity::IdentityProvider;
    use archon_llm::provider::{LlmProvider, LlmRequest};
    use archon_llm::streaming::StreamEvent;
    use archon_llm::types::ContentBlockType;
    use archon_tools::tool::ToolContext;
    use archon_tools::tool::ToolResult;
    use futures::future::join_all;

    use crate::dispatch::ToolRegistry;

    /// A multi-turn subagent that streams LLM responses and dispatches tool calls.
    ///
    /// Unlike the one-shot approach, SubagentRunner loops:
    ///   send request → collect response → if tool_use, dispatch tools → loop
    /// until: no tool_use, max_turns reached, or timeout.
    pub struct SubagentRunner {
        provider: Arc<dyn LlmProvider>,
        system_prompt: String,
        tool_definitions: Vec<serde_json::Value>,
        registry: Arc<ToolRegistry>,
        tool_context: ToolContext,
        model: String,
        max_turns: u32,
        timeout_secs: u64,
        /// Critical system reminder re-injected every turn (AGT-022).
        critical_system_reminder: Option<String>,
        /// Effort level passed to the LLM API (e.g. "low", "medium", "high").
        effort: Option<String>,
        /// Transcript store for fire-and-forget recording (AGT-024).
        transcript_store: Option<crate::agents::transcript::AgentTranscriptStore>,
        /// Agent ID for transcript recording (AGT-024).
        transcript_agent_id: Option<String>,
        /// Initial messages for resume — prepended before the prompt (AGT-024).
        initial_messages: Option<Vec<serde_json::Value>>,
        /// AGT-026: SubagentManager for draining pending messages at tool round boundaries.
        subagent_manager: Option<Arc<tokio::sync::Mutex<super::SubagentManager>>>,
        /// AGT-026: This runner's agent ID (for draining its pending messages).
        runner_agent_id: Option<String>,
        /// Graceful shutdown flag (checked each turn).
        shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        /// TASK-T3 (G4): per-agent progress tracker shared with SubagentManager.
        progress: Option<std::sync::Arc<std::sync::Mutex<super::ProgressTracker>>>,
        /// Parent AgentConfig for aligning LLM request structural fields
        /// (max_tokens, thinking, speed, effort) with the parent's working shape.
        agent_config: std::sync::Arc<crate::agent::AgentConfig>,
        /// Parent identity provider for billing-header prepend in spoof mode
        /// (v0.1.19 — last structural alignment gap between parent and subagent).
        identity: std::sync::Arc<IdentityProvider>,
    }

    /// A single pending tool call collected from the stream.
    #[derive(Debug)]
    struct PendingTool {
        id: String,
        name: String,
        input_json: String,
    }

    impl SubagentRunner {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            provider: Arc<dyn LlmProvider>,
            system_prompt: String,
            tool_definitions: Vec<serde_json::Value>,
            registry: Arc<ToolRegistry>,
            tool_context: ToolContext,
            model: String,
            max_turns: u32,
            timeout_secs: u64,
            agent_config: std::sync::Arc<crate::agent::AgentConfig>,
            identity: std::sync::Arc<IdentityProvider>,
        ) -> Self {
            Self {
                provider,
                system_prompt,
                tool_definitions,
                registry,
                tool_context,
                model,
                max_turns,
                timeout_secs,
                critical_system_reminder: None,
                effort: None,
                transcript_store: None,
                transcript_agent_id: None,
                initial_messages: None,
                subagent_manager: None,
                runner_agent_id: None,
                shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                progress: None,
                agent_config,
                identity,
            }
        }

        /// TASK-T3 (G4): wire a shared ProgressTracker so the runner can
        /// accumulate token usage and tool-use counts during the agentic loop.
        pub fn set_progress_tracker(
            &mut self,
            tracker: std::sync::Arc<std::sync::Mutex<super::ProgressTracker>>,
        ) {
            self.progress = Some(tracker);
        }

        /// Set subagent manager and agent ID for pending message drain (AGT-026).
        pub fn set_pending_message_source(
            &mut self,
            manager: Arc<tokio::sync::Mutex<super::SubagentManager>>,
            agent_id: String,
        ) {
            self.subagent_manager = Some(manager);
            self.runner_agent_id = Some(agent_id);
        }

        /// Set the shutdown flag (shared with SubagentManager for graceful shutdown).
        pub fn set_shutdown_flag(&mut self, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {
            self.shutdown_flag = flag;
        }

        /// Drain pending messages and return them as user turn JSON values.
        async fn drain_pending_as_user_turns(&self) -> Vec<serde_json::Value> {
            let (Some(mgr), Some(aid)) = (&self.subagent_manager, &self.runner_agent_id) else {
                return Vec::new();
            };
            let messages = mgr.lock().await.drain_pending_messages(aid);
            messages
                .into_iter()
                .map(|msg| serde_json::json!({ "role": "user", "content": msg }))
                .collect()
        }

        /// Set the critical system reminder for per-turn injection (AGT-022).
        pub fn set_critical_system_reminder(&mut self, reminder: String) {
            if reminder.is_empty() {
                self.critical_system_reminder = None;
            } else {
                self.critical_system_reminder = Some(reminder);
            }
        }

        /// Set the effort level for the LLM API (e.g. "low", "medium").
        pub fn set_effort(&mut self, effort: String) {
            if effort.is_empty() || effort.eq_ignore_ascii_case("high") {
                self.effort = None; // high is the default, no need to set
            } else {
                self.effort = Some(effort);
            }
        }

        /// Set transcript store and agent ID for fire-and-forget recording (AGT-024).
        pub fn set_transcript(
            &mut self,
            store: crate::agents::transcript::AgentTranscriptStore,
            agent_id: String,
        ) {
            self.transcript_store = Some(store);
            self.transcript_agent_id = Some(agent_id);
        }

        /// Set initial messages for resume — these are prepended before the prompt (AGT-024).
        pub fn set_initial_messages(&mut self, messages: Vec<serde_json::Value>) {
            if !messages.is_empty() {
                self.initial_messages = Some(messages);
            }
        }

        /// Fire-and-forget record a message to the transcript (AGT-024).
        fn record_transcript(&self, message: &serde_json::Value) {
            if let (Some(store), Some(aid)) = (&self.transcript_store, &self.transcript_agent_id) {
                store.record_message(aid, message);
            }
        }

        /// Snip oldest complete turn-pairs when messages exceed context limit.
        ///
        /// A turn-pair is an assistant message (possibly with tool_use) followed by
        /// a user message (possibly with tool_result). Snipping in pairs avoids
        /// breaking the tool_use/tool_result contract required by the Claude API.
        fn snip_context_if_needed(messages: &mut Vec<serde_json::Value>) {
            const MAX_CONTEXT_CHARS: usize = 600_000;
            const PRESERVE_RECENT_TURNS: usize = 3;

            let total_chars: usize = messages
                .iter()
                .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
                .sum();

            if total_chars <= MAX_CONTEXT_CHARS {
                return;
            }

            // Find turn boundaries from the end by scanning for assistant messages.
            // Each assistant message starts a "turn" (assistant + following user = pair).
            let mut turn_count = 0;
            let mut keep_from = messages.len();
            for i in (1..messages.len()).rev() {
                if messages[i].get("role").and_then(|r| r.as_str()) == Some("assistant") {
                    turn_count += 1;
                    if turn_count >= PRESERVE_RECENT_TURNS {
                        keep_from = i;
                        break;
                    }
                }
            }

            if keep_from <= 1 {
                return; // Nothing to snip
            }

            messages.drain(1..keep_from);
            messages.insert(
                1,
                serde_json::json!({
                    "role": "user",
                    "content": "[Earlier conversation was truncated to fit context window]"
                }),
            );
        }

        /// Run the subagent loop with the given initial prompt.
        /// Returns the accumulated text output from the final turn.
        pub async fn run(&self, initial_prompt: &str) -> anyhow::Result<String> {
            // AGT-024: Use initial_messages for resume, or start fresh
            let mut messages: Vec<serde_json::Value> =
                if let Some(ref initial) = self.initial_messages {
                    let mut msgs = initial.clone();
                    let user_msg = serde_json::json!({
                        "role": "user",
                        "content": initial_prompt,
                    });
                    self.record_transcript(&user_msg);
                    msgs.push(user_msg);
                    msgs
                } else {
                    let user_msg = serde_json::json!({
                        "role": "user",
                        "content": initial_prompt,
                    });
                    self.record_transcript(&user_msg);
                    vec![user_msg]
                };

            let deadline = Instant::now() + Duration::from_secs(self.timeout_secs);

            for turn in 0..self.max_turns {
                // Check timeout
                if Instant::now() >= deadline {
                    anyhow::bail!("Subagent timed out after {turn} turns");
                }

                // Check for graceful shutdown request
                if self
                    .shutdown_flag
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Ok("[Agent shutdown requested]".to_string());
                }

                // Assemble system blocks: billing header (spoof mode) →
                // agent body → critical system reminder (AGT-022).
                // Order matters for prompt-cache breakpoints (v0.1.19).
                let first_user_message = messages
                    .first()
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");

                let mut system: Vec<serde_json::Value> = Vec::new();

                // Billing header prepend aligns subagent with parent when
                // IdentityMode::Spoof is active — without this, Anthropic
                // 429s the request (project-zero billing-marker check).
                // IdentityMode::Clean returns None from billing_header(),
                // so the block is only pushed in spoof mode.
                if let Some(billing) = self.identity.billing_header(first_user_message) {
                    system.push(serde_json::json!({
                        "type": "text",
                        "text": billing,
                        "cache_control": { "type": "ephemeral" }
                    }));
                }

                system.push(serde_json::json!({
                    "type": "text",
                    "text": &self.system_prompt,
                }));

                // Inject critical system reminder every turn (AGT-022)
                if let Some(ref reminder) = self.critical_system_reminder {
                    system.push(serde_json::json!({
                        "type": "text",
                        "text": format!("<system-reminder>{reminder}</system-reminder>"),
                    }));
                }

                // Context snipping: prevent unbounded message growth
                Self::snip_context_if_needed(&mut messages);

                // Effort layering: per-agent-definition override wins if Some;
                // otherwise read the parent's live /effort setting (v0.1.18).
                let effort = if self.effort.is_some() {
                    self.effort.clone()
                } else {
                    let level = self.agent_config.effort_level.lock().await;
                    match *level {
                        archon_llm::effort::EffortLevel::High => None,
                        other => Some(other.to_string()),
                    }
                };

                let (max_tokens, thinking, speed) =
                    self.agent_config.build_base_request_fields(&self.model);

                let request = LlmRequest {
                    model: self.model.clone(),
                    max_tokens,
                    system,
                    messages: messages.clone(),
                    tools: self.tool_definitions.clone(),
                    thinking,
                    speed,
                    effort,
                    extra: serde_json::Value::Null,
                    request_origin: Some("subagent".into()),
                };

                // Stream the response
                let mut rx = self
                    .provider
                    .stream(request)
                    .await
                    .map_err(|e| anyhow::anyhow!("LLM stream error: {e}"))?;

                let mut text_content = String::new();
                let mut pending_tools: Vec<PendingTool> = Vec::new();
                let mut current_tool_index: Option<u32> = None;

                while let Some(event) = rx.recv().await {
                    match event {
                        StreamEvent::ContentBlockStart {
                            index,
                            block_type,
                            tool_use_id,
                            tool_name,
                        } => {
                            if block_type == ContentBlockType::ToolUse {
                                current_tool_index = Some(index);
                                pending_tools.push(PendingTool {
                                    id: tool_use_id.unwrap_or_default(),
                                    name: tool_name.unwrap_or_default(),
                                    input_json: String::new(),
                                });
                            }
                        }
                        StreamEvent::TextDelta { text, .. } => {
                            text_content.push_str(&text);
                        }
                        StreamEvent::InputJsonDelta {
                            index,
                            partial_json,
                        } => {
                            if Some(index) == current_tool_index
                                && let Some(tool) = pending_tools.last_mut()
                            {
                                tool.input_json.push_str(&partial_json);
                            }
                        }
                        StreamEvent::ContentBlockStop { .. } => {
                            current_tool_index = None;
                        }
                        StreamEvent::Error {
                            error_type,
                            message,
                        } => {
                            anyhow::bail!("LLM error: {error_type}: {message}");
                        }
                        StreamEvent::MessageStart { ref usage, .. } => {
                            // TASK-T3 (G4): accumulate Usage from message_start.
                            // Lock guard MUST NOT cross an .await — only sync work in here.
                            if let Some(ref t) = self.progress
                                && let Ok(mut g) = t.lock()
                            {
                                g.cumulative_input_tokens += usage.input_tokens;
                                g.cumulative_output_tokens += usage.output_tokens;
                                g.cumulative_cache_creation_tokens +=
                                    usage.cache_creation_input_tokens;
                                g.cumulative_cache_read_tokens += usage.cache_read_input_tokens;
                                g.last_update = chrono::Utc::now();
                            }
                        }
                        StreamEvent::MessageDelta {
                            usage: Some(ref u), ..
                        } => {
                            // TASK-T3 (G4): accumulate Usage from message_delta.
                            if let Some(ref t) = self.progress
                                && let Ok(mut g) = t.lock()
                            {
                                g.cumulative_input_tokens += u.input_tokens;
                                g.cumulative_output_tokens += u.output_tokens;
                                g.cumulative_cache_creation_tokens += u.cache_creation_input_tokens;
                                g.cumulative_cache_read_tokens += u.cache_read_input_tokens;
                                g.last_update = chrono::Utc::now();
                            }
                        }
                        _ => {} // ThinkingDelta, SignatureDelta, MessageDelta{usage:None}, MessageStop, Ping, etc.
                    }
                }

                // If no tool calls, subagent is done — return accumulated text
                if pending_tools.is_empty() {
                    // Record final assistant text to transcript (AGT-024)
                    if !text_content.is_empty() {
                        self.record_transcript(&serde_json::json!({
                            "role": "assistant",
                            "content": text_content,
                        }));
                    }
                    return Ok(text_content);
                }

                // Build assistant message with text + tool_use blocks
                let mut assistant_content: Vec<serde_json::Value> = Vec::new();
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
                let assistant_msg = serde_json::json!({
                    "role": "assistant",
                    "content": assistant_content,
                });
                self.record_transcript(&assistant_msg);
                messages.push(assistant_msg);

                // ── Three-phase parallel tool dispatch (v0.1.12) ──────────────
                // Mirrors claurst's proven pattern:
                //   Phase 1: sequential pre-hook pass (hooks/permissions are
                //            interactive and cannot be parallelized)
                //   Phase 2: concurrent execution via futures::future::join_all
                //            over Either<Left blocked, Right execute>.
                //            join_all preserves input order natively.
                //   Phase 3: assemble tool_result blocks in order, update
                //            progress tracker (sync, no .await across locks)
                //
                // Phase 1 — collect PreparedTool entries.
                struct PreparedTool {
                    id: String,
                    name: String,
                    input: serde_json::Value,
                }
                let mut prepared: Vec<PreparedTool> = Vec::with_capacity(pending_tools.len());
                for tool in &pending_tools {
                    let input: serde_json::Value =
                        serde_json::from_str(&tool.input_json).unwrap_or(serde_json::json!({}));
                    prepared.push(PreparedTool {
                        id: tool.id.clone(),
                        name: tool.name.clone(),
                        input,
                    });
                }

                // Phase 2 — execute all tools concurrently via join_all.
                // Each async block owns its cloned name/input/registry.
                let registry = Arc::clone(&self.registry);
                let exec_futures: Vec<_> = prepared
                    .iter()
                    .map(|p| {
                        let name = p.name.clone();
                        let input = p.input.clone();
                        let registry = Arc::clone(&registry);
                        let ctx = self.tool_context.clone();
                        async move { registry.dispatch(&name, input, &ctx).await }
                    })
                    .collect();

                let exec_results: Vec<ToolResult> = join_all(exec_futures).await;

                // Phase 3 — assemble tool_result blocks IN ORDER.
                // join_all preserves input order, so zip is correct.
                let mut tool_results: Vec<serde_json::Value> = Vec::with_capacity(prepared.len());
                for (p, result) in prepared.iter().zip(exec_results.into_iter()) {
                    // Progress update — sync only, lock never crosses .await
                    if let Some(ref t) = self.progress
                        && let Ok(mut g) = t.lock()
                    {
                        g.tool_use_count += 1;
                        if g.recent_activities.len() >= 5 {
                            g.recent_activities.pop_front();
                        }
                        g.recent_activities.push_back(super::ToolActivity {
                            tool_name: p.name.clone(),
                            timestamp: chrono::Utc::now(),
                        });
                        g.last_update = chrono::Utc::now();
                    }
                    tool_results.push(serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": p.id,
                        "content": result.content,
                        "is_error": result.is_error,
                    }));
                }

                // Add tool results as a user message
                let tool_result_msg = serde_json::json!({
                    "role": "user",
                    "content": tool_results,
                });
                self.record_transcript(&tool_result_msg);
                messages.push(tool_result_msg);

                // AGT-026: Drain pending messages at tool round boundary and inject as user turns
                let pending = self.drain_pending_as_user_turns().await;
                for msg in pending {
                    self.record_transcript(&msg);
                    messages.push(msg);
                }
            }

            anyhow::bail!("Subagent reached max turns ({})", self.max_turns)
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::agent::AgentConfig;
        use archon_llm::identity::IdentityMode;
        use archon_llm::provider::{LlmResponse, ModelInfo, ProviderFeature};
        use archon_llm::types::Usage;
        use archon_tools::tool::{PermissionLevel, Tool};
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::sync::mpsc;

        /// Mock provider that returns pre-configured responses.
        struct MockProvider {
            responses: std::sync::Mutex<Vec<Vec<StreamEvent>>>,
            call_count: AtomicU32,
        }

        impl MockProvider {
            fn new(responses: Vec<Vec<StreamEvent>>) -> Self {
                Self {
                    responses: std::sync::Mutex::new(responses),
                    call_count: AtomicU32::new(0),
                }
            }
        }

        #[async_trait::async_trait]
        impl LlmProvider for MockProvider {
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
            ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, archon_llm::provider::LlmError>
            {
                let idx = self.call_count.fetch_add(1, Ordering::SeqCst) as usize;
                let events = {
                    let mut responses = self.responses.lock().unwrap();
                    if idx < responses.len() {
                        responses[idx].drain(..).collect::<Vec<_>>()
                    } else {
                        vec![
                            StreamEvent::MessageStart {
                                id: "msg-end".into(),
                                model: "mock".into(),
                                usage: Usage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                    cache_creation_input_tokens: 0,
                                    cache_read_input_tokens: 0,
                                },
                            },
                            StreamEvent::ContentBlockStart {
                                index: 0,
                                block_type: ContentBlockType::Text,
                                tool_use_id: None,
                                tool_name: None,
                            },
                            StreamEvent::TextDelta {
                                index: 0,
                                text: "(done)".into(),
                            },
                            StreamEvent::ContentBlockStop { index: 0 },
                            StreamEvent::MessageStop,
                        ]
                    }
                }; // MutexGuard dropped here

                let (tx, rx) = mpsc::channel(events.len() + 1);
                for event in events {
                    let _ = tx.send(event).await;
                }
                Ok(rx)
            }

            async fn complete(
                &self,
                _request: LlmRequest,
            ) -> Result<LlmResponse, archon_llm::provider::LlmError> {
                unimplemented!()
            }
        }

        fn text_response(text: &str) -> Vec<StreamEvent> {
            vec![
                StreamEvent::MessageStart {
                    id: "msg-1".into(),
                    model: "mock".into(),
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 5,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                    },
                },
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                },
                StreamEvent::TextDelta {
                    index: 0,
                    text: text.into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
                StreamEvent::MessageStop,
            ]
        }

        fn tool_use_response(tool_id: &str, tool_name: &str, input_json: &str) -> Vec<StreamEvent> {
            vec![
                StreamEvent::MessageStart {
                    id: "msg-tool".into(),
                    model: "mock".into(),
                    usage: Usage {
                        input_tokens: 10,
                        output_tokens: 20,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                    },
                },
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some(tool_id.into()),
                    tool_name: Some(tool_name.into()),
                },
                StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: input_json.into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
                StreamEvent::MessageStop,
            ]
        }

        fn make_runner(provider: Arc<dyn LlmProvider>, max_turns: u32) -> SubagentRunner {
            let registry = Arc::new(crate::dispatch::create_default_registry(
                std::env::current_dir().unwrap_or_default(),
                None,
            ));
            let tool_defs = registry.tool_definitions();
            let ctx = ToolContext {
                working_dir: std::env::current_dir().unwrap_or_default(),
                session_id: "test-session".into(),
                mode: archon_tools::tool::AgentMode::Normal,
                extra_dirs: vec![],
                ..Default::default()
            };
            SubagentRunner::new(
                provider,
                "You are a test subagent.".into(),
                tool_defs,
                registry,
                ctx,
                "mock-model".into(),
                max_turns,
                300,
                Arc::new(AgentConfig::default()),
                Arc::new(IdentityProvider::new(
                    IdentityMode::Clean,
                    "test".into(),
                    String::new(),
                    String::new(),
                )),
            )
        }

        #[tokio::test]
        async fn text_only_returns_immediately() {
            let provider = Arc::new(MockProvider::new(vec![text_response(
                "Hello from subagent",
            )]));
            let runner = make_runner(provider.clone(), 10);
            let result = runner.run("Say hello").await.unwrap();
            assert_eq!(result, "Hello from subagent");
            assert_eq!(provider.call_count.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn tool_use_then_text_returns_after_two_turns() {
            let provider = Arc::new(MockProvider::new(vec![
                // Turn 1: LLM uses a tool
                tool_use_response("tool-1", "Read", r#"{"file_path":"/tmp/test.txt"}"#),
                // Turn 2: LLM returns text
                text_response("I read the file."),
            ]));
            let runner = make_runner(provider.clone(), 10);
            let result = runner.run("Read /tmp/test.txt").await.unwrap();
            assert_eq!(result, "I read the file.");
            assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn max_turns_enforced() {
            // Every turn uses a tool, never returns text
            let provider = Arc::new(MockProvider::new(vec![
                tool_use_response("t1", "Read", r#"{"file_path":"/tmp/a"}"#),
                tool_use_response("t2", "Read", r#"{"file_path":"/tmp/b"}"#),
                tool_use_response("t3", "Read", r#"{"file_path":"/tmp/c"}"#),
            ]));
            let runner = make_runner(provider.clone(), 2);
            let err = runner.run("keep going").await.unwrap_err();
            assert!(err.to_string().contains("max turns (2)"));
            assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn api_error_propagated() {
            let provider = Arc::new(MockProvider::new(vec![vec![StreamEvent::Error {
                error_type: "server_error".into(),
                message: "internal failure".into(),
            }]]));
            let runner = make_runner(provider, 10);
            let err = runner.run("trigger error").await.unwrap_err();
            assert!(err.to_string().contains("internal failure"));
        }

        #[tokio::test]
        async fn isolated_messages() {
            // Verify that each run starts with fresh messages
            let provider = Arc::new(MockProvider::new(vec![text_response("First run")]));
            let runner = make_runner(provider.clone(), 10);
            let r1 = runner.run("First prompt").await.unwrap();
            assert_eq!(r1, "First run");

            // Second run should start fresh (provider returns default "(done)")
            let r2 = runner.run("Second prompt").await.unwrap();
            assert_eq!(r2, "(done)");
        }

        #[tokio::test]
        async fn tool_dispatch_error_continues() {
            // Use a nonexistent tool — dispatch should return error, loop continues
            let provider = Arc::new(MockProvider::new(vec![
                tool_use_response("t1", "NonexistentTool", r#"{}"#),
                text_response("Recovered after tool error"),
            ]));
            let runner = make_runner(provider.clone(), 10);
            let result = runner.run("use bad tool").await.unwrap();
            assert_eq!(result, "Recovered after tool error");
        }

        #[tokio::test]
        async fn empty_tool_definitions_still_works() {
            let provider = Arc::new(MockProvider::new(vec![text_response("No tools needed")]));
            let registry = Arc::new(crate::dispatch::create_default_registry(
                std::env::current_dir().unwrap_or_default(),
                None,
            ));
            let ctx = ToolContext {
                working_dir: std::env::current_dir().unwrap_or_default(),
                session_id: "test".into(),
                mode: archon_tools::tool::AgentMode::Normal,
                extra_dirs: vec![],
                ..Default::default()
            };
            let runner = SubagentRunner::new(
                provider,
                "Test agent".into(),
                vec![], // Empty tool defs
                registry,
                ctx,
                "mock".into(),
                5,
                60,
                Arc::new(AgentConfig::default()),
                Arc::new(IdentityProvider::new(
                    IdentityMode::Clean,
                    "test".into(),
                    String::new(),
                    String::new(),
                )),
            );
            let result = runner.run("hello").await.unwrap();
            assert_eq!(result, "No tools needed");
        }

        #[test]
        fn snip_context_preserves_recent_turns() {
            let mut messages = Vec::new();
            // Original prompt
            messages.push(serde_json::json!({"role": "user", "content": "do something"}));
            // Generate many turn pairs to exceed threshold
            for i in 0..20 {
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": [{"type": "tool_use", "id": format!("t{i}"), "name": "Bash", "input": {}}]
                }));
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": format!("t{i}"), "content": "x".repeat(40_000)}]
                }));
            }

            // Should be over 600k chars
            let total: usize = messages
                .iter()
                .map(|m| serde_json::to_string(m).unwrap().len())
                .sum();
            assert!(
                total > 600_000,
                "test setup: messages should exceed threshold"
            );

            SubagentRunner::snip_context_if_needed(&mut messages);

            // First message preserved
            assert_eq!(messages[0]["role"], "user");
            assert_eq!(messages[0]["content"], "do something");

            // Truncation notice at index 1
            assert_eq!(messages[1]["role"], "user");
            assert!(
                messages[1]["content"]
                    .as_str()
                    .unwrap()
                    .contains("truncated")
            );

            // Remaining messages should be assistant/user pairs
            for chunk in messages[2..].chunks(2) {
                assert_eq!(chunk[0]["role"], "assistant");
                if chunk.len() > 1 {
                    assert_eq!(chunk[1]["role"], "user");
                }
            }
        }

        // -----------------------------------------------------------------
        // TASK-T3 (G4): SubagentRunner accumulates Usage from a streamed turn
        // -----------------------------------------------------------------

        #[tokio::test]
        async fn runner_accumulates_tokens_from_mock_stream() {
            // Single-turn stream: MessageStart with input/cache tokens, then a
            // text body, then MessageDelta carrying the final output_tokens,
            // then MessageStop.  No tool_use, so the runner returns after one turn.
            let stream_events = vec![
                StreamEvent::MessageStart {
                    id: "msg-prog-1".into(),
                    model: "mock".into(),
                    usage: Usage {
                        input_tokens: 100,
                        output_tokens: 5,
                        cache_creation_input_tokens: 10,
                        cache_read_input_tokens: 20,
                    },
                },
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                },
                StreamEvent::TextDelta {
                    index: 0,
                    text: "ok".into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
                StreamEvent::MessageDelta {
                    stop_reason: Some("end_turn".into()),
                    usage: Some(Usage {
                        input_tokens: 0,
                        output_tokens: 25,
                        cache_creation_input_tokens: 0,
                        cache_read_input_tokens: 0,
                    }),
                },
                StreamEvent::MessageStop,
            ];

            let provider = Arc::new(MockProvider::new(vec![stream_events]));
            let mut runner = make_runner(provider, 5);

            // Wire a fresh ProgressTracker arc into the runner.
            let tracker = std::sync::Arc::new(std::sync::Mutex::new(
                crate::subagent::ProgressTracker::default(),
            ));
            runner.set_progress_tracker(tracker.clone());

            let result = runner.run("hello").await.unwrap();
            assert_eq!(result, "ok");

            let g = tracker.lock().unwrap();
            assert_eq!(g.cumulative_input_tokens, 100);
            // 5 from MessageStart + 25 from MessageDelta
            assert_eq!(g.cumulative_output_tokens, 30);
            assert_eq!(g.cumulative_cache_creation_tokens, 10);
            assert_eq!(g.cumulative_cache_read_tokens, 20);
            // No tool_use blocks were dispatched.
            assert_eq!(g.tool_use_count, 0);
            assert!(g.recent_activities.is_empty());
        }

        #[tokio::test]
        async fn runner_increments_tool_use_count_on_dispatch() {
            // Turn 1: tool_use a (will fail dispatch — fine, counter still bumps)
            // Turn 2: text response, runner returns.
            let provider = Arc::new(MockProvider::new(vec![
                tool_use_response("call-1", "NonexistentTool", r#"{}"#),
                text_response("done"),
            ]));
            let mut runner = make_runner(provider, 5);

            let tracker = std::sync::Arc::new(std::sync::Mutex::new(
                crate::subagent::ProgressTracker::default(),
            ));
            runner.set_progress_tracker(tracker.clone());

            let result = runner.run("use a tool").await.unwrap();
            assert_eq!(result, "done");

            let g = tracker.lock().unwrap();
            assert_eq!(g.tool_use_count, 1);
            assert_eq!(g.recent_activities.len(), 1);
            assert_eq!(
                g.recent_activities.front().unwrap().tool_name,
                "NonexistentTool"
            );
            // Tokens accumulated across two turns:
            // Turn 1 (tool_use_response): MessageStart input=10, output=20
            // Turn 2 (text_response):     MessageStart input=10, output=5
            assert_eq!(g.cumulative_input_tokens, 20);
            assert_eq!(g.cumulative_output_tokens, 25);
        }

        // ── v0.1.12: parallel tool dispatch regression test ──────────

        struct SleeperTool {
            name: String,
            delay_ms: u64,
        }

        #[async_trait::async_trait]
        impl Tool for SleeperTool {
            fn name(&self) -> &str {
                &self.name
            }
            fn description(&self) -> &str {
                "test sleeper"
            }
            fn input_schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
                ToolResult::success(format!("done:{}", self.name))
            }
            fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
                PermissionLevel::Safe
            }
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn parallel_tool_dispatch_concurrent_and_order_preserved() {
            let mut registry = ToolRegistry::new();
            registry.register(Box::new(SleeperTool {
                name: "sleeper-A".into(),
                delay_ms: 400,
            }));
            registry.register(Box::new(SleeperTool {
                name: "sleeper-B".into(),
                delay_ms: 200,
            }));
            registry.register(Box::new(SleeperTool {
                name: "sleeper-C".into(),
                delay_ms: 300,
            }));
            let registry = Arc::new(registry);
            let tool_defs = registry.tool_definitions();

            let provider = Arc::new(MockProvider::new(vec![
                // Turn 1: 3 tool_use blocks with shuffled delays
                vec![
                    StreamEvent::MessageStart {
                        id: "msg-1".into(),
                        model: "mock".into(),
                        usage: Usage {
                            input_tokens: 10,
                            output_tokens: 5,
                            cache_creation_input_tokens: 0,
                            cache_read_input_tokens: 0,
                        },
                    },
                    StreamEvent::ContentBlockStart {
                        index: 0,
                        block_type: ContentBlockType::ToolUse,
                        tool_use_id: Some("t1".into()),
                        tool_name: Some("sleeper-A".into()),
                    },
                    StreamEvent::InputJsonDelta {
                        index: 0,
                        partial_json: "{}".into(),
                    },
                    StreamEvent::ContentBlockStop { index: 0 },
                    StreamEvent::ContentBlockStart {
                        index: 1,
                        block_type: ContentBlockType::ToolUse,
                        tool_use_id: Some("t2".into()),
                        tool_name: Some("sleeper-B".into()),
                    },
                    StreamEvent::InputJsonDelta {
                        index: 1,
                        partial_json: "{}".into(),
                    },
                    StreamEvent::ContentBlockStop { index: 1 },
                    StreamEvent::ContentBlockStart {
                        index: 2,
                        block_type: ContentBlockType::ToolUse,
                        tool_use_id: Some("t3".into()),
                        tool_name: Some("sleeper-C".into()),
                    },
                    StreamEvent::InputJsonDelta {
                        index: 2,
                        partial_json: "{}".into(),
                    },
                    StreamEvent::ContentBlockStop { index: 2 },
                    StreamEvent::MessageStop,
                ],
                text_response("all done"),
            ]));

            let ctx = ToolContext {
                working_dir: std::env::current_dir().unwrap_or_default(),
                session_id: "test-parallel".into(),
                mode: archon_tools::tool::AgentMode::Normal,
                extra_dirs: vec![],
                ..Default::default()
            };

            let runner = SubagentRunner::new(
                provider,
                "test".into(),
                tool_defs,
                registry,
                ctx,
                "mock".into(),
                5,
                60,
                Arc::new(AgentConfig::default()),
                Arc::new(IdentityProvider::new(
                    IdentityMode::Clean,
                    "test".into(),
                    String::new(),
                    String::new(),
                )),
            );

            let start = std::time::Instant::now();
            let result = runner.run("run all three").await.unwrap();
            let elapsed = start.elapsed();

            // The subagent ran turn 1 (3 tool_use dispatched in parallel)
            // then turn 2 (text "all done"). If dispatch had failed, the
            // subagent would have returned an error. "all done" means the
            // loop completed cleanly.
            assert_eq!(result, "all done");

            // Concurrent: max delay is 400ms. Serial sum would be ~900ms.
            // 1.5× headroom for CI variance.
            assert!(
                elapsed.as_millis() < 900,
                "{}ms — expected <900ms for 3×400ms concurrent (serial would be ~900ms)",
                elapsed.as_millis()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (SubagentManager)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SubagentRequest {
        SubagentRequest {
            prompt: "Analyze the codebase".into(),
            model: Some("claude-sonnet-4-6".into()),
            allowed_tools: vec!["Read".into(), "Glob".into()],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
        }
    }

    #[test]
    fn register_returns_uuid() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).expect("should register");

        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(id.len(), 36);
        assert!(id.contains('-'));
    }

    #[test]
    fn get_status_returns_running() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        let info = mgr.get_status(&id).expect("should exist");
        assert_eq!(info.status, SubagentStatus::Running);
        assert_eq!(info.request.prompt, "Analyze the codebase");
        assert!(info.result.is_none());
    }

    #[test]
    fn list_active_only_returns_running() {
        let mut mgr = SubagentManager::default();
        let id1 = mgr.register(sample_request()).unwrap();
        let _id2 = mgr.register(sample_request()).unwrap();

        assert_eq!(mgr.list_active().len(), 2);

        mgr.complete(&id1, "done".into()).unwrap();
        assert_eq!(mgr.list_active().len(), 1);
    }

    #[test]
    fn complete_sets_result() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.complete(&id, "task finished successfully".into())
            .unwrap();

        let info = mgr.get_status(&id).unwrap();
        assert_eq!(info.status, SubagentStatus::Completed);
        assert_eq!(info.result.as_deref(), Some("task finished successfully"));
    }

    #[test]
    fn complete_nonexistent_returns_error() {
        let mut mgr = SubagentManager::default();
        let err = mgr.complete("fake-id", "result".into()).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn complete_already_completed_returns_error() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();
        mgr.complete(&id, "first".into()).unwrap();

        let err = mgr.complete(&id, "second".into()).unwrap_err();
        assert!(err.to_string().contains("not in Running state"));
    }

    #[test]
    fn max_concurrent_enforced() {
        let mut mgr = SubagentManager::new(2);
        mgr.register(sample_request()).unwrap();
        mgr.register(sample_request()).unwrap();

        let err = mgr.register(sample_request()).unwrap_err();
        assert!(err.to_string().contains("max concurrent"));
    }

    #[test]
    fn mark_timed_out_works() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.mark_timed_out(&id).unwrap();
        assert_eq!(
            mgr.get_status(&id).unwrap().status,
            SubagentStatus::TimedOut
        );
    }

    #[test]
    fn mark_failed_works() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.mark_failed(&id, "something went wrong".into()).unwrap();
        assert_eq!(
            mgr.get_status(&id).unwrap().status,
            SubagentStatus::Failed("something went wrong".into())
        );
    }

    #[test]
    fn get_status_nonexistent_returns_none() {
        let mgr = SubagentManager::default();
        assert!(mgr.get_status("nonexistent").is_none());
    }

    // -----------------------------------------------------------------------
    // Auto-background tests (AGT-025)
    // -----------------------------------------------------------------------

    #[test]
    fn auto_background_constant_is_120s() {
        assert_eq!(super::AUTO_BACKGROUND_MS, 120_000);
    }

    #[test]
    fn auto_background_disabled_by_default() {
        unsafe {
            std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
        }
        assert!(!super::is_auto_background_enabled());
        assert_eq!(super::get_auto_background_ms(), 0);
    }

    #[test]
    fn auto_background_enabled_with_1() {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "1");
        }
        assert!(super::is_auto_background_enabled());
        assert_eq!(super::get_auto_background_ms(), 120_000);
        unsafe {
            std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
        }
    }

    #[test]
    fn auto_background_enabled_with_true() {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "true");
        }
        assert!(super::is_auto_background_enabled());
        unsafe {
            std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
        }
    }

    #[test]
    fn auto_background_disabled_for_zero() {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "0");
        }
        assert!(!super::is_auto_background_enabled());
        assert_eq!(super::get_auto_background_ms(), 0);
        unsafe {
            std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
        }
    }

    #[test]
    fn auto_background_case_insensitive() {
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "TRUE");
        }
        assert!(super::is_auto_background_enabled());
        unsafe {
            std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "True");
        }
        assert!(super::is_auto_background_enabled());
        unsafe {
            std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
        }
    }

    // -----------------------------------------------------------------------
    // Name registry tests (AGT-026)
    // -----------------------------------------------------------------------

    #[test]
    fn register_name_and_resolve() {
        let mut mgr = SubagentManager::default();
        mgr.register_name("explorer".into(), "agent-uuid-123".into());

        assert_eq!(mgr.resolve_name("explorer"), Some("agent-uuid-123"));
        assert_eq!(mgr.resolve_name("unknown"), None);
    }

    #[test]
    fn unregister_name_removes_entry() {
        let mut mgr = SubagentManager::default();
        mgr.register_name("explorer".into(), "agent-uuid-123".into());
        mgr.unregister_name("explorer");

        assert_eq!(mgr.resolve_name("explorer"), None);
    }

    #[test]
    fn register_name_overwrites_previous() {
        let mut mgr = SubagentManager::default();
        mgr.register_name("explorer".into(), "old-id".into());
        mgr.register_name("explorer".into(), "new-id".into());

        assert_eq!(mgr.resolve_name("explorer"), Some("new-id"));
    }

    #[test]
    fn is_running_checks_status() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        assert!(mgr.is_running(&id));

        mgr.complete(&id, "done".into()).unwrap();
        assert!(!mgr.is_running(&id));
    }

    #[test]
    fn is_running_false_for_nonexistent() {
        let mgr = SubagentManager::default();
        assert!(!mgr.is_running("nonexistent-id"));
    }

    #[test]
    fn has_agent_checks_existence() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        assert!(mgr.has_agent(&id));
        assert!(!mgr.has_agent("nonexistent-id"));

        // Completed agents still exist in state
        mgr.complete(&id, "done".into()).unwrap();
        assert!(mgr.has_agent(&id));
    }

    // -----------------------------------------------------------------------
    // Pending message tests (AGT-026)
    // -----------------------------------------------------------------------

    #[test]
    fn queue_and_drain_pending_messages() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.queue_pending_message(&id, "msg1".into());
        mgr.queue_pending_message(&id, "msg2".into());
        mgr.queue_pending_message(&id, "msg3".into());

        let drained = mgr.drain_pending_messages(&id);
        assert_eq!(drained, vec!["msg1", "msg2", "msg3"]);

        // Second drain returns empty (queue was cleared)
        let drained2 = mgr.drain_pending_messages(&id);
        assert!(drained2.is_empty());
    }

    #[test]
    fn drain_nonexistent_agent_returns_empty() {
        let mut mgr = SubagentManager::default();
        let drained = mgr.drain_pending_messages("nonexistent-id");
        assert!(drained.is_empty());
    }

    #[test]
    fn pending_messages_are_fifo() {
        let mut mgr = SubagentManager::default();
        mgr.queue_pending_message("agent-1", "first".into());
        mgr.queue_pending_message("agent-1", "second".into());
        mgr.queue_pending_message("agent-1", "third".into());

        let drained = mgr.drain_pending_messages("agent-1");
        assert_eq!(drained[0], "first");
        assert_eq!(drained[1], "second");
        assert_eq!(drained[2], "third");
    }

    #[test]
    fn pending_messages_isolated_per_agent() {
        let mut mgr = SubagentManager::default();
        mgr.queue_pending_message("agent-1", "msg-a".into());
        mgr.queue_pending_message("agent-2", "msg-b".into());

        let drained1 = mgr.drain_pending_messages("agent-1");
        assert_eq!(drained1, vec!["msg-a"]);

        let drained2 = mgr.drain_pending_messages("agent-2");
        assert_eq!(drained2, vec!["msg-b"]);
    }

    // -----------------------------------------------------------------------
    // Cleanup tests (AGT-026)
    // -----------------------------------------------------------------------

    #[test]
    fn cleanup_agent_drops_pending_messages() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();
        mgr.queue_pending_message(&id, "lost message".into());

        mgr.cleanup_agent(&id);

        let drained = mgr.drain_pending_messages(&id);
        assert!(
            drained.is_empty(),
            "pending messages should be lost on cleanup"
        );
    }

    #[test]
    fn cleanup_agent_removes_name_registry_entry() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();
        mgr.register_name("explorer".into(), id.clone());

        mgr.cleanup_agent(&id);

        assert_eq!(
            mgr.resolve_name("explorer"),
            None,
            "name should be removed on cleanup"
        );
    }

    #[test]
    fn cleanup_only_removes_matching_name() {
        let mut mgr = SubagentManager::default();
        let id1 = mgr.register(sample_request()).unwrap();
        let id2 = mgr.register(sample_request()).unwrap();
        mgr.register_name("explorer".into(), id1.clone());
        mgr.register_name("reviewer".into(), id2.clone());

        mgr.cleanup_agent(&id1);

        assert_eq!(mgr.resolve_name("explorer"), None);
        assert_eq!(mgr.resolve_name("reviewer"), Some(id2.as_str()));
    }

    // -----------------------------------------------------------------------
    // TASK-T2 (G2): Structured envelope delivery via queue
    // -----------------------------------------------------------------------

    #[test]
    fn structured_envelope_delivers_through_queue() {
        use archon_tools::send_message::{SendMessageRequest, build_structured_envelope};

        let mut mgr = SubagentManager::new(4);
        let id_a = mgr.register(sample_request()).unwrap();
        let id_b = mgr.register(sample_request()).unwrap();

        let envelope_req = SendMessageRequest {
            to: id_b.clone(),
            message: String::new(),
            summary: None,
            message_type: "shutdown_response".into(),
            request_id: Some("req-1".into()),
            approve: Some(true),
            reason: Some("done".into()),
            feedback: None,
        };
        let envelope = build_structured_envelope(&envelope_req);
        mgr.queue_pending_message(&id_b, envelope);

        let drained = mgr.drain_pending_messages(&id_b);
        assert_eq!(drained.len(), 1);
        assert!(
            drained[0].starts_with("<archon_structured_message type=\"shutdown_response\""),
            "envelope should start with structured opening tag: {}",
            drained[0]
        );
        assert!(drained[0].contains("request_id=\"req-1\""));
        assert!(drained[0].contains("approve=\"true\""));
        assert!(drained[0].contains("<reason>done</reason>"));
        assert!(drained[0].ends_with("</archon_structured_message>"));

        // Agent A's queue should remain untouched
        assert!(mgr.drain_pending_messages(&id_a).is_empty());
    }

    // -----------------------------------------------------------------------
    // TASK-T3 (G4): ProgressTracker accumulation tests
    // -----------------------------------------------------------------------

    #[test]
    fn progress_tracker_default_has_sane_state() {
        let t = ProgressTracker::default();
        assert_eq!(t.tool_use_count, 0);
        assert_eq!(t.cumulative_input_tokens, 0);
        assert_eq!(t.cumulative_output_tokens, 0);
        assert_eq!(t.cumulative_cache_creation_tokens, 0);
        assert_eq!(t.cumulative_cache_read_tokens, 0);
        assert!(t.recent_activities.is_empty());
    }

    #[test]
    fn progress_tracker_activities_bounded_at_five() {
        let mut t = ProgressTracker::default();
        for i in 0..7u32 {
            // Mirror the runner's bounding logic: pop oldest before push when at cap.
            if t.recent_activities.len() >= 5 {
                t.recent_activities.pop_front();
            }
            t.recent_activities.push_back(ToolActivity {
                tool_name: format!("tool-{i}"),
                timestamp: chrono::Utc::now(),
            });
        }
        assert_eq!(t.recent_activities.len(), 5);
        // Oldest two ("tool-0", "tool-1") should have been evicted.
        assert_eq!(t.recent_activities.front().unwrap().tool_name, "tool-2");
        assert_eq!(t.recent_activities.back().unwrap().tool_name, "tool-6");
    }

    #[test]
    fn progress_tracker_accumulates_usage_from_message_start() {
        // Simulate the same accumulation the runner performs in its
        // MessageStart / MessageDelta arms.
        let usages = [
            archon_llm::types::Usage {
                input_tokens: 100,
                output_tokens: 5,
                cache_creation_input_tokens: 10,
                cache_read_input_tokens: 20,
            },
            archon_llm::types::Usage {
                input_tokens: 50,
                output_tokens: 25,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 5,
            },
            archon_llm::types::Usage {
                input_tokens: 0,
                output_tokens: 12,
                cache_creation_input_tokens: 2,
                cache_read_input_tokens: 0,
            },
        ];

        let mut t = ProgressTracker::default();
        for u in &usages {
            t.cumulative_input_tokens += u.input_tokens;
            t.cumulative_output_tokens += u.output_tokens;
            t.cumulative_cache_creation_tokens += u.cache_creation_input_tokens;
            t.cumulative_cache_read_tokens += u.cache_read_input_tokens;
            t.last_update = chrono::Utc::now();
        }

        assert_eq!(t.cumulative_input_tokens, 150);
        assert_eq!(t.cumulative_output_tokens, 42);
        assert_eq!(t.cumulative_cache_creation_tokens, 12);
        assert_eq!(t.cumulative_cache_read_tokens, 25);
    }

    #[test]
    fn subagent_manager_get_progress_returns_snapshot() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        let snap = mgr
            .get_progress(&id)
            .expect("snapshot exists for live agent");
        assert_eq!(snap.tool_use_count, 0);
        assert_eq!(snap.cumulative_input_tokens, 0);
        assert_eq!(snap.cumulative_output_tokens, 0);
        assert_eq!(snap.cumulative_cache_creation_tokens, 0);
        assert_eq!(snap.cumulative_cache_read_tokens, 0);
        assert!(snap.recent_activities.is_empty());

        // Unknown id returns None
        assert!(mgr.get_progress("not-a-real-id").is_none());
    }

    #[test]
    fn subagent_manager_get_progress_tracker_arc_clones_same_arc() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        let arc1 = mgr.get_progress_tracker_arc(&id).expect("arc1");
        let arc2 = mgr.get_progress_tracker_arc(&id).expect("arc2");

        // Mutate via arc1, observe via arc2 — proves both point to the same inner mutex.
        {
            let mut g = arc1.lock().unwrap();
            g.tool_use_count = 42;
            g.cumulative_input_tokens = 1234;
        }
        {
            let g = arc2.lock().unwrap();
            assert_eq!(g.tool_use_count, 42);
            assert_eq!(g.cumulative_input_tokens, 1234);
        }

        // And the manager's snapshot view also reflects the mutation.
        let snap = mgr.get_progress(&id).unwrap();
        assert_eq!(snap.tool_use_count, 42);
        assert_eq!(snap.cumulative_input_tokens, 1234);
    }
}
