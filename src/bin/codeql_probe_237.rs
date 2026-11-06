//! Throwaway CodeQL probe for #237 (never merged): each raw line must be
//! flagged by rust/cleartext-logging; each redact_url line must not be.
fn main() {
    let password = std::env::var("PROBE_PASSWORD").unwrap_or_default();
    let api_key = std::env::var("PROBE_API_KEY").unwrap_or_default();
    tracing::info!("raw password: {}", password);
    tracing::info!("raw api key: {}", api_key);
    tracing::info!("redacted password: {}", archon_core::url_redact::redact_url(&password));
    tracing::info!("redacted api key: {}", archon_core::url_redact::redact_url(&api_key));
}
