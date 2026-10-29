use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Compat429Provider {
    calls: AtomicUsize,
    times: std::sync::Mutex<Vec<tokio::time::Instant>>,
    status: reqwest::StatusCode,
}

#[async_trait::async_trait]
impl LlmProvider for Compat429Provider {
    fn name(&self) -> &str {
        "compat-429"
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
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, "120".parse().unwrap());
        Err(OpenAiCompatProvider::map_http_error(
            self.status,
            &headers,
            "rate limited".into(),
            "compat-429",
        ))
    }
    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        unreachable!()
    }
}

#[test]
fn retry_after_header_accepts_seconds_and_http_date() {
    assert_eq!(OpenAiCompatProvider::parse_retry_after("37"), Some(37));
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(70);
    let date = chrono::DateTime::<chrono::Utc>::from(future)
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    assert!(
        OpenAiCompatProvider::parse_retry_after(&date)
            .is_some_and(|seconds| (68..=70).contains(&seconds))
    );

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "37".parse().unwrap());
    assert!(matches!(
        OpenAiCompatProvider::map_http_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            &headers,
            String::new(),
            "test"
        ),
        LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: Some(37),
            ..
        }
    ));
    let server = OpenAiCompatProvider::map_http_error(
        reqwest::StatusCode::from_u16(529).unwrap(),
        &headers,
        "temporarily overloaded".into(),
        "test",
    );
    assert!(matches!(
        server,
        LlmError::Server {
            status: 529,
            pipeline_retry_after_secs: Some(37),
            ..
        }
    ));
    assert_eq!(
        server.to_string(),
        "server error (529): temporarily overloaded"
    );
}

#[tokio::test(start_paused = true)]
async fn retry_after_120_stays_out_of_retryprovider_wait_and_fail_fast_policy() {
    let inner = Arc::new(Compat429Provider {
        calls: AtomicUsize::new(0),
        times: std::sync::Mutex::new(Vec::new()),
        status: reqwest::StatusCode::TOO_MANY_REQUESTS,
    });
    let retry = crate::retry::RetryProvider::new(
        inner.clone(),
        crate::retry::RetryPolicy {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_secs(10),
            max_backoff: std::time::Duration::from_secs(30),
            multiplier: 2.0,
            jitter: false,
        },
    );
    let request = LlmRequest {
        model: "test".into(),
        max_tokens: 1,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "x"})],
        tools: Default::default(),
        thinking: None,
        speed: None,
        effort: None,
        extra: Value::Null,
        request_origin: None,
        reasoning_encrypted: None,
    };
    let error = retry.complete(request).await.unwrap_err();
    assert!(matches!(
        error,
        LlmError::RateLimited {
            retry_after_secs: 0,
            from_provider: false,
            pipeline_retry_after_secs: Some(120),
            ..
        }
    ));
    assert_eq!(inner.calls.load(Ordering::SeqCst), 3);
    let times = inner.times.lock().unwrap();
    assert_eq!(
        times
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>(),
        [std::time::Duration::ZERO, std::time::Duration::ZERO]
    );
}

#[tokio::test(start_paused = true)]
async fn retry_after_529_stays_out_of_retryprovider_wait() {
    let inner = Arc::new(Compat429Provider {
        calls: AtomicUsize::new(0),
        times: std::sync::Mutex::new(Vec::new()),
        status: reqwest::StatusCode::from_u16(529).unwrap(),
    });
    let retry = crate::retry::RetryProvider::new(
        inner.clone(),
        crate::retry::RetryPolicy {
            max_attempts: 3,
            initial_backoff: std::time::Duration::from_secs(10),
            max_backoff: std::time::Duration::from_secs(30),
            multiplier: 2.0,
            jitter: false,
        },
    );
    let request = LlmRequest {
        model: "test".into(),
        max_tokens: 1,
        system: Vec::new(),
        messages: vec![json!({"role": "user", "content": "x"})],
        tools: Default::default(),
        thinking: None,
        speed: None,
        effort: None,
        extra: Value::Null,
        request_origin: None,
        reasoning_encrypted: None,
    };
    let error = retry.complete(request).await.unwrap_err();
    assert!(matches!(
        error,
        LlmError::Server {
            status: 529,
            pipeline_retry_after_secs: Some(120),
            ..
        }
    ));
    assert_eq!(error.to_string(), "server error (529): rate limited");
    assert_eq!(inner.calls.load(Ordering::SeqCst), 3);
    let times = inner.times.lock().unwrap();
    assert_eq!(
        times
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect::<Vec<_>>(),
        [
            std::time::Duration::from_secs(10),
            std::time::Duration::from_secs(20)
        ]
    );
}

#[tokio::test]
async fn response_header_retains_typed_retry_after() {
    let response = crate::connection_tests::error_response(503, "90").await;
    let status = response.status();
    let headers = response.headers().clone();
    let error = OpenAiCompatProvider::map_http_error(
        status,
        &headers,
        response.text().await.unwrap(),
        "AC-X-001",
    );
    assert_eq!(error.retry_after_hint(), Some(90), "header lost: {error:?}");
}
