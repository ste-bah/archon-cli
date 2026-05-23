use super::*;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(endpoint: String, api_key: Option<&str>) -> OpenAiCompatVlmProvider {
    OpenAiCompatVlmProvider::new(
        endpoint,
        DEFAULT_OPENAI_COMPAT_MODEL,
        DEFAULT_OPENAI_COMPAT_API_KEY_ENV,
        api_key.map(ToString::to_string),
        Duration::from_secs(5),
        DEFAULT_OPENAI_COMPAT_MAX_TOKENS,
        DEFAULT_OPENAI_COMPAT_TEMPERATURE,
    )
    .unwrap()
}

fn png() -> &'static [u8] {
    &[0x89, b'P', b'N', b'G', b'x']
}

#[test]
fn blank_endpoint_guard_returns_provider_error() {
    let err = OpenAiCompatVlmProvider::new(
        "",
        DEFAULT_OPENAI_COMPAT_MODEL,
        DEFAULT_OPENAI_COMPAT_API_KEY_ENV,
        None,
        Duration::from_secs(5),
        DEFAULT_OPENAI_COMPAT_MAX_TOKENS,
        DEFAULT_OPENAI_COMPAT_TEMPERATURE,
    )
    .unwrap_err();

    assert!(matches!(err, DocsError::VlmProvider { provider, .. } if provider == "openai-compat"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_succeeds_with_model_in_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "google/gemma-3-12b-it"}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    tokio::task::spawn_blocking(move || provider(endpoint, None).health_check())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_fails_when_model_missing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "llava:13b"}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err = tokio::task::spawn_blocking(move || provider(endpoint, None).health_check())
        .await
        .unwrap()
        .unwrap_err();
    assert!(err.to_string().contains("not returned"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_sends_bearer_when_key_present() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "google/gemma-3-12b-it"}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    tokio::task::spawn_blocking(move || provider(endpoint, Some("test-key")).health_check())
        .await
        .unwrap()
        .unwrap();

    let requests = server.received_requests().await.unwrap_or_default();
    let auth = requests[0].headers.get("authorization").unwrap();
    assert_eq!(auth.to_str().unwrap(), "Bearer test-key");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_sends_no_auth_without_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "google/gemma-3-12b-it"}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    tokio::task::spawn_blocking(move || provider(endpoint, None).health_check())
        .await
        .unwrap()
        .unwrap();

    let requests = server.received_requests().await.unwrap_or_default();
    assert!(requests[0].headers.get("authorization").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_sends_openai_vision_data_url() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "chart description"}}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    tokio::task::spawn_blocking(move || provider(endpoint, None).describe_image(png(), None))
        .await
        .unwrap()
        .unwrap();

    let requests = server.received_requests().await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["model"], DEFAULT_OPENAI_COMPAT_MODEL);
    assert_eq!(body["messages"][0]["content"][0]["type"], "text");
    assert_eq!(
        body["messages"][0]["content"][0]["text"],
        IMAGE_DESCRIPTION_PROMPT
    );
    assert_eq!(body["messages"][0]["content"][1]["type"], "image_url");
    assert!(
        body["messages"][0]["content"][1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_uses_prompt_override() {
    let server = MockServer::start().await;
    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(png()));
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_json(json!({
            "model": DEFAULT_OPENAI_COMPAT_MODEL,
            "max_tokens": DEFAULT_OPENAI_COMPAT_MAX_TOKENS,
            "temperature": DEFAULT_OPENAI_COMPAT_TEMPERATURE,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": crate::vlm::VIDEO_FRAME_PROMPT},
                    {"type": "image_url", "image_url": {"url": data_url}}
                ]
            }]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "video frame description"}}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let text = tokio::task::spawn_blocking(move || {
        provider(endpoint, None).describe_image(png(), Some(crate::vlm::VIDEO_FRAME_PROMPT))
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(text, "video frame description");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_returns_string_content() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": "visual summary"}}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let text =
        tokio::task::spawn_blocking(move || provider(endpoint, None).describe_image(png(), None))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(text, "visual summary");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_returns_array_content_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content": [
                {"type": "text", "text": "line chart"},
                {"type": "text", "text": "rising trend"}
            ]}}]
        })))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let text =
        tokio::task::spawn_blocking(move || provider(endpoint, None).describe_image(png(), None))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(text, "line chart\nrising trend");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generate_once_maps_429_to_rate_limit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "7"))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err =
        tokio::task::spawn_blocking(move || provider(endpoint, None).generate_once(png(), None))
            .await
            .unwrap()
            .unwrap_err();
    assert!(
        matches!(err, DocsError::VlmRateLimit { provider, retry_after_secs: 7, .. } if provider == "openai-compat")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_handles_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err =
        tokio::task::spawn_blocking(move || provider(endpoint, None).describe_image(png(), None))
            .await
            .unwrap()
            .unwrap_err();
    assert!(
        matches!(err, DocsError::VlmProvider { provider, status_code: Some(500), .. } if provider == "openai-compat")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn describe_image_maps_401_to_authentication() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err = tokio::task::spawn_blocking(move || {
        provider(endpoint, Some("bad-key")).describe_image(png(), None)
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        matches!(err, DocsError::VlmAuthentication { provider, .. } if provider == "openai-compat")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_check_401_without_key_mentions_api_key_env() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err = tokio::task::spawn_blocking(move || provider(endpoint, None).health_check())
        .await
        .unwrap()
        .unwrap_err();
    assert!(err.to_string().contains(DEFAULT_OPENAI_COMPAT_API_KEY_ENV));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_is_provider_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{not-json"))
        .mount(&server)
        .await;
    let endpoint = server.uri();
    let err =
        tokio::task::spawn_blocking(move || provider(endpoint, None).describe_image(png(), None))
            .await
            .unwrap()
            .unwrap_err();
    assert!(matches!(err, DocsError::VlmProvider { provider, .. } if provider == "openai-compat"));
}
