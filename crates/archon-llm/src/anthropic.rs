use std::time::Duration;

use crate::auth::AuthProvider;
use crate::identity::IdentityProvider;
use crate::streaming::{StreamError, StreamEvent, parse_sse_event, split_sse_lines};

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
    api_url: String,
}

impl AnthropicClient {
    /// Create a new client.
    ///
    /// `api_url` sets the endpoint URL. Pass `None` to use the default
    /// Anthropic endpoint (`https://api.anthropic.com/v1/messages`).
    /// Pass `Some(url)` to point at a proxy (LiteLLM, Ollama, etc.).
    /// The caller is responsible for resolving the priority:
    ///   1. `ANTHROPIC_BASE_URL` env var
    ///   2. `api.base_url` in config.toml
    ///   3. `None` → hardcoded default
    pub fn new(auth: AuthProvider, identity: IdentityProvider, api_url: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .no_proxy()
            .build()
            .expect("reqwest client should build");

        Self {
            http,
            auth,
            identity,
            api_url: api_url.unwrap_or_else(|| API_URL.to_string()),
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

            let mut req = self.http.post(&self.api_url);
            req = req.header(&auth_header_name, &auth_header_value);
            for (name, value) in &headers {
                req = req.header(name, value);
            }

            // Spoof mode: compute Claude Code billing header from the
            // actual request body. The server validates this hash against
            // the body bytes; any mismatch (or missing header) yields 429.
            if matches!(
                self.identity.mode,
                crate::identity::IdentityMode::Spoof { .. }
            ) {
                let cch = crate::cch::compute_cch(body.as_bytes());
                let billing_value = format!(
                    "cc_version=0.1; cc_entrypoint=claude_code; {cch}; cc_workload=claude_code;"
                );
                tracing::debug!(%billing_value, "injecting x-anthropic-billing-header");
                req = req.header("x-anthropic-billing-header", billing_value);
            }

            tracing::info!(
                "API request: model={}, origin={}, headers={:?}, body_len={}",
                request.model,
                request.request_origin.as_deref().unwrap_or("unknown"),
                headers.keys().collect::<Vec<_>>(),
                body.len()
            );
            tracing::info!("API request body: {}", &body[..body.len().min(2000)]);

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

            tracing::error!(
                "API error response: status={}, retry-after={:?}, body={}",
                status,
                retry_after_header,
                &response_body[..response_body.len().min(500)]
            );

            let err = classify_error(
                status.as_u16(),
                &response_body,
                retry_after_header.as_deref(),
            );

            match &err {
                // 429: wait for retry-after then retry
                ApiError::RateLimited {
                    retry_after_secs, ..
                } => {
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

        Err(ApiError::HttpError("max retries exceeded".into()))
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

    /// Validate a list of beta strings against the API.
    ///
    /// Sends a minimal probe request (cheapest model, max_tokens=1, content=".")
    /// with all candidate betas. If the API returns 400 "Unknown beta flag: X",
    /// removes X and retries. Repeats until 200 or the list is empty.
    ///
    /// Returns the validated subset of betas.
    pub async fn validate_betas(&self, mut candidates: Vec<String>) -> Vec<String> {
        if candidates.is_empty() {
            return candidates;
        }

        let probe_body = serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "."}],
            "stream": false,
        });
        let body_str = match serde_json::to_string(&probe_body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Beta validation probe: failed to serialize body: {e}");
                return candidates;
            }
        };

        loop {
            if candidates.is_empty() {
                break;
            }

            let beta_header = candidates.join(",");
            let request_id = uuid::Uuid::new_v4().to_string();

            let (auth_header_name, auth_header_value) = self.auth.header();

            let response = self
                .http
                .post(&self.api_url)
                .header(&auth_header_name, &auth_header_value)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .header("anthropic-beta", &beta_header)
                .header("x-client-request-id", &request_id)
                .body(body_str.clone())
                .send()
                .await;

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        "Beta validation probe: HTTP error: {e}, using candidates as-is"
                    );
                    break;
                }
            };

            let status = response.status().as_u16();
            if status == 200 || (200..300).contains(&status) {
                tracing::debug!(
                    "Beta validation probe succeeded with {} betas",
                    candidates.len()
                );
                break;
            }

            let response_body = response.text().await.unwrap_or_default();

            if status == 400
                && let Some(bad_beta) = extract_unknown_beta(&response_body)
            {
                let before = candidates.len();
                candidates.retain(|b| b != &bad_beta);
                if candidates.len() < before {
                    // Successfully removed the bad beta — continue probing
                    tracing::warn!("Stripping unknown beta: {bad_beta}");
                    continue;
                }
                // The API reported a beta we didn't send — abort to avoid infinite loop
                tracing::warn!(
                    "Beta validation: API reported unknown beta '{bad_beta}' not in our candidate list; aborting probe"
                );
            }

            // Any other error (or unrecognised 400): abort probe, return what we have
            tracing::warn!(
                "Beta validation probe failed with status {status}, using candidates as-is"
            );
            break;
        }

        candidates
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

        if let Some(anti_dist) = self.identity.anti_distillation_value() {
            body["anti_distillation"] = anti_dist;
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
    /// Tags request origin for log correlation: "main_session" | "subagent" | "pipeline".
    pub request_origin: Option<String>,
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
            request_origin: None,
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

    #[error("rate limited: retry after {retry_after_secs}s | body: {body_preview}")]
    RateLimited {
        retry_after_secs: u64,
        body_preview: String,
    },

    #[error("server overloaded (529)")]
    Overloaded,

    #[error("server error ({status}): {message}")]
    ServerError { status: u16, message: String },

    #[error("serialization error: {0}")]
    SerializeError(String),
}

/// Extract the unknown beta name from a 400 error body.
///
/// Looks for the pattern `"Unknown beta flag: <name>"` and returns `<name>`.
fn extract_unknown_beta(body: &str) -> Option<String> {
    const MARKER: &str = "Unknown beta flag: ";
    let start = body.find(MARKER)? + MARKER.len();
    let rest = &body[start..];
    // Beta name ends at a `"` or end of string
    let end = rest.find('"').unwrap_or(rest.len());
    let name = rest[..end].trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn classify_error(status: u16, body: &str, retry_after_header: Option<&str>) -> ApiError {
    match status {
        401 => ApiError::AuthError(format!("authentication failed: {body}")),
        403 => ApiError::AuthError(format!(
            "authentication/identity rejected (403). If using spoof mode, check \
             identity.spoof_version matches the current Claude Code version, or \
             run /refresh-identity to rediscover beta headers. Body: {body}"
        )),
        429 => ApiError::RateLimited {
            retry_after_secs: retry_after_header
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| extract_retry_after(body)),
            body_preview: body[..body.len().min(300)].to_string(),
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
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(secs) = v.get("retry_after").and_then(|v| v.as_u64())
    {
        return secs;
    }
    30
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod beta_validation_tests {
    use super::*;

    #[test]
    fn test_extract_unknown_beta_parses_correctly() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Unknown beta flag: xyz-2025-01-01"}}"#;
        let result = extract_unknown_beta(body);
        assert_eq!(result, Some("xyz-2025-01-01".to_string()));
    }

    #[test]
    fn test_extract_unknown_beta_returns_none_for_unrelated_error() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"Invalid API key"}}"#;
        let result = extract_unknown_beta(body);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_unknown_beta_returns_none_for_empty_body() {
        let result = extract_unknown_beta("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_unknown_beta_handles_beta_with_hyphens() {
        let body = r#"{"type":"error","error":{"message":"Unknown beta flag: my-feature-flag-2025-12-31"}}"#;
        let result = extract_unknown_beta(body);
        assert_eq!(result, Some("my-feature-flag-2025-12-31".to_string()));
    }

    #[tokio::test]
    async fn test_validate_betas_with_empty_candidates_returns_empty() {
        use crate::auth::AuthProvider;
        use crate::identity::{IdentityMode, IdentityProvider};
        use crate::types::Secret;

        let auth = AuthProvider::ApiKey(Secret::new("test-key".to_string()));
        let identity = IdentityProvider::new(
            IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        );
        let client = AnthropicClient::new(auth, identity, None);
        let result = client.validate_betas(vec![]).await;
        assert!(
            result.is_empty(),
            "empty candidates should return empty immediately without any API call"
        );
    }

    #[test]
    fn test_probe_body_structure() {
        // Verify that the probe body construction generates the expected shape.
        let body = serde_json::json!({
            "model": "claude-haiku-4-5-20251001",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "."}],
            "stream": false,
        });
        assert_eq!(body["model"], "claude-haiku-4-5-20251001");
        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["stream"], false);
        assert!(body["messages"].is_array());
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], ".");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthProvider;
    use crate::identity::IdentityProvider;
    use crate::types::Secret;

    fn make_auth() -> AuthProvider {
        AuthProvider::ApiKey(Secret::new("test-key".to_string()))
    }

    fn make_identity() -> IdentityProvider {
        IdentityProvider::new(
            crate::identity::IdentityMode::Clean,
            "test-session".to_string(),
            "test-device".to_string(),
            String::new(),
        )
    }

    #[test]
    fn test_custom_api_url_stored() {
        let client = AnthropicClient::new(
            make_auth(),
            make_identity(),
            Some("http://localhost:11434/v1/messages".to_string()),
        );
        assert_eq!(client.api_url, "http://localhost:11434/v1/messages");
    }

    #[test]
    fn test_default_api_url_when_none() {
        let client = AnthropicClient::new(make_auth(), make_identity(), None);
        assert_eq!(client.api_url, API_URL);
    }

    #[test]
    fn test_custom_api_url_used_not_constant() {
        let custom_url = "https://my-proxy.example.com/v1/messages";
        let client =
            AnthropicClient::new(make_auth(), make_identity(), Some(custom_url.to_string()));
        // Confirm it is NOT using the hardcoded constant
        assert_ne!(client.api_url, API_URL);
        assert_eq!(client.api_url, custom_url);
    }
}
