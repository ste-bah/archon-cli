//! Pipeline identity type.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Globally unique pipeline run identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineId(pub Uuid);

impl PipelineId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PipelineId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PipelineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for PipelineId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}
