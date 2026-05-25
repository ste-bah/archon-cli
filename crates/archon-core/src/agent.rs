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
mod cognitive_gate;
mod compaction;
mod compaction_serde;
mod events;
mod lifecycle;
mod memory_integration;
mod message_delivery;
mod payloads;
mod permission_gate;
mod process_message;
mod process_message_steps;
mod process_message_support;
mod runtime_hooks;
mod summary_text;
mod support;
#[cfg(test)]
mod tests;
mod tool_context;
mod tool_dispatch;
pub(crate) mod tool_input_json;
mod tool_postprocess;
mod tool_postprocess_steps;
mod tool_preflight;
mod tool_preflight_gates;
mod tool_preflight_steps;
pub(crate) mod tool_result_context;
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
    auto_extraction_tasks: Vec<tokio::task::JoinHandle<()>>,
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
    current_situation: Option<archon_cognitive::Situation>,
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
}
