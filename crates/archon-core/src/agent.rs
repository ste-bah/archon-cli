use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

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
use archon_observability::{
    AgentActivityEvent, AgentActivityKind, AgentActivitySink, AgentActivityStatus,
};
use archon_permissions::auto::{AutoDecision, AutoModeEvaluator};
use archon_permissions::is_default_safe_tool;
use archon_session::checkpoint::CheckpointStore;
use archon_session::plan::PlanStore;
use archon_tools::plan_mode::is_tool_allowed_in_mode;
use archon_tools::send_message::SendMessageRequest;
use archon_tools::tool::{AgentMode, ToolContext, ToolResult};
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::ChannelMetricSink;
use crate::agents::AgentRegistry;
use crate::auto_extraction::AutoExtractor;
use crate::dispatch::ToolRegistry;
use crate::subagent::SubagentManager;

mod compaction;
mod events;
mod lifecycle;
mod memory_integration;
mod support;
#[cfg(test)]
mod tests;
mod types;

use events::emit_tool_result_activity;
pub use support::AgentLoopError;
use support::{parse_plan_from_text, user_correction_excerpt};
pub use types::{AgentConfig, AgentEvent, ConversationState, SessionStats, TimestampedEvent};

/// Single source of truth gate: does the agent loop auto-allow this tool in
/// default mode? Must always agree with `archon_permissions::DEFAULT_SAFE_TOOLS`.
/// Called by the lockstep regression test.
pub fn is_safe_in_default_mode(name: &str) -> bool {
    is_default_safe_tool(name)
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
    event_tx: tokio::sync::mpsc::UnboundedSender<TimestampedEvent>,
    checkpoint_store: Option<Arc<Mutex<CheckpointStore>>>,
    plan_store: Option<PlanStore>,
    turn_number: u64,
    // GAP 5/7: Memory graph + injector for per-turn injection and auto-extraction
    memory: Option<Arc<dyn MemoryTrait>>,
    memory_injector: MemoryInjector,
    extraction_config: ExtractionConfig,
    extraction_state: ExtractionState,
    // v0.1.23: AutoExtraction (LLM-based) learning system.
    auto_extractor: Option<Arc<AutoExtractor>>,
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
    /// Channel instrumentation sink for tracking sent/drained counts.
    metrics: Option<Arc<dyn ChannelMetricSink>>,
    /// GNN auto-trainer hook: invoked with `n=count` after a successful memory
    /// store (auto-extraction, inner-voice snapshot). archon-core cannot depend
    /// on archon-pipeline (would create a cycle), so the AutoTrainer is injected
    /// as a closure by the binary at startup. Reference:
    /// `archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs`.
    record_memory_callback: Option<Arc<dyn Fn(u64) + Send + Sync>>,
    /// GNN auto-trainer hook: invoked after a successful correction record.
    /// Same injection rationale as `record_memory_callback`.
    record_correction_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Governed-learning hook: invoked after correction handling and rule
    /// reinforcement so embedders can persist a UserCorrected LearningEvent.
    /// archon-core cannot depend on archon-learning, so this is closure-wired.
    record_user_correction_event_callback:
        Option<Arc<dyn Fn(UserCorrectionEventPayload) + Send + Sync>>,
    /// Personality-mirror hook: invoked with the post-mutation `&InnerVoice`
    /// after every write site (per-tool-call, per-turn-complete, user
    /// correction). Wired by the binary at startup so a sync-Mutex mirror
    /// stays in lock-step with the async-Mutex inner_voice. The mirror is
    /// read by the panic hook (which has no tokio runtime to await on the
    /// async Mutex). Reference: `src/panic_save.rs` and TASK #245.
    #[allow(clippy::type_complexity)]
    inner_voice_change_callback: Option<Arc<dyn Fn(&InnerVoice) + Send + Sync>>,
}

/// Payload emitted by the agent loop when a user correction is detected.
///
/// Kept in archon-core as plain data so the binary/pipeline layer can map it
/// into archon-learning without introducing a crate cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCorrectionEventPayload {
    pub correction_type: String,
    pub top_rule_id: Option<String>,
    pub user_input_excerpt: String,
    pub session_context: String,
}

impl Agent {
    /// Process a single user message through the full agent loop.
    /// Returns when the LLM produces a final text response (no more tool calls).
    pub async fn process_message(&mut self, user_input: &str) -> Result<(), AgentLoopError> {
        self.turn_number += 1;
        self.emit_activity(
            AgentActivityKind::ParentTurnStarted,
            AgentActivityStatus::Running,
            format!("turn {} started", self.turn_number),
        );
        self.state.add_user_message(user_input);

        // v0.1.23: AutoExtraction — LLM-driven fact extraction every N turns.
        if let Some(ref extractor) = self.auto_extractor {
            let extractor = Arc::clone(extractor);
            let turns: Vec<String> = self
                .state
                .messages
                .iter()
                .filter_map(|m| {
                    m.get("content").and_then(|c| {
                        if let Some(s) = c.as_str() {
                            Some(s.to_string())
                        } else if let Some(arr) = c.as_array() {
                            let text: String = arr
                                .iter()
                                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join(" ");
                            if text.is_empty() { None } else { Some(text) }
                        } else {
                            None
                        }
                    })
                })
                .collect();
            let model = self.config.model.clone();
            let turn = self.turn_number as u32;
            tokio::spawn(async move {
                let _ = extractor.maybe_extract(&turns, turn, &model).await;
            });
        }

        let mut agentic_iterations: u32 = 0;
        loop {
            // GAP 7: Inject recalled memories into system prompt
            let mut system_with_memories = self.inject_memories();
            // Append inner voice block (consciousness state) if enabled
            self.inject_inner_voice(&mut system_with_memories).await;
            // Append critical system reminder (AGT-022) — re-injected every turn
            self.inject_critical_reminder(&mut system_with_memories);

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

            let (max_tokens, thinking, speed) =
                self.config.build_base_request_fields(&active_model);

            // Build the API request
            let request = LlmRequest {
                model: active_model.clone(),
                max_tokens,
                system: system_with_memories,
                messages: self.state.messages.clone(),
                tools: self.config.tools.clone(),
                thinking,
                speed,
                effort,
                extra: serde_json::Value::Null,
                request_origin: Some("main_session".into()),
                reasoning_encrypted: None,
            };

            self.send_event(AgentEvent::ApiCallStarted {
                model: active_model.clone(),
            })
            .await;

            // Send request and get streaming events
            let mut rx = match self.client.stream(request).await {
                Ok(rx) => rx,
                Err(e) => {
                    self.emit_activity(
                        AgentActivityKind::ParentTurnCompleted,
                        AgentActivityStatus::Failed,
                        format!("turn {} failed: {e}", self.turn_number),
                    );
                    return Err(AgentLoopError::ApiError(format!("{e}")));
                }
            };

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
                    StreamEvent::ReasoningEncrypted { .. } => {}

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
                        self.emit_activity(
                            AgentActivityKind::ParentTurnCompleted,
                            AgentActivityStatus::Failed,
                            format!("turn {} failed: {error_type}: {message}", self.turn_number),
                        );
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
                    // TASK-AGS-107: propagate cancel token so subagent spawns
                    // create child_token() chains for Ctrl+C cascading.
                    cancel_parent: self.config.cancel_token.clone(),
                    // GHOST-006: sandbox backend from session boot, checked at
                    // both dispatch sites.
                    sandbox: self.config.sandbox.clone(),
                    activity_sink: self.provider_model_activity_sink(&active_model),
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
                        "default" | "ask" => {
                            if is_default_safe_tool(&tool.name) {
                                tracing::debug!(tool = %tool.name, "default-mode: safe, allowed");
                                true
                            } else {
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
                        }
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
                                    "TodoWrite" | "Sleep" => AutoDecision::Allow,
                                    _ if is_default_safe_tool(&tool.name) => AutoDecision::Allow,
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
                            // GHOST-006: sandbox pre-check (main-agent direct path).
                            let result = if let Some(ref backend) = ctx_clone.sandbox {
                                match backend.check(tool.name(), &input) {
                                    Err(reason) => {
                                        crate::dispatch::emit_tool_activity(
                                            &ctx_clone,
                                            tool.name(),
                                            AgentActivityKind::ToolFailed,
                                            AgentActivityStatus::Failed,
                                        );
                                        ToolResult::error(reason)
                                    }
                                    Ok(()) => {
                                        crate::dispatch::emit_tool_activity(
                                            &ctx_clone,
                                            tool.name(),
                                            AgentActivityKind::ToolStarted,
                                            AgentActivityStatus::Running,
                                        );
                                        let result = tool.execute(input, &ctx_clone).await;
                                        emit_tool_result_activity(&ctx_clone, tool.name(), &result);
                                        result
                                    }
                                }
                            } else {
                                crate::dispatch::emit_tool_activity(
                                    &ctx_clone,
                                    tool.name(),
                                    AgentActivityKind::ToolStarted,
                                    AgentActivityStatus::Running,
                                );
                                let result = tool.execute(input, &ctx_clone).await;
                                emit_tool_result_activity(&ctx_clone, tool.name(), &result);
                                result
                            };
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
                        // GHOST-006: sandbox pre-check (main-agent sequential path).
                        let result = if let Some(ref backend) = ctx.sandbox {
                            match backend.check(pre.tool_arc.name(), &pre.input) {
                                Err(reason) => {
                                    crate::dispatch::emit_tool_activity(
                                        &ctx,
                                        pre.tool_arc.name(),
                                        AgentActivityKind::ToolFailed,
                                        AgentActivityStatus::Failed,
                                    );
                                    ToolResult::error(reason)
                                }
                                Ok(()) => {
                                    crate::dispatch::emit_tool_activity(
                                        &ctx,
                                        pre.tool_arc.name(),
                                        AgentActivityKind::ToolStarted,
                                        AgentActivityStatus::Running,
                                    );
                                    let result =
                                        pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                                    emit_tool_result_activity(&ctx, pre.tool_arc.name(), &result);
                                    result
                                }
                            }
                        } else {
                            crate::dispatch::emit_tool_activity(
                                &ctx,
                                pre.tool_arc.name(),
                                AgentActivityKind::ToolStarted,
                                AgentActivityStatus::Running,
                            );
                            let result = pre.tool_arc.execute(pre.input.clone(), &ctx).await;
                            emit_tool_result_activity(&ctx, pre.tool_arc.name(), &result);
                            result
                        };
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
                                                let cancel =
                                                    tokio_util::sync::CancellationToken::new();
                                                let resume_ctx = archon_tools::tool::ToolContext {
                                                    working_dir: self.config.working_dir.clone(),
                                                    session_id: self.config.session_id.clone(),
                                                    mode: archon_tools::tool::AgentMode::Normal,
                                                    extra_dirs: vec![],
                                                    in_fork: crate::agents::built_in::is_in_fork_child_by_messages(&self.state.messages),
                                                    nested: false,
                                                    cancel_parent: self.config.cancel_token.clone(),
                                                    sandbox: self.config.sandbox.clone(),
                                                    activity_sink: self.provider_model_activity_sink(&active_model),
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
                                other => {
                                    ToolResult::error(format!("Unknown message_type: {}", other))
                                }
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
                    if pre.tool_name == "Bash"
                        && let Some(cmd) = pre.input.get("command").and_then(|v| v.as_str())
                        && (cmd.trim_start().starts_with("cd ")
                            || cmd.contains(" && cd ")
                            || cmd.contains("; cd "))
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
                        // TASK #245: keep panic-mirror in lock-step.
                        if let Some(ref cb) = self.inner_voice_change_callback {
                            cb(&iv);
                        }
                    }

                    // Wire 3: Track plan step progress on Write/Edit completions.
                    if !result.is_error
                        && (pre.tool_name == "Write" || pre.tool_name == "Edit")
                        && let Some(ref plan_store) = self.plan_store
                    {
                        let sid = self.config.session_id.clone();
                        if let Ok(Some(plan)) = plan_store.load_latest_plan(&sid)
                            && (plan.status == "active" || plan.status == "draft")
                            && let Some(ref fp) = pre.file_path
                        {
                            for step in &plan.steps {
                                if step.status == archon_session::plan::PlanStepStatus::Pending
                                    && step
                                        .affected_files
                                        .iter()
                                        .any(|f| fp.ends_with(f) || f.ends_with(fp))
                                    && let Err(e) = plan_store.update_step_status(
                                        &sid,
                                        &plan.id,
                                        step.number,
                                        archon_session::plan::PlanStepStatus::InProgress,
                                    )
                                {
                                    tracing::debug!("plan step update failed: {e}");
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
                if let Some(max) = self.config.max_turns
                    && agentic_iterations >= max
                {
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
                let mut iv_guard = iv.lock().await;
                iv_guard.on_turn_complete();
                // TASK #245: keep panic-mirror in lock-step.
                if let Some(ref cb) = self.inner_voice_change_callback {
                    cb(&iv_guard);
                }
            }

            self.send_event(AgentEvent::TurnComplete {
                input_tokens: turn_input_tokens,
                output_tokens: turn_output_tokens,
            })
            .await;

            // CRIT-14 (ITEM 4): Decay rule scores every 50 turns.
            if self.turn_number.is_multiple_of(50)
                && let Some(ref graph) = self.memory
            {
                let engine = RulesEngine::new(graph.as_ref());
                if let Err(e) = engine.decay_scores(1.0) {
                    tracing::warn!("rules decay_scores failed: {e}");
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

        self.emit_activity(
            AgentActivityKind::ParentTurnCompleted,
            AgentActivityStatus::Completed,
            format!("turn {} completed", self.turn_number),
        );
        Ok(())
    }
}
