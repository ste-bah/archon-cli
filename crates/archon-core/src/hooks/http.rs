use std::sync::LazyLock;
use std::time::Duration;

use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use url::{Host, Url};

use super::types::{HookConfig, HookResult};
use crate::url_redact::redact_url;

const MAX_RESPONSE_BYTES: usize = 64 * 1024; // 64KB
static HTTP_HOOK_CLIENT: LazyLock<Client> = LazyLock::new(Client::new);

pub(crate) fn shared_client() -> &'static Client {
    &HTTP_HOOK_CLIENT
}

fn event_name_from_context(context: &Value) -> &str {
    context
        .get("hook_event")
        .or_else(|| context.get("event"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Execute an HTTP hook by POSTing context JSON to the URL in config.command.
/// Failure handling follows the hook's configured or event-default policy.
pub async fn execute_http_hook(
    config: &HookConfig,
    context: &Value,
    client: &Client,
) -> HookResult {
    let event_name = event_name_from_context(context);
    execute_http_hook_for_event(config, context, client, event_name).await
}

pub(crate) async fn execute_http_hook_for_event(
    config: &HookConfig,
    context: &Value,
    client: &Client,
    event_name: &str,
) -> HookResult {
    // The configured URL may carry the webhook credential; only its origin
    // is ever logged.
    let shown_url = redact_url(&config.command);
    let timeout_secs = config.timeout.unwrap_or(60);
    let timeout_duration = Duration::from_secs(u64::from(timeout_secs));

    let url = match admit_hook_url(&config.command) {
        Ok(url) => url,
        Err(reason) => {
            tracing::warn!(url = %shown_url, reason, "HTTP hook rejected");
            return config.failure_result(event_name, reason);
        }
    };

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
            // reqwest errors embed the full request URL; strip it.
            let timed_out = e.is_timeout();
            let e = e.without_url();
            if timed_out {
                tracing::warn!(url = %shown_url, timeout_secs, "HTTP hook timed out; applying failure policy");
            } else {
                tracing::warn!(url = %shown_url, error = %e, "HTTP hook network error; applying failure policy");
            }
            return config.failure_result(event_name, &e.to_string());
        }
    };

    // Read response body with size limit
    let body_bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            let e = e.without_url();
            tracing::warn!(
                url = %shown_url,
                error = %e,
                "HTTP hook: failed to read response body; applying failure policy"
            );
            return config.failure_result(event_name, &e.to_string());
        }
    };

    if body_bytes.len() > MAX_RESPONSE_BYTES {
        tracing::warn!(
            url = %shown_url,
            body_len = body_bytes.len(),
            limit = MAX_RESPONSE_BYTES,
            "HTTP hook response exceeded 64KB, truncating"
        );
    }

    let body_str = String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(MAX_RESPONSE_BYTES)]);

    // Parse JSON response as HookResult
    match serde_json::from_str::<HookResult>(&body_str) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(
                url = %shown_url,
                error = %e,
                "HTTP hook response is not valid HookResult JSON; applying failure policy"
            );
            config.failure_result(event_name, &e.to_string())
        }
    }
}

/// Parse a hook URL and admit it only if it is HTTPS, or plain HTTP to a
/// loopback host. Anything else would send the hook payload and its
/// configured headers in cleartext.
fn admit_hook_url(raw: &str) -> Result<Url, &'static str> {
    let url = Url::parse(raw).map_err(|_| "hook URL is not a valid absolute URL")?;
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback_host(&url) => Ok(url),
        "http" => Err("TLS is required for non-localhost URLs"),
        _ => Err("hook URL scheme must be https, or http to localhost"),
    }
}

/// Check if a URL points to localhost (`localhost`, 127.0.0.0/8, or `[::1]`).
///
/// The host is taken from a real URL parse, so `http://localhost:x@evil/`
/// (userinfo) and `http://localhost.evil/` are not mistaken for loopback.
pub fn is_localhost(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| is_loopback_host(&url))
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
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

#[cfg(test)]
mod tests {
    use super::{execute_http_hook, is_localhost, shared_client};
    use crate::hooks::{HookCommandType, HookConfig, HookFailurePolicy, HookOutcome};

    const SECRET: &str = "sk-live-hook-5e4d3c2b1a";

    fn blocking_http_hook(url: String) -> HookConfig {
        HookConfig {
            hook_type: HookCommandType::Http,
            command: url,
            if_condition: None,
            timeout: Some(2),
            once: None,
            r#async: None,
            async_rewake: None,
            status_message: None,
            headers: Default::default(),
            allowed_env_vars: Default::default(),
            on_failure: Some(HookFailurePolicy::Block),
            enabled: true,
        }
    }

    #[test]
    fn shared_http_client_reuses_one_instance() {
        assert!(std::ptr::eq(shared_client(), shared_client()));
    }

    #[test]
    fn localhost_lookalikes_are_not_loopback() {
        assert!(!is_localhost("http://localhost:x@evil.example/hook"));
        assert!(!is_localhost("http://localhost@evil.example/hook"));
        assert!(!is_localhost("http://localhost.evil.example/hook"));
        assert!(!is_localhost("http://127.0.0.1.evil.example/hook"));
        assert!(!is_localhost("not a url"));
        assert!(is_localhost("http://127.0.0.2:8080/hook"));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn userinfo_cannot_smuggle_plain_http_to_a_remote_host() {
        let config = blocking_http_hook(format!("http://localhost:{SECRET}@evil.invalid/hook"));
        let event = serde_json::json!({"event": "PreToolUse"});

        let result = execute_http_hook(&config, &event, shared_client()).await;

        assert_eq!(result.outcome, HookOutcome::Blocking);
        let reason = result.reason.unwrap_or_default();
        assert!(reason.contains("TLS is required"), "{reason}");
        assert!(
            !reason.contains(SECRET),
            "secret in failure reason: {reason}"
        );
        assert!(!logs_contain(SECRET), "secret reached the log");
        assert!(logs_contain("HTTP hook rejected"));
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn network_failure_never_echoes_the_hook_url() {
        let config = blocking_http_hook(format!(
            "http://127.0.0.1:1/services/{SECRET}?token={SECRET}"
        ));
        let event = serde_json::json!({"event": "PreToolUse"});

        let result = execute_http_hook(&config, &event, shared_client()).await;

        assert_eq!(result.outcome, HookOutcome::Blocking);
        let reason = result.reason.unwrap_or_default();
        assert!(reason.contains("http://127.0.0.1:1/<redacted>"), "{reason}");
        assert!(
            !reason.contains(SECRET),
            "secret in failure reason: {reason}"
        );
        // An unreachable port is a connection refusal on some hosts and a
        // connect timeout on others (Windows), and both are the redacted
        // failure path; the invariant under test is that neither echoes the URL.
        assert!(
            logs_contain("HTTP hook network error") || logs_contain("HTTP hook timed out"),
            "no redacted network-failure log line"
        );
        assert!(!logs_contain(SECRET), "secret reached the log");
    }
}
