use super::*;
use crate::anthropic::AnthropicClient;
use crate::auth::AuthProvider;
use crate::cache_models::CachePlatform;
use crate::cache_strategy::CacheStrategy;
use crate::compaction_policy::{CompactionBackend, CompactionPolicy, ProviderFamily, WireShape};
use crate::identity::{IdentityMode, IdentityProvider};
use std::sync::atomic::{AtomicUsize, Ordering};

struct StreamScriptProvider {
    calls: AtomicUsize,
    events: Vec<StreamEvent>,
}

#[async_trait]
impl LlmProvider for StreamScriptProvider {
    fn name(&self) -> &str {
        "stream-script"
    }
    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        true
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = tokio::sync::mpsc::channel(self.events.len().max(1));
        for event in self.events.clone() {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }
}

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

#[tokio::test]
async fn retry_provider_releases_stream_on_keep_alive_without_waiting_for_content() {
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    let probe =
        tokio::spawn(RetryProvider::<StreamScriptProvider>::drain_until_content_or_error(rx));

    tx.send(StreamEvent::Ping).await.unwrap();
    let forwarded = tokio::time::timeout(std::time::Duration::from_secs(1), probe)
        .await
        .expect("keep-alive must release the wrapper while the response is still open")
        .unwrap();
    let StreamProbe::Usable(mut forwarded) = forwarded else {
        panic!("keep-alive should make the stream usable");
    };
    assert!(matches!(forwarded.recv().await, Some(StreamEvent::Ping)));
}

struct MetadataProbeProvider {
    anthropic: AnthropicClient,
}

#[async_trait]
impl LlmProvider for MetadataProbeProvider {
    fn name(&self) -> &str {
        "metadata-probe"
    }
    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "probe-model".into(),
            display_name: "Probe".into(),
            context_window: 1,
        }]
    }
    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        unreachable!()
    }
    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }
    fn request_identity(&self, _: &LlmRequest) -> Option<String> {
        Some("probe-identity".into())
    }
    fn supports_temperature(&self) -> bool {
        true
    }
    fn supports_feature(&self, _: ProviderFeature) -> bool {
        true
    }
    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Cloud
    }
    fn compaction_provider_family(&self) -> ProviderFamily {
        ProviderFamily::AnthropicOAuth
    }
    fn compaction_policy(&self) -> CompactionPolicy {
        CompactionPolicy {
            provider_family: ProviderFamily::AnthropicOAuth,
            wire_shape: WireShape::AnthropicMessages,
            backend: CompactionBackend::Anthropic,
            generic_fallback: true,
            max_tool_result_bytes: Some(123_456),
        }
    }
    fn resolve_alias(&self, alias: &str) -> Option<String> {
        (alias == "probe-alias").then(|| "probe-concrete-model".into())
    }
    fn resolve_request_model(&self, request: &mut LlmRequest) {
        request.model = format!("resolved:{}", request.model);
    }
    fn cache_strategy(&self, _: &str) -> CacheStrategy {
        CacheStrategy::AnthropicBreakpoints {
            max: 7,
            min_tokens: 321,
            ttl_1h: true,
        }
    }
    fn cache_platform(&self) -> CachePlatform {
        CachePlatform::Vertex
    }
    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        Some(&self.anthropic)
    }
}

#[test]
fn retry_provider_forwards_provider_metadata() {
    let anthropic = AnthropicClient::new(
        AuthProvider::keyless(),
        IdentityProvider::new(
            IdentityMode::Clean,
            "session".into(),
            "device".into(),
            "account".into(),
        ),
        None,
    );
    let inner = Arc::new(MetadataProbeProvider { anthropic });
    let wrapped = RetryProvider::new(inner.clone(), RetryPolicy::default());

    assert_eq!(wrapped.name(), "metadata-probe");
    assert_eq!(wrapped.models()[0].id, inner.models()[0].id);
    assert!(wrapped.supports_temperature());
    assert!(wrapped.supports_feature(ProviderFeature::ToolUse));
    assert_eq!(
        wrapped.request_identity(&request()).as_deref(),
        Some("probe-identity")
    );
    assert_eq!(
        wrapped.data_flow_classification(),
        DataFlowClassification::Cloud
    );
    assert_eq!(
        wrapped.compaction_provider_family(),
        ProviderFamily::AnthropicOAuth
    );
    assert_eq!(wrapped.compaction_policy(), inner.compaction_policy());
    assert_eq!(
        wrapped.resolve_alias("probe-alias").as_deref(),
        Some("probe-concrete-model")
    );
    let mut resolved = request();
    wrapped.resolve_request_model(&mut resolved);
    assert_eq!(resolved.model, "resolved:test");
    assert_eq!(
        wrapped.cache_strategy("probe-model"),
        inner.cache_strategy("probe-model")
    );
    assert_eq!(wrapped.cache_platform(), CachePlatform::Vertex);
    assert!(std::ptr::eq(
        wrapped.as_anthropic().unwrap(),
        &inner.anthropic
    ));
}

#[test]
fn retry_provider_impl_mentions_every_trait_method() {
    let trait_source = include_str!("../provider.rs");
    let wrapper_source = include_str!("../retry.rs");
    let trait_body = trait_source
        .split("pub trait LlmProvider:")
        .nth(1)
        .and_then(|body| body.split("\n}").next())
        .expect("LlmProvider trait source should be present");
    let wrapper_impl = wrapper_source
        .split("impl<P: LlmProvider + ?Sized> LlmProvider for RetryProvider<P>")
        .nth(1)
        .and_then(|body| body.split("\n}").next())
        .expect("RetryProvider implementation source should be present");
    for line in trait_body.lines() {
        let line = line.trim();
        let Some(method) = line
            .strip_prefix("fn ")
            .or_else(|| line.strip_prefix("async fn "))
        else {
            continue;
        };
        let method = method.split('(').next().expect("method name");
        assert!(
            wrapper_impl.contains(&format!("fn {method}("))
                || wrapper_impl.contains(&format!("async fn {method}(")),
            "RetryProvider does not explicitly implement LlmProvider::{method}"
        );
    }
}

#[tokio::test]
async fn streaming_error_redacts_after_content_and_leaves_retries_to_the_runner() {
    let secret = format!("sk-ant-api03-{}", uuid::Uuid::new_v4().simple());
    let inner = Arc::new(StreamScriptProvider {
        calls: AtomicUsize::new(0),
        events: vec![
            StreamEvent::TextDelta {
                index: 0,
                text: "answer".into(),
            },
            StreamEvent::Error {
                error_type: "overloaded_error".into(),
                message: format!("api_key={secret}").into(),
            },
        ],
    });
    let provider = RetryProvider::new(inner.clone(), RetryPolicy::default());
    let mut events = provider.stream(request()).await.unwrap();
    let mut forwarded = Vec::new();
    while let Some(event) = events.recv().await {
        forwarded.push(event);
    }
    let text = format!("{forwarded:?}");
    println!(
        "EVIDENCE #364 post-content-error BEFORE: provider event contained generated api_key AFTER: {text}"
    );
    assert!(
        !text.contains(&secret),
        "event leaked generated key: {text}"
    );
    assert!(
        text.contains("REDACTED"),
        "event should carry redacted marker: {text}"
    );
    assert_eq!(
        inner.calls.load(Ordering::SeqCst),
        1,
        "stream wrapper must not retry"
    );
}

#[tokio::test]
async fn persistent_overload_is_forwarded_for_runner_resend() {
    let inner = Arc::new(StreamScriptProvider {
        calls: AtomicUsize::new(0),
        events: vec![StreamEvent::Error {
            error_type: "overloaded_error".into(),
            message: "busy".into(),
        }],
    });
    let provider = RetryProvider::new(inner.clone(), RetryPolicy::default());
    let mut events = provider.stream(request()).await.unwrap();
    assert!(matches!(
        events.recv().await,
        Some(StreamEvent::Error { error_type, .. }) if error_type == "overloaded_error"
    ));
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rejected_stream_error_fails_fast_with_redacted_text() {
    let fake_key = format!("sk-ant-api03-{}", uuid::Uuid::new_v4().simple());
    let inner = Arc::new(StreamScriptProvider {
        calls: AtomicUsize::new(0),
        events: vec![StreamEvent::Error {
            error_type: "authentication_error".into(),
            message: format!("api_key={fake_key}").into(),
        }],
    });
    let provider = RetryProvider::new(inner.clone(), RetryPolicy::default());
    let error = provider.stream(request()).await.unwrap_err();
    let displayed = error.to_string();
    println!(
        "EVIDENCE #364 stream-rejection BEFORE: generated key in provider event AFTER: {displayed}"
    );
    assert!(matches!(error, LlmError::Rejected { .. }));
    assert!(
        !displayed.contains(&fake_key),
        "rejected error leaked: {displayed}"
    );
    assert_eq!(inner.calls.load(Ordering::SeqCst), 1);
}

struct AnthropicCacheProvider;

#[async_trait]
impl LlmProvider for AnthropicCacheProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    fn cache_strategy(&self, _model: &str) -> crate::cache_strategy::CacheStrategy {
        crate::cache_strategy::ANTHROPIC_API
    }

    async fn complete(&self, _: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!()
    }

    async fn stream(&self, _: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        unreachable!()
    }
}

/// The retry wrapper must forward the strategy. Dropping it here would
/// disable caching for every retried request without any error.
#[test]
fn retry_provider_preserves_anthropic_message_cache_capability() {
    let provider = RetryProvider::new(Arc::new(AnthropicCacheProvider), RetryPolicy::default());

    assert_eq!(
        provider.cache_strategy("claude-sonnet-4-6"),
        crate::cache_strategy::ANTHROPIC_API
    );
}

#[test]
fn protocol_retry_requires_the_original_eof_message_and_transport_idle_stays_separate() {
    assert!(stream_error_is_retryable(
        "protocol",
        "stream ended before message_stop"
    ));
    assert!(!stream_error_is_retryable(
        "protocol",
        "invalid protocol payload"
    ));
    assert!(!stream_error_is_retryable(
        "transport_idle",
        "provider transport stalled"
    ));
    assert!(stream_error_is_retryable("network", "connection reset"));
    assert!(stream_error_is_retryable(
        "http_error",
        "error decoding response body"
    ));
}

#[test]
fn decided_sse_error_types_and_unknown_sse_events_enter_the_stream_retry_window() {
    for error_type in [
        "parse_error",
        "timeout_error",
        "api_error",
        "overloaded_error",
    ] {
        assert!(
            stream_error_is_retryable(error_type, "provider event"),
            "{error_type} should enter the retry window"
        );
    }
    assert!(stream_error_is_retryable(
        "future_provider_event",
        "unknown"
    ));

    for error_type in [
        "server_error",
        "rate_limited",
        "rate_limit_error",
        "authentication_error",
        "invalid_request_error",
    ] {
        assert!(
            !stream_error_is_retryable(error_type, "provider event"),
            "{error_type} should stay on the existing error route"
        );
    }
}
