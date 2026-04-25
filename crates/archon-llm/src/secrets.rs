//! TASK-AGS-701: API key secret wrapper.
//!
//! NFR-SECURITY-001: API keys must never appear in logs, tracing spans,
//! Debug output, or Display output. `ApiKey` wraps `secrecy::SecretString`
//! and provides redacting `Debug`/`Display` impls so it is safe to pass
//! through `tracing::debug!`, `println!("{:?}", ...)`, and error chains.

use secrecy::{ExposeSecret, SecretString};
use std::env;

use crate::providers::error::ProviderError;

/// Newtype wrapping a secret API key.
///
/// The stored value is a `SecretString`, not a plain `String`. The only way
/// to retrieve the underlying bytes is through `expose()`, which makes every
/// read site grep-able.
pub struct ApiKey(SecretString);

impl ApiKey {
    /// Wrap a raw string as an `ApiKey`.
    pub fn new(value: String) -> Self {
        ApiKey(SecretString::new(value.into()))
    }

    /// Borrow the underlying secret as `&str`. Call sites must not log the
    /// result. Prefer passing `&ApiKey` through APIs and only exposing at the
    /// HTTP boundary.
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }

    /// Read an API key from an environment variable.
    ///
    /// Returns `ProviderError::MissingCredential { var }` if the variable is
    /// unset or empty.
    pub fn from_env(var: &str) -> Result<Self, ProviderError> {
        match env::var(var) {
            Ok(v) if !v.is_empty() => Ok(ApiKey::new(v)),
            _ => Err(ProviderError::MissingCredential {
                var: var.to_string(),
            }),
        }
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***redacted***)")
    }
}

impl std::fmt::Display for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***redacted***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let k = ApiKey::new("sk-abc".to_string());
        assert_eq!(format!("{:?}", k), "ApiKey(***redacted***)");
    }

    #[test]
    fn display_is_redacted() {
        let k = ApiKey::new("sk-abc".to_string());
        assert_eq!(format!("{}", k), "***redacted***");
    }

    #[test]
    fn expose_returns_raw() {
        let k = ApiKey::new("sk-abc".to_string());
        assert_eq!(k.expose(), "sk-abc");
    }

    #[test]
    fn from_env_missing_returns_error() {
        let var = "ARCHON_AGS_701_UNIT_TEST_MISSING";
        let err = ApiKey::from_env(var).unwrap_err();
        match err {
            ProviderError::MissingCredential { var: got } => assert_eq!(got, var),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
