use std::time::Duration;

use crate::auth::AuthProvider;
use crate::identity::IdentityProvider;
use crate::streaming::{parse_sse_event, split_sse_lines, StreamError, StreamEvent};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MAX_RETRIES: u32 = 3;
const OVERLOAD_BASE_DELAY_SECS: u64 = 10;

// ---------------------------------------------------------------------------
// API client
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    http: reqwest::Client,
    auth: AuthProvider,
    identity: IdentityProvider,
}

impl AnthropicClient {
    pub fn new(auth: AuthProvider, identity: IdentityProvider) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .no_proxy()
            .build()
            .expect("reqwest client should build");

        Self {
            http,
            auth,
            identity,
        }
    }

    /// Get a reference to the auth provider.
    pub fn auth(&self) -> &AuthProvider {
        &self.auth
    }

    /// Get a reference to the identity provider.
    pub fn identity(&self) -> &IdentityProvider {
        &self.identity
    }

    /// Send a streaming messages request with automatic retry on 429/5xx.
    pub async fn stream_message(
        &self,
        request: MessageRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, ApiError> {
        let body = self.build_request_body(&request)?;

        for attempt in 0..=MAX_RETRIES {
            let request_id = uuid::Uuid::new_v4().to_string();
            let mut headers = self.identity.request_headers(&request_id);

            // Inject fast mode / effort beta headers if needed
            let mut extra_betas: Vec<&str> = Vec::new();
            if request.speed.is_some() {
                extra_betas.push("fast-mode-2026-02-01");
            }
            if request.effort.is_some() {
                extra_betas.push("effort-2025-11-24");
            }
            if !extra_betas.is_empty() {
                let existing = headers.get("anthropic-beta").cloned().unwrap_or_default();
                let combined = if existing.is_empty() {
                    extra_betas.join(",")
                } else {
                    format!("{},{}", existing, extra_betas.join(","))
                };
                headers.insert("anthropic-beta".into(), combined);
            }

            let (auth_header_name, auth_header_value) = self.auth.header();

            let mut req = self.http.post(API_URL);
            req = req.header(&auth_header_name, &auth_header_value);
            for (name, value) in &headers {
                req = req.header(name, value);
            }

            tracing::debug!(
                "API request: model={}, headers={:?}, body_len={}",
                request.model,
                headers.keys().collect::<Vec<_>>(),
                body.len()
            );
            tracing::debug!("API request body: {}", &body[..body.len().min(2000)]);

            let response = req
                .body(body.clone())
                .send()
                .await
                .map_err(|e| ApiError::HttpError(format!("request failed: {e}")))?;

            let status = response.status();

            if status.is_success() {
                return self.spawn_stream_reader(response).await;
            }

            // Log full error details for debugging
            let retry_after_header = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let response_body = response.text().await.unwrap_or_default();

            tracing::debug!(
                "API error response: status={}, retry-after={:?}, body={}",
                status,
                retry_after_header,
                &response_body[..response_body.len().min(500)]
            );

            let err = classify_error(status.as_u16(), &response_body, retry_after_header.as_deref());

            match &err {
                // 429: wait for retry-after then retry
                ApiError::RateLimited { retry_after_secs } => {
                    if attempt < MAX_RETRIES {
                        let delay = *retry_after_secs;
                        tracing::warn!(
                            "rate limited, retrying in {delay}s (attempt {}/{})",
                            attempt + 1,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                    return Err(err);
                }

                // 529: overloaded, use longer backoff
                ApiError::Overloaded => {
                    if attempt < MAX_RETRIES {
                        let delay = OVERLOAD_BASE_DELAY_SECS * (attempt as u64 + 1);
                        tracing::warn!(
                            "server overloaded, retrying in {delay}s (attempt {}/{})",
                            attempt + 1,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                    return Err(err);
                }

                // 500/502/503: exponential backoff
                ApiError::ServerError { status, .. } => {
                    if attempt < MAX_RETRIES {
                        let delay = 2u64.pow(attempt) * 2; // 2s, 4s, 8s
                        tracing::warn!(
                            "server error {status}, retrying in {delay}s (attempt {}/{})",
                            attempt + 1,
                            MAX_RETRIES
                        );
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }
                    return Err(err);
                }

                // 401, other errors: don't retry
                _ => return Err(err),
            }
        }

        Err(ApiError::HttpError(
            "max retries exceeded".into(),
        ))
    }

    async fn spawn_stream_reader(
        &self,
        response: reqwest::Response,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, ApiError> {
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        tokio::spawn(async move {
            let text = response.text().await.unwrap_or_default();
            let pairs = split_sse_lines(&text);
            for (event_type, data) in pairs {
                match parse_sse_event(event_type, data) {
                    Ok(event) => {
                        if tx.send(event).await.is_err() {
                            break;
                        }
                    }
                    Err(StreamError::UnknownEvent(_)) => {}
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type: "parse_error".into(),
                                message: format!("{e}"),
                            })
                            .await;
                    }
                }
            }
        });

        Ok(rx)
    }

    fn build_request_body(&self, request: &MessageRequest) -> Result<String, ApiError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "stream": true,
            "messages": request.messages,
        });

        if !request.system.is_empty() {
            body["system"] = serde_json::json!(request.system);
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(request.tools);
        }

        if let Some(ref thinking) = request.thinking {
            body["thinking"] = serde_json::json!(thinking);
        }

        // GAP 3: Inject speed parameter when fast mode is active
        if let Some(ref speed) = request.speed {
            body["speed"] = serde_json::json!(speed);
        }

        // GAP 4: Inject output_config.effort when not default (High)
        if let Some(ref effort) = request.effort {
            body["output_config"] = serde_json::json!({ "effort": effort });
        }

        let metadata = self.identity.metadata();
        if !metadata.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            body["metadata"] = metadata;
        }

        serde_json::to_string(&body).map_err(|e| ApiError::SerializeError(format!("{e}")))
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MessageRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<serde_json::Value>,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub thinking: Option<serde_json::Value>,
    /// When fast mode is active, set to `Some("fast")`.
    pub speed: Option<String>,
    /// When effort is not High, set to the effort level string (e.g. `"low"`, `"medium"`).
    pub effort: Option<String>,
}

impl Default for MessageRequest {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            max_tokens: 8192,
            system: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            thinking: None,
            speed: None,
            effort: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("authentication error: {0}")]
    AuthError(String),

    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

    #[error("server overloaded (529)")]
    Overloaded,

    #[error("server error ({status}): {message}")]
    ServerError { status: u16, message: String },

    #[error("serialization error: {0}")]
    SerializeError(String),
}

fn classify_error(status: u16, body: &str, retry_after_header: Option<&str>) -> ApiError {
    match status {
        401 => ApiError::AuthError(format!("authentication failed: {body}")),
        429 => ApiError::RateLimited {
            retry_after_secs: retry_after_header
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| extract_retry_after(body)),
        },
        529 => ApiError::Overloaded,
        500 | 502 | 503 => ApiError::ServerError {
            status,
            message: body.to_string(),
        },
        _ => ApiError::HttpError(format!("HTTP {status}: {body}")),
    }
}

fn extract_retry_after(body: &str) -> u64 {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(secs) = v.get("retry_after").and_then(|v| v.as_u64()) {
            return secs;
        }
    }
    30
}
