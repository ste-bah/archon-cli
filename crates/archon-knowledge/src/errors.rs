use thiserror::Error;

#[derive(Debug, Error)]
pub enum KnowledgeError {
    #[error("knowledge schema error: {0}")]
    Schema(String),
    #[error("knowledge store error: {0}")]
    Store(String),
    #[error("invalid search mode: {0}")]
    InvalidSearchMode(String),
    #[error("invalid search options: {0}")]
    InvalidSearchOptions(String),
    /// A traceability input could not be read as declared.
    ///
    /// Raised rather than defaulted: a task file whose `implements:` list does
    /// not parse is a decomposition the graph cannot describe, and silently
    /// treating it as "claims nothing" would turn a malformed file into a
    /// clean report.
    #[error("traceability input error: {0}")]
    Traceability(String),
}

pub type Result<T> = std::result::Result<T, KnowledgeError>;
