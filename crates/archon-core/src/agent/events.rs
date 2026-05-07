use super::*;

#[derive(Debug)]
struct ProviderModelActivitySink {
    inner: Arc<dyn AgentActivitySink>,
    provider: String,
    model: String,
}

impl AgentActivitySink for ProviderModelActivitySink {
    fn emit(&self, event: AgentActivityEvent) {
        self.inner
            .emit(event.with_provider_model(self.provider.clone(), self.model.clone()));
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
