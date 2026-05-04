use thiserror::Error;

pub type Result<T> = std::result::Result<T, MeaningError>;

#[derive(Debug, Error)]
pub enum MeaningError {
    #[error("schema error: {0}")]
    Schema(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid triplet: {0}")]
    InvalidTriplet(String),
}
