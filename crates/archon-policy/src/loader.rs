use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::errors::{PolicyError, Result};
use crate::models::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySource {
    pub label: &'static str,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyLoad {
    pub policy: EffectivePolicy,
    pub loaded_sources: Vec<PathBuf>,
    pub missing_sources: Vec<PathBuf>,
}

pub fn load_policy_for_workspace(workspace_dir: &Path) -> Result<PolicyLoad> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    load_policy_from_sources(&[
        PolicySource {
            label: "system",
            path: PathBuf::from("/etc/archon/policy.toml"),
        },
        PolicySource {
            label: "user",
            path: home.join(".archon").join("policy.toml"),
        },
        PolicySource {
            label: "workspace",
            path: workspace_dir.join(".archon").join("policy.toml"),
        },
    ])
}

pub fn load_policy_from_sources(sources: &[PolicySource]) -> Result<PolicyLoad> {
    let mut policy = EffectivePolicy::default();
    let mut loaded_sources = Vec::new();
    let mut missing_sources = Vec::new();
    for source in sources {
        if !source.path.exists() {
            missing_sources.push(source.path.clone());
            continue;
        }
        let raw = std::fs::read_to_string(&source.path).map_err(|e| PolicyError::Io {
            path: source.path.display().to_string(),
            message: e.to_string(),
        })?;
        let parsed: RawPolicyRoot = toml::from_str(&raw).map_err(|e| PolicyError::Parse {
            path: source.path.display().to_string(),
            message: e.to_string(),
        })?;
        apply_raw(&mut policy, parsed);
        loaded_sources.push(source.path.clone());
    }
    Ok(PolicyLoad {
        policy,
        loaded_sources,
        missing_sources,
    })
}

#[derive(Debug, Default, Deserialize)]
struct RawPolicyRoot {
    policy: Option<RawPolicy>,
    vlm: Option<RawVlmPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPolicy {
    network: Option<RawNetworkPolicy>,
    workers: Option<RawWorkersPolicy>,
    gametheory: Option<RawGameTheoryPolicy>,
    learning: Option<RawLearningPolicy>,
    docs: Option<RawDocsPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawNetworkPolicy {
    default: Option<String>,
    allow_cloud_vlm: Option<bool>,
    allow_web_strategy_agents: Option<bool>,
    allow_mcp_server_exposure: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorkersPolicy {
    ocr: Option<String>,
    embedding: Option<String>,
    vlm: Option<String>,
    web_fetch: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGameTheoryPolicy {
    max_agents_per_council: Option<usize>,
    max_cost_usd: Option<f64>,
    enable_tier11: Option<bool>,
    allow_web_tools: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLearningPolicy {
    auto_apply_low_risk: Option<bool>,
    require_approval_for_prompt_changes: Option<bool>,
    require_approval_for_blocking_gates: Option<bool>,
    require_approval_for_network_changes: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDocsPolicy {
    vlm: Option<RawVlmPolicy>,
    retrieval: Option<RawRetrievalPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRetrievalPolicy {
    exact_weight: Option<f64>,
    semantic_weight: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVlmPolicy {
    enabled: Option<bool>,
    mode: Option<String>,
    allow_cloud: Option<bool>,
    require_user_confirmation_for_cloud: Option<bool>,
}

fn apply_raw(policy: &mut EffectivePolicy, root: RawPolicyRoot) {
    if let Some(raw) = root.policy {
        if let Some(network) = raw.network {
            apply_network(&mut policy.network, network);
        }
        if let Some(workers) = raw.workers {
            apply_workers(&mut policy.workers, workers);
        }
        if let Some(gametheory) = raw.gametheory {
            apply_gametheory(&mut policy.gametheory, gametheory);
        }
        if let Some(learning) = raw.learning {
            apply_learning(&mut policy.learning, learning);
        }
        if let Some(docs) = raw.docs {
            if let Some(vlm) = docs.vlm {
                apply_vlm(&mut policy.docs.vlm, vlm);
            }
            if let Some(retrieval) = docs.retrieval {
                apply_retrieval(&mut policy.docs.retrieval, retrieval);
            }
        }
    }
    if let Some(legacy_vlm) = root.vlm {
        apply_vlm(&mut policy.docs.vlm, legacy_vlm);
    }
}

fn apply_network(policy: &mut NetworkPolicy, raw: RawNetworkPolicy) {
    if let Some(value) = raw.default {
        policy.default = value;
    }
    if let Some(value) = raw.allow_cloud_vlm {
        policy.allow_cloud_vlm = value;
    }
    if let Some(value) = raw.allow_web_strategy_agents {
        policy.allow_web_strategy_agents = value;
    }
    if let Some(value) = raw.allow_mcp_server_exposure {
        policy.allow_mcp_server_exposure = value;
    }
}

fn apply_workers(policy: &mut WorkersPolicy, raw: RawWorkersPolicy) {
    if let Some(value) = raw.ocr {
        policy.ocr = value;
    }
    if let Some(value) = raw.embedding {
        policy.embedding = value;
    }
    if let Some(value) = raw.vlm {
        policy.vlm = value;
    }
    if let Some(value) = raw.web_fetch {
        policy.web_fetch = value;
    }
}

fn apply_gametheory(policy: &mut GameTheoryPolicy, raw: RawGameTheoryPolicy) {
    if let Some(value) = raw.max_agents_per_council {
        policy.max_agents_per_council = value;
    }
    if let Some(value) = raw.max_cost_usd {
        policy.max_cost_usd = value;
    }
    if let Some(value) = raw.enable_tier11 {
        policy.enable_tier11 = value;
    }
    if let Some(value) = raw.allow_web_tools {
        policy.allow_web_tools = value;
    }
}

fn apply_learning(policy: &mut LearningPolicy, raw: RawLearningPolicy) {
    if let Some(value) = raw.auto_apply_low_risk {
        policy.auto_apply_low_risk = value;
    }
    if let Some(value) = raw.require_approval_for_prompt_changes {
        policy.require_approval_for_prompt_changes = value;
    }
    if let Some(value) = raw.require_approval_for_blocking_gates {
        policy.require_approval_for_blocking_gates = value;
    }
    if let Some(value) = raw.require_approval_for_network_changes {
        policy.require_approval_for_network_changes = value;
    }
}

fn apply_vlm(policy: &mut VlmPolicy, raw: RawVlmPolicy) {
    if let Some(value) = raw.enabled {
        policy.enabled = value;
    }
    if let Some(value) = raw.mode {
        policy.mode = value;
    }
    if let Some(value) = raw.allow_cloud {
        policy.allow_cloud = value;
    }
    if let Some(value) = raw.require_user_confirmation_for_cloud {
        policy.require_user_confirmation_for_cloud = value;
    }
}

fn apply_retrieval(policy: &mut RetrievalPolicy, raw: RawRetrievalPolicy) {
    if let Some(value) = raw.exact_weight {
        policy.exact_weight = value;
    }
    if let Some(value) = raw.semantic_weight {
        policy.semantic_weight = value;
    }
}
