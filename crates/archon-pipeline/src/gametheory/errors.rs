//! Game-theory pipeline error types.

/// Errors from the game-theory pipeline.
#[derive(Debug, thiserror::Error)]
pub enum GameTheoryError {
    #[error("empty situation: a situation description is required")]
    EmptySituation,

    #[error("missing agent source file: {path}")]
    MissingAgentFile { path: String },

    #[error("tier 1 execution failed: {message}")]
    Tier1Execution { message: String },

    #[error("fingerprint parse error: {message}")]
    FingerprintParse { message: String },

    #[error("storage error: {message}")]
    Storage { message: String },

    #[error(transparent)]
    Pipeline(#[from] crate::error::PipelineError),
}
