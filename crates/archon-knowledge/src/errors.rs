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
}

pub type Result<T> = std::result::Result<T, KnowledgeError>;
