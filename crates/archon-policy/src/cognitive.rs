use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CognitivePolicy {
    pub enabled: bool,
    pub allow_autonomous_tick: bool,
    pub allow_background_daemon: bool,
    pub allow_tool_suppression: bool,
    pub allow_jepa_action_scoring: bool,
    pub allow_self_model_updates: bool,
    pub allow_autonomous_low_risk_apply: bool,
    pub max_autonomous_risk: String,
    pub require_human_for_prompt_changes: bool,
    pub require_human_for_policy_changes: bool,
    pub require_human_for_network_changes: bool,
    pub require_human_for_blocking_gate_changes: bool,
    pub store_raw_turn_text: bool,
}

impl Default for CognitivePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_autonomous_tick: false,
            allow_background_daemon: false,
            allow_tool_suppression: true,
            allow_jepa_action_scoring: false,
            allow_self_model_updates: false,
            allow_autonomous_low_risk_apply: false,
            max_autonomous_risk: "Low".into(),
            require_human_for_prompt_changes: true,
            require_human_for_policy_changes: true,
            require_human_for_network_changes: true,
            require_human_for_blocking_gate_changes: true,
            store_raw_turn_text: false,
        }
    }
}

impl CognitivePolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.max_autonomous_risk.as_str(), "Low" | "Medium") {
            return Err(format!(
                "policy.cognitive.max_autonomous_risk must be Low or Medium, got {:?}",
                self.max_autonomous_risk
            ));
        }
        Ok(())
    }

    pub fn is_passthrough(&self) -> bool {
        !self.enabled
    }

    pub fn can_suppress_tools(&self) -> bool {
        self.enabled && self.allow_tool_suppression
    }

    pub fn can_run_daemon(&self) -> bool {
        self.enabled && self.allow_autonomous_tick && self.allow_background_daemon
    }

    pub fn can_use_jepa(&self) -> bool {
        self.enabled && self.allow_jepa_action_scoring
    }

    pub fn can_update_self_model(&self) -> bool {
        self.enabled && self.allow_self_model_updates
    }

    pub fn can_auto_apply(&self) -> bool {
        self.enabled && self.allow_autonomous_low_risk_apply
    }

    pub fn prompt_changes_require_human(&self) -> bool {
        self.require_human_for_prompt_changes
    }

    pub fn policy_changes_require_human(&self) -> bool {
        self.require_human_for_policy_changes
    }

    pub fn network_changes_require_human(&self) -> bool {
        self.require_human_for_network_changes
    }

    pub fn blocking_gate_changes_require_human(&self) -> bool {
        self.require_human_for_blocking_gate_changes
    }
}
