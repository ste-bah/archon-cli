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

mod runtime;
#[cfg(test)]
mod tests;

const ACTIVITY_STREAM_PREFIX: &str = "archon_activity_stream:";

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
    activity_actor_id: Option<String>,
    activity_actor_name: Option<String>,
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

#[derive(Debug, Default)]
struct PendingThinkingBlock {
    thinking: String,
    signature: String,
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
        let model = resolved_model(provider.as_ref(), &model);
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
            activity_actor_id: None,
            activity_actor_name: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            progress: None,
            agent_config,
            identity,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
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

    pub fn set_activity_actor(&mut self, actor_id: String, actor_name: String) {
        self.activity_actor_id = Some(actor_id);
        self.activity_actor_name = Some(actor_name);
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

    fn emit_activity_stream(
        &self,
        kind: &str,
        text: impl Into<String>,
        tool: Option<&str>,
        is_error: bool,
    ) {
        let Some(sink) = &self.tool_context.activity_sink else {
            return;
        };
        let Some(actor_id) = &self.activity_actor_id else {
            return;
        };
        let payload = serde_json::json!({
            "kind": kind,
            "text": text.into(),
            "tool": tool,
            "is_error": is_error,
        });
        let event = archon_observability::AgentActivityEvent::new(
            self.tool_context.session_id.clone(),
            archon_observability::AgentActivityKind::AgentRunning,
            archon_observability::AgentActivityStatus::Running,
            format!("{ACTIVITY_STREAM_PREFIX}{payload}"),
        )
        .with_subagent_id(actor_id.clone())
        .with_subagent_type(
            self.activity_actor_name
                .clone()
                .unwrap_or_else(|| "subagent".into()),
        )
        .with_provider_model(self.provider.name(), self.model.clone());
        sink.emit(event);
    }
}

fn resolved_model(provider: &dyn LlmProvider, model: &str) -> String {
    let mut request = LlmRequest {
        model: model.to_string(),
        ..LlmRequest::default()
    };
    provider.resolve_request_model(&mut request);
    request.model
}
