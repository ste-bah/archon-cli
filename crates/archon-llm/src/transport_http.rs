//! Connection lifetime policy shared by long-lived provider clients.

/// Open a fresh connection for every request, including retries. A socket can
/// become unusable during host sleep without the monotonic clock advancing.
/// Not retaining connections covers that case for any wall-clock jump, and
/// also prevents reuse after a connection-class error. Request timeouts remain
/// the caller's policy.
pub fn client_builder() -> reqwest::ClientBuilder {
    // In the underlying pool, zero disables pooling entirely (including
    // multiplexed connections); an idle expiry alone cannot cover host sleep.
    reqwest::Client::builder().pool_max_idle_per_host(0)
}

/// Decode the common status/body boundary used by native HTTP providers.
pub(crate) async fn response_error(
    response: reqwest::Response,
    map: fn(u16, String) -> crate::provider::LlmError,
) -> crate::provider::LlmError {
    let wait = response_retry_after(&response);
    let status = response.status().as_u16();
    crate::provider_fault::note_response(status, wait);
    let headers = response.headers().clone();
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) => return crate::provider_fault::body_read_error(status, &headers, error),
    };
    let classified = crate::provider_fault::from_status(status, &headers, body.clone());
    let error = map(status, body);
    // Keep established provider remedies and default waits only when their
    // recovery class agrees with the shared status/header/body conversion.
    let error = if error.is_operational() != classified.is_operational() {
        classified
    } else {
        error
    };
    with_retry_after(error, wait)
}

pub(crate) fn response_retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()
        .and_then(|value| {
            crate::anthropic_support::retry_after_from_header(value, chrono::Utc::now())
        })
}

pub(crate) fn with_retry_after(
    mut error: crate::provider::LlmError,
    wait: Option<u64>,
) -> crate::provider::LlmError {
    use crate::provider::LlmError;
    if let Some(wait) = wait {
        match &mut error {
            LlmError::RateLimited {
                retry_after_secs,
                from_provider,
                pipeline_retry_after_secs,
            } => {
                *retry_after_secs = wait;
                *from_provider = true;
                *pipeline_retry_after_secs = Some(wait);
            }
            LlmError::Overloaded {
                retry_after_secs, ..
            } => *retry_after_secs = Some(wait),
            LlmError::Server {
                pipeline_retry_after_secs,
                ..
            } => *pipeline_retry_after_secs = Some(wait),
            _ => {}
        }
    }
    error
}
