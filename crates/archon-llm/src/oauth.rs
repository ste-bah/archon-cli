use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::auth::{AuthError, OAuthCredentials};
use crate::tokens::write_credentials_atomic;
use crate::types::Secret;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AUTH_URL: &str = "https://claude.com/cai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const LOGIN_TIMEOUT_SECS: u64 = 120;

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code verifier (32 random bytes, base64url, no padding).
pub fn generate_code_verifier() -> String {
    let bytes: [u8; 32] = rand_bytes();
    base64url_encode(&bytes)
}

/// Generate a PKCE code challenge from a verifier (SHA256, base64url, no padding).
pub fn generate_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64url_encode(&hash)
}

/// Generate a random state parameter (32 random bytes, base64url).
pub fn generate_state() -> String {
    let bytes: [u8; 32] = rand_bytes();
    base64url_encode(&bytes)
}

/// Base64url encode with no padding.
fn base64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

/// Generate 32 random bytes using system randomness.
fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("system RNG should be available");
    buf
}

// ---------------------------------------------------------------------------
// Authorization URL
// ---------------------------------------------------------------------------

/// Build the authorization URL for the OAuth PKCE flow.
pub fn build_auth_url(code_challenge: &str, state: &str, redirect_port: u16) -> String {
    let redirect_uri = format!("http://localhost:{redirect_port}/callback");
    format!(
        "{AUTH_URL}?response_type=code\
         &client_id={CLIENT_ID}\
         &redirect_uri={redirect_uri}\
         &scope={}\
         &state={state}\
         &code_challenge={code_challenge}\
         &code_challenge_method=S256",
        urlencoding::encode(SCOPES)
    )
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

/// Exchange an auth code for tokens via the token endpoint.
pub async fn exchange_code(
    code: &str,
    code_verifier: &str,
    state: &str,
    redirect_port: u16,
    client: &reqwest::Client,
) -> Result<OAuthCredentials, AuthError> {
    let redirect_uri = format!("http://localhost:{redirect_port}/callback");

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": CLIENT_ID,
        "code_verifier": code_verifier,
        "state": state,
    });

    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AuthError::ParseError(format!("token exchange HTTP error: {e}")))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| AuthError::ParseError(format!("failed to read exchange response: {e}")))?;

    if !status.is_success() {
        return Err(AuthError::ParseError(format!(
            "token exchange failed (HTTP {status}): {body}"
        )));
    }

    parse_exchange_response(&body)
}

fn parse_exchange_response(body: &str) -> Result<OAuthCredentials, AuthError> {
    #[derive(serde::Deserialize)]
    struct ExchangeResponse {
        access_token: String,
        refresh_token: String,
        expires_in: Option<u64>,
        #[serde(default)]
        scope: String,
    }

    let resp: ExchangeResponse = serde_json::from_str(body)
        .map_err(|e| AuthError::ParseError(format!("invalid exchange response: {e}")))?;

    let expires_at =
        chrono::Utc::now() + chrono::Duration::seconds(resp.expires_in.unwrap_or(3600) as i64);

    let scopes: Vec<String> = resp
        .scope
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    Ok(OAuthCredentials {
        access_token: Secret::new(resp.access_token),
        refresh_token: Secret::new(resp.refresh_token),
        expires_at,
        scopes,
        subscription_type: String::new(),
    })
}

// ---------------------------------------------------------------------------
// Localhost callback server
// ---------------------------------------------------------------------------

/// Start a localhost HTTP server, wait for the OAuth callback, return the auth code.
///
/// Returns `(code, state)` from the callback URL parameters.
pub fn start_callback_server(
    expected_state: &str,
) -> Result<(u16, std::sync::mpsc::Receiver<Result<String, AuthError>>), AuthError> {
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| AuthError::ParseError(format!("failed to start callback server: {e}")))?;

    let port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| AuthError::ParseError("callback server has no IP address".into()))?
        .port();

    let expected_state = expected_state.to_string();
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let result = handle_callback(&server, &expected_state);
        let _ = tx.send(result);
    });

    Ok((port, rx))
}

fn handle_callback(server: &tiny_http::Server, expected_state: &str) -> Result<String, AuthError> {
    let timeout = std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS);

    let request = server
        .recv_timeout(timeout)
        .map_err(|e| AuthError::ParseError(format!("callback server error: {e}")))?
        .ok_or_else(|| {
            AuthError::NoCredentials(format!(
                "OAuth login timed out after {LOGIN_TIMEOUT_SECS}s. No callback received."
            ))
        })?;

    let url = request.url().to_string();

    // Send success response to browser
    let response = tiny_http::Response::from_string(
        "<html><body><h1>Login successful!</h1><p>You can close this tab.</p></body></html>",
    )
    .with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..])
            .expect("valid header"),
    );
    let _ = request.respond(response);

    // Parse query parameters from the URL
    let query = url
        .split('?')
        .nth(1)
        .ok_or_else(|| AuthError::ParseError("callback URL has no query parameters".into()))?;

    let params: std::collections::HashMap<&str, &str> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next()?))
        })
        .collect();

    // Check for error
    if let Some(error) = params.get("error") {
        let desc = params.get("error_description").unwrap_or(&"unknown error");
        return Err(AuthError::ParseError(format!(
            "OAuth authorization failed: {error} -- {desc}"
        )));
    }

    // Validate state
    let state = params
        .get("state")
        .ok_or_else(|| AuthError::ParseError("callback missing state parameter".into()))?;

    if *state != expected_state {
        return Err(AuthError::ParseError(
            "state parameter mismatch -- possible CSRF attack".into(),
        ));
    }

    // Extract code
    let code = params
        .get("code")
        .ok_or_else(|| AuthError::ParseError("callback missing code parameter".into()))?;

    Ok(code.to_string())
}

// ---------------------------------------------------------------------------
// Full login flow
// ---------------------------------------------------------------------------

/// Perform the full OAuth PKCE login flow:
/// 1. Generate PKCE parameters
/// 2. Start callback server
/// 3. Open browser
/// 4. Wait for callback
/// 5. Exchange code for tokens
/// 6. Save credentials
pub async fn login(
    credentials_path: &Path,
    client: &reqwest::Client,
) -> Result<OAuthCredentials, AuthError> {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);
    let state = generate_state();

    let (port, callback_rx) = start_callback_server(&state)?;
    let auth_url = build_auth_url(&challenge, &state, port);

    tracing::info!("Opening browser for OAuth login...");
    if let Err(e) = open::that(&auth_url) {
        tracing::warn!("Failed to open browser: {e}");
        eprintln!("\nOpen this URL in your browser to log in:\n{auth_url}\n");
    }

    eprintln!("Waiting for OAuth callback (timeout: {LOGIN_TIMEOUT_SECS}s)...");

    let code = callback_rx
        .recv()
        .map_err(|_| AuthError::ParseError("callback channel closed unexpectedly".into()))??;

    let creds = exchange_code(&code, &verifier, &state, port, client).await?;

    // Save credentials
    if let Some(parent) = credentials_path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_credentials_atomic(credentials_path, &creds)?;

    tracing::info!("OAuth login successful, credentials saved");
    Ok(creds)
}
