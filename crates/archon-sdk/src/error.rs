//! SDK error types for TASK-CLI-305.

/// All errors from the Archon SDK.
///
/// This enum is `#[non_exhaustive]` — new variants may be added in future releases.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SdkError {
    /// Authentication failure: missing or invalid API key / token.
    #[error("authentication error: {0}")]
    Auth(String),

    /// Upstream API returned an HTTP error.
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },

    /// A tool handler returned an error or the tool could not be invoked.
    #[error("tool error: {0}")]
    Tool(String),

    /// Invalid or missing SDK configuration.
    #[error("configuration error: {0}")]
    Config(String),

    /// Session-related failure (not found, serialisation, etc.).
    #[error("session error: {0}")]
    Session(String),

    /// Filesystem I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation / deserialisation failure.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// No API key or auth token was provided to the builder.
    #[error("missing required field: auth (provide an API key or bearer token)")]
    MissingApiKey,

    /// Model field was explicitly cleared to an empty string.
    #[error("missing required field: model cannot be empty")]
    MissingModel,
}
