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
    world_model: Option<RawWorldModelPolicy>,
    web: Option<RawWebPolicy>,
    reasoning_quality: Option<RawReasoningQualityPolicy>,
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

#[derive(Debug, Default, Deserialize)]
struct RawDocsPolicy {
    vlm: Option<RawVlmPolicy>,
    pdf: Option<RawPdfPolicy>,
    retrieval: Option<RawRetrievalPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawRetrievalPolicy {
    exact_weight: Option<f64>,
    semantic_weight: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawPdfPolicy {
    extract_embedded_images: Option<bool>,
    min_image_dimension: Option<u32>,
    min_image_bytes: Option<u64>,
    vlm_per_page_image: Option<bool>,
    render_text_pdf_pages: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct RawVlmPolicy {
    enabled: Option<bool>,
    mode: Option<String>,
    provider: Option<String>,
    allow_cloud: Option<bool>,
    require_user_confirmation_for_cloud: Option<bool>,
    ollama: Option<RawOllamaVlmPolicy>,
    gemini: Option<RawGeminiVlmPolicy>,
    anthropic: Option<RawAnthropicVlmPolicy>,
    openai_compat: Option<RawOpenAiCompatVlmPolicy>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOllamaVlmPolicy {
    endpoint: Option<String>,
    model: Option<String>,
    timeout_secs: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeminiVlmPolicy {
    api_key_env: Option<String>,
    model: Option<String>,
    endpoint_base: Option<String>,
    rpm_limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct RawAnthropicVlmPolicy {
    model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawOpenAiCompatVlmPolicy {
    endpoint: Option<String>,
    model: Option<String>,
    api_key_env: Option<String>,
    timeout_secs: Option<u64>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
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
        if let Some(docs) = raw.docs {
            if let Some(vlm) = docs.vlm {
                apply_vlm(&mut policy.docs.vlm, vlm);
            }
            if let Some(pdf) = docs.pdf {
                apply_pdf(&mut policy.docs.pdf, pdf);
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

fn apply_vlm(policy: &mut VlmPolicy, raw: RawVlmPolicy) {
    if let Some(value) = raw.enabled {
        policy.enabled = value;
    }
    if let Some(value) = raw.mode {
        policy.mode = value;
    }
    if let Some(value) = raw.provider {
        policy.provider = value;
    }
    if let Some(value) = raw.allow_cloud {
        policy.allow_cloud = value;
    }
    if let Some(value) = raw.require_user_confirmation_for_cloud {
        policy.require_user_confirmation_for_cloud = value;
    }
    if let Some(value) = raw.ollama {
        apply_ollama_vlm(&mut policy.ollama, value);
    }
    if let Some(value) = raw.gemini {
        apply_gemini_vlm(&mut policy.gemini, value);
    }
    if let Some(value) = raw.anthropic {
        apply_anthropic_vlm(&mut policy.anthropic, value);
    }
    if let Some(value) = raw.openai_compat {
        apply_openai_compat_vlm(&mut policy.openai_compat, value);
    }
}

fn apply_ollama_vlm(policy: &mut OllamaVlmPolicy, raw: RawOllamaVlmPolicy) {
    if let Some(value) = raw.endpoint {
        policy.endpoint = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.timeout_secs {
        policy.timeout_secs = value;
    }
}

fn apply_gemini_vlm(policy: &mut GeminiVlmPolicy, raw: RawGeminiVlmPolicy) {
    if let Some(value) = raw.api_key_env {
        policy.api_key_env = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.endpoint_base {
        policy.endpoint_base = value;
    }
    if let Some(value) = raw.rpm_limit {
        policy.rpm_limit = value;
    }
}

fn apply_anthropic_vlm(policy: &mut AnthropicVlmPolicy, raw: RawAnthropicVlmPolicy) {
    if let Some(value) = raw.model {
        policy.model = value;
    }
}

fn apply_openai_compat_vlm(policy: &mut OpenAiCompatVlmPolicy, raw: RawOpenAiCompatVlmPolicy) {
    if let Some(value) = raw.endpoint {
        policy.endpoint = value;
    }
    if let Some(value) = raw.model {
        policy.model = value;
    }
    if let Some(value) = raw.api_key_env {
        policy.api_key_env = value;
    }
    if let Some(value) = raw.timeout_secs {
        policy.timeout_secs = value;
    }
    if let Some(value) = raw.max_tokens {
        policy.max_tokens = value;
    }
    if let Some(value) = raw.temperature {
        policy.temperature = value;
    }
}

fn apply_pdf(policy: &mut PdfPolicy, raw: RawPdfPolicy) {
    if let Some(value) = raw.extract_embedded_images {
        policy.extract_embedded_images = value;
    }
    if let Some(value) = raw.min_image_dimension {
        policy.min_image_dimension = value;
    }
    if let Some(value) = raw.min_image_bytes {
        policy.min_image_bytes = value;
    }
    if let Some(value) = raw.vlm_per_page_image {
        policy.vlm_per_page_image = value;
    }
    if let Some(value) = raw.render_text_pdf_pages {
        policy.render_text_pdf_pages = value;
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
