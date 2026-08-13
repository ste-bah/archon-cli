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
use crate::types::Usage;

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

impl BedrockProvider {
    /// Create a new Bedrock provider.
    pub fn new(region: String, model_id: String) -> Self {
        Self {
            region,
            model_id,
            http: reqwest::Client::new(),
        }
    }

    /// Build the Bedrock Converse request body.
    pub fn build_converse_body(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        max_tokens: u32,
    ) -> serde_json::Value {
        Self::build_converse_body_cached(system, messages, tools, max_tokens, None)
    }

    /// Build the Converse body, optionally placing prompt-cache checkpoints.
    ///
    /// Converse takes a checkpoint as its own array element rather than as an
    /// attribute on a neighbouring block, so a checkpoint is appended after the
    /// content it should cache:
    ///
    /// ```json
    /// { "cachePoint": { "type": "default", "ttl": "1h" } }
    /// ```
    ///
    /// Placement follows Bedrock's own evaluation order, `tools` -> `system` ->
    /// `messages`. The sections are chained: changing `tools` invalidates the
    /// `system` and `messages` caches, so the checkpoints go after the most
    /// stable content first. The messages checkpoint deliberately lands on the
    /// last message, which advances each turn and keeps the growing prefix
    /// cached behind it.
    pub fn build_converse_body_cached(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        max_tokens: u32,
        cache: Option<crate::cache_strategy::CachePointPlacement>,
    ) -> serde_json::Value {
        // System prompt: array of {text: "..."} objects.
        let mut system_arr: Vec<serde_json::Value> = system
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|text| serde_json::json!({"text": text}))
            })
            .collect();

        // Messages: convert Archon format to Bedrock Converse format.
        //
        // `filter_map`, because a message with no Converse-representable content
        // must be dropped rather than sent: Converse rejects an empty content
        // field and fails the entire request, so one such message makes every
        // subsequent turn in the session fail too.
        let mut bedrock_messages: Vec<serde_json::Value> = messages
            .iter()
            .filter_map(convert_message_to_bedrock)
            .collect();

        if let Some(cache) = cache {
            if cache.system && !system_arr.is_empty() {
                // After the stable head, not after everything.
                //
                // Archon appends per-turn content to the end of the system
                // prompt — recalled memories, the inner voice, reminders — and
                // that changes most turns. A checkpoint behind it is rewritten
                // every turn and almost never read, and a cache write bills at
                // 1.25x plain input (2x at the one-hour TTL), so it costs more
                // than not caching at all.
                //
                // Clamped, because `system_arr` is built by `filter_map` over
                // the caller's blocks: anything without a `text` field is
                // dropped, so the index can be shorter than the caller's count.
                // Out of range falls back to the end, which is the old
                // behaviour and always valid.
                let at = cache
                    .system_stable_blocks
                    .filter(|n| *n > 0 && *n < system_arr.len())
                    .unwrap_or(system_arr.len());
                system_arr.insert(at, cache.point());
            }
            if cache.messages
                && let Some(last) = bedrock_messages.last_mut()
                && let Some(content) = last
                    .get_mut("content")
                    .and_then(serde_json::Value::as_array_mut)
                && !content.is_empty()
            {
                content.push(cache.point());
            }
        }

        let mut body = serde_json::json!({
            "inferenceConfig": {
                "maxTokens": max_tokens
            },
            "messages": bedrock_messages
        });

        if !system_arr.is_empty() {
            body["system"] = serde_json::Value::Array(system_arr);
        }

        if !tools.is_empty() {
            let mut bedrock_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    let name = tool.get("name").cloned().unwrap_or(serde_json::Value::Null);
                    let description = tool
                        .get("description")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let input_schema = tool
                        .get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({"type": "object"}));
                    serde_json::json!({
                        "toolSpec": {
                            "name": name,
                            "description": description,
                            "inputSchema": {
                                "json": input_schema
                            }
                        }
                    })
                })
                .collect();

            // Tools are the first section Bedrock evaluates and the most stable
            // thing Archon sends, so caching them is the largest single saving
            // available — and changing them invalidates everything downstream.
            if let Some(cache) = cache
                && cache.tools
            {
                bedrock_tools.push(cache.point());
            }

            body["toolConfig"] = serde_json::json!({"tools": bedrock_tools});
        }

        body
    }

    /// Build the Bedrock runtime URL for this provider.
    fn endpoint_url(&self) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse-stream",
            self.region,
            urlencoding::encode(&self.model_id)
        )
    }

    /// Where to place checkpoints for this request, if anywhere.
    ///
    /// The decision is not made here. It arrives resolved on the request, under
    /// [`crate::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY`] in `extra`, where
    /// `archon-core` put it after weighing everything this provider cannot see:
    /// `prompt_cache` on or off, the emission mode, the configured TTL, and any
    /// `[context.prompt_cache_models]` override. No directive means no
    /// checkpoints — an earlier draft decided down here from the model table
    /// alone, which meant `prompt_cache = false` did not turn caching off and
    /// the five-minute TTL preference was ignored in favour of one-hour writes,
    /// the expensive tier, on every request.
    ///
    /// What is decided here is what only the wire layer knows: whether the
    /// prompt clears the minimum. Bedrock measures that against `tools`,
    /// `system` and `messages` **combined** rather than per section, so the
    /// estimate spans all three. A checkpoint under the floor is discarded
    /// without an error, so emitting one anyway would not fail — it would just
    /// put fields on the wire that do nothing and report a cache that was never
    /// written.
    pub fn cache_placement(
        &self,
        request: &LlmRequest,
    ) -> Option<crate::cache_strategy::CachePointPlacement> {
        let directive = request
            .extra
            .get(crate::cache_strategy::BEDROCK_CACHE_DIRECTIVE_KEY)?;
        // Belt and braces: the directive is only written for a BedrockCachePoint
        // strategy, which requires Claude — but the provider is the last line
        // before the wire, and a non-Claude body with a cachePoint is a 400.
        if !is_claude_model_id(&self.model_id) {
            return None;
        }
        let max = directive.get("max")?.as_u64()? as usize;
        let min_tokens = directive.get("min_tokens")?.as_u64()? as usize;
        let ttl_1h = directive
            .get("ttl_1h")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if estimated_prompt_tokens(&request.system, &request.messages, &request.tools) < min_tokens
        {
            return None;
        }

        let stable_system_blocks = directive
            .get("stable_system_blocks")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n as usize);

        let mut placement = crate::cache_strategy::CachePointPlacement::all(ttl_1h)
            .with_stable_system_blocks(stable_system_blocks);
        // `prompt_cache_conversation = false` declines a checkpoint on the
        // message history and nothing else. Absent means yes, so a directive
        // written before this field existed keeps its old behaviour.
        placement.messages = directive
            .get("conversation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        // Exceeding the checkpoint limit is a hard 400, unlike falling short of
        // the minimum. Drop from the least stable section first: the messages
        // checkpoint moves every turn and is worth the least.
        if placement.count() > max {
            placement.messages = false;
        }
        if placement.count() > max {
            placement.system = false;
        }
        (placement.count() > 0).then_some(placement)
    }

    /// Whether `model_id` names an Anthropic model.
    ///
    /// Current Claude models are only reachable through a cross-region
    /// inference profile, whose ID carries a geo prefix (`us.anthropic....`)
    /// and may be given as a full ARN. A bare `starts_with("anthropic.")` misses
    /// both forms and silently drops thinking, prompt caching, and vision.
    fn is_claude_model(&self) -> bool {
        is_claude_model_id(&self.model_id)
    }

    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let creds = resolve_credentials().await?;

        let body = Self::build_converse_body_cached(
            &request.system,
            &request.messages,
            &request.tools,
            request.max_tokens,
            self.cache_placement(&request),
        );

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Serialize(e.to_string()))?;

        // The one place a missing or misplaced `cachePoint` is visible. Bedrock
        // discards a checkpoint under the model's minimum without an error, so
        // a body that looks right and a body that silently caches nothing are
        // indistinguishable from the response alone.
        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!(
                provider = "bedrock",
                model_id = %self.model_id,
                "Bedrock Converse request body: {}",
                crate::debug_body::debug_body(&String::from_utf8_lossy(&body_bytes))
            );
        }

        let url = self.endpoint_url();
        let url_parsed =
            Url::parse(&url).map_err(|e| LlmError::Http(format!("invalid URL: {e}")))?;
        let host = url_parsed.host_str().unwrap_or("").to_string();
        let path = url_parsed.path().to_string();

        let signed = signed_headers(&creds, &host, &path, &self.region, &body_bytes);

        let mut req = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .header("x-amz-date", &signed.x_amz_date)
            .header("authorization", &signed.authorization);

        // Temporary credentials are only accepted with the session token
        // attached; it is part of the signature computed above.
        if let Some(ref token) = signed.security_token {
            req = req.header("x-amz-security-token", token);
        }

        let resp = req
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let msg = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(map_http_error(status, msg));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = Vec::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type: "http_error".to_string(),
                                message: e.to_string(),
                            })
                            .await;
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk);

                // ConverseStream replies in `application/vnd.amazon.eventstream`
                // — binary frames whose `:event-type` header names the event.
                // This used to run `String::from_utf8_lossy` over those frames
                // and scan for JSON, which found the payloads but never the
                // wrapper key `parse_bedrock_event` matches on, so no text ever
                // reached the caller and every turn ended empty without error.
                let (events, consumed) = decode_eventstream_frames(&buffer);
                if consumed > 0 {
                    buffer.drain(..consumed);
                }

                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }

            // Drain any remaining buffer content.
            if !buffer.is_empty() {
                let (events, _) = decode_eventstream_frames(&buffer);
                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

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
        let mut usage = Usage::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::MessageStart { usage: u, .. } => {
                    usage.merge(&u);
                }
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    usage: Some(u),
                } => {
                    stop_reason = sr;
                    usage.merge(&u);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    ..
                } => {
                    stop_reason = sr;
                }
                StreamEvent::MessageDelta { usage: Some(u), .. } => {
                    usage.merge(&u);
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
            usage,
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
