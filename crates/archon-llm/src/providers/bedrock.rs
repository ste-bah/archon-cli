/// AWS Bedrock Converse API provider implementing `LlmProvider`.
///
/// Uses the Bedrock Converse streaming API with SigV4 request signing.
/// Supports all Bedrock-hosted models; Claude models get additional feature flags.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use reqwest::Url;

use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use crate::providers::aws_auth::{resolve_credentials, signed_headers};
use crate::providers::bedrock_wire::{
    convert_message_to_bedrock, decode_eventstream_frames, map_http_error,
};
use crate::streaming::StreamEvent;

// Keeps the `providers::bedrock::parse_bedrock_event` path stable now that the
// implementation lives in `bedrock_wire`.
pub use crate::providers::bedrock_wire::parse_bedrock_event;

/// Whether a Bedrock model id names an Anthropic model.
///
/// Current Claude models are only reachable through a cross-region inference
/// profile, whose id carries a geo prefix (`us.anthropic....`) and may be given
/// as a full ARN. A bare `starts_with("anthropic.")` misses both forms and
/// silently drops thinking, prompt caching, and vision.
fn is_claude_model_id(model_id: &str) -> bool {
    let id = model_id.rsplit_once('/').map_or(model_id, |(_, tail)| tail);
    let base = ["us.", "eu.", "apac.", "us-gov."]
        .iter()
        .find_map(|geo| id.strip_prefix(geo))
        .unwrap_or(id);
    base.starts_with("anthropic.")
}

// Shared with the OpenAI path, which gates its own breakpoint the same way.
use crate::cache_wire::estimated_prompt_tokens;

// ---------------------------------------------------------------------------
// BedrockProvider
// ---------------------------------------------------------------------------

pub struct BedrockProvider {
    region: String,
    model_id: String,
    http: reqwest::Client,
}

#[path = "bedrock_request.rs"]
mod request_impl;

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    /// AWS documents its own minimums and they are higher than Anthropic's for
    /// several models — 4,096 rather than 1,024 on Sonnet 4.5 and Sonnet 5 —
    /// and it withholds the one-hour TTL that Opus 4.6 and Sonnet 4.6 accept on
    /// the first-party API. For traffic to this endpoint AWS is the authority,
    /// whatever the model vendor documents elsewhere.
    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        crate::cache_models::CachePlatform::Bedrock
    }

    /// Converse wants `{"cachePoint": {...}}` as its own array element, which is
    /// a different shape from every other provider's — hence its own variant
    /// rather than a reuse of the Anthropic one.
    ///
    /// Limited to Claude. Bedrock hosts models from several vendors and this
    /// only claims what is documented; an unverified model gets nothing rather
    /// than a field its API may reject on every request.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        // Resolved against `self.model_id`, not the passed model. This provider
        // dials exactly one model — the endpoint URL is built from
        // `self.model_id` regardless of what the request says — and the request
        // often says something else: an alias, a bare `claude-sonnet-4-6`
        // without the `anthropic.` vendor prefix, or nothing. Gating on the
        // request's spelling silently disabled caching for every one of those.
        let _ = model;
        if !is_claude_model_id(&self.model_id) {
            return crate::cache_strategy::CacheStrategy::None;
        }
        let params = crate::cache_models::ModelCacheTable::default()
            .lookup_on(&self.model_id, crate::cache_models::CachePlatform::Bedrock);
        crate::cache_strategy::CacheStrategy::BedrockCachePoint {
            max: params.max_checkpoints,
            min_tokens: params.min_tokens,
            ttl_1h: params.ttl_1h,
        }
    }

    fn models(&self) -> Vec<ModelInfo> {
        // Return the configured model as the available model.
        vec![ModelInfo {
            id: self.model_id.clone(),
            display_name: self.model_id.clone(),
            context_window: 0,
        }]
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::Bedrock
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        request.messages = crate::message_invariants::sanitize_anthropic_shape(request.messages);
        self.do_stream(request).await
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut rx = self.stream(request).await?;
        let mut text_parts: Vec<String> = Vec::new();
        // Issue 424: delta usage is cumulative for its message, never added.
        let mut usage = crate::usage::UsageAccumulator::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            usage.record_event(&event);
            match event {
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    usage: Some(_),
                } => {
                    stop_reason = sr;
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    ..
                } => {
                    stop_reason = sr;
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    return Err(crate::provider_fault::from_stream_error(
                        &error_type,
                        &message,
                    ));
                }
                _ => {}
            }
        }

        let full_text = text_parts.join("");
        let content = if full_text.is_empty() {
            vec![]
        } else {
            vec![serde_json::json!({"type": "text", "text": full_text})]
        };

        Ok(LlmResponse {
            content,
            usage: usage.to_usage(),
            stop_reason,
        })
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        let is_claude = self.is_claude_model();
        match feature {
            ProviderFeature::Thinking | ProviderFeature::PromptCaching => is_claude,
            ProviderFeature::ToolUse
            | ProviderFeature::Streaming
            | ProviderFeature::SystemPrompt => true,
            ProviderFeature::Vision => is_claude,
        }
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Cloud
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bedrock_token_overflow_maps_to_context_window() {
        let body = r#"{"__type":"ValidationException","message":"too many input tokens"}"#;
        assert!(matches!(
            map_http_error(400, body.to_string()),
            LlmError::ContextWindowExceeded { .. }
        ));
    }
}

#[cfg(test)]
mod connection_recovery_tests {
    use super::*;

    #[tokio::test]
    async fn calls_after_sleep_use_fresh_connections() {
        let client = BedrockProvider::new("AC-X-001".into(), "AC-X-001".into());
        crate::connection_tests::assert_fresh_connections(client.http).await;
    }
}

#[cfg(test)]
mod header_recovery_tests {
    #[tokio::test]
    async fn response_header_retains_typed_retry_after() {
        let response = crate::connection_tests::error_response(503, "90").await;
        let error = crate::transport_http::response_error(response, super::map_http_error).await;
        assert!(
            matches!(
                error,
                crate::provider::LlmError::Overloaded {
                    retry_after_secs: Some(90),
                    ..
                } | crate::provider::LlmError::Server {
                    pipeline_retry_after_secs: Some(90),
                    ..
                }
            ),
            "header lost: {error:?}"
        );
    }
}

#[cfg(test)]
#[path = "bedrock_recovery_tests.rs"]
mod recovery_http_tests;
