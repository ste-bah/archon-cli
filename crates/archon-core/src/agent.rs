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
use archon_tools::tool::{
    AgentMode, ToolContext, ToolResult, ToolRunAdmissionCallback, ToolRunOutcomeCallback,
};
use tokio::sync::Mutex;

use crate::ChannelMetricSink;
use crate::agents::AgentRegistry;
use crate::auto_extraction::AutoExtractor;
use crate::dispatch::ToolRegistry;
use crate::subagent::SubagentManager;

pub mod autocompact;
mod cognitive_gate;
#[cfg(test)]
mod cognitive_gate_tests;
mod compaction;
mod compaction_serde;
pub(crate) mod events;
mod lifecycle;
#[cfg(test)]
mod memory_attribution_tests;
mod memory_integration;
mod message_delivery;
mod payloads;
mod permission_gate;
mod process_message;
mod process_message_recovery;
mod process_message_steps;
mod process_message_support;
pub(crate) mod request_cache;
mod runtime_attribution;
mod runtime_hooks;
mod segment_compaction_runtime;
mod summary_text;
mod support;
#[cfg(test)]
mod tests;
mod tool_context;
mod tool_dispatch;
pub(crate) mod tool_input_json;
mod tool_postprocess;
mod tool_postprocess_steps;
#[cfg(test)]
mod tool_postprocess_steps_tests;
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
pub use runtime_attribution::RuntimeAttribution;
pub use support::AgentLoopError;
use support::{message_text_content, parse_plan_from_text, user_correction_excerpt};
pub use types::{AgentConfig, AgentEvent, ConversationState, SessionStats, TimestampedEvent};

pub const AGENT_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Single source of truth gate: does the agent loop auto-allow this tool in
/// default mode? Must always agree with `archon_permissions::DEFAULT_SAFE_TOOLS`.
/// Called by the lockstep regression test.
pub fn is_safe_in_default_mode(name: &str) -> bool {
    is_default_safe_tool(name)
}

pub type FirstToolActionCallback =
    Arc<dyn Fn(&str, &str, &str, &serde_json::Value) -> Option<String> + Send + Sync>;
pub type TurnFinalizationCallback =
    Arc<dyn Fn(&str, &str) -> TurnFinalizationVerdict + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnFinalizationVerdict {
    Allowed,
    Blocked { repair_prompt: String },
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
    event_tx: tokio::sync::mpsc::Sender<TimestampedEvent>,
    checkpoint_store: Option<Arc<Mutex<CheckpointStore>>>,
    plan_store: Option<PlanStore>,
    turn_number: u64,
    // GAP 5/7: Memory graph + injector for per-turn injection and auto-extraction
    memory: Option<Arc<dyn MemoryTrait>>,
    /// Shared so the per-turn recall can run on the blocking pool without
    /// taking ownership of it. A `spawn_blocking` task cannot be cancelled, so
    /// anything moved into one is unrecoverable if the caller stops waiting —
    /// a handle keeps the injector, and its cache, owned by the agent.
    memory_injector: Arc<std::sync::Mutex<MemoryInjector>>,
    extraction_config: ExtractionConfig,
    extraction_state: ExtractionState,
    // v0.1.23: AutoExtraction (LLM-based) learning system.
    auto_extractor: Option<Arc<AutoExtractor>>,
    auto_extraction_tasks: Vec<tokio::task::JoinHandle<()>>,
    session_store: Option<Arc<archon_session::storage::SessionStore>>,
    compaction_summary_tasks: Vec<tokio::task::JoinHandle<()>>,
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
    first_tool_action_callback: Option<FirstToolActionCallback>,
    tool_run_admission_callback: Option<ToolRunAdmissionCallback>,
    tool_run_outcome_callback: Option<ToolRunOutcomeCallback>,
    turn_finalization_callback: Option<TurnFinalizationCallback>,
    guardrail_action_id: Option<String>,
    turn_requirement_reminder: Option<String>,
    reasoning_evidence_refs: Vec<ReasoningEvidenceEventPayload>,
    current_situation: Option<archon_cognitive::Situation>,
    cognitive_store: Option<Arc<std::sync::Mutex<archon_cognitive::PersistentCognitiveStore>>>,
    cognitive_config: Option<archon_cognitive::CognitiveConfig>,
    cognitive_policy: Option<archon_cognitive::CognitivePolicy>,
    cognitive_ledger_dir: Option<std::path::PathBuf>,
    cognitive_executive_reminder: Option<String>,
    /// World-model prediction backend, injected by the binary crate (the only
    /// one that can see both `archon-cognitive` and `archon-world-model`).
    /// `None` means advisories fall back to heuristic scoring.
    cognitive_prediction_backend: Option<archon_cognitive::SharedPredictionBackend>,
    /// Which world model is live. Defaults to "no model", and should be left
    /// `shadow_only` until an eval report says the candidate beats the
    /// nearest-neighbour baseline — the scorer ignores predictions in that
    /// state, so shadow wiring cannot change any decision.
    cognitive_world_model_state: archon_cognitive::WorldModelState,
    #[allow(clippy::type_complexity)]
    inner_voice_change_callback: Option<Arc<dyn Fn(&InnerVoice) + Send + Sync>>,
}
