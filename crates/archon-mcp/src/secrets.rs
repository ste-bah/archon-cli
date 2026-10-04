//! Sanitize protocol errors at the boundary, preserving their typed variants.
use crate::types::{McpError, ServerConfig};
use archon_observability::redaction::redact_text;
use archon_observability::secret_values::SecretValues;

impl ServerConfig {
    /// Environment and header values supplied to this server (eight chars or more).
    pub fn configured_secrets(&self) -> SecretValues {
        SecretValues::new(
            self.env
                .values()
                .chain(self.headers.iter().flat_map(|headers| headers.values()))
                .map(String::as_str),
        )
    }
}
impl McpError {
    pub(crate) fn redacted(self) -> Self {
        match self {
            Self::ConfigParse(text) => Self::ConfigParse(redact_text(&text)),
            Self::ConfigIo(error) => Self::ConfigIo(std::io::Error::new(
                error.kind(),
                redact_text(&error.to_string()),
            )),
            Self::Transport(text) => Self::Transport(redact_text(&text)),
            Self::InitFailed { server, reason } => Self::InitFailed {
                server: redact_text(&server),
                reason: redact_text(&reason),
            },
            Self::ToolCallFailed(text) => Self::ToolCallFailed(redact_text(&text)),
            Self::ServerNotFound(name) => Self::ServerNotFound(redact_text(&name)),
            Self::ServerNotReady(name, state) => Self::ServerNotReady(redact_text(&name), state),
            Self::Shutdown(text) => Self::Shutdown(redact_text(&text)),
            Self::Json(error) => Self::Json(<serde_json::Error as serde::de::Error>::custom(
                redact_text(&error.to_string()),
            )),
            Self::MaxRestartsExceeded(name) => Self::MaxRestartsExceeded(redact_text(&name)),
            Self::Timeout(duration) => Self::Timeout(duration),
        }
    }
}
