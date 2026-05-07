pub type Result<T> = std::result::Result<T, ConstellationError>;

#[derive(Debug, thiserror::Error)]
pub enum ConstellationError {
    #[error("store error: {0}")]
    Store(String),
    #[error("schema error: {0}")]
    Schema(String),
    #[error("meaning error: {0}")]
    Meaning(#[from] archon_meaning::MeaningError),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("no constellation centroid found for target '{0}'")]
    MissingCentroid(String),
    #[error("unknown constellation target '{0}'")]
    UnknownTarget(String),
    #[error("constellation target '{0}' requires an explicit --session <id> bootstrap source")]
    NeedsExplicitSession(String),
    #[error(
        "invalid constellation target '{0}' (expected project, research-domain, strategic-workflow, memory, docs, or session)"
    )]
    InvalidTarget(String),
    #[error("input text is empty")]
    EmptyInput,
}
