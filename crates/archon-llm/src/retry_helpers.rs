use super::*;

impl<P: LlmProvider + ?Sized> RetryProvider<P> {
    pub fn new(inner: Arc<P>, policy: RetryPolicy) -> Self {
        Self {
            inner,
            policy,
            supervisor: None,
        }
    }

    pub fn new_with_supervisor(
        inner: Arc<P>,
        policy: RetryPolicy,
        supervisor: Arc<Mutex<ProviderRuntimeSupervisor>>,
    ) -> Self {
        Self {
            inner,
            policy,
            supervisor: Some(supervisor),
        }
    }

    /// Expose the policy for telemetry/introspection.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    /// Expose the wrapped provider.
    pub fn inner(&self) -> &Arc<P> {
        &self.inner
    }

    pub fn supervisor(&self) -> Option<Arc<Mutex<ProviderRuntimeSupervisor>>> {
        self.supervisor.as_ref().map(Arc::clone)
    }

    /// Peek at a freshly opened stream until it either produces content or
    /// fails.
    ///
    /// Everything consumed while peeking is replayed into a new channel ahead
    /// of the rest of the stream, so the caller sees an identical sequence —
    /// peeking must not cost the consumer the opening events.
    ///
    /// "Content" means anything the consumer can act on. Bookkeeping events
    /// (MessageStart, ContentBlockStart) do not count: a stream that announces
    /// a block and then dies has still produced nothing, and is the exact case
    /// worth retrying.
    pub(super) async fn drain_until_content_or_error(mut rx: Receiver<StreamEvent>) -> StreamProbe {
        let mut buffered: Vec<StreamEvent> = Vec::new();
        loop {
            match rx.recv().await {
                Some(StreamEvent::Error {
                    error_type,
                    message,
                }) if buffered.iter().all(|event| !Self::is_content(event)) => {
                    return StreamProbe::FailedBeforeContent {
                        error_type,
                        message,
                    };
                }
                Some(event) => {
                    let is_content = Self::is_content(&event);
                    buffered.push(event);
                    if is_content {
                        break;
                    }
                }
                None => break, // clean end with no content — nothing to retry
            }
        }
        let (tx, new_rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(async move {
            for event in buffered {
                if tx.send(event).await.is_err() {
                    return;
                }
            }
            while let Some(event) = rx.recv().await {
                // 86fbab995 redacted forwarded error text here.
                let event = match event {
                    StreamEvent::Error {
                        error_type,
                        message,
                    } => StreamEvent::Error {
                        error_type,
                        message: crate::provider::DiagnosticMessage::redacted_at_source(
                            message.as_str(),
                        ),
                    },
                    event => event,
                };
                if tx.send(event).await.is_err() {
                    return;
                }
            }
        });
        StreamProbe::Usable(new_rx)
    }

    fn is_content(event: &StreamEvent) -> bool {
        matches!(
            event,
            StreamEvent::TextDelta { .. }
                | StreamEvent::InputJsonDelta { .. }
                | StreamEvent::ContentBlockStop { .. }
                | StreamEvent::MessageStop
                // Anthropic keep-alives are visible activity. Releasing the
                // stream at the first ping keeps the caller's idle clock and
                // HTTP read backstop in control while preserving pre-content
                // error classification for streams that fail before activity.
                | StreamEvent::Ping
        )
    }

    /// Compute the sleep duration for retry `attempt` (0-indexed), honoring
    /// `max_backoff` clamp and optional ±25% jitter.
    fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let initial_ms = self.policy.initial_backoff.as_millis() as f64;
        let raw = initial_ms * self.policy.multiplier.powi(attempt as i32);
        let cap = self.policy.max_backoff.as_millis() as f64;
        let clamped = raw.min(cap);
        let final_ms = if self.policy.jitter {
            // ±25% jitter: uniform in [0.75, 1.25).
            let j = rand::random::<f64>() * 0.5 + 0.75;
            clamped * j
        } else {
            clamped
        };
        Duration::from_millis(final_ms.round() as u64)
    }

    /// Determine how long to sleep after the given error on retry `attempt`.
    /// `LlmError::RateLimited` overrides the formula with the server hint.
    pub(super) fn sleep_for_error(&self, err: &LlmError, attempt: u32) -> Duration {
        if let LlmError::RateLimited {
            retry_after_secs, ..
        } = err
        {
            return Duration::from_secs(*retry_after_secs);
        }
        self.backoff_for_attempt(attempt)
    }

    pub(super) fn record_runtime_event(
        &self,
        request: &LlmRequest,
        event_type: ProviderRuntimeEventType,
        severity: ProviderRuntimeSeverity,
        reason_code: Option<&str>,
        retry_count: Option<u32>,
    ) {
        let Some(supervisor) = &self.supervisor else {
            return;
        };
        let mut event = ProviderRuntimeEvent::new(
            self.inner.name().to_string(),
            request
                .request_origin
                .clone()
                .unwrap_or_else(|| "provider_builder".to_string()),
            event_type,
            severity,
        )
        .with_model(request.model.clone());
        if let Some(reason) = reason_code {
            event = event.with_reason(reason);
        }
        if let Some(count) = retry_count {
            event = event.with_retry_count(count);
        }
        if let Ok(mut guard) = supervisor.lock() {
            let _ = guard.record_event(event);
        }
    }
}
