use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProvenanceError>;

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("schema error: {0}")]
    Schema(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("verification error: {0}")]
    Verification(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
