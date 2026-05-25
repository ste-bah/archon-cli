use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::errors::{PolicyError, Result};
use crate::models::*;
use crate::video::{RawVideoPolicy, apply_video};

mod loader_docs;
use loader_docs::{RawDocsPolicy, RawVlmPolicy, apply_docs, apply_legacy_vlm};

macro_rules! set {
    ($target:expr, $value:expr) => {
        if let Some(value) = $value {
            $target = value;
        }
    };
}

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
    world_model: Option<RawWorldModelPolicy>,
    web: Option<RawWebPolicy>,
    reasoning_quality: Option<RawReasoningQualityPolicy>,
    video: Option<RawVideoPolicy>,
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
    autonomous_apply: Option<bool>,
    autonomous_max_risk: Option<String>,
    autonomous_min_evidence: Option<usize>,
    autonomous_max_recent_incidents: Option<usize>,
    require_approval_for_prompt_changes: Option<bool>,
    require_approval_for_blocking_gates: Option<bool>,
    require_approval_for_network_changes: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWorldModelPolicy {
    allow_third_party_embeddings: Option<bool>,
    allow_llm_labeler: Option<bool>,
    allow_behavior_changes: Option<bool>,
    allow_embedding_cache: Option<bool>,
    allow_world_model_raw_text_storage: Option<bool>,
    allow_eval_background_jobs: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWebPolicy {
    allow_mutating_actions: Option<bool>,
    allow_file_uploads: Option<bool>,
    allow_pipeline_controls: Option<bool>,
    allow_model_training_actions: Option<bool>,
    allow_corpus_open_paths: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawReasoningQualityPolicy {
    allow_llm_critic: Option<bool>,
    allow_critic_cloud_data_flow: Option<bool>,
    allow_third_party_critic: Option<bool>,
    allow_raw_text_storage: Option<bool>,
    allow_behavior_proposal_generation: Option<bool>,
    allow_session_start_injection: Option<bool>,
    allow_trust_updates_during_shadow: Option<bool>,
    auto_migrate_reasoning_quality: Option<bool>,
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
        if let Some(world_model) = raw.world_model {
            apply_world_model(&mut policy.world_model, world_model);
        }
        if let Some(web) = raw.web {
            apply_web(&mut policy.web, web);
        }
        if let Some(reasoning_quality) = raw.reasoning_quality {
            apply_reasoning_quality(&mut policy.reasoning_quality, reasoning_quality);
        }
        if let Some(video) = raw.video {
            apply_video(&mut policy.video, video);
        }
        if let Some(docs) = raw.docs {
            apply_docs(&mut policy.docs, docs);
        }
    }
    if let Some(legacy_vlm) = root.vlm {
        apply_legacy_vlm(&mut policy.docs.vlm, legacy_vlm);
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
    set!(policy.auto_apply_low_risk, raw.auto_apply_low_risk);
    set!(policy.autonomous_apply, raw.autonomous_apply);
    set!(policy.autonomous_max_risk, raw.autonomous_max_risk);
    set!(policy.autonomous_min_evidence, raw.autonomous_min_evidence);
    set!(
        policy.autonomous_max_recent_incidents,
        raw.autonomous_max_recent_incidents
    );
    set!(
        policy.require_approval_for_prompt_changes,
        raw.require_approval_for_prompt_changes
    );
    set!(
        policy.require_approval_for_blocking_gates,
        raw.require_approval_for_blocking_gates
    );
    set!(
        policy.require_approval_for_network_changes,
        raw.require_approval_for_network_changes
    );
}

fn apply_world_model(policy: &mut WorldModelPolicy, raw: RawWorldModelPolicy) {
    if let Some(value) = raw.allow_third_party_embeddings {
        policy.allow_third_party_embeddings = value;
    }
    if let Some(value) = raw.allow_llm_labeler {
        policy.allow_llm_labeler = value;
    }
    if let Some(value) = raw.allow_behavior_changes {
        policy.allow_behavior_changes = value;
    }
    if let Some(value) = raw.allow_embedding_cache {
        policy.allow_embedding_cache = value;
    }
    if let Some(value) = raw.allow_world_model_raw_text_storage {
        policy.allow_world_model_raw_text_storage = value;
    }
    if let Some(value) = raw.allow_eval_background_jobs {
        policy.allow_eval_background_jobs = value;
    }
}

fn apply_web(policy: &mut WebPolicy, raw: RawWebPolicy) {
    if let Some(value) = raw.allow_mutating_actions {
        policy.allow_mutating_actions = value;
    }
    if let Some(value) = raw.allow_file_uploads {
        policy.allow_file_uploads = value;
    }
    if let Some(value) = raw.allow_pipeline_controls {
        policy.allow_pipeline_controls = value;
    }
    if let Some(value) = raw.allow_model_training_actions {
        policy.allow_model_training_actions = value;
    }
    if let Some(value) = raw.allow_corpus_open_paths {
        policy.allow_corpus_open_paths = value;
    }
}

fn apply_reasoning_quality(policy: &mut ReasoningQualityPolicy, raw: RawReasoningQualityPolicy) {
    if let Some(value) = raw.allow_llm_critic {
        policy.allow_llm_critic = value;
    }
    if let Some(value) = raw.allow_critic_cloud_data_flow {
        policy.allow_critic_cloud_data_flow = value;
    }
    if let Some(value) = raw.allow_third_party_critic {
        policy.allow_third_party_critic = value;
    }
    if let Some(value) = raw.allow_raw_text_storage {
        policy.allow_raw_text_storage = value;
    }
    if let Some(value) = raw.allow_behavior_proposal_generation {
        policy.allow_behavior_proposal_generation = value;
    }
    if let Some(value) = raw.allow_session_start_injection {
        policy.allow_session_start_injection = value;
    }
    if let Some(value) = raw.allow_trust_updates_during_shadow {
        policy.allow_trust_updates_during_shadow = value;
    }
    if let Some(value) = raw.auto_migrate_reasoning_quality {
        policy.auto_migrate_reasoning_quality = value;
    }
}
