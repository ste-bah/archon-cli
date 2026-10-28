//! Stored provider diagnostics: construction always applies the shared redactor.
use std::{fmt, ops::Deref};

/// An immutable, credential-redacted provider diagnostic.
///
/// The private field and absence of mutable access ensure direct error variant
/// construction and forwarding cannot bypass redaction before storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticMessage(String);

impl From<&str> for DiagnosticMessage {
    fn from(message: &str) -> Self {
        Self(crate::provider_fault::diagnostic_message(message))
    }
}

impl From<String> for DiagnosticMessage {
    fn from(message: String) -> Self {
        Self::from(message.as_str())
    }
}

impl From<DiagnosticMessage> for String {
    fn from(message: DiagnosticMessage) -> Self {
        message.0
    }
}

impl DiagnosticMessage {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for DiagnosticMessage {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for DiagnosticMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
