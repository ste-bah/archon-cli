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

    #[error("agent not found in registry: {key}")]
    AgentNotFound { key: String },

    #[error("routing cycle detected: {cycle}")]
    RoutingCycle { cycle: String },

    #[error("condition error in expression '{expression}': {message}")]
    ConditionError { expression: String, message: String },

    #[error("budget exceeded: spent ${spent_usd:.2} of ${cap_usd:.2} cap")]
    BudgetExceeded { spent_usd: f64, cap_usd: f64 },

    #[error("specialist '{agent_key}' failed: {message}")]
    SpecialistFailed { agent_key: String, message: String },

    #[error(
        "LLM provider required for {operation}; configure ANTHROPIC_API_KEY or an Anthropic auth token"
    )]
    LlmUnavailable { operation: String },

    #[error("section writer failed for '{section_title}': {message}")]
    SectionWriterFailed {
        section_title: String,
        message: String,
    },

    #[error("validation error: {message}")]
    Validation { message: String },

    #[error("I/O error: {message}")]
    Io { message: String },

    #[error("spec file not found; searched: {}", .searched_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    SpecNotFound {
        searched_paths: Vec<std::path::PathBuf>,
    },

    #[error(transparent)]
    Pipeline(#[from] crate::error::PipelineError),
}
