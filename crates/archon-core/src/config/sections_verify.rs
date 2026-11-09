//! Limits for declared verifier invocations.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyConfig {
    #[serde(default = "tokens")]
    pub refuse_tokens: Vec<String>,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
    #[serde(default = "floor")]
    pub min_free_disk_mib: u64,
}

fn tokens() -> Vec<String> {
    super::embedded_defaults::embedded_section(&["workflow", "verify", "refuse_tokens"])
}
fn timeout() -> u64 {
    super::embedded_defaults::embedded_section(&["workflow", "verify", "timeout_secs"])
}
fn floor() -> u64 {
    super::embedded_defaults::embedded_section(&["workflow", "verify", "min_free_disk_mib"])
}

impl Default for VerifyConfig {
    fn default() -> Self {
        super::embedded_defaults::embedded_section(&["workflow", "verify"])
    }
}

pub(super) fn validate(config: &VerifyConfig) -> Result<(), super::ConfigError> {
    if config
        .refuse_tokens
        .iter()
        .any(|token| token.trim().is_empty())
    {
        return Err(super::ConfigError::ValidationError(
            "workflow.verify.refuse_tokens must contain no empty token".into(),
        ));
    }
    if config.timeout_secs == 0 {
        return Err(super::ConfigError::ValidationError(
            "workflow.verify.timeout_secs must be at least 1".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "sections_verify_tests.rs"]
mod tests;
