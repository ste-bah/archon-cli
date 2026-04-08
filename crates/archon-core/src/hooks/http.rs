use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde_json::Value;

use super::types::{HookConfig, HookResult};

const MAX_RESPONSE_BYTES: usize = 64 * 1024; // 64KB

/// Execute an HTTP hook by POSTing context JSON to the URL in config.command.
/// Fail-open on all errors (timeout, network, parse). TLS required for non-localhost.
pub async fn execute_http_hook(
    config: &HookConfig,
    context: &Value,
    client: &Client,
) -> HookResult {
    let url = &config.command;
    let timeout_secs = config.timeout.unwrap_or(60);
    let timeout_duration = Duration::from_secs(u64::from(timeout_secs));

    // TLS check: reject non-localhost plain HTTP
    if !is_localhost(url) && !url.starts_with("https://") {
        tracing::warn!(url = %url, "HTTP hook rejected: TLS required for non-localhost URLs");
        return HookResult::default(); // fail-open
    }

    // Build headers with env var interpolation
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    for (key, value_template) in &config.headers {
        let value = interpolate_env_vars(value_template, &config.allowed_env_vars);
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            headers.insert(name, val);
        } else {
            tracing::warn!(header = %key, "HTTP hook: invalid header name or value, skipping");
        }
    }

    // POST with timeout
    let send_future = client
        .post(url)
        .headers(headers)
        .json(context)
        .timeout(timeout_duration)
        .send();

    let response = match send_future.await {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                tracing::warn!(url = %url, timeout_secs, "HTTP hook timed out (fail-open)");
            } else {
                tracing::warn!(url = %url, error = %e, "HTTP hook network error (fail-open)");
            }
            return HookResult::default();
        }
    };

    // Read response body with size limit
    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                url = %url,
                error = %e,
                "HTTP hook: failed to read response body (fail-open)"
            );
            return HookResult::default();
        }
    };

    if body_bytes.len() > MAX_RESPONSE_BYTES {
        tracing::warn!(
            url = %url,
            body_len = body_bytes.len(),
            limit = MAX_RESPONSE_BYTES,
            "HTTP hook response exceeded 64KB, truncating"
        );
    }

    let body_str =
        String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(MAX_RESPONSE_BYTES)]);

    // Parse JSON response as HookResult
    match serde_json::from_str::<HookResult>(&body_str) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(
                url = %url,
                error = %e,
                "HTTP hook response is not valid HookResult JSON (fail-open)"
            );
            HookResult::default()
        }
    }
}

/// Check if a URL points to localhost (127.0.0.1, [::1], or "localhost").
pub fn is_localhost(url: &str) -> bool {
    // Parse past the scheme (http:// or https://)
    let host_part = url.split("://").nth(1).unwrap_or(url);

    // Strip path
    let host_port = host_part.split('/').next().unwrap_or(host_part);

    // Handle IPv6: [::1]:port or [::1]
    if host_port.starts_with('[') {
        let bracket_end = host_port.find(']').unwrap_or(host_port.len());
        let ipv6_host = &host_port[1..bracket_end];
        ipv6_host == "::1"
    } else {
        // Strip port
        let host = host_port.split(':').next().unwrap_or(host_port);
        host == "localhost" || host == "127.0.0.1"
    }
}

/// Replace `${VAR_NAME}` with env var value, only if `VAR_NAME` is in allowed list.
/// Non-allowed vars are left as literal `${VAR_NAME}`.
pub fn interpolate_env_vars(template: &str, allowed: &[String]) -> String {
    let mut result = template.to_string();
    for var in allowed {
        if let Ok(value) = std::env::var(var) {
            result = result.replace(&format!("${{{}}}", var), &value);
        }
    }
    result
}
