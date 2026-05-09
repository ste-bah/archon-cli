//! Request-time Codex runtime routing for auto mode.

use std::sync::Arc;

use archon_core::config::ArchonConfig;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc::Receiver};

pub(crate) struct CodexAutoProvider {
    app_server: Arc<dyn LlmProvider>,
    direct: Mutex<Option<Arc<dyn LlmProvider>>>,
    config: ArchonConfig,
    surface: String,
    emit_events: bool,
}

impl CodexAutoProvider {
    pub(crate) fn new(
        app_server: Arc<dyn LlmProvider>,
        config: ArchonConfig,
        surface: &str,
    ) -> Self {
        Self {
            app_server,
            direct: Mutex::new(None),
            config,
            surface: surface.to_string(),
            emit_events: true,
        }
    }

    async fn provider_for(&self, request: &LlmRequest) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if request.tools.is_empty() {
            return Ok(Arc::clone(&self.app_server));
        }
        let provider = self.direct_provider().await?;
        self.record_tool_fallback_selected(request.tools.len());
        Ok(provider)
    }

    async fn direct_provider(&self) -> Result<Arc<dyn LlmProvider>, LlmError> {
        let mut direct = self.direct.lock().await;
        if let Some(provider) = direct.as_ref() {
            return Ok(Arc::clone(provider));
        }
        let provider =
            super::codex_provider::build_direct_codex_provider(&self.config, &self.surface)
                .await
                .map_err(|error| {
                    self.record_tool_fallback_denied("codex_direct_fallback_provider_unavailable");
                    LlmError::Auth(format!("direct Codex fallback unavailable: {error:#}"))
                })?;
        *direct = Some(Arc::clone(&provider));
        Ok(provider)
    }

    fn record_tool_fallback_selected(&self, tool_count: usize) {
        if !self.emit_events {
            return;
        }
        super::provider_fallback_events::record_provider_fallback_selected(
            "openai-codex",
            "app_server",
            "direct",
            "codex_tool_use_requires_archon_direct_runtime",
            fallback_metadata(&self.surface, tool_count),
        );
    }

    fn record_tool_fallback_denied(&self, reason_code: &'static str) {
        if !self.emit_events {
            return;
        }
        super::provider_fallback_events::record_provider_fallback_denied(
            "openai-codex",
            "app_server",
            "direct",
            reason_code,
            fallback_metadata(&self.surface, 0),
        );
    }

    #[cfg(test)]
    fn with_direct(
        app_server: Arc<dyn LlmProvider>,
        direct: Arc<dyn LlmProvider>,
        mut config: ArchonConfig,
    ) -> Self {
        config.providers.openai_codex.runtime = "auto".into();
        config.providers.openai_codex.direct_fallback = true;
        Self {
            app_server,
            direct: Mutex::new(Some(direct)),
            config,
            surface: "test".into(),
            emit_events: false,
        }
    }
}

#[async_trait]
impl LlmProvider for CodexAutoProvider {
    fn name(&self) -> &str {
        "openai-codex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.app_server.models()
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let provider = self.provider_for(&request).await?;
        provider.stream(request).await
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let provider = self.provider_for(&request).await?;
        provider.complete(request).await
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        if matches!(feature, ProviderFeature::ToolUse) {
            return true;
        }
        self.app_server.supports_feature(feature)
    }
}

fn fallback_metadata(surface: &str, tool_count: usize) -> serde_json::Value {
    serde_json::json!({
        "surface": surface,
        "tool_count": tool_count,
        "tool_route": "archon_direct_runtime",
        "reason": "Codex app-server dynamic tools require provider-side callbacks; Archon routes tool turns through its existing governed tool loop instead",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use archon_llm::types::Usage;

    struct FakeProvider {
        label: &'static str,
        calls: AtomicUsize,
    }

    impl FakeProvider {
        fn new(label: &'static str) -> Self {
            Self {
                label,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            "openai-codex"
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo {
                id: self.label.into(),
                display_name: self.label.into(),
                context_window: 1,
            }]
        }

        async fn stream(&self, _request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            tx.send(StreamEvent::MessageStart {
                id: self.label.into(),
                model: self.label.into(),
                usage: Usage::default(),
            })
            .await
            .ok();
            Ok(rx)
        }

        async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(LlmResponse {
                content: vec![serde_json::json!({"type": "text", "text": self.label})],
                usage: Usage::default(),
                stop_reason: "end_turn".into(),
            })
        }

        fn supports_feature(&self, _feature: ProviderFeature) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn no_tool_request_uses_app_server() {
        let app_server = Arc::new(FakeProvider::new("app-server"));
        let direct = Arc::new(FakeProvider::new("direct"));
        let provider = CodexAutoProvider::with_direct(
            app_server.clone(),
            direct.clone(),
            ArchonConfig::default(),
        );

        provider.stream(LlmRequest::default()).await.unwrap();

        assert_eq!(app_server.calls(), 1);
        assert_eq!(direct.calls(), 0);
    }

    #[tokio::test]
    async fn tool_request_uses_direct_runtime() {
        let app_server = Arc::new(FakeProvider::new("app-server"));
        let direct = Arc::new(FakeProvider::new("direct"));
        let provider = CodexAutoProvider::with_direct(
            app_server.clone(),
            direct.clone(),
            ArchonConfig::default(),
        );
        let request = LlmRequest {
            tools: vec![serde_json::json!({"name": "Read"})],
            ..LlmRequest::default()
        };

        provider.stream(request).await.unwrap();

        assert_eq!(app_server.calls(), 0);
        assert_eq!(direct.calls(), 1);
    }
}
