use std::time::Duration;

use crate::provider::LlmError;

pub async fn run_codex_tls_preflight_url(
    http: &reqwest::Client,
    url: &str,
) -> Result<(), LlmError> {
    let resp = http.head(url).timeout(Duration::from_secs(10)).send().await;
    match resp {
        Ok(r) if r.status().as_u16() < 500 && !matches!(r.status().as_u16(), 408 | 429) => Ok(()),
        Ok(r) => {
            Err(
                crate::transport_http::response_error(r, |status, message| LlmError::Server {
                    status,
                    message,
                    pipeline_retry_after_secs: None,
                })
                .await,
            )
        }
        Err(error) => Err(crate::provider_fault::from_transport(error)),
    }
}

pub async fn run_codex_tls_preflight(http: &reqwest::Client) -> Result<(), LlmError> {
    run_codex_tls_preflight_url(http, "https://auth.openai.com/").await
}
