use thiserror::Error;

#[derive(Error, Debug)]
pub enum DocsError {
    #[error("OCR API error: {message}")]
    OcrApi {
        message: String,
        status_code: Option<u16>,
    },

    #[error("OCR rate limited: retry after {retry_after_secs}s — {message}")]
    OcrRateLimit {
        retry_after_secs: u64,
        message: String,
    },

    #[error("OCR timeout: {message}")]
    OcrTimeout { message: String },

    #[error("OCR file error ({path}): {message}")]
    OcrFile { path: String, message: String },

    #[error("OCR authentication failed: {message}")]
    OcrAuthentication { message: String },

    #[error("Storage error: {message}")]
    Storage { message: String },

    #[error("Validation error: {message}")]
    Validation { message: String },

    #[error("Unsupported media type: {media_type}")]
    UnsupportedMediaType { media_type: String },

    #[error("Embedding error: {message}")]
    Embedding { message: String },

    #[error("Retrieval error: {message}")]
    Retrieval { message: String },

    #[error("Model not configured: {message}")]
    ModelNotConfigured { message: String },

    #[error("VLM policy denied: {message}")]
    VlmPolicyDenied { message: String },

    #[error("VLM provider error ({provider}): {message}")]
    VlmProvider {
        provider: String,
        message: String,
        status_code: Option<u16>,
    },

    #[error("VLM rate limited ({provider}): retry after {retry_after_secs}s — {message}")]
    VlmRateLimit {
        provider: String,
        retry_after_secs: u64,
        message: String,
    },

    #[error("VLM authentication failed ({provider}): {message}")]
    VlmAuthentication { provider: String, message: String },

    #[error("VLM timeout ({provider}): {message}")]
    VlmTimeout { provider: String, message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// CozoDB version-coupled string matchers
// ---------------------------------------------------------------------------

/// Substring that Cozo 0.7.x returns when a queried relation does not exist.
/// If Cozo changes phrasing in a future version, update here only.
pub const COZO_RELATION_NOT_FOUND: &str = "Cannot find requested stored relation";

/// Phrases Cozo 0.7.x uses for "relation already exists" errors.
/// Used by `run_create` to suppress idempotent-create errors.
pub const COZO_RELATION_ALREADY_EXISTS: &[&str] = &["conflicts with an existing", "already exists"];
