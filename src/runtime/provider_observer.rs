//! Observe real LLM provider traffic and persist redacted runtime events.

use std::sync::Arc;

use anyhow::Result;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};
use archon_llm::streaming::StreamEvent;
use async_trait::async_trait;
use cozo::DbInstance;
use tokio::sync::mpsc::Receiver;

use super::provider_event_record::provider_event_record;
use super::provider_limit_windows;

#[path = "provider_identity_events.rs"]
mod identity_events;
#[path = "provider_observer_stream.rs"]
mod stream;

#[derive(Clone)]
pub(crate) struct ProviderRuntimeEventRecorder {
    db: Option<Arc<DbInstance>>,
}

impl ProviderRuntimeEventRecorder {
    pub(crate) fn default_learning_store() -> Self {
        match open_learning_db() {
            Ok(db) => Self {
                db: Some(Arc::new(db)),
            },
            Err(error) => {
                tracing::warn!(%error, "provider runtime event store unavailable");
                Self { db: None }
            }
        }
    }

    #[cfg(test)]
    fn with_db(db: DbInstance) -> Self {
        Self {
            db: Some(Arc::new(db)),
        }
    }

    fn record(&self, event: ProviderRuntimeEvent) -> Option<String> {
        let event_id = event.event_id.clone();
        let Some(db) = &self.db else {
            return None;
        };
        let record = provider_event_record(event);
        if let Err(error) =
            archon_learning::runtime_events::insert_provider_runtime_event(db, &record)
        {
            tracing::warn!(
                %error,
                provider = %record.provider_id,
                event_type = %record.event_type,
                "provider runtime event persistence failed"
            );
            return None;
        }
        Some(event_id)
    }

    fn record_limit_window(&self, provider_id: &str, model_id: Option<&str>, error: &LlmError) {
        provider_limit_windows::record_limit_window(self.db.as_ref(), provider_id, model_id, error);
    }
}

pub(crate) fn observe_llm_provider_with_profile(
    provider: Arc<dyn LlmProvider>,
    runtime_mode: impl Into<String>,
    profile_id: Option<String>,
) -> Arc<dyn LlmProvider> {
    Arc::new(ObservedLlmProvider::new(
        provider,
        runtime_mode,
        profile_id,
        ProviderRuntimeEventRecorder::default_learning_store(),
    ))
}

pub(crate) fn runtime_mode_for_provider_name(provider_name: &str) -> &'static str {
    match provider_name {
        "openai-codex" => "auto",
        "local" => "local",
        _ => "direct",
    }
}

pub(crate) fn record_provider_fallback(
    requested_provider: &str,
    selected_provider: &str,
    runtime_mode: &str,
    reason_code: &str,
) {
    if requested_provider == selected_provider {
        return;
    }
    let event = base_event(
        selected_provider,
        runtime_mode,
        ProviderRuntimeEventType::FallbackSelected,
        ProviderRuntimeSeverity::Warn,
    )
    .with_reason(reason_code)
    .with_fallback(requested_provider, selected_provider)
    .with_redacted_json(serde_json::json!({
        "requested_provider": requested_provider,
        "selected_provider": selected_provider,
        "source": "provider_construction"
    }));
    ProviderRuntimeEventRecorder::default_learning_store().record(event);
}

pub(crate) struct ObservedLlmProvider {
    inner: Arc<dyn LlmProvider>,
    runtime_mode: String,
    profile_id: Option<String>,
    recorder: ProviderRuntimeEventRecorder,
}

impl ObservedLlmProvider {
    fn new(
        inner: Arc<dyn LlmProvider>,
        runtime_mode: impl Into<String>,
        profile_id: Option<String>,
        recorder: ProviderRuntimeEventRecorder,
    ) -> Self {
        let observed = Self {
            inner,
            runtime_mode: runtime_mode.into(),
            profile_id,
            recorder,
        };
        identity_events::record_provider_identity_decision(
            &observed.recorder,
            observed.inner.as_ref(),
            &observed.runtime_mode,
            observed.profile_id.as_deref(),
        );
        observed
    }

    fn event(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        event_type: ProviderRuntimeEventType,
        severity: ProviderRuntimeSeverity,
    ) -> ProviderRuntimeEvent {
        let event = base_event(self.inner.name(), &self.runtime_mode, event_type, severity)
            .with_request_id(request_id)
            .with_model(request.model.clone())
            .with_redacted_json(serde_json::json!({
                "request_origin": request.origin.as_deref(),
                "identity_status": identity_events::identity_status_label(
                    identity_events::identity_status_for_provider(self.inner.as_ref())
                ),
            }));
        if let Some(profile_id) = &self.profile_id {
            event.with_profile(profile_id.clone())
        } else {
            event
        }
    }

    fn record_start(&self, request_id: &str, request: &ObservedRequest, operation: &str) {
        self.recorder.record(
            self.event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestStarted,
                ProviderRuntimeSeverity::Debug,
            )
            .with_reason(operation),
        );
        crate::command::world_model::record_provider_runtime_advisory(
            request.run_id.as_deref().unwrap_or(self.inner.name()),
            request_id,
            &format!(
                "{} provider={} model={} origin={}",
                operation,
                self.inner.name(),
                request.model,
                request.origin.as_deref().unwrap_or("unknown")
            ),
        );
    }

    fn record_success(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        metadata: serde_json::Value,
    ) {
        self.recorder.record(
            self.event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestSucceeded,
                ProviderRuntimeSeverity::Info,
            )
            .with_reason("ok")
            .with_redacted_json(metadata),
        );
        super::provider_profile_updates::mark_success(
            self.recorder.db.as_ref(),
            self.inner.name(),
            &self.runtime_mode,
            self.profile_id.as_deref(),
            Some(&request.model),
            Some(request_id),
        );
    }

    fn record_failure(&self, request_id: &str, request: &ObservedRequest, error: &LlmError) {
        let error_kind = error_kind(error);
        let event = self
            .event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestFailed,
                error_severity(error),
            )
            .with_reason(error_kind)
            .with_message(error_message(error))
            .with_redacted_json(error_metadata(error));
        if let Some(event_id) = self.recorder.record(event) {
            self.record_agent_provider_incident(&event_id, request, error_kind);
            if let Some(run_id) = request.run_id.as_deref()
                && let Ok(config) = archon_core::config::load_config()
            {
                let attached =
                    crate::command::world_model::record_guardrail_provider_incident_for_session(
                        &config, run_id, &event_id, error_kind,
                    );
                if attached {
                    tracing::debug!(
                        run_id,
                        provider_event_id = %event_id,
                        reason_code = error_kind,
                        "world_model.guardrail_provider_incident"
                    );
                }
            }
        }

        if let Some(event_type) = limit_event_type(error) {
            self.recorder.record(
                self.event(
                    request_id,
                    request,
                    event_type,
                    ProviderRuntimeSeverity::Warn,
                )
                .with_reason(error_kind)
                .with_message(error_message(error))
                .with_redacted_json(error_metadata(error)),
            );
            self.recorder
                .record_limit_window(self.inner.name(), Some(&request.model), error);
        }
        super::provider_profile_updates::mark_failure(
            self.recorder.db.as_ref(),
            self.inner.name(),
            &self.runtime_mode,
            self.profile_id.as_deref(),
            Some(&request.model),
            Some(request_id),
            error,
        );
    }

    fn record_agent_provider_incident(
        &self,
        provider_event_id: &str,
        request: &ObservedRequest,
        reason_code: &str,
    ) {
        super::provider_incident_ledger::record_provider_incident(
            super::provider_incident_ledger::ProviderIncidentLedgerInput {
                db: self.recorder.db.as_ref(),
                agent_type: request.agent_type.as_deref(),
                agent_version: request.agent_version.as_deref(),
                run_id: request.run_id.as_deref(),
                model_id: &request.model,
                provider_id: self.inner.name(),
                provider_event_id,
                reason_code,
            },
        );
    }
}

#[async_trait]
impl LlmProvider for ObservedLlmProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let observed = ObservedRequest::from_request(&request);
        let request_id = uuid::Uuid::new_v4().to_string();
        self.record_start(&request_id, &observed, "stream");

        match self.inner.stream(request).await {
            Ok(inner_rx) => Ok(stream::forward_stream(
                inner_rx,
                self.recorder.clone(),
                self.inner.name().to_string(),
                self.runtime_mode.clone(),
                self.profile_id.clone(),
                observed,
                request_id,
            )),
            Err(error) => {
                self.record_failure(&request_id, &observed, &error);
                Err(error)
            }
        }
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let observed = ObservedRequest::from_request(&request);
        let request_id = uuid::Uuid::new_v4().to_string();
        self.record_start(&request_id, &observed, "complete");

        match self.inner.complete(request).await {
            Ok(response) => {
                self.record_success(
                    &request_id,
                    &observed,
                    serde_json::json!({
                        "request_origin": observed.origin.as_deref(),
                        "stop_reason": response.stop_reason.clone(),
                        "usage": {
                            "input_count": response.usage.input_tokens,
                            "output_count": response.usage.output_tokens,
                            "cache_creation_input_count": response.usage.cache_creation_input_tokens,
                            "cache_read_input_count": response.usage.cache_read_input_tokens,
                        }
                    }),
                );
                Ok(response)
            }
            Err(error) => {
                self.record_failure(&request_id, &observed, &error);
                Err(error)
            }
        }
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        self.inner.supports_feature(feature)
    }

    fn as_anthropic(&self) -> Option<&archon_llm::anthropic::AnthropicClient> {
        self.inner.as_anthropic()
    }
}

#[derive(Clone)]
pub(super) struct ObservedRequest {
    model: String,
    origin: Option<String>,
    run_id: Option<String>,
    agent_type: Option<String>,
    agent_version: Option<String>,
}

impl ObservedRequest {
    fn from_request(request: &LlmRequest) -> Self {
        let runtime = request.extra.get("archon_runtime");
        Self {
            model: request.model.clone(),
            origin: request.request_origin.clone(),
            run_id: runtime_field(runtime, "run_id"),
            agent_type: runtime_field(runtime, "agent_type"),
            agent_version: runtime_field(runtime, "agent_version"),
        }
    }
}

fn runtime_field(runtime: Option<&serde_json::Value>, field: &str) -> Option<String> {
    runtime?
        .get(field)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn open_learning_db() -> Result<DbInstance> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    let path = parent.join("learning.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_str = path.to_string_lossy().to_string();
    let db = DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("open learning db: {e}"))?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn base_event(
    provider_id: &str,
    runtime_mode: &str,
    event_type: ProviderRuntimeEventType,
    severity: ProviderRuntimeSeverity,
) -> ProviderRuntimeEvent {
    ProviderRuntimeEvent::new(provider_id, runtime_mode, event_type, severity)
}

fn error_kind(error: &LlmError) -> &'static str {
    match error {
        LlmError::Http(_) => "http_error",
        LlmError::Auth(_) => "auth_error",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded => "overloaded",
        LlmError::Server { .. } => "server_error",
        LlmError::Serialize(_) => "serialization_error",
        LlmError::Unsupported(_) => "unsupported_feature",
        LlmError::ProviderNotFound { .. } => "provider_not_found",
        LlmError::QuotaExceeded(_) => "quota_exceeded",
        LlmError::Aborted => "aborted",
        LlmError::ContextWindowExceeded { .. } => "context_window_exceeded",
        _ => "unknown_error",
    }
}

fn error_message(error: &LlmError) -> &'static str {
    match error {
        LlmError::RateLimited { .. } => "provider reported a rate limit",
        LlmError::QuotaExceeded(_) => "provider reported a usage or quota limit",
        LlmError::Auth(_) => "provider authentication failed",
        LlmError::Server { .. } => "provider returned a server error",
        LlmError::ProviderNotFound { .. } => "provider was not found",
        LlmError::Unsupported(_) => "provider does not support the requested feature",
        LlmError::Aborted => "provider request was aborted",
        LlmError::Http(_) => "provider HTTP request failed",
        LlmError::Overloaded => "provider reported overload",
        LlmError::Serialize(_) => "provider request or response serialization failed",
        LlmError::ContextWindowExceeded { .. } => "provider context window was exceeded",
        _ => "provider request failed",
    }
}

fn error_severity(error: &LlmError) -> ProviderRuntimeSeverity {
    match error {
        LlmError::RateLimited { .. } | LlmError::QuotaExceeded(_) | LlmError::Overloaded => {
            ProviderRuntimeSeverity::Warn
        }
        _ => ProviderRuntimeSeverity::Error,
    }
}

fn limit_event_type(error: &LlmError) -> Option<ProviderRuntimeEventType> {
    match error {
        LlmError::RateLimited { .. } => Some(ProviderRuntimeEventType::RateLimitObserved),
        LlmError::QuotaExceeded(_) => Some(ProviderRuntimeEventType::UsageLimitObserved),
        _ => None,
    }
}

fn error_metadata(error: &LlmError) -> serde_json::Value {
    match error {
        LlmError::RateLimited { retry_after_secs } => serde_json::json!({
            "error_kind": error_kind(error),
            "retry_after_secs": retry_after_secs,
        }),
        LlmError::Server { status, .. } => serde_json::json!({
            "error_kind": error_kind(error),
            "status": status,
        }),
        _ => serde_json::json!({
            "error_kind": error_kind(error),
        }),
    }
}

#[cfg(test)]
#[path = "provider_observer_tests.rs"]
mod tests;
