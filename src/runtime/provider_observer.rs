use anyhow::Result;
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;

// The `#[path]`-included test modules resolve names against this scope, so these
// stay for them even though the wrapper itself now reaches persistence only
// through `recorder`.
#[cfg(test)]
use super::provider_event_record::provider_event_record;
#[cfg(test)]
use cozo::DbInstance;
#[cfg(test)]
use recorder::run_provider_persistence_async_with;

#[path = "provider_observer_errors.rs"]
mod errors;
#[path = "provider_identity_events.rs"]
mod identity_events;
#[path = "provider_observer_runtime.rs"]
mod observer_runtime;
#[path = "provider_observer_recorder.rs"]
mod recorder;
#[path = "provider_observer_stream.rs"]
mod stream;
#[path = "provider_observer_usage.rs"]
mod usage;
pub(crate) use recorder::ProviderRuntimeEventRecorder;

use errors::{error_kind, error_message, error_metadata, error_severity, limit_event_type};
use observer_runtime::record_agent_provider_incident_sync;
pub(crate) use observer_runtime::{record_provider_fallback, runtime_mode_for_provider_name};

use usage::ObservedRequest;
#[cfg(test)]
use usage::context_input_tokens;

pub(crate) async fn observe_llm_provider_with_profile(
    provider: Arc<dyn LlmProvider>,
    runtime_mode: impl Into<String>,
    profile_id: Option<String>,
) -> Arc<dyn LlmProvider> {
    // #189 Phase 5: record/replay goes innermost, closest to the real provider,
    // so observation records the same events either way — telemetry from a
    // replayed run looks like telemetry from the run it was recorded from. Does
    // nothing unless `ARCHON_LLM_REPLAY` is set. Placed here because this is
    // the one function every provider in the binary passes through; wrapping at
    // any single call site would record part of a run.
    let provider = archon_llm::replay::wrap_if_enabled(provider);
    Arc::new(
        ObservedLlmProvider::new(
            provider,
            runtime_mode,
            profile_id,
            ProviderRuntimeEventRecorder::default_learning_store().await,
        )
        .await,
    )
}

pub(crate) struct ObservedLlmProvider {
    inner: Arc<dyn LlmProvider>,
    runtime_mode: String,
    profile_id: Option<String>,
    recorder: ProviderRuntimeEventRecorder,
}

impl ObservedLlmProvider {
    async fn new(
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
        let event = identity_events::provider_identity_decision_event(
            observed.inner.as_ref(),
            &observed.runtime_mode,
            observed.profile_id.as_deref(),
        );
        if let Some(event) = event {
            observed.recorder.record(event).await;
        }
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

    async fn record_start(&self, request_id: &str, request: &ObservedRequest, operation: &str) {
        let event = self
            .event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestStarted,
                ProviderRuntimeSeverity::Debug,
            )
            .with_reason(operation);
        let provider_id = self.inner.name().to_string();
        let model = request.model.clone();
        let origin = request.origin.clone();
        let run_id = request.run_id.clone();
        let request_id = request_id.to_string();
        let operation = operation.to_string();
        self.recorder
            .persist("record provider request start", move |recorder| {
                recorder.record_sync(event);
                crate::command::world_model::record_provider_runtime_advisory(
                    run_id.as_deref().unwrap_or(&provider_id),
                    &request_id,
                    &format!(
                        "{} provider={} model={} origin={}",
                        operation,
                        provider_id,
                        model,
                        origin.as_deref().unwrap_or("unknown")
                    ),
                );
            })
            .await;
    }

    async fn record_call_usage(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        usage: Option<Usage>,
        status: &str,
    ) {
        let request_id = request_id.to_owned();
        let request = request.clone();
        let status = status.to_owned();
        self.recorder
            .persist("record logical call usage", move |recorder| {
                recorder.record_call_usage_sync(&request_id, &request, usage.as_ref(), &status);
            })
            .await;
    }

    async fn record_success(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        metadata: serde_json::Value,
    ) {
        let event = self
            .event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestSucceeded,
                ProviderRuntimeSeverity::Info,
            )
            .with_reason("ok")
            .with_redacted_json(metadata);
        let provider_id = self.inner.name().to_string();
        let runtime_mode = self.runtime_mode.clone();
        let profile_id = self.profile_id.clone();
        let model = request.model.clone();
        let request_id = request_id.to_string();
        self.recorder
            .persist("record provider request success", move |recorder| {
                recorder.record_sync(event);
                super::provider_profile_updates::mark_success(
                    recorder.db.as_ref(),
                    &provider_id,
                    &runtime_mode,
                    profile_id.as_deref(),
                    Some(&model),
                    Some(&request_id),
                );
            })
            .await;
    }

    async fn record_failure(
        &self,
        request_id: &str,
        request: &ObservedRequest,
        error: LlmError,
    ) -> LlmError {
        let error_kind = error_kind(&error);
        let event = self
            .event(
                request_id,
                request,
                ProviderRuntimeEventType::RequestFailed,
                error_severity(&error),
            )
            .with_reason(error_kind)
            .with_message(error_message(&error))
            .with_redacted_json(error_metadata(&error));
        let limit_event = limit_event_type(&error).map(|event_type| {
            self.event(
                request_id,
                request,
                event_type,
                ProviderRuntimeSeverity::Warn,
            )
            .with_reason(error_kind)
            .with_message(error_message(&error))
            .with_redacted_json(error_metadata(&error))
        });
        let provider_id = self.inner.name().to_string();
        let runtime_mode = self.runtime_mode.clone();
        let profile_id = self.profile_id.clone();
        let observed = request.clone();
        let request_id = request_id.to_string();
        let shared_error = Arc::new(std::sync::Mutex::new(Some(error)));
        let persistence_error = Arc::clone(&shared_error);
        self.recorder
            .persist("record provider request failure", move |recorder| {
                let guard = persistence_error
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let error = guard
                    .as_ref()
                    .expect("provider error remains available during persistence");
                if let Some(event_id) = recorder.record_sync(event) {
                    record_agent_provider_incident_sync(
                        recorder.db.as_ref(),
                        &provider_id,
                        &event_id,
                        &observed,
                        error_kind,
                    );
                    if let Some(run_id) = observed.run_id.as_deref()
                        && let Ok(config) = archon_core::config::load_config()
                    {
                        let attached = crate::command::world_model::record_guardrail_provider_incident_for_session(
                            &config,
                            run_id,
                            &event_id,
                            error_kind,
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

                if let Some(limit_event) = limit_event {
                    recorder.record_sync(limit_event);
                    recorder.record_limit_window_sync(&provider_id, Some(&observed.model), error);
                }
                super::provider_profile_updates::mark_failure(
                    recorder.db.as_ref(),
                    &provider_id,
                    &runtime_mode,
                    profile_id.as_deref(),
                    Some(&observed.model),
                    Some(&request_id),
                    error,
                );
            })
            .await;
        shared_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("provider error remains available after persistence")
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

    /// Forward alias resolution to the wrapped provider.
    ///
    /// This wrapper is decoration, not policy: every method it does not forward
    /// silently substitutes the trait default, and for `resolve_alias` that
    /// default is `None` — "this provider has no bespoke substitution". The
    /// wrapped provider always does have one. Both `CodexProvider` and
    /// `AnthropicProvider` implement `resolve_alias` and are handed a config-built
    /// alias map at construction; wrapping them here made that map unreachable.
    ///
    /// The failure was silent and looked like a config bug. `resolve_request_model`
    /// falls back to `self.models().first()` for any recognised tier alias, so a
    /// `Coder` stage resolving `sonnet` got the registry's first model — `gpt-5.5`
    /// — while `[models.openai-codex] default` said otherwise, and editing that
    /// config changed nothing because nothing read it. Measured on a live
    /// workflow: 698 of 704 subagent requests took the fallback. The direct
    /// session path was unaffected precisely because it is not wrapped, which is
    /// what made this look like a subagent-specific problem rather than a
    /// wrapper-specific one.
    fn resolve_alias(&self, alias: &str) -> Option<String> {
        self.inner.resolve_alias(alias)
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let observed = ObservedRequest::from_request(self.inner.name(), &request);
        let request_id = uuid::Uuid::new_v4().to_string();
        self.record_start(&request_id, &observed, "stream").await;

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
                self.record_call_usage(&request_id, &observed, None, "failed")
                    .await;
                let error = self.record_failure(&request_id, &observed, error).await;
                Err(error)
            }
        }
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let observed = ObservedRequest::from_request(self.inner.name(), &request);
        let request_id = uuid::Uuid::new_v4().to_string();
        self.record_start(&request_id, &observed, "complete").await;

        match self.inner.complete(request).await {
            Ok(response) => {
                self.record_call_usage(
                    &request_id,
                    &observed,
                    Some(response.usage.clone()),
                    "succeeded",
                )
                .await;
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
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                self.record_call_usage(&request_id, &observed, None, "failed")
                    .await;
                let error = self.record_failure(&request_id, &observed, error).await;
                Err(error)
            }
        }
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        self.inner.supports_feature(feature)
    }
    fn cache_strategy(&self, model: &str) -> archon_llm::cache_strategy::CacheStrategy {
        self.inner.cache_strategy(model)
    }

    fn as_anthropic(&self) -> Option<&archon_llm::anthropic::AnthropicClient> {
        self.inner.as_anthropic()
    }
}

fn base_event(
    provider_id: &str,
    runtime_mode: &str,
    event_type: ProviderRuntimeEventType,
    severity: ProviderRuntimeSeverity,
) -> ProviderRuntimeEvent {
    ProviderRuntimeEvent::new(provider_id, runtime_mode, event_type, severity)
}

#[cfg(test)]
#[path = "provider_observer_persistence_tests.rs"]
mod persistence_tests;
#[cfg(test)]
#[path = "provider_observer_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "provider_observer_usage_tests.rs"]
mod usage_tests;
