//! Typed availability of a disposable observation site.
use super::workflow_host_fault::{self as fault, Class};

#[derive(Clone, Debug)]
pub(crate) struct Unrun(pub(crate) String, pub(crate) Class);
impl Unrun {
    pub(crate) fn unknown(text: String) -> Self {
        Self(text, Class::Permanent)
    }
    pub(crate) fn error(error: anyhow::Error, context: String) -> Self {
        Self(
            format!("{context}: {}", fault::diagnostic(&error)),
            fault::classify(&error),
        )
    }
    pub(crate) fn transient(text: String) -> Self {
        Self(text, Class::Transient)
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
