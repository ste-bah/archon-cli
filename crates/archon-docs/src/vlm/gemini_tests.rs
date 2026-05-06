use super::*;
use wiremock::matchers::{body_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(endpoint_base: String) -> GeminiVlmProvider {
    GeminiVlmProvider::new_with_retry_delay(
        "test-key",
        DEFAULT_GEMINI_MODEL,
        endpoint_base,
        100,
        Duration::from_millis(1),
    )
    .unwrap()
}

#[test]
fn rate_limiter_blocks_when_rpm_exceeded() {
    let limiter = GeminiRateLimiter::new(1);
    let now = Instant::now();
    assert!(limiter.try_acquire_at(now).is_ok());
    assert!(
        limiter
            .try_acquire_at(now + Duration::from_secs(1))
            .is_err()
    );
    assert!(
        limiter
            .try_acquire_at(now + Duration::from_secs(61))
            .is_ok()
    );
}

#[test]
fn provider_refuses_when_api_key_missing() {
    let err = GeminiVlmProvider::new("", "model", DEFAULT_GEMINI_ENDPOINT_BASE, 15).unwrap_err();
    assert!(matches!(err, DocsError::VlmAuthentication { provider, .. } if provider == "gemini"));
}

#[test]
fn default_rpm_limit_is_12() {
    assert_eq!(archon_policy::GeminiVlmPolicy::default().rpm_limit, 12);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_succeeds_when_model_exists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(query_param("key", "test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{"name": "models/gemini-3-flash-preview"}]
        })))
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    tokio::task::spawn_blocking(move || provider(endpoint_base).health_check())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_sends_inlinedata_with_correct_mime() {
    let server = MockServer::start().await;
    let png = &[0x89, b'P', b'N', b'G', b'x'];
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .and(query_param("key", "test-key"))
        .and(body_json(json!({
            "contents": [{
                "parts": [
                    {"text": IMAGE_DESCRIPTION_PROMPT},
                    {"inlineData": {"mimeType": "image/png", "data": STANDARD.encode(png)}}
                ]
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"parts": [{"text": "upward line chart"}]}}]
        })))
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    let image = png.to_vec();
    let text = tokio::task::spawn_blocking(move || provider(endpoint_base).describe_image(&image))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(text, "upward line chart");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_retries_up_to_five_attempts_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(4)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"parts": [{"text": "retried ok"}]}}]
        })))
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    let text = tokio::task::spawn_blocking(move || {
        provider(endpoint_base).describe_image(&[0x89, b'P', b'N', b'G'])
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(text, "retried ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_uses_exponential_backoff() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(4)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "candidates": [{"content": {"parts": [{"text": "ok"}]}}]
        })))
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    let elapsed = tokio::task::spawn_blocking(move || {
        let started = Instant::now();
        provider(endpoint_base)
            .describe_image(&[0x89, b'P', b'N', b'G'])
            .unwrap();
        started.elapsed()
    })
    .await
    .unwrap();
    assert!(elapsed >= Duration::from_millis(15), "elapsed={elapsed:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_gives_up_after_five_failed_429s() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    let err = tokio::task::spawn_blocking(move || {
        provider(endpoint_base).describe_image(&[0x89, b'P', b'N', b'G'])
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(err, DocsError::VlmRateLimit { provider, .. } if provider == "gemini"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_does_not_retry_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-3-flash-preview:generateContent"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint_base = server.uri();
    let err = tokio::task::spawn_blocking(move || {
        provider(endpoint_base).describe_image(&[0x89, b'P', b'N', b'G'])
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        matches!(err, DocsError::VlmProvider { provider, status_code: Some(500), .. } if provider == "gemini")
    );
}
