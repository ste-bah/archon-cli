//! Typed availability of a disposable observation site.
use super::workflow_host_fault::{self as fault, Class};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnrunCause {
    BaselineUnavailable,
    BaselineFetchFailed,
    SnapshotTampered,
}

#[derive(Clone, Debug)]
pub(crate) struct Unrun(
    pub(crate) String,
    pub(crate) Class,
    // Read by the base-proof stage when it acquires a copy.
    #[allow(dead_code)] pub(crate) Option<UnrunCause>,
);

impl Unrun {
    pub(crate) fn refused(cause: UnrunCause, text: String) -> Self {
        let label = match cause {
            UnrunCause::BaselineUnavailable => "baseline_unavailable",
            UnrunCause::BaselineFetchFailed => "baseline_fetch_failed",
            UnrunCause::SnapshotTampered => "snapshot_tampered",
        };
        Self(format!("{label}: {text}"), Class::Permanent, Some(cause))
    }

    pub(crate) fn unknown(text: String) -> Self {
        Self(text, Class::Permanent, None)
    }
    pub(crate) fn error(error: anyhow::Error, context: String) -> Self {
        Self(
            format!("{context}: {}", fault::diagnostic(&error)),
            fault::classify(&error),
            None,
        )
    }
    pub(crate) fn transient(text: String) -> Self {
        Self(text, Class::Transient, None)
    }
}
impl From<String> for Unrun {
    fn from(text: String) -> Self {
        Self::unknown(text)
    }
}
impl From<&str> for Unrun {
    fn from(text: &str) -> Self {
        Self::unknown(text.into())
    }
}
