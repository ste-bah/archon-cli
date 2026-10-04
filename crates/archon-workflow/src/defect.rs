//! Stable identities owned by deterministic validators, separate from diagnostics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefectProvenance {
    HostValidator,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeterministicDefect {
    pub provenance: DefectProvenance,
    pub code: String,
    /// A valid task/check identity, or a structural slot when the id is rejected.
    pub subject: String,
    /// Validator-owned field/slot, never a submitted filename, path or value.
    pub location: String,
}

impl DeterministicDefect {
    pub fn new(code: &str, subject: impl Into<String>, location: impl Into<String>) -> Self {
        Self {
            provenance: DefectProvenance::HostValidator,
            code: code.into(),
            subject: subject.into(),
            location: location.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDefect {
    pub identity: DeterministicDefect,
    pub message: String,
}

impl ValidationDefect {
    pub fn new(code: &str, subject: &str, location: &str, message: impl Into<String>) -> Self {
        Self {
            identity: DeterministicDefect::new(code, subject, location),
            message: message.into(),
        }
    }
}

pub fn defect_message(defects: &[ValidationDefect]) -> String {
    defects
        .iter()
        .map(|defect| defect.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}
