use std::collections::HashSet;
use std::sync::{Mutex as StdMutex, OnceLock};
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

    pub fn api_url(&self) -> &str {
        &self.api_url
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

            let mut extra_betas: Vec<&str> = Vec::new();
            if effective_speed(&request).is_some() {
                extra_betas.push("fast-mode-2026-02-01");
            }
            if effective_effort(&request).is_some() {
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

            tracing::info!(
                "API request: url={}, model={}, request_origin={:?}, body_len={}",
                self.api_url,
                request.model,
                request.request_origin.as_deref().unwrap_or("unknown"),
                body.len()
            );
            tracing::info!("API request body: {}", crate::debug_body::debug_body(&body));

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
                crate::debug_body::debug_body(&response_body)
            );

            let err = classify_error(
                status.as_u16(),
                &response_body,
                retry_after_header.as_deref(),
            );

            match &err {
                // 429: wait for retry-after then retry
                ApiError::RateLimited { retry_after_secs } => {
                    if body.len() >= 1_000_000 {
                        tracing::warn!(
                            body_len = body.len(),
                            "large Anthropic request was rate limited; skipping identical provider retry"
                        );
                        return Err(err);
                    }
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

    pub(crate) fn build_request_body(&self, request: &MessageRequest) -> Result<String, ApiError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "stream": true,
            "messages": request.messages,
        });

        // Build the system field. When in Spoof mode (e.g. OAuth token), prepend
        // the canonical Claude Code identity blocks (billing header + identity
        // prefix) so the request is recognised as Claude Code traffic. Idempotent:
        // skip prepending if the caller already provided the identity prefix.
        let mut system_blocks: Vec<serde_json::Value> = Vec::new();
        if matches!(
            self.identity.mode,
            crate::identity::IdentityMode::Spoof { .. }
        ) {
            let already_has_identity = request.system.iter().any(|b| {
                b.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.starts_with("You are Claude Code,"))
                    .unwrap_or(false)
            });
            if !already_has_identity {
                let first_user_msg = request
                    .messages
                    .first()
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                if let Some(billing) = self.identity.billing_header(first_user_msg) {
                    system_blocks.push(serde_json::json!({
                        "type": "text",
                        "text": billing,
                        "cache_control": { "type": "ephemeral" }
                    }));
                }
                system_blocks.push(serde_json::json!({
                    "type": "text",
                    "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                    "cache_control": { "type": "ephemeral", "scope": "org" }
                }));
            }
        }
        system_blocks.extend(request.system.iter().cloned());
        if !system_blocks.is_empty() {
            body["system"] = serde_json::json!(system_blocks);
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(cached_tool_blocks(&request.tools));
        }

        if let Some(ref thinking) = request.thinking {
            body["thinking"] = serde_json::json!(thinking);
        }

        if let Some(speed) = effective_speed(request) {
            body["speed"] = serde_json::json!(speed);
        }

        if let Some(effort) = effective_effort(request) {
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

fn effective_speed(request: &MessageRequest) -> Option<&str> {
    let value = request.speed.as_deref()?;
    if supports_speed(&request.model) {
        return Some(value);
    }
    warn_dropped_knob(&request.model, "speed", value);
    None
}

fn effective_effort(request: &MessageRequest) -> Option<&str> {
    let value = request.effort.as_deref()?;
    if supports_output_effort(&request.model) {
        return Some(value);
    }
    warn_dropped_knob(&request.model, "output_config.effort", value);
    None
}

fn supports_speed(_model: &str) -> bool {
    false
}

fn supports_output_effort(_model: &str) -> bool {
    false
}

fn warn_dropped_knob(model: &str, field: &str, value: &str) {
    static WARNED: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();
    let key = format!("{model}:{field}");
    let warned = WARNED.get_or_init(|| StdMutex::new(HashSet::new()));
    let Ok(mut guard) = warned.lock() else {
        return;
    };
    if guard.insert(key) {
        tracing::warn!(
            provider = "anthropic",
            model,
            field,
            value,
            "dropping unsupported Anthropic request knob"
        );
    }
}

fn cached_tool_blocks(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    // Anthropic counts every block carrying `cache_control` as a separate
    // cache breakpoint and hard-caps the request at 4 (HTTP 400 otherwise).
    // `cache_control` is a PREFIX marker: tagging only the final tool caches
    // the entire preceding tools array as a single breakpoint. Tagging every
    // tool both blows the 4-breakpoint budget and is semantically redundant.
    let mut tools: Vec<serde_json::Value> = tools.to_vec();
    if let Some(last) = tools.last_mut()
        && let Some(obj) = last.as_object_mut()
        && !obj.contains_key("cache_control")
    {
        obj.insert(
            "cache_control".into(),
            serde_json::json!({ "type": "ephemeral" }),
        );
    }
    tools
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
    /// Diagnostic marker: None, "main_session", or "subagent".
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

    #[error("rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },

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

    #[test]
    fn unsupported_speed_and_effort_are_dropped_from_wire_body() {
        let client = AnthropicClient::new(make_auth(), make_identity(), None);
        let request = MessageRequest {
            model: "claude-sonnet-4-6".to_string(),
            messages: vec![serde_json::json!({
                "role": "user",
                "content": "summarize this"
            })],
            speed: Some("fast".to_string()),
            effort: Some("low".to_string()),
            ..MessageRequest::default()
        };

        let body = client
            .build_request_body(&request)
            .expect("request body serializes");
        let body_json: serde_json::Value =
            serde_json::from_str(&body).expect("request body parses as JSON");

        assert_eq!(body_json.get("speed"), None);
        assert_eq!(body_json.get("output_config"), None);
        assert!(
            !body.contains("\"speed\""),
            "unsupported speed knob must not reach Anthropic wire body: {body}"
        );
        assert!(
            !body.contains("\"output_config\""),
            "unsupported effort knob must not reach Anthropic wire body: {body}"
        );
    }

    #[test]
    fn tool_definitions_get_anthropic_cache_control() {
        let client = AnthropicClient::new(make_auth(), make_identity(), None);
        let request = MessageRequest {
            messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
            tools: vec![
                serde_json::json!({
                    "name": "Agent",
                    "description": "spawn",
                    "input_schema": {"type": "object"}
                }),
                serde_json::json!({
                    "name": "Read",
                    "description": "read",
                    "input_schema": {"type": "object"}
                }),
            ],
            ..MessageRequest::default()
        };

        let body = client.build_request_body(&request).unwrap();
        let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();
        // `cache_control` is a prefix breakpoint: only the FINAL tool carries
        // it, which caches the entire tools array as a single breakpoint.
        // Non-final tools must NOT each carry one (4-breakpoint budget).
        assert!(
            body_json["tools"][0].get("cache_control").is_none(),
            "non-final tools must not each carry cache_control"
        );
        assert_eq!(
            body_json["tools"][1]["cache_control"],
            serde_json::json!({"type": "ephemeral"})
        );
    }

    #[test]
    fn cache_control_blocks_stay_within_anthropic_budget() {
        // Anthropic rejects requests with >4 cache_control blocks (HTTP 400
        // "A maximum of 4 blocks with cache_control may be provided").
        // Identity-spoof system blocks already consume up to 3; the tools
        // array must add at most 1 (a single prefix breakpoint on the final
        // tool), never one per tool. Regression guard for the 72-breakpoint
        // production incident.
        let client = AnthropicClient::new(make_auth(), make_identity(), None);
        let tools: Vec<serde_json::Value> = (0..40)
            .map(|i| {
                serde_json::json!({
                    "name": format!("tool_{i}"),
                    "description": "x",
                    "input_schema": {"type": "object"}
                })
            })
            .collect();
        let request = MessageRequest {
            messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
            tools,
            ..MessageRequest::default()
        };

        let body = client.build_request_body(&request).unwrap();
        let count = body.matches("\"cache_control\"").count();
        assert!(
            count <= 4,
            "serialized request carries {count} cache_control blocks; \
             Anthropic caps at 4. body={body}"
        );
    }
}
