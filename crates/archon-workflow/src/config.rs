use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::spec::ProviderTier;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_parallelism")]
    pub default_max_parallelism: u32,
    #[serde(default = "default_agents")]
    pub default_max_agents: u32,
    #[serde(default = "default_retention")]
    pub default_artifact_retention_days: u32,
    #[serde(default = "default_allow_generated")]
    pub allow_generated_workflow_specs: bool,
    #[serde(default = "default_allow_templates")]
    pub allow_saved_templates: bool,
    #[serde(default = "default_local_provider_max_agents")]
    pub local_provider_max_agents: u32,
    #[serde(default)]
    pub provider_tiers: BTreeMap<ProviderTier, String>,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_max_parallelism: default_parallelism(),
            default_max_agents: default_agents(),
            default_artifact_retention_days: default_retention(),
            allow_generated_workflow_specs: true,
            allow_saved_templates: true,
            local_provider_max_agents: default_local_provider_max_agents(),
            provider_tiers: default_provider_tiers(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_parallelism() -> u32 {
    8
}

fn default_agents() -> u32 {
    200
}

fn default_retention() -> u32 {
    90
}

fn default_allow_generated() -> bool {
    true
}

fn default_allow_templates() -> bool {
    true
}

/// OQ-DWF-003 provisional default: conservative cap for local-only providers.
fn default_local_provider_max_agents() -> u32 {
    4
}

pub fn default_provider_tiers() -> BTreeMap<ProviderTier, String> {
    [
        ProviderTier::Planner,
        ProviderTier::Researcher,
        ProviderTier::Coder,
        ProviderTier::Critic,
        ProviderTier::Cheap,
        ProviderTier::Vision,
        ProviderTier::Local,
        ProviderTier::Reducer,
    ]
    .into_iter()
    .map(|tier| (tier, "auto".to_string()))
    .collect()
}
