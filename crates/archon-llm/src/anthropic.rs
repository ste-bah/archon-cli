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
        // `.timeout()` bounds the WHOLE request, streaming response body
        // included, so a 300s cap silently killed every generation that ran
        // longer than five minutes: reqwest drops the body stream and reports
        // `error decoding response body`, which reads as a network fault even
        // though the provider is healthy and still sending. Observed live —
        // litellm logged 200 OK with zero errors while archon failed stage
        // after stage on hour-long reasoning turns, each retry dying the same
        // way.
        //
        // `.read_timeout()` is the streaming-safe equivalent: it bounds the
        // gap BETWEEN reads rather than the total duration, so a slow but live
        // stream survives while a genuinely dead connection is still cut.
        //
        // The value has to sit ABOVE the stall guard that owns this decision,
        // `[subagent] stream_idle_timeout_secs` (default 600s, and the knob a
        // user actually tunes). A reasoning model emits NOTHING on the wire
        // while it thinks — with a large thinking budget that silence runs for
        // many minutes — so a transport read gap is not evidence of a stall.
        // Setting this at or below the guard makes the transport fire first and
        // silently overrides it: a 300s value still killed live reducers
        // mid-reasoning, reported as `error decoding response body`, exactly
        // the failure the guard's own comment records it was widened to stop.
        // Keep it a strict backstop for a truly dead socket and let the
        // configurable guard make the real call.
        const TRANSPORT_READ_BACKSTOP_SECS: u64 = 1800;
        let http = reqwest::Client::builder()
            .read_timeout(Duration::from_secs(TRANSPORT_READ_BACKSTOP_SECS))
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

    pub fn build_request_body(&self, request: &MessageRequest) -> Result<String, ApiError> {
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
        let mut system_blocks = request.system.clone();
        if matches!(
            self.identity.mode,
            crate::identity::IdentityMode::Spoof { .. }
        ) {
            let has_billing = system_blocks.iter().any(|block| {
                block
                    .get("text")
                    .and_then(|text| text.as_str())
                    .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
            });
            let has_identity = system_blocks.iter().any(|block| {
                block
                    .get("text")
                    .and_then(|text| text.as_str())
                    .is_some_and(|text| text.starts_with("You are Claude Code,"))
            });
            if !has_billing {
                let first_user_msg = request
                    .messages
                    .first()
                    .and_then(|message| message.get("content"))
                    .and_then(first_text_content)
                    .unwrap_or("");
                if let Some(billing) = self.identity.billing_header(first_user_msg) {
                    system_blocks.insert(
                        0,
                        serde_json::json!({
                            "type": "text",
                            "text": billing,
                            "cache_control": { "type": "ephemeral" }
                        }),
                    );
                }
            }
            if !has_identity {
                let identity_index = system_blocks
                    .iter()
                    .position(|block| {
                        block
                            .get("text")
                            .and_then(|text| text.as_str())
                            .is_some_and(|text| text.starts_with("x-anthropic-billing-header:"))
                    })
                    .map_or(0, |index| index + 1);
                system_blocks.insert(
                    identity_index,
                    serde_json::json!({
                        "type": "text",
                        "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                        "cache_control": { "type": "ephemeral", "scope": "org" }
                    }),
                );
            }
        }
        if !system_blocks.is_empty() {
            body["system"] = serde_json::json!(system_blocks);
        }

        if !request.tools.is_empty() {
            body["tools"] = if crate::anthropic_url::is_official_messages_url(&self.api_url) {
                serde_json::json!(cached_tool_blocks(&request.tools))
            } else {
                serde_json::json!(request.tools.as_ref())
            };
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

        if crate::anthropic_url::is_official_messages_url(&self.api_url) {
            enforce_cache_breakpoint_budget(&mut body);
        } else {
            remove_cache_directives(&mut body);
        }
        serde_json::to_string(&body).map_err(|e| ApiError::SerializeError(format!("{e}")))
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
