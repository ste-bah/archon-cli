//! Error types for the completion integrity subsystem.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EvidenceEngineError {
    #[error("storage error: {message}")]
    Storage { message: String },

    #[error("validation error: {message}")]
    Validation { message: String },

    #[error("claim extraction failed: {message}")]
    ClaimExtraction { message: String },

    #[error("evidence resolution failed: {message}")]
    EvidenceResolution { message: String },

    #[error("verification gate '{gate}' failed: {message}")]
    GateFailure { gate: String, message: String },

    #[error("report assembly failed: {message}")]
    ReportAssembly { message: String },

    #[error("incident recording failed: {message}")]
    IncidentRecording { message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
