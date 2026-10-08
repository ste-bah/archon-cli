use std::path::PathBuf;

use thiserror::Error;

use crate::lifecycle_host_port::TERMINAL_HOST_CALL_MARKER;

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

/// The prefix of the error a write session ends with when the tool guard
/// stopped it for thrashing past the read wall without writing (Issue-54).
/// Spelled by `archon_tools::workflow_read_guard::READ_WALL_THRASH_MARKER`;
/// this crate does not depend on that one, so the text is pinned here and
/// held to the guard's by the bin crate's host dispatch tests. Like the
/// host timeout it is a host cut, not a transport failure and not a verdict.
pub const READ_WALL_THRASH_MARKER: &str = "read-wall thrash:";

/// Did the tool guard end this session for read-wall thrash, however deeply
/// a host or a retry wrapper nested the text?
pub fn is_read_wall_thrash_text(error: &str) -> bool {
    error.contains(READ_WALL_THRASH_MARKER)
}

/// The prefix of the error a workflow session ends with when the runner
/// stopped it for making no progress: the same answers after its reminder,
/// and an unchanged working tree (Issue-213 C2d). Spelled by
/// `archon_tools::NO_PROGRESS_STOP_MARKER`; pinned here for the same reason as
/// [`READ_WALL_THRASH_MARKER`], and like it a host cut: never transport, never
/// a verdict, and the partial work is kept.
pub const NO_PROGRESS_STOP_MARKER: &str = "no-progress stop:";

/// Did the runner end this session for making no progress, however wrapped?
pub fn is_no_progress_stop_text(error: &str) -> bool {
    error.contains(NO_PROGRESS_STOP_MARKER)
}

/// Issue 364: the text a session ends with when its provider gave no answer
/// for a whole no-progress window of resends. Spelled by
/// `archon_llm::transport_idle::TRANSPORT_STALL_MARKER`. Not a verdict on the
/// work and not worth an immediate re-ask: the host pauses the run on it.
pub const TRANSPORT_STALL_MARKER: &str = archon_llm::transport_idle::TRANSPORT_STALL_MARKER;

/// Did the provider stay silent for a whole no-progress window, however
/// wrapped? Reads the whole source chain of `error`.
pub fn is_transport_stall(error: &(dyn std::error::Error + 'static)) -> bool {
    std::iter::successors(Some(error), |error| error.source())
        .any(|error| error.to_string().contains(TRANSPORT_STALL_MARKER))
}

/// The prefix of the error a session ends with when the host cut it for
/// inactivity — no model output, tool round or tool result for the configured
/// bound — rather than at its wall clock. Spelled by
/// `archon_tools::subagent_activity::INACTIVITY_TIMEOUT_MARKER`; pinned here
/// for the same reason as [`READ_WALL_THRASH_MARKER`], and held to it by the
/// bin crate's host dispatch tests. The host types the cut as a
/// [`WorkflowError::HostCallTimeout`], so no transport re-ask restarts it; this
/// marker is what tells the two host cuts apart in every record.
pub const INACTIVITY_TIMEOUT_MARKER: &str = "subagent inactivity timeout:";

/// Did the host cut this session for inactivity, however deeply wrapped?
pub fn is_inactivity_timeout_text(error: &str) -> bool {
    error.contains(INACTIVITY_TIMEOUT_MARKER)
}

/// Batch G2: the prefix of a failure the host resolved as its own
/// operational error -- an environment violation that survived a restore
/// and a re-run, a verifier that could not start or finish twice -- never a
/// verdict on the work. Classified with transport and timeout failures
/// (`BranchFailureKind::Execution`) wherever error text is classified, and
/// checked first, so no other phrase in the text can reroute it to a task.
pub const HOST_OPERATIONAL_ERROR_MARKER: &str =
    "host operational error (not a verdict on the work):";

/// Is this text the host's own operational error? Only text that BEGINS
/// with the marker counts: the host builds these messages itself
/// ([`WorkflowError::HostOperational`] renders it first), while agent or
/// product output quoted anywhere after the start of a host message can
/// never turn a real failure into a refunded one.
pub fn is_host_operational_text(error: &str) -> bool {
    let text = error.trim_start();
    // The one host wrapper a failed call's record carries around it
    // (`v2::script::failed_v2_result`'s summary).
    let text = text
        .strip_prefix("workflow v2 call '")
        .and_then(|rest| rest.split_once("' failed: "))
        .map_or(text, |(_, error)| error);
    text.starts_with(HOST_OPERATIONAL_ERROR_MARKER)
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
    /// A trusted host terminal stop; JavaScript text cannot create this evidence.
    #[error("{TERMINAL_HOST_CALL_MARKER} {0}")]
    TerminalHostCall(String),
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
    /// Batch G2: the host's own operational error -- an environment the
    /// host could not make trustworthy after restoring and re-running --
    /// never a verdict on the work. Rendered with
    /// [`HOST_OPERATIONAL_ERROR_MARKER`] first, so every text classifier
    /// files it with transport failures.
    #[error("{HOST_OPERATIONAL_ERROR_MARKER} {0}")]
    HostOperational(String),
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

    /// The refusal to continue this carries, when it is one (#241). Only the
    /// typed refusal counts; its words in any other error do not.
    pub fn continuation_refusal(&self) -> Option<String> {
        match self {
            Self::Port(source) => source
                .downcast_ref::<archon_tools::subagent_session::ContinuationRefused>()
                .map(|refused| refused.0.clone()),
            _ => None,
        }
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

#[cfg(test)]
mod operational_marker_tests {
    use super::*;

    /// Batch G2: only a message the host BEGINS with its marker is its
    /// operational error; agent or product text quoting it later is not.
    #[test]
    fn only_a_host_message_that_begins_with_the_marker_is_operational() {
        let typed = WorkflowError::HostOperational("verifier gave no verdict twice".into());
        assert!(is_host_operational_text(&typed.to_string()));
        // As a failed call's record summarises it: still the host's.
        let summary = crate::v2::script::failed_v2_result("verify-x-1", &typed).summary;
        assert!(is_host_operational_text(&summary), "{summary}");
        assert!(crate::v2::lifecycle_driver::is_transport_failure_text(
            &summary
        ));
        let quoted = WorkflowError::StageFailed(format!(
            "schema repair failed: agent said {HOST_OPERATIONAL_ERROR_MARKER} please refund"
        ));
        assert!(!is_host_operational_text(&quoted.to_string()));
        assert!(!is_host_operational_text(&format!(
            "declared artifact verifier failed with exit 1: {HOST_OPERATIONAL_ERROR_MARKER}"
        )));
    }
}
