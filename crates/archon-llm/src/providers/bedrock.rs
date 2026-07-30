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
    convert_message_to_bedrock, extract_bedrock_events, map_http_error,
};
use crate::streaming::StreamEvent;
use crate::types::Usage;

// Keeps the `providers::bedrock::parse_bedrock_event` path stable now that the
// implementation lives in `bedrock_wire`.
pub use crate::providers::bedrock_wire::parse_bedrock_event;

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
        // System prompt: array of {text: "..."} objects.
        let system_arr: Vec<serde_json::Value> = system
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|text| serde_json::json!({"text": text}))
            })
            .collect();

        // Messages: convert Archon format to Bedrock Converse format.
        let bedrock_messages: Vec<serde_json::Value> =
            messages.iter().map(convert_message_to_bedrock).collect();

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
            let bedrock_tools: Vec<serde_json::Value> = tools
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

    /// Whether `model_id` names an Anthropic model.
    ///
    /// Current Claude models are only reachable through a cross-region
    /// inference profile, whose ID carries a geo prefix (`us.anthropic....`)
    /// and may be given as a full ARN. A bare `starts_with("anthropic.")` misses
    /// both forms and silently drops thinking, prompt caching, and vision.
    fn is_claude_model(&self) -> bool {
        let id = self
            .model_id
            .rsplit_once('/')
            .map_or(self.model_id.as_str(), |(_, tail)| tail);
        let base = ["us.", "eu.", "apac.", "us-gov."]
            .iter()
            .find_map(|geo| id.strip_prefix(geo))
            .unwrap_or(id);
        base.starts_with("anthropic.")
    }

    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let creds = resolve_credentials().await?;

        let body = Self::build_converse_body(
            &request.system,
            &request.messages,
            &request.tools,
            request.max_tokens,
        );

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Serialize(e.to_string()))?;

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

                // Bedrock sends JSON events. Try to extract complete JSON objects.
                let text = String::from_utf8_lossy(&buffer).to_string();
                let (events, consumed) = extract_bedrock_events(&text);
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
                let text = String::from_utf8_lossy(&buffer).to_string();
                let (events, _) = extract_bedrock_events(&text);
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
