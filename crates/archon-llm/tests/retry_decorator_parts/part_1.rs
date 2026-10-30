#[test]
fn classify_fail_fast_variants() {
    assert_eq!(
        classify(&LlmError::Auth("x".into())),
        RetryDecision::FailFast,
        "Auth -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Serialize("x".into())),
        RetryDecision::FailFast,
        "Serialize -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Unsupported("x".into())),
        RetryDecision::FailFast,
        "Unsupported -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Server {
            status: 400,
            message: "x".into(),
            pipeline_retry_after_secs: None,
        }),
        RetryDecision::FailFast,
        "Server 4xx -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Server {
            status: 404,
            message: "x".into(),
            pipeline_retry_after_secs: None,
        }),
        RetryDecision::FailFast,
        "Server 404 -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::ProviderNotFound {
            name: "x".into(),
            available: "y".into()
        }),
        RetryDecision::FailFast,
        "ProviderNotFound -> FailFast"
    );
}

// ---------------------------------------------------------------------------
// Validation Criterion 8: RetryPolicy::default().max_attempts == 3
// ---------------------------------------------------------------------------

#[test]
fn default_policy_matches_err_prov_02() {
    let p = RetryPolicy::default();
    assert_eq!(
        p.max_attempts, 3,
        "ERR-PROV-02 (line 1850): default must be 3 attempts"
    );
    assert_eq!(p.initial_backoff, Duration::from_millis(500));
    assert_eq!(p.max_backoff, Duration::from_secs(8));
    assert_eq!(p.multiplier, 2.0);
    assert!(
        p.jitter,
        "default policy must apply jitter to avoid thundering herd"
    );
}

// ---------------------------------------------------------------------------
// Bonus: RateLimited honors retry_after_secs (NFR-RELIABILITY-003)
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn rate_limited_sleeps_for_retry_after() {
    let policy = RetryPolicy {
        max_attempts: 2,
        initial_backoff: Duration::from_millis(10), // would be much smaller if honored
        max_backoff: Duration::from_secs(8),
        multiplier: 2.0,
        jitter: false,
    };
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::RateLimited {
            retry_after_secs: 2,
            from_provider: true,
            pipeline_retry_after_secs: None,
            message: String::new().into(),
        }),
        Ok(ok_response()),
    ]));
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), policy);

    let start = tokio::time::Instant::now();
    let resp = provider
        .complete(base_request())
        .await
        .expect("retry succeeds");
    let elapsed = start.elapsed();
    assert_eq!(resp.stop_reason, "stop");
    assert_eq!(
        elapsed,
        Duration::from_secs(2),
        "RateLimited must honor retry_after_secs instead of backoff formula"
    );
}

// ---------------------------------------------------------------------------
// Helper: wrap Arc<MockProvider> in a newtype implementing LlmProvider by
// delegation so the tests can observe call counts via the Arc after moving
// the provider into RetryProvider.
// ---------------------------------------------------------------------------

struct ArcDelegate(Arc<MockProvider>);

#[async_trait]
impl LlmProvider for ArcDelegate {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn models(&self) -> Vec<ModelInfo> {
        self.0.models()
    }
    fn supports_feature(&self, f: ProviderFeature) -> bool {
        self.0.supports_feature(f)
    }
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.0.complete(request).await
    }
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.0.stream(request).await
    }
}
