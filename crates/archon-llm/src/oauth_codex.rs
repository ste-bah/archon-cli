use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use serde::Deserialize;

use crate::auth::{AuthError, CodexCredentials};
use crate::oauth::{generate_code_challenge, generate_code_verifier, generate_state};
use crate::types::Secret;

const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_BIND_HOST: &str = "127.0.0.1";
const CALLBACK_PORT: u16 = 1455;
const SCOPE: &str = "openid profile email offline_access";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
const LOGIN_TIMEOUT_SECS: u64 = 120;
const ORIGINATOR: &str = "openclaw";

const SUCCESS_HTML: &str =
    "<html><body><h1>Codex login successful</h1><p>You can close this tab.</p></body></html>";
const ERROR_HTML: &str =
    "<html><body><h1>Codex login failed</h1><p>Return to archon-cli.</p></body></html>";

pub fn auth_url() -> &'static str {
    AUTH_URL
}

pub fn token_url() -> &'static str {
    TOKEN_URL
}

#[derive(Debug, Clone)]
pub struct CodexOAuthClient {
    auth_url: String,
    token_url: String,
    http: reqwest::Client,
}

impl CodexOAuthClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            auth_url: AUTH_URL.into(),
            token_url: TOKEN_URL.into(),
            http,
        }
    }

    pub fn new_with_urls(http: reqwest::Client, auth_url: String, token_url: String) -> Self {
        Self {
            auth_url,
            token_url,
            http,
        }
    }

    pub async fn login<F>(&self, open_browser: F) -> Result<CodexCredentials, AuthError>
    where
        F: FnOnce(&str),
    {
        let code_verifier = generate_code_verifier();
        let code_challenge = generate_code_challenge(&code_verifier);
        let state = generate_state();
        let rx = start_callback_server(&state, CALLBACK_PORT)?;
        let url = self.build_authorize_url(&code_challenge, &state);
        open_browser(&url);
        let code = receive_callback_code(rx)?;
        self.exchange_code(&code, &code_verifier).await
    }

    pub fn build_authorize_url(&self, code_challenge: &str, state: &str) -> String {
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&id_token_add_organizations=true&codex_cli_simplified_flow=true&originator={}",
            self.auth_url,
            urlencoding::encode(CLIENT_ID),
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPE),
            urlencoding::encode(state),
            urlencoding::encode(code_challenge),
            urlencoding::encode(ORIGINATOR),
        )
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<CodexCredentials, AuthError> {
        let token_url = sensitive_endpoint_url(&self.token_url, "Codex OAuth token")?;
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ];
        let response = self
            .http
            .post(token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::ParseError(format!("token exchange HTTP error: {e}")))?;
        parse_token_response(response, TokenFailureKind::Exchange).await
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<CodexCredentials, AuthError> {
        let token_url = sensitive_endpoint_url(&self.token_url, "Codex OAuth token")?;
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ];
        let response = self
            .http
            .post(token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| AuthError::ParseError(format!("token refresh HTTP error: {e}")))?;
        parse_token_response(response, TokenFailureKind::Refresh).await
    }
}

fn sensitive_endpoint_url(raw: &str, label: &str) -> Result<reqwest::Url, AuthError> {
    let url = reqwest::Url::parse(raw)
        .map_err(|e| AuthError::ParseError(format!("{label} endpoint URL is invalid: {e}")))?;
    if endpoint_allows_sensitive_data(&url) {
        Ok(url)
    } else {
        Err(AuthError::ParseError(format!(
            "{label} endpoint must use HTTPS unless it targets loopback/local test host"
        )))
    }
}

fn endpoint_allows_sensitive_data(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        || (url.scheme() == "http" && url.host_str().map(is_loopback_host).unwrap_or(false))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

enum TokenFailureKind {
    Exchange,
    Refresh,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: Option<u64>,
}

async fn parse_token_response(
    response: reqwest::Response,
    failure_kind: TokenFailureKind,
) -> Result<CodexCredentials, AuthError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| AuthError::ParseError(format!("failed to read token response: {e}")))?;

    if !status.is_success() {
        let status = status.as_u16();
        let body = safe_error_body(&body);
        return match failure_kind {
            TokenFailureKind::Exchange => Err(AuthError::TokenExchangeFailed { status, body }),
            TokenFailureKind::Refresh => Err(AuthError::RefreshFailed { status, body }),
        };
    }

    let token: TokenResponse = serde_json::from_str(&body)
        .map_err(|e| AuthError::ParseError(format!("invalid token response: {e}")))?;
    let account_id = decode_account_id_from_jwt(&token.access_token)?;
    let expires_at =
        Utc::now() + chrono::Duration::seconds(token.expires_in.unwrap_or(3600) as i64);

    Ok(CodexCredentials {
        access_token: Secret::new(token.access_token),
        refresh_token: Secret::new(token.refresh_token),
        expires_at,
        account_id,
    })
}

fn safe_error_body(body: &str) -> String {
    body.chars().take(500).collect()
}

pub fn decode_account_id_from_jwt(jwt: &str) -> Result<String, AuthError> {
    let payload = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| AuthError::JwtDecodeFailed("JWT has no payload segment".into()))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| AuthError::JwtDecodeFailed(format!("payload base64 decode failed: {e}")))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| AuthError::JwtDecodeFailed(format!("payload JSON parse failed: {e}")))?;
    value
        .get(JWT_CLAIM_PATH)
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| AuthError::JwtDecodeFailed("missing chatgpt_account_id claim".into()))
}

pub fn parse_callback_url(url: &str, expected_state: &str) -> Result<String, AuthError> {
    let query = url
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or_else(|| AuthError::ParseError("callback URL has no query parameters".into()))?;
    let params = parse_query(query);
    let state = params.get("state").ok_or(AuthError::StateMismatch)?;
    if state != expected_state {
        return Err(AuthError::StateMismatch);
    }
    params
        .get("code")
        .cloned()
        .ok_or_else(|| AuthError::ParseError("callback missing code parameter".into()))
}

fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((decode_component(key), decode_component(value)))
        })
        .collect()
}

fn decode_component(value: &str) -> String {
    urlencoding::decode(value)
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_else(|_| value.to_string())
}

pub fn start_callback_server(
    expected_state: &str,
    port: u16,
) -> Result<mpsc::Receiver<Result<String, AuthError>>, AuthError> {
    let bind = format!("{CALLBACK_BIND_HOST}:{port}");
    let server = tiny_http::Server::http(&bind)
        .map_err(|e| AuthError::CallbackBindFailed(format!("{e}")))?;
    let expected_state = expected_state.to_string();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let result = handle_callback(&server, &expected_state);
        let _ = tx.send(result);
    });

    Ok(rx)
}

fn receive_callback_code(
    rx: mpsc::Receiver<Result<String, AuthError>>,
) -> Result<String, AuthError> {
    rx.recv_timeout(Duration::from_secs(LOGIN_TIMEOUT_SECS))
        .map_err(|_| AuthError::CallbackTimeout(LOGIN_TIMEOUT_SECS))?
}

fn handle_callback(server: &tiny_http::Server, expected_state: &str) -> Result<String, AuthError> {
    let request = server
        .recv_timeout(Duration::from_secs(LOGIN_TIMEOUT_SECS))
        .map_err(|e| AuthError::ParseError(format!("callback server error: {e}")))?
        .ok_or(AuthError::CallbackTimeout(LOGIN_TIMEOUT_SECS))?;
    let url = request.url().to_string();
    let result = parse_callback_url(&url, expected_state);
    let html = if result.is_ok() {
        SUCCESS_HTML
    } else {
        ERROR_HTML
    };
    let mut response = tiny_http::Response::from_string(html);
    if let Ok(header) = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html"[..]) {
        response.add_header(header);
    }
    let _ = request.respond(response);
    result
}
