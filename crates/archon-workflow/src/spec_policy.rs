use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            base_delay_ms: default_base_delay_ms(),
        }
    }
}

fn default_max_attempts() -> u32 {
    1
}

fn default_base_delay_ms() -> u64 {
    1_000
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPolicy {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default = "default_store_agent_outputs")]
    pub store_agent_outputs: bool,
    #[serde(default)]
    pub redact_provider_private_payloads: bool,
}

impl Default for ArtifactPolicy {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
            store_agent_outputs: default_store_agent_outputs(),
            redact_provider_private_payloads: true,
        }
    }
}

fn default_retention_days() -> u32 {
    90
}

fn default_store_agent_outputs() -> bool {
    true
}
