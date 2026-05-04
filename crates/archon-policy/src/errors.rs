pub type Result<T> = std::result::Result<T, PolicyError>;

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy I/O error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("policy parse error at {path}: {message}")]
    Parse { path: String, message: String },
}
