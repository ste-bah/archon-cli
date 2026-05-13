use super::*;

const MICRO_COMPACT_FRACTION: f32 = 0.65;
const MAX_COMPACT_FAILURES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactAction {
    Micro,
    Full,
}

#[derive(Debug, Clone, Default)]
pub struct AutoCompactState {
    pub compaction_count: u32,
    pub consecutive_failures: u32,
    pub disabled: bool,
    pub compact_in_flight: bool,
    pub last_compact_at_tokens: u64,
}

impl AutoCompactState {
    pub fn should_attempt(&self) -> bool {
        !self.disabled && !self.compact_in_flight
    }

    pub fn on_success(&mut self, tokens: u64) {
        self.compaction_count += 1;
        self.consecutive_failures = 0;
        self.compact_in_flight = false;
        self.last_compact_at_tokens = tokens;
    }

    pub fn on_real_failure(&mut self) {
        self.consecutive_failures += 1;
        self.compact_in_flight = false;
        if self.consecutive_failures >= MAX_COMPACT_FAILURES {
            self.disabled = true;
        }
    }

    pub fn on_cancel(&mut self) {
        self.compact_in_flight = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionOutcome {
    Compacted {
        before_tokens: u64,
        after_tokens: u64,
        messages_before: usize,
        messages_after: usize,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum CompactionError {
    #[error("no safe compaction boundary")]
    NoSafeBoundary,
}

pub fn evaluate_compaction(
    tokens_used: u64,
    context_window: u64,
    state: &AutoCompactState,
    threshold: f32,
) -> Option<CompactAction> {
    if context_window == 0 || !state.should_attempt() {
        return None;
    }
    let fraction = tokens_used as f32 / context_window as f32;
    if fraction >= threshold {
        Some(CompactAction::Full)
    } else if fraction >= MICRO_COMPACT_FRACTION {
        Some(CompactAction::Micro)
    } else {
        None
    }
}

pub fn estimate_message_tokens(message: &serde_json::Value) -> u64 {
    (message.to_string().len() as f64 / 4.0).ceil() as u64
}

pub fn estimate_messages_tokens(messages: &[serde_json::Value]) -> u64 {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn classify_stream_error(
    provider: &str,
    error_type: &str,
    message: &str,
) -> archon_llm::provider::LlmError {
    archon_llm::context_window::classify_context_window_error(
        None,
        Some(error_type),
        None,
        message,
        Some(provider),
        None,
    )
    .unwrap_or_else(|| archon_llm::provider::LlmError::Http(format!("{error_type}: {message}")))
}

impl Agent {
    pub(super) fn context_window_for(&self, active_model: &str) -> u64 {
        archon_llm::context_window::resolve_context_window(
            active_model,
            self.config.context.max_tokens.map(u64::from),
            Some(self.client.as_ref()),
        )
        .context_window
    }

    pub(super) async fn maybe_auto_compact(
        &mut self,
        active_model: &str,
    ) -> Result<(), AgentLoopError> {
        let window = self.context_window_for(active_model);
        let tokens = self
            .state
            .total_input_tokens
            .max(estimate_messages_tokens(&self.state.messages));
        let Some(action) = evaluate_compaction(
            tokens,
            window,
            &self.state.auto_compact,
            self.config.context.compact_threshold,
        ) else {
            return Ok(());
        };
        self.run_auto_compaction(action, false).await
    }

    pub(super) async fn force_reactive_compact(&mut self) -> Result<(), AgentLoopError> {
        self.run_auto_compaction(CompactAction::Full, true).await
    }

    async fn run_auto_compaction(
        &mut self,
        action: CompactAction,
        force: bool,
    ) -> Result<(), AgentLoopError> {
        self.state.auto_compact.compact_in_flight = true;
        let result = compact_json_messages(&self.state.messages, action, force);
        match result {
            Ok(CompactionOutcome::Compacted { after_tokens, .. }) => {
                self.state.messages = compact_json_messages_apply(&self.state.messages, action)
                    .map_err(|err| {
                        AgentLoopError::ApiError(format!("auto-compaction failed: {err}"))
                    })?;
                self.state.total_input_tokens = after_tokens;
                self.memory_injector.invalidate_cache();
                self.state.auto_compact.on_success(after_tokens);
                self.send_event(AgentEvent::CompactionTriggered).await;
                Ok(())
            }
            Ok(CompactionOutcome::Skipped { .. }) => {
                self.state.auto_compact.on_cancel();
                Ok(())
            }
            Err(err) => {
                self.state.auto_compact.on_real_failure();
                Err(AgentLoopError::ApiError(format!(
                    "auto-compaction failed: {err}"
                )))
            }
        }
    }
}

pub fn compact_json_messages(
    messages: &[serde_json::Value],
    action: CompactAction,
    force: bool,
) -> Result<CompactionOutcome, CompactionError> {
    let compacted = compact_json_messages_apply(messages, action)?;
    let before = estimate_messages_tokens(messages);
    let after = estimate_messages_tokens(&compacted);
    if compacted.len() == messages.len() && !force {
        return Ok(CompactionOutcome::Skipped {
            reason: "no safe boundary".into(),
        });
    }
    Ok(CompactionOutcome::Compacted {
        before_tokens: before,
        after_tokens: after,
        messages_before: messages.len(),
        messages_after: compacted.len(),
    })
}

pub fn compact_json_messages_apply(
    messages: &[serde_json::Value],
    action: CompactAction,
) -> Result<Vec<serde_json::Value>, CompactionError> {
    let context_messages = to_context_messages(messages);
    if context_messages.len() < 5 {
        return Err(CompactionError::NoSafeBoundary);
    }
    let summary = synthetic_summary(&context_messages);
    let compacted = match action {
        CompactAction::Micro => {
            let (msgs, _) = archon_context::microcompact::microcompact_messages(
                &context_messages,
                &summary,
                archon_context::compact::DEFAULT_PRESERVE_RECENT_TURNS,
            );
            msgs
        }
        CompactAction::Full => {
            archon_context::compact::compact_messages_default(&context_messages, &summary)
        }
    };
    Ok(from_context_messages(&compacted))
}

fn to_context_messages(
    messages: &[serde_json::Value],
) -> Vec<archon_context::messages::ContextMessage> {
    messages
        .iter()
        .map(|m| archon_context::messages::ContextMessage {
            role: m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            content: m.get("content").cloned().unwrap_or(serde_json::Value::Null),
            estimated_tokens: estimate_message_tokens(m),
        })
        .collect()
}

fn from_context_messages(
    messages: &[archon_context::messages::ContextMessage],
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let role = if m.role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            serde_json::json!({ "role": role, "content": m.content })
        })
        .collect()
}

fn synthetic_summary(messages: &[archon_context::messages::ContextMessage]) -> String {
    let first_user = messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.to_string())
        .unwrap_or_default();
    format!(
        "Earlier context was auto-compacted. Preserved task seed: {}. Compacted {} earlier messages.",
        first_user.chars().take(500).collect::<String>(),
        messages.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_compaction_does_not_emit_system_role_or_orphan_tool_result() {
        let mut messages: Vec<serde_json::Value> = (0..4)
            .map(|i| serde_json::json!({"role": "user", "content": format!("old {i}")}))
            .collect();
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": [
                {"type": "text", "text": "calling"},
                {"type": "tool_use", "id": "tool-1", "name": "Bash", "input": {}}
            ]
        }));
        messages.push(serde_json::json!({
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}
            ]
        }));
        messages.extend(
            (0..5).map(|i| serde_json::json!({"role": "user", "content": format!("recent {i}")})),
        );

        let compacted = compact_json_messages_apply(&messages, CompactAction::Full).unwrap();
        assert!(compacted.iter().all(|m| m["role"] != "system"));
        assert_eq!(compacted[1]["content"][1]["id"], "tool-1");
        assert_eq!(compacted[2]["content"][0]["tool_use_id"], "tool-1");
    }
}
