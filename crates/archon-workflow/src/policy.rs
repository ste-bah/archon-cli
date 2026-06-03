use serde::{Deserialize, Serialize};

use crate::config::WorkflowConfig;
use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{StageKind, StageSpec, WorkflowSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPolicy {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub allow_dynamic_specs: bool,
    #[serde(default)]
    pub allow_sandboxed_scripts: bool,
    #[serde(default = "default_true")]
    pub allow_parallel_agents: bool,
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: u32,
    #[serde(default = "default_max_agents")]
    pub max_agents_per_run: u32,
    #[serde(default = "default_local_provider_max_agents")]
    pub local_provider_max_agents: u32,
    #[serde(default = "default_true")]
    pub require_human_for_dangerous_tools: bool,
    #[serde(default = "default_true")]
    pub require_human_for_policy_mutation: bool,
    #[serde(default = "default_true")]
    pub require_human_for_config_mutation: bool,
    #[serde(default = "default_true")]
    pub require_human_for_git_push: bool,
}

impl Default for WorkflowPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_dynamic_specs: true,
            allow_sandboxed_scripts: false,
            allow_parallel_agents: true,
            max_parallelism: default_max_parallelism(),
            max_agents_per_run: default_max_agents(),
            local_provider_max_agents: default_local_provider_max_agents(),
            require_human_for_dangerous_tools: true,
            require_human_for_policy_mutation: true,
            require_human_for_config_mutation: true,
            require_human_for_git_push: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
    RequireHuman(String),
}

impl WorkflowPolicy {
    /// Build a policy seeded from `[workflow]` config so operator-tuned values
    /// (e.g. `local_provider_max_agents`, OQ-DWF-003) reach the executor.
    pub fn from_config(config: &WorkflowConfig) -> Self {
        Self {
            local_provider_max_agents: config.local_provider_max_agents,
            ..Self::default()
        }
    }

    pub fn validate_spec(&self, spec: &WorkflowSpec) -> WorkflowResult<()> {
        if !self.enabled {
            return Err(WorkflowError::PolicyDenied(
                "workflow policy disabled".into(),
            ));
        }
        if !self.allow_dynamic_specs {
            return Err(WorkflowError::PolicyDenied("dynamic specs disabled".into()));
        }
        if spec.max_parallelism > self.max_parallelism {
            return Err(WorkflowError::PolicyDenied(format!(
                "max_parallelism {} exceeds policy {}",
                spec.max_parallelism, self.max_parallelism
            )));
        }
        if spec.max_agents > self.max_agents_per_run {
            return Err(WorkflowError::PolicyDenied(format!(
                "max_agents {} exceeds policy {}",
                spec.max_agents, self.max_agents_per_run
            )));
        }
        for stage in &spec.stages {
            match self.stage_decision(stage) {
                PolicyDecision::Allow => {}
                PolicyDecision::Deny(reason) | PolicyDecision::RequireHuman(reason) => {
                    return Err(WorkflowError::PolicyDenied(reason));
                }
            }
        }
        Ok(())
    }

    pub fn stage_decision(&self, stage: &StageSpec) -> PolicyDecision {
        if stage.kind == StageKind::Fanout && !self.allow_parallel_agents {
            return PolicyDecision::Deny("parallel fan-out denied by policy".into());
        }
        if stage.kind == StageKind::Tool {
            let tool = stage.tool.as_deref().unwrap_or_default();
            if is_dangerous_tool(tool) && self.require_human_for_dangerous_tools {
                return PolicyDecision::RequireHuman(format!(
                    "tool stage '{}' requires human approval",
                    stage.id
                ));
            }
        }
        PolicyDecision::Allow
    }
}

fn is_dangerous_tool(tool: &str) -> bool {
    let lower = tool.to_ascii_lowercase();
    ["git push", "rm ", "chmod", "policy", "config", "release"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn default_true() -> bool {
    true
}

fn default_max_parallelism() -> u32 {
    16
}

fn default_max_agents() -> u32 {
    1_000
}

/// OQ-DWF-003 provisional default cap for local-only provider fan-out.
fn default_local_provider_max_agents() -> u32 {
    4
}
