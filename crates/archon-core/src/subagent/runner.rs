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
}
