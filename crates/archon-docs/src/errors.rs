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

    #[error("VLM policy denied: {message}")]
    VlmPolicyDenied { message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
