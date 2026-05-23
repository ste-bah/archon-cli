#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("video source not found: {path}")]
    SourceNotFound { path: String },

    #[error("unsupported video source scheme: {scheme}")]
    UnsupportedScheme { scheme: String },

    #[error("playlist and channel video URLs are not supported: {url}")]
    PlaylistRejected { url: String },

    #[error("policy denied: {reason}")]
    PolicyDenied { reason: String },

    #[error("binary not found: {name} at {path}")]
    BinaryNotFound { name: String, path: String },

    #[error("metadata extraction failed: {message}")]
    MetadataFailed { message: String },

    #[error("ASR provider unavailable: {message}")]
    AsrProviderUnavailable { message: String },

    #[error("acquisition failed: {message}")]
    AcquisitionFailed { message: String },

    #[error("frame extraction failed: {message}")]
    FrameExtractionFailed { message: String },

    #[error("image decode failed: {message}")]
    ImageDecodeFailed { message: String },

    #[error("no evidence extracted; all enabled paths produced zero chunks")]
    NoEvidenceExtracted,

    #[error("schema error: {0}")]
    Schema(#[from] anyhow::Error),

    #[error("store error: {message}")]
    Store { message: String },
}
