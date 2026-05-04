use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub network: NetworkPolicy,
    pub workers: WorkersPolicy,
    pub gametheory: GameTheoryPolicy,
    pub learning: LearningPolicy,
    pub docs: DocsPolicy,
}

impl Default for EffectivePolicy {
    fn default() -> Self {
        Self {
            network: NetworkPolicy::default(),
            workers: WorkersPolicy::default(),
            gametheory: GameTheoryPolicy::default(),
            learning: LearningPolicy::default(),
            docs: DocsPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub default: String,
    pub allow_cloud_vlm: bool,
    pub allow_web_strategy_agents: bool,
    pub allow_mcp_server_exposure: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            default: "deny".into(),
            allow_cloud_vlm: false,
            allow_web_strategy_agents: false,
            allow_mcp_server_exposure: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkersPolicy {
    pub ocr: String,
    pub embedding: String,
    pub vlm: String,
    pub web_fetch: String,
}

impl Default for WorkersPolicy {
    fn default() -> Self {
        Self {
            ocr: "allow-local".into(),
            embedding: "allow-local".into(),
            vlm: "deny".into(),
            web_fetch: "deny".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameTheoryPolicy {
    pub max_agents_per_council: usize,
    pub max_cost_usd: f64,
    pub enable_tier11: bool,
    pub allow_web_tools: bool,
}

impl Default for GameTheoryPolicy {
    fn default() -> Self {
        Self {
            max_agents_per_council: 12,
            max_cost_usd: 20.0,
            enable_tier11: false,
            allow_web_tools: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningPolicy {
    pub auto_apply_low_risk: bool,
    pub require_approval_for_prompt_changes: bool,
    pub require_approval_for_blocking_gates: bool,
    pub require_approval_for_network_changes: bool,
}

impl Default for LearningPolicy {
    fn default() -> Self {
        Self {
            auto_apply_low_risk: false,
            require_approval_for_prompt_changes: true,
            require_approval_for_blocking_gates: true,
            require_approval_for_network_changes: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocsPolicy {
    pub vlm: VlmPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlmPolicy {
    pub enabled: bool,
    pub mode: String,
    pub allow_cloud: bool,
    pub require_user_confirmation_for_cloud: bool,
}

impl Default for VlmPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "disabled".into(),
            allow_cloud: false,
            require_user_confirmation_for_cloud: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: String,
}

impl PolicyDecision {
    pub fn allow(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
        }
    }
}
