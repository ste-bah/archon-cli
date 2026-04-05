/// Local/Ollama provider implementing `LlmProvider`.
///
/// Delegates to the OpenAI-compatible API format that Ollama exposes at
/// `http://localhost:11434/v1`. Uses the same SSE parsing logic as `OpenAiProvider`.
use async_trait::async_trait;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::providers::openai::{build_openai_request_body, parse_openai_sse_chunk};
use crate::streaming::StreamEvent;
use crate::types::Usage;

// ---------------------------------------------------------------------------
// LocalProvider
// ---------------------------------------------------------------------------

pub struct LocalProvider {
    base_url: String,
    model: String,
    #[allow(dead_code)]
    timeout_secs: u64,
    pull_if_missing: bool,
    http: reqwest::Client,
}

impl LocalProvider {
    /// Create a new LocalProvider.
    pub fn new(base_url: String, model: String, timeout_secs: u64, pull_if_missing: bool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            base_url,
            model,
            timeout_secs,
            pull_if_missing,
            http,
        }
    }

    /// Return the base URL for this provider.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build the URL for the health check endpoint.
    pub fn health_check_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    /// Derive the Ollama API root from the base URL (strips `/v1` suffix).
    fn ollama_api_root(&self) -> String {
        self.base_url
            .strip_suffix("/v1")
            .unwrap_or(&self.base_url)
            .to_string()
    }

    /// Parse Ollama `/api/tags` response into `ModelInfo` list.
    pub fn parse_ollama_tags(tags_response: &serde_json::Value) -> Vec<ModelInfo> {
        tags_response
            .get("models")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|model| {
                        let name = model.get("name").and_then(|n| n.as_str())?;
                        Some(ModelInfo {
                            id: name.to_string(),
                            display_name: name.to_string(),
                            context_window: 128_000,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Delegate SSE chunk parsing to the OpenAI SSE parser.
    pub fn parse_sse_chunk(chunk: &str) -> Vec<StreamEvent> {
        parse_openai_sse_chunk(chunk)
    }

    /// Check if Ollama is running.
    ///
    /// Returns `Err` if the server is not reachable.
    pub async fn health_check(&self) -> Result<(), LlmError> {
        let url = self.health_check_url();
        self.http.get(&url).send().await.map_err(|e| {
            if e.is_connect() {
                LlmError::Http(format!(
                    "Ollama not running? Could not connect to {url}: {e}"
                ))
            } else {
                LlmError::Http(e.to_string())
            }
        })?;
        Ok(())
    }

    /// Attempt to pull the model via Ollama if `pull_if_missing` is enabled.
    async fn maybe_pull_model(&self) -> Result<(), LlmError> {
        if !self.pull_if_missing {
            return Ok(());
        }

        let api_root = self.ollama_api_root();
        let pull_url = format!("{api_root}/api/pull");
        let body = serde_json::json!({"name": self.model, "stream": false});

        // Best-effort — if pull fails, the request will fail naturally.
        let _ = self.http.post(&pull_url).json(&body).send().await;

        Ok(())
    }

    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        // Attempt to pull the model if configured and missing.
        self.maybe_pull_model().await?;

        // Use the configured model, not the one in request.model (which may be from Anthropic defaults).
        let effective_model = if request.model == "claude-sonnet-4-6" || request.model.is_empty() {
            self.model.clone()
        } else {
            request.model.clone()
        };

        let body = build_openai_request_body(
            &effective_model,
            request.max_tokens,
            &request.system,
            &request.messages,
            &request.tools,
            true,
        );

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.http.post(&url).json(&body).send().await.map_err(|e| {
            if e.is_connect() {
                LlmError::Http(format!(
                    "Ollama not running? Could not connect to {}: {e}",
                    self.base_url
                ))
            } else {
                LlmError::Http(e.to_string())
            }
        })?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let msg = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(LlmError::Server {
                status,
                message: msg,
            });
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = String::new();

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

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer.drain(..=newline_pos);

                    if line.is_empty() {
                        continue;
                    }
                    if line == "data: [DONE]" {
                        let _ = tx.send(StreamEvent::MessageStop).await;
                        return;
                    }
                    if let Some(data) = line.strip_prefix("data: ") {
                        for event in parse_openai_sse_chunk(data) {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new(
            "http://localhost:11434/v1".to_string(),
            "llama3:8b".to_string(),
            300,
            true,
        )
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for LocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn models(&self) -> Vec<ModelInfo> {
        // Return the configured model as a static list.
        // Live model discovery happens via fetch_ollama_models (async).
        vec![ModelInfo {
            id: self.model.clone(),
            display_name: self.model.clone(),
            context_window: 128_000,
        }]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
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
                    ..
                } => {
                    stop_reason = sr;
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
        match feature {
            ProviderFeature::Streaming => true,
            ProviderFeature::Thinking | ProviderFeature::PromptCaching => false,
            // ToolUse/Vision/SystemPrompt: depends on model — return true optimistically.
            ProviderFeature::ToolUse | ProviderFeature::Vision | ProviderFeature::SystemPrompt => {
                true
            }
        }
    }
}
