use std::time::Duration;

pub use crate::anthropic_support::{ApiError, MessageRequest};
use crate::anthropic_support::{
    apply_conditional_betas, cached_tool_blocks, classify_error, effective_effort, effective_speed,
    enforce_cache_breakpoint_budget, extract_unknown_beta, remove_cache_directives,
    should_retry_without_knob,
};
use crate::auth::{AuthError, AuthProvider, OAuthCredentials};
use crate::identity::IdentityProvider;
use crate::streaming::StreamEvent;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

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

/// Sized against the 600s `stream_idle_timeout_secs` default.
const DEFAULT_TRANSPORT_READ_BACKSTOP_SECS: u64 = 1800;
/// How far the transport must outlast the guard that is meant to fire first.
const TRANSPORT_BACKSTOP_MARGIN_SECS: u64 = 600;

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
        // `.timeout()` bounds the WHOLE request, streaming body included: a
        // 300s cap killed any generation over five minutes, reported as
        // `error decoding response body` while the provider was healthy.
        // `.read_timeout()` bounds the gap BETWEEN reads, so a live stream
        // survives. It must sit ABOVE `[subagent] stream_idle_timeout_secs`
        // (600s) — a reasoning model emits nothing while it thinks, so set at
        // or below that guard the transport fires first and overrides it.
        Self::with_read_backstop(
            auth,
            identity,
            api_url,
            DEFAULT_TRANSPORT_READ_BACKSTOP_SECS,
        )
    }

    /// Build a client whose transport backstop clears the caller's stream idle
    /// guard.
    ///
    /// The backstop is a hardcoded constant sized against the 600s default. An
    /// operator who raises `[subagent] stream_idle_timeout_secs` above it gets
    /// the constant instead of the value they set: a 40-minute guard was
    /// overridden by the 1800s transport, which cut every longer think at ~33
    /// minutes and made the round restart and lose its work.
    pub fn with_read_backstop(
        auth: AuthProvider,
        identity: IdentityProvider,
        api_url: Option<String>,
        read_backstop_secs: u64,
    ) -> Self {
        let http = reqwest::Client::builder()
            .read_timeout(Duration::from_secs(read_backstop_secs.max(1)))
            .no_proxy()
            .build()
            .expect("reqwest client should build");

        Self {
            http,
            auth,
            identity,
            api_url: crate::anthropic_url::messages_url(api_url),
        }
    }

    /// The transport backstop that clears `stream_idle_timeout_secs`.
    ///
    /// The guard decides when a silent stream is abandoned; the transport must
    /// outlast it, or the transport decides instead and the configured value
    /// silently does not apply.
    pub fn read_backstop_for_idle_guard(stream_idle_timeout_secs: u64) -> u64 {
        DEFAULT_TRANSPORT_READ_BACKSTOP_SECS
            .max(stream_idle_timeout_secs.saturating_add(TRANSPORT_BACKSTOP_MARGIN_SECS))
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
        // `mut` so the effort-degrade path below can rebuild it after the API
        // tells us this model does not accept `output_config.effort`.
        let mut body = self.build_request_body(&request)?;
        let mut refreshed_after_401 = false;

        for attempt in 0..=MAX_RETRIES {
            let request_id = uuid::Uuid::new_v4().to_string();
            let mut headers = self.identity.request_headers(&request_id);

            apply_conditional_betas(&request, &mut headers);

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
            tracing::debug!("API request body: {}", crate::debug_body::debug_body(&body));

            let response = req
                .body(body.clone())
                .send()
                .await
                .map_err(|e| ApiError::HttpError(format!("request failed: {e}")))?;

            let status = response.status();

            if status.is_success() {
                return self.spawn_stream_reader(response, auth_header_value).await;
            }

            // Log full error details for debugging
            let retry_after_header = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let capture = crate::transport_evidence::Capture::new(&response, vec![auth_header_value]);
            let response_body = response.text().await.unwrap_or_default();
            capture.body(response_body.as_bytes());

            tracing::debug!(
                "API error response: status={}, retry-after={:?}, body={}",
                status,
                retry_after_header,
                crate::debug_body::debug_body(&response_body)
            );

            // #123: effort and speed are sent for every model, so a model that
            // rejects one is discovered here rather than guessed at up front.
            if should_retry_without_knob(&request, status.as_u16(), &response_body) {
                body = self.build_request_body(&request)?;
                continue;
            }

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
        auth: String,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, ApiError> {
        Ok(crate::anthropic_stream::spawn_anthropic_stream_reader(
            response.bytes_stream(),
        ))
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

            let capture = crate::transport_evidence::Capture::new(&response, vec![auth_header_value]);
            let response_body = response.text().await.unwrap_or_default();
            capture.body(response_body.as_bytes());

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
}

fn first_text_content(content: &serde_json::Value) -> Option<&str> {
    content.as_str().or_else(|| {
        content.as_array()?.iter().find_map(|block| {
            (block.get("type").and_then(|value| value.as_str()) == Some("text"))
                .then(|| block.get("text").and_then(|value| value.as_str()))
                .flatten()
        })
    })
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

#[cfg(test)]
mod transport_backstop_tests {
    use super::AnthropicClient;

    #[test]
    fn the_backstop_always_outlasts_the_configured_idle_guard() {
        // The guard decides when a silent stream is abandoned. A hardcoded
        // 1800s transport overrode a configured 2400s guard, cutting every
        // think longer than ~33 minutes and restarting the round.
        assert_eq!(AnthropicClient::read_backstop_for_idle_guard(600), 1800);
        assert!(AnthropicClient::read_backstop_for_idle_guard(2400) > 2400);
        assert!(AnthropicClient::read_backstop_for_idle_guard(5400) > 5400);
    }
}

#[path = "anthropic_body.rs"]
mod body;
