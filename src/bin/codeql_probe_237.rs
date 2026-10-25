//! Throwaway CodeQL probe for #237 (never merged): the raw line must be
//! flagged by rust/cleartext-logging; the redact_url line must not be.
fn main() {
    let token_url = std::env::var("PROBE_TOKEN_URL").unwrap_or_default();
    tracing::info!("raw: {}", token_url);
    tracing::info!("redacted: {}", archon_core::url_redact::redact_url(&token_url));
}
