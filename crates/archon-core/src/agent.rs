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
use archon_permissions::auto::AutoModeEvaluator;
use archon_permissions::is_default_safe_tool;
use archon_session::checkpoint::CheckpointStore;
use archon_session::plan::PlanStore;
use archon_tools::tool::{AgentMode, ToolContext, ToolResult};
use tokio::sync::Mutex;

use crate::ChannelMetricSink;
use crate::agents::AgentRegistry;
use crate::auto_extraction::AutoExtractor;
use crate::dispatch::ToolRegistry;
use crate::subagent::SubagentManager;

pub mod autocompact;
mod compaction;
mod compaction_serde;
mod events;
mod lifecycle;
mod memory_integration;
mod message_delivery;
mod payloads;
mod permission_gate;
mod runtime_hooks;
mod summary_text;
mod support;
#[cfg(test)]
mod tests;
mod tool_context;
mod tool_dispatch;
mod tool_postprocess;
mod tool_preflight;
mod tool_types;
mod turn_completion;
mod types;

pub use autocompact::{AutoCompactState, CompactAction, evaluate_compaction};
pub use compaction::ManualCompactOutcome;
pub use payloads::{
    ReasoningEvidenceEventPayload, ReasoningTurnEventPayload, UserCorrectionEventPayload,
};
pub use support::AgentLoopError;
use support::{parse_plan_from_text, user_correction_excerpt};
pub use types::{AgentConfig, AgentEvent, ConversationState, SessionStats, TimestampedEvent};

/// Single source of truth gate: does the agent loop auto-allow this tool in
/// default mode? Must always agree with `archon_permissions::DEFAULT_SAFE_TOOLS`.
/// Called by the lockstep regression test.
pub fn is_safe_in_default_mode(name: &str) -> bool {
    is_default_safe_tool(name)
}

#[derive(Debug)]
pub(super) struct PendingToolCall {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) input_json: String,
}

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
    record_memory_callback: Option<Arc<dyn Fn(u64) + Send + Sync>>,
    record_correction_callback: Option<Arc<dyn Fn() + Send + Sync>>,
    record_user_correction_event_callback:
        Option<Arc<dyn Fn(UserCorrectionEventPayload) + Send + Sync>>,
    record_reasoning_turn_callback: Option<Arc<dyn Fn(ReasoningTurnEventPayload) + Send + Sync>>,
    reasoning_evidence_refs: Vec<ReasoningEvidenceEventPayload>,
    #[allow(clippy::type_complexity)]
    inner_voice_change_callback: Option<Arc<dyn Fn(&InnerVoice) + Send + Sync>>,
}

impl Agent {
    async fn maybe_request_pressure_compact(
        &mut self,
        active_model: &str,
        trigger_tokens: u64,
        trigger_body_bytes: usize,
        context_window: u64,
    ) -> Result<bool, AgentLoopError> {
        let token_pressure = self
            .config
            .context
            .rate_limit_pressure_tokens
            .is_some_and(|threshold| trigger_tokens >= threshold);
        let body_pressure = self
            .config
            .context
            .rate_limit_pressure_body_bytes
            .is_some_and(|threshold| trigger_body_bytes as u64 >= threshold);
        if (!token_pressure && !body_pressure) || !self.state.auto_compact.should_attempt() {
            return Ok(false);
        }

        let reason = match (token_pressure, body_pressure) {
            (true, true) => "request_pressure_tokens_and_bytes",
            (true, false) => "request_pressure_tokens",
            (false, true) => "request_pressure_bytes",
            (false, false) => unreachable!(),
        };
        let telemetry = self.compaction_telemetry_for(active_model);
        tracing::info!(
            compaction.reason = reason,
            trigger_tokens,
            trigger_body_bytes,
            context_window,
            provider_family = telemetry.provider_family,
            wire_shape = telemetry.wire_shape,
            native_context_window = telemetry.native_context_window,
            runtime_context_budget = telemetry.runtime_context_budget,
            context_source = telemetry.context_source,
            compaction_backend = telemetry.compaction_backend,
            scope = "main_session",
            force = false,
            consecutive_failures = self.state.auto_compact.consecutive_failures,
            "request pressure threshold reached; attempting proactive compaction"
        );

        let before = self.state.messages.clone();
        self.state.auto_compact.compact_in_flight = true;
        let result = autocompact::compact_json_messages_with_provider(
            self.client.as_ref(),
            active_model,
            &self.state.messages,
            CompactAction::Full,
            false,
        )
        .await;

        match result {
            Ok((
                autocompact::CompactionOutcome::Compacted {
                    after_estimated_tokens,
                    ..
                },
                compacted,
            )) => {
                self.state.messages = compacted;
                self.state.last_known_context_tokens = 0;
                self.memory_injector.invalidate_cache();
                self.state.auto_compact.on_success(after_estimated_tokens);
                self.send_event(AgentEvent::CompactionTriggered).await;
                Ok(self.state.messages != before)
            }
            Ok((autocompact::CompactionOutcome::Skipped { .. }, _)) => {
                self.state.auto_compact.on_cancel();
                Ok(false)
            }
            Err(autocompact::CompactionError::Cancelled) => {
                self.state.auto_compact.on_cancel();
                Ok(false)
            }
            Err(err) => {
                self.state.auto_compact.on_real_failure();
                tracing::warn!(
                    compaction.reason = reason,
                    trigger_tokens,
                    trigger_body_bytes,
                    context_window,
                    provider_family = telemetry.provider_family,
                    wire_shape = telemetry.wire_shape,
                    native_context_window = telemetry.native_context_window,
                    runtime_context_budget = telemetry.runtime_context_budget,
                    context_source = telemetry.context_source,
                    compaction_backend = telemetry.compaction_backend,
                    scope = "main_session",
                    force = false,
                    consecutive_failures = self.state.auto_compact.consecutive_failures,
                    breaker_tripped = self.state.auto_compact.disabled,
                    error = %err,
                    "request-pressure compaction failed; continuing turn"
                );
                Ok(false)
            }
        }
    }

    /// Process a single user message through the full agent loop.
    /// Returns when the LLM produces a final text response (no more tool calls).
    pub async fn process_message(&mut self, user_input: &str) -> Result<(), AgentLoopError> {
        self.turn_number += 1;
        self.fire_before_agent_run_hook(user_input).await;
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
        let mut reactive_overflow_retried = false;
        let mut reactive_rate_limit_retried = false;
        let mut proactive_pressure_attempted = false;
        'agent_loop: loop {
            self.fire_before_prompt_build_hook(agentic_iterations).await;
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

            self.maybe_auto_compact(&active_model).await?;

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
                extra: self.config.runtime_context_extra(),
                request_origin: Some("main_session".into()),
                reasoning_encrypted: None,
            };
            self.fire_after_prompt_build_hook(&request, agentic_iterations)
                .await;
            let request_body_bytes = autocompact::request_body_bytes(&request);
            let large_retry_body_bytes =
                autocompact::large_request_retry_body_bytes(&self.config.context);
            let trigger_tokens = if self.state.last_known_context_tokens > 0 {
                self.state.last_known_context_tokens
            } else {
                autocompact::trigger_tokens(&self.state.messages)
            };
            let context_window = self.context_window_for(&active_model);
            if !proactive_pressure_attempted
                && self
                    .maybe_request_pressure_compact(
                        &active_model,
                        trigger_tokens,
                        request_body_bytes,
                        context_window,
                    )
                    .await?
            {
                proactive_pressure_attempted = true;
                continue 'agent_loop;
            }

            self.send_event(AgentEvent::ApiCallStarted {
                model: active_model.clone(),
            })
            .await;

            // Send request and get streaming events
            let mut rx = match self.client.stream(request.clone()).await {
                Ok(rx) => rx,
                Err(e) if e.is_context_window_exceeded() => {
                    reactive_overflow_retried = true;
                    self.force_reactive_compact().await?;
                    let retry_request = LlmRequest {
                        messages: self.state.messages.clone(),
                        ..request
                    };
                    self.client.stream(retry_request).await.map_err(|retry| {
                        AgentLoopError::ApiError(format!(
                            "reactive compaction retry failed: {retry}"
                        ))
                    })?
                }
                Err(e)
                    if autocompact::is_rate_limited_error(&e)
                        && !reactive_rate_limit_retried
                        && request_body_bytes >= large_retry_body_bytes =>
                {
                    reactive_rate_limit_retried = true;
                    let telemetry = self.compaction_telemetry_for(&active_model);
                    tracing::warn!(
                        compaction.reason = "rate_limit_large_request",
                        trigger_body_bytes = request_body_bytes,
                        threshold_body_bytes = large_retry_body_bytes,
                        provider_family = telemetry.provider_family,
                        wire_shape = telemetry.wire_shape,
                        native_context_window = telemetry.native_context_window,
                        runtime_context_budget = telemetry.runtime_context_budget,
                        context_source = telemetry.context_source,
                        compaction_backend = telemetry.compaction_backend,
                        scope = "main_session",
                        force = true,
                        "rate-limited main request is large; compacting before one retry"
                    );
                    self.force_reactive_compact().await?;
                    let retry_request = LlmRequest {
                        messages: self.state.messages.clone(),
                        ..request
                    };
                    self.client.stream(retry_request).await.map_err(|retry| {
                        AgentLoopError::ApiError(format!(
                            "rate-limit compaction retry failed: {retry}"
                        ))
                    })?
                }
                Err(e) => {
                    self.emit_activity(
                        AgentActivityKind::ParentTurnCompleted,
                        AgentActivityStatus::Failed,
                        format!("turn {} failed: {e}", self.turn_number),
                    );
                    self.fire_after_agent_run_hook("failed", Some(e.to_string()))
                        .await;
                    return Err(AgentLoopError::ApiError(format!("{e}")));
                }
            };

            let mut text_content = String::new();
            let mut thinking_content = String::new();
            let mut thinking_signature = String::new();
            let mut pending_tools: Vec<PendingToolCall> = Vec::new();
            let mut _current_tool_index: Option<u32> = None;
            let mut _stop_reason: Option<String> = None;
            let mut usage_acc = archon_llm::usage::UsageAccumulator::default();

            while let Some(event) = rx.recv().await {
                usage_acc.record_event(&event);
                match event {
                    StreamEvent::MessageStart { .. } => {}

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
                        let _ = usage;
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
                        let classified = autocompact::classify_stream_error(
                            self.client.name(),
                            &error_type,
                            &message,
                        );
                        if classified.is_context_window_exceeded() && !reactive_overflow_retried {
                            reactive_overflow_retried = true;
                            self.force_reactive_compact().await?;
                            continue 'agent_loop;
                        }
                        if autocompact::is_rate_limited_error(&classified)
                            && !reactive_rate_limit_retried
                            && request_body_bytes >= large_retry_body_bytes
                        {
                            reactive_rate_limit_retried = true;
                            let telemetry = self.compaction_telemetry_for(&active_model);
                            tracing::warn!(
                                compaction.reason = "rate_limit_large_request_stream",
                                trigger_body_bytes = request_body_bytes,
                                threshold_body_bytes = large_retry_body_bytes,
                                provider_family = telemetry.provider_family,
                                wire_shape = telemetry.wire_shape,
                                native_context_window = telemetry.native_context_window,
                                runtime_context_budget = telemetry.runtime_context_budget,
                                context_source = telemetry.context_source,
                                compaction_backend = telemetry.compaction_backend,
                                scope = "main_session",
                                force = true,
                                "rate-limited stream error on large request; compacting before one retry"
                            );
                            self.force_reactive_compact().await?;
                            continue 'agent_loop;
                        }
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
                        self.fire_after_agent_run_hook(
                            "failed",
                            Some(format!("{error_type}: {message}")),
                        )
                        .await;
                        return Err(AgentLoopError::ApiError(format!("{error_type}: {message}")));
                    }
                }
            }
            reactive_overflow_retried = false;
            reactive_rate_limit_retried = false;

            let turn_input_tokens = usage_acc.billable_input_tokens;
            let turn_cache_creation = usage_acc.cache_creation_input_tokens;
            let turn_cache_read = usage_acc.cache_read_input_tokens;
            let turn_output_tokens = usage_acc.output_tokens;
            // Billing-only accumulator. last_known_context_tokens carries the authoritative trigger value.
            self.state.total_input_tokens += usage_acc.context_input_tokens;
            self.state.last_known_context_tokens = usage_acc.context_input_tokens;
            self.state.total_output_tokens += turn_output_tokens;

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
            self.emit_reasoning_turn(&text_content);

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
                let ctx = self.build_tool_context(effective_mode, &active_model).await;

                let allowed = self.preflight_tools(&pending_tools, effective_mode).await;

                let dispatch_results = self.dispatch_allowed_tools(&allowed, &ctx).await;

                let prevent_continuation_reason = self
                    .postprocess_tools(&allowed, dispatch_results, &ctx, &active_model)
                    .await;

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

            self.complete_turn_without_tools(
                user_input,
                turn_input_tokens,
                turn_output_tokens,
                turn_cache_creation,
                turn_cache_read,
            )
            .await;

            break;
        }

        self.emit_activity(
            AgentActivityKind::ParentTurnCompleted,
            AgentActivityStatus::Completed,
            format!("turn {} completed", self.turn_number),
        );
        self.fire_after_agent_run_hook("completed", None).await;
        Ok(())
    }
}
