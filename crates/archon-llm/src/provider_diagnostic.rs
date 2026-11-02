//! Stored provider diagnostics: construction always applies the shared redactor.
//!
//! The classified text stays in memory, private, for classification only. It
//! is the field text exactly as 86fbab995 stored it: unredacted where that
//! code stored provider text as is, redacted where it redacted at the source
//! (`redacted_at_source`) or rendered it (`map_text`). So every classifier
//! sees exactly the input it saw at 86fbab995. Every egress (Display, Debug, Serialize, String conversion, Deref) reads the
//! redacted text. This follows the `secrecy` crate pattern: the secret has no
//! leaking Debug and no Serialize, and access to it is explicit and auditable
//! (docs.rs/secrecy: "Prevent accidental leakage of secrets via channels like
//! debug logging"; reads go through `ExposeSecret`). Here the one explicit,
//! crate-private reader is `raw()`.
use std::{fmt, ops::Deref};
#[path = "provider_diagnostic_reset.rs"]
mod reset;

/// An immutable, credential-redacted provider diagnostic.
///
/// The private fields and absence of mutable access ensure direct error variant
/// construction and forwarding cannot bypass redaction before storage. `raw`
/// is never displayed, serialized, cloned into a value that leaves this type,
/// or returned by a public accessor; only crate classifiers read it.
#[derive(Clone, Default)]
pub struct DiagnosticMessage {
    raw: String,
    text: String,
}

impl From<&str> for DiagnosticMessage {
    fn from(message: &str) -> Self {
        Self {
            raw: message.to_string(),
            text: crate::provider_fault::diagnostic_message(message),
        }
    }
}

impl From<String> for DiagnosticMessage {
    fn from(message: String) -> Self {
        Self::from(message.as_str())
    }
}
impl From<&String> for DiagnosticMessage {
    fn from(message: &String) -> Self {
        Self::from(message.as_str())
    }
}
impl From<&DiagnosticMessage> for DiagnosticMessage {
    fn from(message: &DiagnosticMessage) -> Self {
        message.clone()
    }
}
impl From<DiagnosticMessage> for String {
    fn from(message: DiagnosticMessage) -> Self {
        message.text
    }
}
impl DiagnosticMessage {
    /// Pair raw text with its egress rendering; the rendering is redacted again.
    pub(crate) fn from_raw_and_text(raw: String, text: &str) -> Self {
        Self {
            raw,
            text: crate::provider_fault::diagnostic_message(text),
        }
    }
    /// The unredacted provider text, for classifiers in this crate only.
    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }
    /// A diagnostic whose source redacted the text before storing it, as
    /// 86fbab995 did at that site: classifiers see the redacted text too.
    pub fn redacted_at_source(message: &str) -> Self {
        let text = crate::provider_fault::diagnostic_message(message);
        Self {
            raw: text.clone(),
            text,
        }
    }
    /// Replace the stored text with a rendering of it. 86fbab995 stored the
    /// rendering itself, so classifiers read the rendering from then on.
    pub fn map_text(self, render: impl FnOnce(&str) -> String) -> Self {
        Self::redacted_at_source(&render(self.as_str()))
    }
    /// Prefix context, as the original composed `format!("{prefix}{body}")`.
    pub fn with_prefix(&self, prefix: &str) -> Self {
        Self {
            raw: format!("{prefix}{}", self.raw),
            // Redact the composed egress text again: context can complete a
            // credential shape, and a rendered body keeps its rendering.
            text: crate::provider_fault::diagnostic_message(&format!("{prefix}{}", self.text)),
        }
    }
    pub fn reset_hint_at(
        &self,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        reset::parse(&self.raw).map(|hint| hint.at(observed_at))
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn mentions_status(&self, status: u16) -> bool {
        self.raw.contains(&status.to_string())
    }
    pub fn is_rate_limited(&self) -> bool {
        crate::provider_fault::rate_limit_matches(
            &self.raw.to_ascii_lowercase(),
            crate::provider_fault::RateLimitEvidence::RequestPressure,
        )
    }
    pub fn is_user_cancelled(&self) -> bool {
        let message = self.raw.trim().to_ascii_lowercase();
        message.contains("cancelled by user")
            || message.contains("canceled by user")
            || message.contains("user cancelled")
            || message.contains("user canceled")
            || message.contains("aborted by user")
            || message.contains("user aborted")
    }
    pub fn is_incomplete_stream(&self) -> bool {
        self.raw.contains("stream ended before message_stop")
    }
    pub fn leading_status(&self) -> Option<u16> {
        crate::provider_fault::status_field(&self.raw)
    }
}
impl Deref for DiagnosticMessage {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Debug for DiagnosticMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DiagnosticMessage")
            .field(&self.text)
            .finish()
    }
}
impl fmt::Display for DiagnosticMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl serde::Serialize for DiagnosticMessage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> serde::Deserialize<'de> for DiagnosticMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Stored text is already redacted: it is classified as stored, and
        // the raw text is never reconstructed. Earlier records may carry a
        // `facts` object; it is ignored.
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Text(String),
            WithFacts {
                text: String,
                #[allow(dead_code)]
                facts: serde::de::IgnoredAny,
            },
        }
        let text = match <Stored as serde::Deserialize>::deserialize(deserializer)? {
            Stored::Text(text) | Stored::WithFacts { text, .. } => text,
        };
        let text = crate::provider_fault::diagnostic_message(&text);
        Ok(Self {
            raw: text.clone(),
            text,
        })
    }
}
// Equality compares the stored (egress) text only, so it is no oracle for
// the classified text.
impl PartialEq for DiagnosticMessage {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}
impl Eq for DiagnosticMessage {}
impl PartialEq<str> for DiagnosticMessage {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for DiagnosticMessage {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}
impl PartialEq<String> for DiagnosticMessage {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

#[cfg(test)]
#[path = "provider_diagnostic_tests.rs"]
mod tests;
