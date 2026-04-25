//! Pipeline error types.

/// Errors from the pipeline execution system.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("parse error in '{path}'{}: {msg}", line.map(|l| format!(" at line {l}")).unwrap_or_default())]
    ParseError {
        path: String,
        line: Option<usize>,
        msg: String,
    },

    #[error("validation error: {0}")]
    ValidationError(String),

    #[error("cycle detected: {0:?}")]
    CycleDetected(Vec<String>),

    #[error("missing step: {0}")]
    MissingStep(String),

    #[error("step '{step}' failed: {msg}")]
    StepFailed { step: String, msg: String },

    #[error("timeout{}", step.as_ref().map(|s| format!(" on step '{s}'")).unwrap_or_default())]
    Timeout { step: Option<String> },

    #[error("state corrupted: {0}")]
    StateCorrupted(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl PipelineError {
    /// Returns `true` for errors that should be retried (transient failures).
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::StepFailed { .. } | Self::Timeout { .. })
    }
}
