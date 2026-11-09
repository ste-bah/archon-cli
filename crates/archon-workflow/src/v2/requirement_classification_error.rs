//! Typed classification failures, retaining legacy deterministic string records.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClassificationError {
    Deterministic(String),
    Transient { transient: String },
}
impl std::ops::Deref for ClassificationError {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Self::Deterministic(message) | Self::Transient { transient: message } => message,
        }
    }
}
impl std::fmt::Display for ClassificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}
impl From<String> for ClassificationError {
    fn from(message: String) -> Self {
        Self::Deterministic(message)
    }
}
impl From<&str> for ClassificationError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}
