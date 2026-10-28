use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn request() -> LlmRequest {
    LlmRequest {
        model: "test".into(),
        max_tokens: 1,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role":"user","content":"x"})],
        tools: Default::default(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
        request_origin: None,
        reasoning_encrypted: None,
    }
}

#[derive(Clone, Copy)]
enum FailureScript {
    Overload(Option<u64>),
    RateLimit(u64),
}

struct CompleteFailureProvider {
    calls: AtomicUsize,
    failure: FailureScript,
    times: std::sync::Mutex<Vec<tokio::time::Instant>>,
}

#[async_trait]
impl LlmProvider for CompleteFailureProvider {
    fn name(&self) -> &str {
        "complete-failure"
    }
    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        true
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.times.lock().unwrap().push(tokio::time::Instant::now());
        Err(match self.failure {
            FailureScript::Overload(retry_after_secs) => LlmError::Overloaded {
                status: Some(529),
                retry_after_secs,
                message: String::new(),
            },
            FailureScript::RateLimit(retry_after_secs) => LlmError::RateLimited {
                retry_after_secs,
                from_provider: true,
                pipeline_retry_after_secs: None,
                message: String::new(),
            },
        })
    }
    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        unreachable!()
    }
}

async fn exercise_complete_retry(
    failure: FailureScript,
) -> (usize, Vec<std::time::Duration>, String) {
    let inner = Arc::new(CompleteFailureProvider {
        calls: AtomicUsize::new(0),
        failure,
        times: std::sync::Mutex::new(Vec::new()),
    });
    let provider = RetryProvider::new(
        inner.clone(),
        RetryPolicy {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_secs(10),
            max_backoff: std::time::Duration::from_secs(30),
            multiplier: 2.0,
            jitter: false,
        },
    );
    let result = provider.complete(request()).await.unwrap_err();
    let calls = inner.calls.load(Ordering::SeqCst);
    let times = inner.times.lock().unwrap();
    let waits = times
        .windows(2)
        .map(|pair| pair[1].duration_since(pair[0]))
        .collect();
    (calls, waits, result.to_string())
}

#[tokio::test(start_paused = true)]
async fn hinted_overload_and_rate_limit_keep_retryprovider_attempts_waits_and_display() {
    let hinted_529 = tokio::spawn(exercise_complete_retry(FailureScript::Overload(Some(
        86_400,
    ))));
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(20)).await;
    let hinted_529 = hinted_529.await.unwrap();
    assert_eq!(
        hinted_529,
        (
            3,
            vec![
                std::time::Duration::from_secs(10),
                std::time::Duration::from_secs(20)
            ],
            "server overloaded".into()
        )
    );

    let hinted_429 = tokio::spawn(exercise_complete_retry(FailureScript::RateLimit(7)));
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(7)).await;
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(7)).await;
    let hinted_429 = hinted_429.await.unwrap();
    assert_eq!(
        hinted_429,
        (
            3,
            vec![
                std::time::Duration::from_secs(7),
                std::time::Duration::from_secs(7)
            ],
            "rate limited: retry after 7s".into()
        )
    );
}
