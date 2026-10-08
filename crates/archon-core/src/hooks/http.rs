use std::sync::LazyLock;
use std::time::Duration;

use reqwest::Client;

#[path = "http_transport.rs"]
mod transport;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
pub use transport::{HookHttpTransport, HookHttpTransportBuilder};
use url::{Host, Url};

use super::executor::{HookExecutionResult, NoProgressWindow, hook_failure_execution_result};
use super::types::{ElicitationAction, HookConfig, HookOutcome, HookResult, PermissionBehavior};
use crate::url_redact::redact_url;

const MAX_RESPONSE_BYTES: usize = 64 * 1024; // 64KB
static HTTP_HOOK_CLIENT: LazyLock<HookHttpTransport> = LazyLock::new(HookHttpTransport::new);

pub(crate) fn shared_client() -> &'static HookHttpTransport {
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
/// Use HookHttpTransport to retain headers, TLS, proxy and redirect settings
/// without total/read clocks. An opaque `reqwest::Client` is rejected at
/// compile time: its total/read clocks cannot be removed or inspected.
///
/// ```compile_fail
/// # async fn f(config: &archon_core::hooks::HookConfig) {
/// let client = reqwest::Client::new();
/// archon_core::hooks::execute_http_hook(config, &serde_json::json!({}), &client).await;
/// # }
/// ```
pub async fn execute_http_hook(
    config: &HookConfig,
    context: &Value,
    client: &HookHttpTransport,
) -> HookResult {
    let event_name = event_name_from_context(context);
    execute_http_hook_for_event(config, context, client, event_name)
        .await
        .result
}

pub(in crate::hooks) async fn execute_http_hook_for_event(
    config: &HookConfig,
    context: &Value,
    client: &HookHttpTransport,
    event_name: &str,
) -> HookExecutionResult {
    // The configured URL may carry the webhook credential; only its origin
    // is ever logged.
    let shown_url = redact_url(&config.command);
    let timeout_secs = config.timeout.unwrap_or(60);
    let timeout_duration = Duration::from_secs(u64::from(timeout_secs));

    let url = match admit_hook_url(&config.command) {
        Ok(url) => url,
        Err(reason) => {
            tracing::warn!(url = %shown_url, reason, "HTTP hook rejected");
            return config.failure_result(event_name, reason).into();
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

    // Bound silence while waiting for the response, then renew on body data.
    let progress = NoProgressWindow::new(timeout_duration);
    let client = hook_transport(client);
    let send_future = client.post(url).headers(headers).json(context).send();

    let response = match progress.wait("HTTP response", send_future).await {
        Ok(Ok(resp)) => resp,
        Err(error) => return hook_failure_execution_result(config, event_name, &error),
        Ok(Err(e)) => {
            // reqwest errors embed the full request URL; strip it.
            let timed_out = e.is_timeout();
            let e = e.without_url();
            if timed_out {
                tracing::warn!(url = %shown_url, timeout_secs, "HTTP hook timed out; applying failure policy");
            } else {
                tracing::warn!(url = %shown_url, error = %e, "HTTP hook network error; applying failure policy");
            }
            return transport_failure_result(config, event_name, &e.to_string(), timed_out);
        }
    };

    read_hook_response(config, event_name, response, &progress, &shown_url).await
}

async fn read_hook_response(
    config: &HookConfig,
    event_name: &str,
    mut response: reqwest::Response,
    progress: &NoProgressWindow,
    shown_url: &str,
) -> HookExecutionResult {
    // An error status is read too: its body may carry an explicit refusal.
    let status = response.status();
    progress.record_output();
    let mut body_bytes = Vec::new();
    let mut received = 0_usize;
    loop {
        let chunk = match progress.wait("HTTP body", response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk,
            Ok(Ok(None)) => break,
            Err(error) => return hook_failure_execution_result(config, event_name, &error),
            Ok(Err(error)) => {
                let timed_out = error.is_timeout();
                let error = error.without_url();
                tracing::warn!(url = %shown_url, error = %error, "HTTP hook body read failed");
                if !status.is_success() {
                    let reason = format!("HTTP hook returned {status}; body read failed: {error}");
                    return diagnostic_failure_result(config, event_name, &reason);
                }
                return transport_failure_result(config, event_name, &error.to_string(), timed_out);
            }
        };
        if !chunk.is_empty() {
            progress.record_output();
        }
        received = received.saturating_add(chunk.len());
        let retained = chunk
            .len()
            .min(MAX_RESPONSE_BYTES.saturating_sub(body_bytes.len()));
        body_bytes.extend_from_slice(&chunk[..retained]);
    }
    if received > MAX_RESPONSE_BYTES {
        tracing::warn!(url = %shown_url, body_len = received, limit = MAX_RESPONSE_BYTES,
            "HTTP hook response exceeded 64KB, truncating");
    }

    let body_str = String::from_utf8_lossy(&body_bytes[..body_bytes.len().min(MAX_RESPONSE_BYTES)]);

    let parsed = serde_json::from_str::<HookResult>(&body_str);
    if !status.is_success() {
        // A refusal is a decision, not an outage: no policy may downgrade it.
        // Anything else in an error response must not become a clean result.
        let reason = format!("HTTP hook returned {status}");
        let mut failure = diagnostic_failure_result(config, event_name, &reason);
        if let Ok(body) = parsed
            && keep_refusals(&mut failure.result, body)
        {
            tracing::warn!(url = %shown_url, %status, "HTTP hook refused with an error status");
        }
        return failure;
    }

    // Parse JSON response as HookResult
    match parsed {
        Ok(result) => result.into(),
        Err(e) => {
            tracing::warn!(
                url = %shown_url,
                error = %e,
                "HTTP hook response is not valid HookResult JSON; applying failure policy"
            );
            config.failure_result(event_name, &e.to_string()).into()
        }
    }
}

/// Copy only the refusal signals of an error-status body onto the failure
/// result: a block, a deny, a stop, and an elicitation decline or cancel.
/// Grants and modifications (allow, ask, updated input or output, permission
/// updates, accept) are dropped. Returns whether any refusal was kept.
fn keep_refusals(target: &mut HookResult, body: HookResult) -> bool {
    let mut kept = false;
    if body.outcome == HookOutcome::Blocking {
        target.outcome = HookOutcome::Blocking;
        if body.reason.is_some() {
            target.reason = body.reason;
        }
        kept = true;
    }
    if body.permission_behavior == Some(PermissionBehavior::Deny) {
        target.permission_behavior = Some(PermissionBehavior::Deny);
        target.permission_decision_reason = body.permission_decision_reason;
        kept = true;
    }
    if body.prevent_continuation == Some(true) {
        target.prevent_continuation = Some(true);
        target.stop_reason = body.stop_reason;
        kept = true;
    }
    if matches!(
        body.elicitation_action,
        Some(ElicitationAction::Decline | ElicitationAction::Cancel)
    ) {
        target.elicitation_action = body.elicitation_action;
        kept = true;
    }
    kept
}

fn diagnostic_failure_result(
    config: &HookConfig,
    event_name: &str,
    reason: &str,
) -> HookExecutionResult {
    tracing::warn!(
        hook = %config.display_command(),
        event = %event_name,
        reason,
        policy = ?config.failure_policy(event_name),
        "HTTP hook failed"
    );
    let mut result = config.failure_result(event_name, reason);
    if result.outcome == HookOutcome::Success {
        result.outcome = HookOutcome::NonBlockingError;
        result.reason = Some(reason.to_owned());
    }
    result.into()
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
        // An unreachable port is refused at once on Unix; Windows retries the
        // connect for about as long as the hook's 2 s no-progress window, so
        // the window may end it first. Each path logs the redacted URL only.
        assert!(
            logs_contain("HTTP hook network error")
                || logs_contain("HTTP hook timed out")
                || logs_contain("hook execution failed"),
            "no network-failure log line"
        );
        assert!(
            logs_contain("http://127.0.0.1:1/<redacted>"),
            "no redacted network-failure log line"
        );
        assert!(!logs_contain(SECRET), "secret reached the log");
    }
}

fn transport_failure_result(
    config: &HookConfig,
    event_name: &str,
    reason: &str,
    timed_out: bool,
) -> HookExecutionResult {
    if timed_out {
        let mut execution = diagnostic_failure_result(config, event_name, reason);
        let error = format!("timed out: no progress during {reason}");
        execution.result.reason = Some(config.no_progress_reason(None, event_name, &error));
        if execution.result.outcome == HookOutcome::NonBlockingError {
            execution.no_progress_stop = Some(error);
        }
        execution
    } else {
        config.failure_result(event_name, reason).into()
    }
}

#[cfg(test)]
#[test]
fn allowing_transport_timeouts_remain_explicit_errors() {
    let config: HookConfig = serde_json::from_value(serde_json::json!({
        "type":"http", "command":"https://example.invalid", "on_failure":"allow"
    }))
    .unwrap();
    for phase in ["response", "body"] {
        let reason = format!("HTTP {phase} transport timeout");
        let result = transport_failure_result(&config, "PostToolUse", &reason, true);
        assert_eq!(result.result.outcome, HookOutcome::NonBlockingError);
        assert!(
            result
                .result
                .reason
                .as_deref()
                .unwrap()
                .contains("no progress")
        );
        assert!(
            result
                .no_progress_stop
                .as_deref()
                .unwrap()
                .contains("no progress")
        );
    }
}

fn hook_transport(client: &HookHttpTransport) -> &Client {
    client.client()
}

#[cfg(test)]
mod transport_retention_tests {
    use super::*;
    #[test]
    fn authentication_transport_is_not_discarded() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", HeaderValue::from_static("Bearer test"));
        let client = HookHttpTransport::builder()
            .default_headers(headers)
            .build()
            .unwrap();
        assert!(std::ptr::eq(hook_transport(&client), client.client()));
    }
    #[test]
    fn private_ca_transport_is_not_discarded() {
        let material = super::test_tls::generate();
        assert!(material.dir.path().join("server-key.pem").exists());
        assert!(!material.client_identity.is_empty());
        let ca = reqwest::Certificate::from_pem(&material.ca).unwrap();
        let client = HookHttpTransport::builder()
            .add_root_certificate(ca)
            .build()
            .unwrap();
        assert!(std::ptr::eq(hook_transport(&client), client.client()));
    }
    #[test]
    fn redirect_transport_is_not_discarded() {
        let client = HookHttpTransport::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert!(std::ptr::eq(hook_transport(&client), client.client()));
    }
}

#[cfg(test)]
#[path = "http_response_tests.rs"]
mod response_tests;

#[cfg(test)]
#[path = "../../tests/support/hook_tls.rs"]
mod test_tls;
