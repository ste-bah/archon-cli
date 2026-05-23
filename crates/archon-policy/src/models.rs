use serde::{Deserialize, Serialize};

pub use crate::video::{
    VideoAcquirePolicy, VideoAsrPolicy, VideoFramesPolicy, VideoPolicy, VideoSummaryPolicy,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectivePolicy {
    pub network: NetworkPolicy,
    pub workers: WorkersPolicy,
    pub gametheory: GameTheoryPolicy,
    pub learning: LearningPolicy,
    pub world_model: WorldModelPolicy,
    pub web: WebPolicy,
    pub reasoning_quality: ReasoningQualityPolicy,
    pub video: VideoPolicy,
    pub docs: DocsPolicy,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldModelPolicy {
    pub allow_third_party_embeddings: bool,
    pub allow_llm_labeler: bool,
    pub allow_behavior_changes: bool,

    /// Permit persisting embedding vectors to ~/.archon/world-model/embedding-cache/.
    /// Fail-closed default: false. T015 wires the gate.
    #[serde(default)]
    pub allow_embedding_cache: bool,

    /// Permit storing raw (unredacted) text in world-model cache records.
    /// Named `allow_world_model_raw_text_storage` to AVOID collision with
    /// ReasoningQualityPolicy::allow_raw_text_storage (F-CRIT-01, DEC-JEVAL-04).
    /// Fail-closed default: false.
    #[serde(default)]
    pub allow_world_model_raw_text_storage: bool,

    /// Permit `archon world eval-jepa --background` to spawn detached worker.
    /// Fail-closed default: false. T023 wires the gate before spawn_background_worker.
    #[serde(default)]
    pub allow_eval_background_jobs: bool,
}

impl Default for WorldModelPolicy {
    fn default() -> Self {
        Self {
            allow_third_party_embeddings: false,
            allow_llm_labeler: false,
            allow_behavior_changes: false,
            allow_embedding_cache: false,              // fail-closed
            allow_world_model_raw_text_storage: false, // fail-closed
            allow_eval_background_jobs: false,         // fail-closed
        }
    }
}

#[cfg(test)]
mod world_model_policy_tests {
    use super::*;

    #[test]
    fn old_policy_document_fails_closed_for_all_new_keys() {
        // Simulate a policy document without the new keys (e.g. written before T022)
        let toml = r#"
allow_third_party_embeddings = false
allow_llm_labeler = false
allow_behavior_changes = false
"#;
        let policy: WorldModelPolicy = toml::from_str(toml).expect("deserialize");
        assert!(
            !policy.allow_embedding_cache,
            "allow_embedding_cache must default to false (fail-closed)"
        );
        assert!(
            !policy.allow_world_model_raw_text_storage,
            "allow_world_model_raw_text_storage must default to false (fail-closed)"
        );
        assert!(
            !policy.allow_eval_background_jobs,
            "allow_eval_background_jobs must default to false (fail-closed)"
        );
    }

    #[test]
    fn no_field_name_collision_with_reasoning_quality_policy() {
        let wm = WorldModelPolicy::default();
        let rq = ReasoningQualityPolicy::default();
        // Both names exist; the existence of both fields with distinct names is
        // a compile-time guarantee that there is no collision.
        let _wm_field: bool = wm.allow_world_model_raw_text_storage;
        let _rq_field: bool = rq.allow_raw_text_storage;
        // If either field were renamed to match the other, this test would fail to compile.
    }

    #[test]
    fn default_world_model_policy_has_all_three_new_fields_false() {
        let p = WorldModelPolicy::default();
        assert!(!p.allow_embedding_cache);
        assert!(!p.allow_world_model_raw_text_storage);
        assert!(!p.allow_eval_background_jobs);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPolicy {
    pub allow_mutating_actions: bool,
    pub allow_file_uploads: bool,
    pub allow_pipeline_controls: bool,
    pub allow_model_training_actions: bool,
    pub allow_corpus_open_paths: bool,
}

impl Default for WebPolicy {
    fn default() -> Self {
        Self {
            allow_mutating_actions: false,
            allow_file_uploads: false,
            allow_pipeline_controls: false,
            allow_model_training_actions: false,
            allow_corpus_open_paths: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningQualityPolicy {
    pub allow_llm_critic: bool,
    pub allow_critic_cloud_data_flow: bool,
    pub allow_third_party_critic: bool,
    pub allow_raw_text_storage: bool,
    pub allow_behavior_proposal_generation: bool,
    pub allow_session_start_injection: bool,
    pub allow_trust_updates_during_shadow: bool,
    pub auto_migrate_reasoning_quality: bool,
}

impl Default for ReasoningQualityPolicy {
    fn default() -> Self {
        Self {
            allow_llm_critic: false,
            allow_critic_cloud_data_flow: false,
            allow_third_party_critic: false,
            allow_raw_text_storage: false,
            allow_behavior_proposal_generation: true,
            allow_session_start_injection: true,
            allow_trust_updates_during_shadow: false,
            auto_migrate_reasoning_quality: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocsPolicy {
    pub vlm: VlmPolicy,
    pub pdf: PdfPolicy,
    pub retrieval: RetrievalPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfPolicy {
    pub extract_embedded_images: bool,
    pub min_image_dimension: u32,
    pub min_image_bytes: u64,
    pub vlm_per_page_image: bool,
    pub render_text_pdf_pages: bool,
}

impl Default for PdfPolicy {
    fn default() -> Self {
        Self {
            extract_embedded_images: true,
            min_image_dimension: 200,
            min_image_bytes: 4096,
            vlm_per_page_image: true,
            render_text_pdf_pages: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPolicy {
    pub exact_weight: f64,
    pub semantic_weight: f64,
}

impl Default for RetrievalPolicy {
    fn default() -> Self {
        Self {
            exact_weight: 0.45,
            semantic_weight: 0.55,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VlmPolicy {
    pub enabled: bool,
    pub mode: String,
    pub provider: String,
    pub allow_cloud: bool,
    pub require_user_confirmation_for_cloud: bool,
    pub ollama: OllamaVlmPolicy,
    pub gemini: GeminiVlmPolicy,
    pub anthropic: AnthropicVlmPolicy,
    pub openai_compat: OpenAiCompatVlmPolicy,
}

impl Default for VlmPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "disabled".into(),
            provider: "disabled".into(),
            allow_cloud: false,
            require_user_confirmation_for_cloud: true,
            ollama: OllamaVlmPolicy::default(),
            gemini: GeminiVlmPolicy::default(),
            anthropic: AnthropicVlmPolicy::default(),
            openai_compat: OpenAiCompatVlmPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OllamaVlmPolicy {
    pub endpoint: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl Default for OllamaVlmPolicy {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".into(),
            model: "gemma4:e4b".into(),
            timeout_secs: 120,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeminiVlmPolicy {
    pub api_key_env: String,
    pub model: String,
    pub endpoint_base: String,
    pub rpm_limit: u32,
}

impl Default for GeminiVlmPolicy {
    fn default() -> Self {
        Self {
            api_key_env: "GOOGLE_API_KEY".into(),
            model: "gemini-3-flash-preview".into(),
            endpoint_base: "https://generativelanguage.googleapis.com/v1beta".into(),
            rpm_limit: 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicVlmPolicy {
    pub model: String,
}

impl Default for AnthropicVlmPolicy {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiCompatVlmPolicy {
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
    pub timeout_secs: u64,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl Default for OpenAiCompatVlmPolicy {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:1234/v1".into(),
            model: "google/gemma-3-12b-it".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            timeout_secs: 120,
            max_tokens: 1024,
            temperature: 0.2,
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
