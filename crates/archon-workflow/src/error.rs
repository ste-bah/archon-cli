use std::path::PathBuf;

use thiserror::Error;

pub type WorkflowResult<T> = Result<T, WorkflowError>;

/// The prefix every [`WorkflowError::HostCallTimeout`] renders with, so a
/// classifier that only has the text (the port carries errors as text once a
/// host wraps them) can still tell a host cut from a transport failure. Named
/// once here and matched by [`is_host_call_timeout_text`]; the display test
/// beside them keeps the two from drifting.
pub const HOST_CALL_TIMEOUT_MARKER: &str = "host call timeout:";

/// Does this error text carry a [`WorkflowError::HostCallTimeout`], however
/// deeply a host or a retry wrapper nested it?
pub fn is_host_call_timeout_text(error: &str) -> bool {
    error.contains(HOST_CALL_TIMEOUT_MARKER)
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("invalid workflow schema: expected archon.workflow.v1, got {0}")]
    InvalidSchema(String),
    #[error("invalid workflow spec: {0}")]
    SpecInvalid(String),
    #[error("unknown dependency '{dependency}' referenced by stage '{stage}'")]
    UnknownDependency { stage: String, dependency: String },
    #[error("dependency cycle detected: {0:?}")]
    DependencyCycle(Vec<String>),
    #[error("stage '{0}' requires a reducer")]
    MissingReducer(String),
    #[error("hard-coded provider/model is forbidden on stage '{0}'")]
    HardcodedModel(String),
    #[error("invalid fan-out contract: {0}")]
    InvalidFanout(String),
    #[error(
        "implementation fanout stage '{stage}' item '{item}' declares no per-item target_files; required while [workflow.write_coordinator] fail_on_undeclared_write = true"
    )]
    ImplementationFanoutMissingPerItemTargets { stage: String, item: String },
    #[error("stage '{stage}' requires field '{field}'")]
    MissingStageField { stage: String, field: &'static str },
    #[error("duplicate stage id '{0}'")]
    DuplicateStage(String),
    #[error("workflow run already exists: {0}")]
    RunAlreadyExists(String),
    #[error("workflow run not found: {0}")]
    RunNotFound(String),
    #[error("workflow state is corrupt: {0}")]
    StateCorrupt(String),
    #[error("artifact cannot be reused: {0}")]
    ArtifactInvalid(String),
    #[error("forbidden provider-private payload field stripped: {0}")]
    ForbiddenPayload(String),
    #[error("policy denied workflow action: {0}")]
    PolicyDenied(String),
    #[error("provider tier '{0}' could not be resolved")]
    ProviderTierUnresolved(String),
    #[error("workflow stage blocked: {0}")]
    StageBlocked(String),
    #[error("workflow stage failed: {0}")]
    StageFailed(String),
    /// The host's own per-dispatch timer ended an agent call.
    ///
    /// Typed apart from [`Self::StageFailed`] because the two are answered
    /// differently by every re-ask loop: a provider that dropped the call is
    /// re-asked, a session the host itself cut is not — the host decided that
    /// budget, and re-dispatching under it merely restarts the same session.
    /// Observed live as a 1800 s retry budget producing a third session: the
    /// cut arrived as `agent transport failed: subagent timed out after 1800s`
    /// and matched the transport-retry markers. The host raises this at the
    /// same point it writes the `call_timeout` transport row; the text carried
    /// is the underlying error, so the phrase-keyed interruption classifiers
    /// still see it.
    #[error("{HOST_CALL_TIMEOUT_MARKER} {0}")]
    HostCallTimeout(String),
    #[error("required workflow notification delivery failed: {0}")]
    NotificationDelivery(String),
    #[error("workflow paused by run control: {0}")]
    ControlPaused(String),
    #[error("workflow cancelled by run control: {0}")]
    ControlCancelled(String),
    #[error("workflow template is unsafe: {0}")]
    UnsafeTemplate(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A host-injected port (see [`crate::llm_client_port`]) failed. Carried
    /// transparently: the host's error already says what went wrong, and this
    /// crate knows nothing about the host's machinery that it could usefully
    /// add in front of it.
    #[error(transparent)]
    Port(Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error(transparent)]
    TomlSerialize(#[from] toml::ser::Error),
    #[error(transparent)]
    TomlDeserialize(#[from] toml::de::Error),
}

impl WorkflowError {
    /// Was this call ended by the host's own per-dispatch timer?
    pub fn is_host_call_timeout(&self) -> bool {
        matches!(self, Self::HostCallTimeout(_))
    }

    /// Wraps a host port failure. Takes the boxed error rather than a concrete
    /// type so hosts using `anyhow` (which converts into this box) do not have
    /// to flatten their context chain to cross the boundary.
    pub fn port(source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Port(source.into())
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

#[cfg(test)]
mod host_call_timeout_tests {
    use super::*;

    /// The marker the text classifier keys on is the one the variant renders.
    #[test]
    fn host_call_timeout_renders_its_marker_and_the_underlying_error() {
        let error = WorkflowError::HostCallTimeout(
            "agent transport failed: subagent timed out after 1800s".to_string(),
        );
        let text = error.to_string();
        assert!(error.is_host_call_timeout());
        assert!(text.starts_with(HOST_CALL_TIMEOUT_MARKER), "{text}");
        assert!(is_host_call_timeout_text(&text));
        assert!(text.contains("subagent timed out after 1800s"), "{text}");
        assert!(!is_host_call_timeout_text(
            "workflow stage failed: agent transport failed: subagent timed out after 1800s"
        ));
        assert!(!WorkflowError::StageFailed("x".into()).is_host_call_timeout());
    }
}
