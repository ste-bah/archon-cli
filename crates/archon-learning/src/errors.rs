//! Typed errors for the governed learning subsystem per TSPEC §7.3.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum LearningError {
    #[error("proposal not found: {proposal_id}")]
    ProposalNotFound { proposal_id: String },

    #[error("manifest version not found: {version_id}")]
    ManifestVersionNotFound { version_id: String },

    #[error("concurrent modification: expected version {expected}, found {actual}")]
    ConcurrentModification { expected: String, actual: String },

    #[error("policy denied auto-apply for proposal {proposal_id}: {reason}")]
    PolicyDeniedAutoApply { proposal_id: String, reason: String },

    #[error("approval required for proposal {proposal_id}")]
    ApprovalRequired { proposal_id: String },

    #[error("rollback target unreachable: {version_id}")]
    RollbackTargetUnreachable { version_id: String },

    #[error("storage error: {message}")]
    Storage { message: String },

    #[error("validation error: {message}")]
    Validation { message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Anyhow(String),
}

impl From<anyhow::Error> for LearningError {
    fn from(e: anyhow::Error) -> Self {
        LearningError::Anyhow(e.to_string())
    }
}

// ── CozoDB version-coupled string matchers ────────────────────────────────────

/// Substring that Cozo returns when a queried relation does not exist.
pub const COZO_RELATION_NOT_FOUND: &str = "Cannot find requested stored relation";

/// Phrases Cozo uses for "relation already exists" errors.
pub const COZO_RELATION_ALREADY_EXISTS: &[&str] =
    &["conflicts with an existing", "already exists"];
