//! Credentials for the Anthropic-compatible endpoint, honouring `[api] auth`.

use archon_core::config::{ApiAuth, ApiConfig};
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::auth::{AuthError, AuthProvider, resolve_auth_with_keys};

/// Resolve credentials the way every Anthropic client always has, then let an
/// endpoint declared keyless (`[api] auth = "none"`) proceed without one. A
/// credential that resolves is used in both modes; with the default, a missing
/// one is the same error as before, and every other failure stays an error.
/// `auth_token` is `ANTHROPIC_AUTH_TOKEN`, which callers read from the process.
pub(crate) fn resolve_configured_auth(
    api: &ApiConfig,
    env_vars: &ArchonEnvVars,
    auth_token: Option<&str>,
) -> Result<AuthProvider, AuthError> {
    let resolved = resolve_auth_with_keys(
        env_vars.anthropic_api_key.as_deref(),
        env_vars.archon_api_key.as_deref(),
        env_vars.archon_oauth_token.as_deref(),
        auth_token,
    );
    match (api.auth, resolved) {
        (ApiAuth::None, Err(AuthError::NoCredentials(_))) => Ok(AuthProvider::keyless()),
        (_, resolved) => resolved,
    }
}

#[cfg(test)]
#[path = "llm_auth_tests.rs"]
mod tests;
