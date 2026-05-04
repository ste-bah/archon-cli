use std::time::Duration;

use crate::provider::LlmError;

pub async fn run_codex_tls_preflight_url(
    http: &reqwest::Client,
    url: &str,
) -> Result<(), LlmError> {
    let resp = http.head(url).timeout(Duration::from_secs(10)).send().await;
    match resp {
        Ok(r) if r.status().as_u16() < 500 => Ok(()),
        Ok(r) => Err(LlmError::Http(format!(
            "auth.openai.com unexpected {}: {}",
            r.status(),
            r.url()
        ))),
        Err(e) if e.is_timeout() => Err(LlmError::Http(
            "auth.openai.com timeout - check HTTPS_PROXY / network".into(),
        )),
        Err(e) if e.is_connect() => Err(LlmError::Http(format!(
            "auth.openai.com unreachable: {e}"
        ))),
        Err(e) if e.to_string().to_lowercase().contains("certificate") => Err(LlmError::Auth(
            "TLS certificate error connecting to auth.openai.com - corporate proxy MitM? Check HTTPS_PROXY and system trust store".into(),
        )),
        Err(e) => Err(LlmError::Http(format!("auth.openai.com error: {e}"))),
    }
}

pub async fn run_codex_tls_preflight(http: &reqwest::Client) -> Result<(), LlmError> {
    run_codex_tls_preflight_url(http, "https://auth.openai.com/").await
}
