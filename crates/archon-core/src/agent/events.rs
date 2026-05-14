use super::*;
use sha2::{Digest, Sha256};

#[derive(Debug)]
struct ProviderModelActivitySink {
    inner: Arc<dyn AgentActivitySink>,
    provider: String,
    model: String,
}

impl AgentActivitySink for ProviderModelActivitySink {
    fn emit(&self, mut event: AgentActivityEvent) {
        if event.provider.is_none() {
            event.provider = Some(self.provider.clone());
        }
        if event.model.is_none() {
            event.model = Some(self.model.clone());
        }
        self.inner.emit(event);
    }
}

pub(super) fn emit_tool_result_activity(ctx: &ToolContext, tool_name: &str, result: &ToolResult) {
    if result.is_error {
        crate::dispatch::emit_tool_activity(
            ctx,
            tool_name,
            AgentActivityKind::ToolFailed,
            AgentActivityStatus::Failed,
        );
    } else {
        crate::dispatch::emit_tool_activity(
            ctx,
            tool_name,
            AgentActivityKind::ToolCompleted,
            AgentActivityStatus::Completed,
        );
    }
}

impl Agent {
    pub(super) fn emit_activity(
        &self,
        kind: AgentActivityKind,
        status: AgentActivityStatus,
        message: impl Into<String>,
    ) {
        if let Some(sink) = &self.config.activity_sink {
            sink.emit(
                AgentActivityEvent::new(self.config.session_id.clone(), kind, status, message)
                    .with_provider_model(self.client.name().to_string(), self.config.model.clone()),
            );
        }
    }

    pub(super) fn provider_model_activity_sink(
        &self,
        model: &str,
    ) -> Option<Arc<dyn AgentActivitySink>> {
        self.config.activity_sink.as_ref().map(|sink| {
            Arc::new(ProviderModelActivitySink {
                inner: Arc::clone(sink),
                provider: self.client.name().to_string(),
                model: model.to_string(),
            }) as Arc<dyn AgentActivitySink>
        })
    }

    pub(super) fn emit_reasoning_turn(&self, assistant_text: &str) {
        if assistant_text.trim().is_empty() {
            return;
        }
        if let Some(ref cb) = self.record_reasoning_turn_callback {
            cb(ReasoningTurnEventPayload {
                session_id: self.config.session_id.clone(),
                turn_number: self.turn_number,
                assistant_text: assistant_text.to_string(),
                evidence_refs: self.reasoning_evidence_refs.clone(),
                cwd: std::env::current_dir()
                    .ok()
                    .map(|path| path.display().to_string()),
                workspace_root: Some(self.config.working_dir.display().to_string()),
            });
        }
    }

    pub(super) fn record_reasoning_tool_evidence(
        &mut self,
        tool_name: &str,
        tool_id: &str,
        input: &serde_json::Value,
        result: &ToolResult,
        file_path: Option<&str>,
    ) {
        if result.is_error {
            return;
        }
        let kind = classify_tool_evidence_kind(tool_name, input);
        let entity_key = file_path
            .map(ToOwned::to_owned)
            .or_else(|| input_entity_key(tool_name, input));
        let excerpt: String = result.content.chars().take(600).collect();
        let output_hash = Some(hash_text(&result.content));
        self.reasoning_evidence_refs
            .push(ReasoningEvidenceEventPayload {
                evidence_id: format!("{}:{}", tool_name, tool_id),
                kind,
                entity_key,
                output_hash,
                redacted_excerpt: Some(excerpt),
                created_at: chrono::Utc::now().to_rfc3339(),
            });
    }

    pub(super) async fn send_event(&self, event: AgentEvent) {
        // TASK-AGS-102: unbounded send — synchronous, fails only if rx dropped.
        // TASK-AGS-108 ERR-ARCH-02: WARN on closed channel, continue execution.
        let event_name = event.event_name();
        let timestamped = TimestampedEvent {
            sent_at: std::time::Instant::now(),
            inner: event,
        };
        if self.event_tx.send(timestamped).is_err() {
            tracing::warn!(
                event_id = event_name,
                "Agent event channel closed: dropping event"
            );
        }
        if let Some(m) = &self.metrics {
            m.record_sent();
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
}

fn classify_tool_evidence_kind(tool_name: &str, input: &serde_json::Value) -> String {
    let lower = tool_name.to_lowercase();
    let command = input
        .get("command")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_lowercase();
    if lower.contains("read") || lower.contains("view") || lower.contains("open") {
        "file_read".to_string()
    } else if lower.contains("grep") || lower.contains("search") || lower.contains("glob") {
        "search".to_string()
    } else if command.contains("cargo test")
        || command.contains("cargo build")
        || command.contains("npm test")
        || command.contains("pytest")
    {
        "test_output".to_string()
    } else if command.trim_start().starts_with("git ") {
        "git".to_string()
    } else if lower.contains("pipeline") || lower.contains("artifact") {
        "pipeline_artifact".to_string()
    } else if lower.contains("mcp") {
        "mcp_result".to_string()
    } else {
        "plugin_result".to_string()
    }
}

fn input_entity_key(tool_name: &str, input: &serde_json::Value) -> Option<String> {
    for key in ["path", "file_path", "filename", "query", "pattern"] {
        if let Some(value) = input.get(key).and_then(|value| value.as_str())
            && !value.trim().is_empty()
        {
            return Some(value.to_string());
        }
    }
    input
        .get("command")
        .and_then(|value| value.as_str())
        .map(|command| {
            if command.contains("cargo test") {
                "test-status".to_string()
            } else {
                format!(
                    "{}:{}",
                    tool_name,
                    command.chars().take(80).collect::<String>()
                )
            }
        })
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_model_sink_preserves_subagent_metadata() {
        let inner = Arc::new(archon_observability::InMemoryActivitySink::new());
        let trait_inner: Arc<dyn AgentActivitySink> = inner.clone();
        let sink = ProviderModelActivitySink {
            inner: trait_inner,
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
        };

        sink.emit(
            AgentActivityEvent::new(
                "session-1",
                AgentActivityKind::AgentRunning,
                AgentActivityStatus::Running,
                "subagent activity",
            )
            .with_provider_model("anthropic", "opus"),
        );

        let events = inner.events();
        assert_eq!(events[0].provider.as_deref(), Some("anthropic"));
        assert_eq!(events[0].model.as_deref(), Some("opus"));
    }

    #[test]
    fn provider_model_sink_fills_missing_metadata() {
        let inner = Arc::new(archon_observability::InMemoryActivitySink::new());
        let trait_inner: Arc<dyn AgentActivitySink> = inner.clone();
        let sink = ProviderModelActivitySink {
            inner: trait_inner,
            provider: "anthropic".into(),
            model: "claude-sonnet-4-6".into(),
        };

        sink.emit(AgentActivityEvent::new(
            "session-1",
            AgentActivityKind::ToolCompleted,
            AgentActivityStatus::Completed,
            "tool done",
        ));

        let events = inner.events();
        assert_eq!(events[0].provider.as_deref(), Some("anthropic"));
        assert_eq!(events[0].model.as_deref(), Some("claude-sonnet-4-6"));
    }
}
