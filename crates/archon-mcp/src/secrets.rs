//! Sanitize protocol errors at the boundary, preserving their typed variants.
use crate::types::{McpError, ServerConfig};
use archon_observability::secret_values::{SecretValues, is_credential_name};

impl ServerConfig {
    /// Only credential-named environment/header values belong to this server.
    pub fn configured_secrets(&self) -> SecretValues {
        let headers = self
            .headers
            .iter()
            .flat_map(|headers| headers.iter())
            .filter(|(name, _)| is_credential_name(name) || name.eq_ignore_ascii_case("Cookie"));
        let mut secrets = SecretValues::new(
            self.env
                .iter()
                .filter(|(name, _)| is_credential_name(name))
                .map(|(_, value)| value.as_str())
                .chain(headers.clone().map(|(_, value)| value.as_str())),
        );
        for (name, value) in headers {
            if name.to_ascii_uppercase().contains("AUTH") {
                secrets = secrets.with_authorization(value);
            }
        }
        secrets
    }
}
impl McpError {
    pub(crate) fn redacted(self, secrets: &SecretValues) -> Self {
        match self {
            Self::ConfigParse(text) => Self::ConfigParse(secrets.text(&text)),
            Self::ConfigIo(error) => Self::ConfigIo(std::io::Error::new(
                error.kind(),
                secrets.text(&error.to_string()),
            )),
            Self::Transport(text) => Self::Transport(secrets.text(&text)),
            Self::InitFailed { server, reason } => Self::InitFailed {
                server: secrets.text(&server),
                reason: secrets.text(&reason),
            },
            Self::ToolCallFailed(text) => Self::ToolCallFailed(secrets.text(&text)),
            Self::ServerNotFound(name) => Self::ServerNotFound(secrets.text(&name)),
            Self::ServerNotReady(name, state) => Self::ServerNotReady(secrets.text(&name), state),
            Self::Shutdown(text) => Self::Shutdown(secrets.text(&text)),
            Self::Json(error) => Self::Json(<serde_json::Error as serde::de::Error>::custom(
                secrets.text(&error.to_string()),
            )),
            Self::MaxRestartsExceeded(name) => Self::MaxRestartsExceeded(secrets.text(&name)),
            Self::Timeout(duration) => Self::Timeout(duration),
        }
    }
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
