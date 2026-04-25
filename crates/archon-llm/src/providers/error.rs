//! TASK-AGS-701: Unified provider error type.
//!
//! Spec: 01-functional-spec.md ERR-PROV-01 (line 1841) and
//! 02-technical-spec.md TECH-AGS-PROVIDERS. Every Phase 7 provider impl
//! (702..706) returns `ProviderError`. Retry/backoff (TASK-AGS-708) inspects
//! the variant to decide whether a failure is retryable.

use thiserror::Error;

/// Errors returned by `archon-llm` providers.
///
/// `Http` transparently wraps `reqwest::Error` so `?` propagation works
/// from any HTTP call site. Downstream retry logic matches on the other
/// variants for fine-grained classification.
#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("missing credential: env var {var} not set")]
    MissingCredential { var: String },

    #[error("provider {name} authentication failed: {detail}")]
    AuthFailed { name: String, detail: String },

    #[error("provider {name} unreachable: {cause}")]
    Unreachable { name: String, cause: String },

    #[error("invalid response from {name}: {detail}")]
    InvalidResponse { name: String, detail: String },

    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
