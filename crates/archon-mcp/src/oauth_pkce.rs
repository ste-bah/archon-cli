//! PKCE (RFC 7636) primitives + minimal OAuth 2.0 authorization-code client.
//!
//! Used by [`crate::sse_oauth_transport`] to front the MCP SSE transport with
//! a Bearer-token auth layer that transparently refreshes on server 401.
//!
//! This module implements *just* the pieces MCP needs:
//!   * `code_verifier` + S256 `code_challenge` generators
//!   * Authorization-code token exchange (client→IdP POST with form body)
//!   * Refresh-token exchange (client→IdP POST with form body)
//!   * Shared `OAuthClient` handle that owns the current token pair and
//!     exposes `access_token()` + `refresh()`
//!
//! The actual browser-based authorization step (user visits `/authorize`,
//! approves, gets redirected with a code) is NOT in this module — callers
//! either drive that themselves or skip it in tests by calling `/authorize`
//! on a mock IdP and feeding the resulting code into
//! [`OAuthClient::exchange_code`].

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use url::form_urlencoded;

use crate::types::McpError;

/// Build an `application/x-www-form-urlencoded` body from key/value pairs.
fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

/// RFC 7636 §4.1 — unreserved URL-safe alphabet for code_verifier.
const UNRESERVED: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Length of the generated code_verifier. 64 is comfortably inside the
/// RFC 7636 bounds of [43, 128] and gives 64 * log2(66) ≈ 387 bits of entropy.
const VERIFIER_LEN: usize = 64;

/// HTTP client build / call timeout for token endpoint requests.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Generate a cryptographically random PKCE `code_verifier`.
///
/// Length: 64 characters. Alphabet: unreserved (`[A-Z][a-z][0-9]-._~`).
/// Entropy source: `rand::rngs::OsRng` via `rand::rng()` (crate default for
/// thread-local cryptographically secure RNG).
pub fn generate_code_verifier() -> String {
    let mut rng = rand::rng();
    let mut out = String::with_capacity(VERIFIER_LEN);
    for _ in 0..VERIFIER_LEN {
        let idx = rng.random_range(0..UNRESERVED.len());
        out.push(UNRESERVED[idx] as char);
    }
    out
}

/// Compute the S256 PKCE `code_challenge`: `base64url-nopad(sha256(verifier))`.
///
/// Matches the test vector in RFC 7636 Appendix B:
///   verifier  = `"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"`
///   challenge = `"E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"`
pub fn code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

/// Configuration for an OAuth 2.0 authorization-code flow with PKCE.
#[derive(Clone, Debug)]
pub struct OAuthConfig {
    /// Full URL of the IdP authorization endpoint (`/authorize`).
    pub authorize_url: String,
    /// Full URL of the IdP token endpoint (`/token`).
    pub token_url: String,
    /// OAuth client_id. Sent with every /token POST.
    pub client_id: String,
    /// Redirect URI registered with the IdP (must match the /authorize call).
    pub redirect_uri: String,
}

#[derive(Clone, Debug)]
struct TokenPair {
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// A live OAuth client — owns the current access/refresh tokens + can refresh.
///
/// Cheap to clone: internally an `Arc<RwLock<TokenPair>>` + a reqwest `Client`
/// (which is itself `Clone`-friendly — inner state is an Arc).
#[derive(Clone)]
pub struct OAuthClient {
    config: OAuthConfig,
    http: reqwest::Client,
    tokens: Arc<RwLock<TokenPair>>,
}

impl OAuthClient {
    /// Exchange an authorization code + PKCE verifier for an access/refresh
    /// token pair. Builds the HTTP client and seeds the shared token state.
    pub async fn exchange_code(
        config: OAuthConfig,
        code: &str,
        code_verifier: &str,
    ) -> Result<Self, McpError> {
        let http = reqwest::Client::builder()
            .connect_timeout(HTTP_TIMEOUT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| McpError::Transport(format!("oauth: build HTTP client: {e}")))?;

        let body = form_body(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", code_verifier),
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", config.redirect_uri.as_str()),
        ]);
        let resp = http
            .post(&config.token_url)
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("oauth: token POST: {e}")))?;

        if !resp.status().is_success() {
            return Err(McpError::Transport(format!(
                "oauth: token endpoint returned {}",
                resp.status()
            )));
        }

        let parsed: TokenResponse = resp
            .json()
            .await
            .map_err(|e| McpError::Transport(format!("oauth: token response parse: {e}")))?;

        Ok(Self {
            config,
            http,
            tokens: Arc::new(RwLock::new(TokenPair {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
            })),
        })
    }

    /// Return a clone of the current access token.
    pub async fn access_token(&self) -> String {
        self.tokens.read().await.access_token.clone()
    }

    /// Refresh the access token using the stored refresh token.
    ///
    /// On success, both the access token and refresh token (if the IdP
    /// returns one — token rotation per RFC 6749 best practice) are updated
    /// in the shared state. Returns the new access token.
    ///
    /// On any failure (no refresh token, non-2xx response, parse error) the
    /// shared state is NOT modified and an `McpError::Transport` is returned.
    pub async fn refresh(&self) -> Result<String, McpError> {
        let refresh_token = self
            .tokens
            .read()
            .await
            .refresh_token
            .clone()
            .ok_or_else(|| McpError::Transport("oauth: no refresh_token available".into()))?;

        let body = form_body(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", self.config.client_id.as_str()),
        ]);
        let resp = self
            .http
            .post(&self.config.token_url)
            .header(
                http::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body)
            .send()
            .await
            .map_err(|e| McpError::Transport(format!("oauth: refresh POST: {e}")))?;

        if !resp.status().is_success() {
            return Err(McpError::Transport(format!(
                "oauth: refresh endpoint returned {}",
                resp.status()
            )));
        }

        let parsed: TokenResponse = resp
            .json()
            .await
            .map_err(|e| McpError::Transport(format!("oauth: refresh response parse: {e}")))?;

        let new_access = parsed.access_token.clone();
        let mut w = self.tokens.write().await;
        w.access_token = parsed.access_token;
        if let Some(rt) = parsed.refresh_token {
            w.refresh_token = Some(rt);
        }
        Ok(new_access)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_rfc7636_appendix_b() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(code_challenge(v), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn verifier_len_bounds() {
        let v = generate_code_verifier();
        assert!(v.len() >= 43 && v.len() <= 128);
    }

    #[test]
    fn verifier_charset_is_unreserved() {
        let v = generate_code_verifier();
        for c in v.chars() {
            assert!(
                c.is_ascii_alphanumeric() || "-._~".contains(c),
                "bad char {c:?}"
            );
        }
    }
}
