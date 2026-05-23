use std::time::Duration;

pub use crate::anthropic_support::{ApiError, MessageRequest};
use crate::anthropic_support::{
    cached_tool_blocks, classify_error, effective_effort, effective_speed, extract_unknown_beta,
};
use crate::auth::{AuthError, AuthProvider, OAuthCredentials};
use crate::identity::IdentityProvider;
use crate::streaming::{StreamError, StreamEvent, parse_sse_event, split_sse_lines};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MAX_RETRIES: u32 = 3;
const OVERLOAD_BASE_DELAY_SECS: u64 = 10;
const LARGE_RATE_LIMIT_RETRY_BODY_BYTES: usize = 320_000;
const MAX_INLINE_RATE_LIMIT_RETRY_SECS: u64 = 60;

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

    async fn request_auth_header(&self) -> Result<(String, String), ApiError> {
        if let AuthProvider::OAuthToken(_) = &self.auth {
            let credentials_path = crate::tokens::credentials_path();
            let creds = crate::tokens::refresh_if_needed(&credentials_path, &self.http)
                .await
                .map_err(auth_error_to_api)?;
            return Ok(oauth_header(&creds));
        }

        Ok(self.auth.header())
    }

    async fn force_refresh_oauth(&self) -> Result<(), ApiError> {
        if !matches!(&self.auth, AuthProvider::OAuthToken(_)) {
            return Ok(());
        }

        let credentials_path = crate::tokens::credentials_path();
        crate::tokens::force_refresh(&credentials_path, &self.http)
            .await
            .map(|_| ())
            .map_err(auth_error_to_api)
    }

    /// Send a streaming messages request with automatic retry on 429/5xx.
    pub async fn stream_message(
        &self,
        request: MessageRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, ApiError> {
        let body = self.build_request_body(&request)?;
        let mut refreshed_after_401 = false;

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

            let (auth_header_name, auth_header_value) = self.request_auth_header().await?;

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
                    if body.len() >= LARGE_RATE_LIMIT_RETRY_BODY_BYTES {
                        tracing::warn!(
                            body_len = body.len(),
                            threshold_body_bytes = LARGE_RATE_LIMIT_RETRY_BODY_BYTES,
                            "large Anthropic request was rate limited; returning to caller for compaction instead of retrying identical body"
                        );
                        return Err(err);
                    }
                    if *retry_after_secs > MAX_INLINE_RATE_LIMIT_RETRY_SECS {
                        tracing::warn!(
                            retry_after_secs,
                            max_inline_retry_secs = MAX_INLINE_RATE_LIMIT_RETRY_SECS,
                            "Anthropic retry-after is too long for an inline client sleep; returning rate limit to caller"
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

                // 401 on OAuth: force-refresh once, then retry with the
                // refreshed request-local header on the next loop iteration.
                ApiError::AuthError(_)
                    if status.as_u16() == 401
                        && matches!(&self.auth, AuthProvider::OAuthToken(_))
                        && !refreshed_after_401 =>
                {
                    refreshed_after_401 = true;
                    tracing::warn!("Anthropic OAuth token rejected, refreshing and retrying once");
                    self.force_refresh_oauth().await?;
                    continue;
                }

                // Repeated 401, non-OAuth auth, and other errors: don't retry.
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

            let (auth_header_name, auth_header_value) = match self.request_auth_header().await {
                Ok(header) => header,
                Err(e) => {
                    tracing::warn!("Beta validation probe: auth refresh failed: {e}");
                    break;
                }
            };

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

fn oauth_header(creds: &OAuthCredentials) -> (String, String) {
    (
        "Authorization".to_string(),
        format!("Bearer {}", creds.access_token.expose()),
    )
}

fn auth_error_to_api(err: AuthError) -> ApiError {
    ApiError::AuthError(err.to_string())
}
