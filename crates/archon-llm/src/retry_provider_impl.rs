use super::*;

#[async_trait]
impl<P: LlmProvider + ?Sized> LlmProvider for RetryProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }

    fn request_identity(&self, request: &LlmRequest) -> Option<String> {
        self.inner.request_identity(request)
    }

    fn supports_temperature(&self) -> bool {
        self.inner.supports_temperature()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        self.inner.supports_feature(feature)
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        self.inner.data_flow_classification()
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        self.inner.compaction_provider_family()
    }

    fn compaction_policy(&self) -> crate::compaction_policy::CompactionPolicy {
        self.inner.compaction_policy()
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        self.inner.resolve_alias(alias)
    }

    fn resolve_request_model(&self, request: &mut LlmRequest) {
        self.inner.resolve_request_model(request)
    }

    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        self.inner.cache_strategy(model)
    }

    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        self.inner.cache_platform()
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        self.inner.as_anthropic()
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.record_runtime_event(
            &request,
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
            None,
            None,
        );
        let max = self.policy.max_attempts.max(1);
        let mut attempt: u32 = 0;
        loop {
            match self.inner.complete(request.clone()).await {
                Ok(resp) => {
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestSucceeded,
                        ProviderRuntimeSeverity::Info,
                        None,
                        Some(attempt),
                    );
                    return Ok(resp);
                }
                Err(err) => {
                    attempt += 1;
                    if crate::provider_fault::retry_is_deferred()
                        || attempt >= max
                        || classify(&err) == RetryDecision::FailFast
                    {
                        let reason = reason_code_for_error(&err);
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestFailed,
                            ProviderRuntimeSeverity::Error,
                            Some(reason),
                            Some(attempt),
                        );
                        return Err(err);
                    }
                    let reason = reason_code_for_error(&err);
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestRetry,
                        ProviderRuntimeSeverity::Warn,
                        Some(reason),
                        Some(attempt),
                    );
                    let sleep = self.sleep_for_error(&err, attempt - 1);
                    tokio::time::sleep(sleep).await;
                }
            }
        }
    }

    /// The runner owns streaming retries through its no-progress window.
    /// This wrapper only classifies pre-content rejection events and forwards
    /// transient errors so the runner can resend them under that window.
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.record_runtime_event(
            &request,
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
            None,
            None,
        );
        match self.inner.stream(request.clone()).await {
            Ok(rx) => {
                // A successful open ends provider-attempt accounting. Waiting
                // for first content here would hide an opened but stalled
                // stream from the caller's no-progress clocks.
                if crate::provider_fault::retry_is_deferred() {
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestSucceeded,
                        ProviderRuntimeSeverity::Info,
                        None,
                        Some(0),
                    );
                    return Ok(rx);
                }
                // Peek only to classify pre-content provider rejection. A
                // transient error is replayed as an event for the runner's
                // no-progress window to own the resend decision.
                match Self::drain_until_content_or_error(rx).await {
                    StreamProbe::Usable(rx) => {
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestSucceeded,
                            ProviderRuntimeSeverity::Info,
                            None,
                            Some(0),
                        );
                        return Ok(rx);
                    }
                    StreamProbe::FailedBeforeContent {
                        error_type,
                        message,
                    } => {
                        // Streaming retries belong to the runner's existing
                        // no-progress window. Classify rejections here, but
                        // replay transient provider errors for the runner.
                        let error = stream_error_before_content(&error_type, &message);
                        if classify(&error) == RetryDecision::FailFast {
                            self.record_runtime_event(
                                &request,
                                ProviderRuntimeEventType::RequestFailed,
                                ProviderRuntimeSeverity::Error,
                                Some("stream_failed_before_content"),
                                Some(0),
                            );
                            return Err(error);
                        }
                        let (tx, rx) = tokio::sync::mpsc::channel(1);
                        let message = crate::provider::DiagnosticMessage::redacted_at_source(
                            message.as_str(),
                        );
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type,
                                message,
                            })
                            .await;
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestFailed,
                            ProviderRuntimeSeverity::Warn,
                            Some("stream_failed_before_content"),
                            Some(0),
                        );
                        return Ok(rx);
                    }
                }
            }
            Err(LlmError::Rejected {
                error_type,
                message,
            }) => Err(LlmError::Rejected {
                error_type,
                message: crate::provider::DiagnosticMessage::redacted_at_source(message.as_str()),
            }),
            Err(err) => Err(err),
        }
    }
}
