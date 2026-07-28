use serde::{Deserialize, Serialize};

use super::runtime::ReflexionConfig;
use super::world_model::WorldModelConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct LearningConfig {
    pub sona: SonaLearningConfig,
    pub cognitive: archon_cognitive::CognitiveConfig,
    pub provenance: ToggleConfig,
    pub desc: ToggleConfig,
    pub gnn: GnnModelConfig,
    pub world_model: WorldModelConfig,
    pub reasoning_quality: ReasoningQualityConfig,
    pub session_briefing: SessionBriefingConfig,
    pub causal_memory: ToggleConfig,
    pub shadow_vector: ToggleConfig,
    pub reasoning_bank: ToggleConfig,
    pub reflexion: ReflexionConfig,
    pub agent_evolution: AgentEvolutionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SonaLearningConfig {
    pub enabled: bool,
    /// Interactive sessions record SONA trajectories when `enabled`; pipeline
    /// and batch runs require this explicit opt-in.
    pub pipeline_recording: bool,
}

impl Default for SonaLearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            pipeline_recording: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityConfig {
    pub enabled: bool,
    pub emit_inline_events: bool,
    pub post_turn_analysis: bool,
    pub post_session_analysis: bool,
    pub shadow_mode_days: u32,
    pub apply_trust_updates_after_shadow: bool,
    pub max_claims_per_turn: usize,
    pub max_excerpt_chars: usize,
    pub store_raw_text: bool,
    pub link_user_corrections: bool,
    pub update_self_trust: bool,
    pub feed_world_model: bool,
    pub feed_retrospective: bool,
    pub critic: ReasoningQualityCriticConfig,
    pub extractor_eval: ReasoningQualityExtractorEvalConfig,
    pub patterns: ReasoningQualityPatternsConfig,
}

impl Default for ReasoningQualityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            emit_inline_events: true,
            post_turn_analysis: true,
            post_session_analysis: true,
            shadow_mode_days: 30,
            apply_trust_updates_after_shadow: true,
            max_claims_per_turn: 12,
            max_excerpt_chars: 600,
            store_raw_text: false,
            link_user_corrections: true,
            update_self_trust: true,
            feed_world_model: true,
            feed_retrospective: true,
            critic: ReasoningQualityCriticConfig::default(),
            extractor_eval: ReasoningQualityExtractorEvalConfig::default(),
            patterns: ReasoningQualityPatternsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityCriticConfig {
    pub mode: String,
    pub allow_llm: bool,
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_turns_per_session: usize,
    pub run_async: bool,
    pub fallback_to_heuristic: bool,
    pub budget: ReasoningQualityCriticBudgetConfig,
}

impl Default for ReasoningQualityCriticConfig {
    fn default() -> Self {
        Self {
            mode: "hybrid".to_string(),
            allow_llm: false,
            provider: "default".to_string(),
            model: String::new(),
            max_tokens: 1200,
            temperature: 0.0,
            max_turns_per_session: 50,
            run_async: true,
            fallback_to_heuristic: true,
            budget: ReasoningQualityCriticBudgetConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityCriticBudgetConfig {
    pub per_session_token_cap: u64,
    pub daily_usd_cap: f64,
    pub weekly_usd_cap: f64,
    pub respect_provider_cooldowns: bool,
    pub emit_cost_events: bool,
}

impl Default for ReasoningQualityCriticBudgetConfig {
    fn default() -> Self {
        Self {
            per_session_token_cap: 200_000,
            daily_usd_cap: 10.0,
            weekly_usd_cap: 50.0,
            respect_provider_cooldowns: true,
            emit_cost_events: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityExtractorEvalConfig {
    pub fixture_dir: String,
    pub min_claim_precision: f32,
    pub min_claim_recall: f32,
    pub min_claim_before_source_precision: f32,
    pub max_code_fence_false_positive_rate: f32,
    pub max_quoted_user_false_positive_rate: f32,
}

impl Default for ReasoningQualityExtractorEvalConfig {
    fn default() -> Self {
        Self {
            fixture_dir: "crates/archon-reasoning-quality/tests/fixtures/labeled_turns".to_string(),
            min_claim_precision: 0.85,
            min_claim_recall: 0.50,
            min_claim_before_source_precision: 0.90,
            max_code_fence_false_positive_rate: 0.05,
            max_quoted_user_false_positive_rate: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReasoningQualityPatternsConfig {
    pub window_days: u32,
    pub min_events: usize,
    pub min_distinct_sessions: usize,
    pub repeated_pattern_trust_weight: f32,
}

impl Default for ReasoningQualityPatternsConfig {
    fn default() -> Self {
        Self {
            window_days: 30,
            min_events: 3,
            min_distinct_sessions: 3,
            repeated_pattern_trust_weight: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionBriefingConfig {
    pub enabled: bool,
    pub include_memory: bool,
    pub include_reasoning_quality: bool,
    pub include_pending_behaviour_proposals: bool,
    pub include_world_model: bool,
    pub max_items: usize,
    pub max_chars: usize,
    pub world_model_requires_ready: bool,
}

impl Default for SessionBriefingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_memory: true,
            include_reasoning_quality: true,
            include_pending_behaviour_proposals: true,
            include_world_model: true,
            max_items: 8,
            max_chars: 4000,
            world_model_requires_ready: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentEvolutionConfig {
    /// Governed Cozo profile versions exist by default, but runtime overlay is
    /// opt-in until enough shadow/e2e coverage proves the path for operators.
    pub active_profile_overlay_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToggleConfig {
    pub enabled: bool,
}

impl ToggleConfig {
    pub const fn enabled() -> Self {
        Self { enabled: true }
    }
}

impl Default for ToggleConfig {
    fn default() -> Self {
        Self::enabled()
    }
}

// ---------------------------------------------------------------------------
// GNN model configuration — [learning.gnn]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnModelConfig {
    pub enabled: bool,
    pub input_dim: usize,
    pub output_dim: usize,
    pub num_layers: usize,
    pub attention_heads: usize,
    pub max_nodes: usize,
    pub use_residual: bool,
    pub use_layer_norm: bool,
    pub activation: String,
    pub weight_seed: u64,
    #[serde(alias = "training")]
    pub training: GnnTrainingConfig,
    pub auto_trainer: GnnAutoTrainerConfig,
}

impl Default for GnnModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            input_dim: 1536,
            output_dim: 1536,
            num_layers: 3,
            attention_heads: 12,
            max_nodes: 50,
            use_residual: true,
            use_layer_norm: true,
            activation: "relu".to_string(),
            weight_seed: 0,
            training: GnnTrainingConfig::default(),
            auto_trainer: GnnAutoTrainerConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// GNN training configuration — [learning.gnn.training]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnTrainingConfig {
    pub learning_rate: f32,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub early_stopping_patience: usize,
    pub validation_split: f32,
    pub ewc_lambda: f32,
    pub margin: f32,
    pub triplet_loss_coefficient: f32,
    pub max_gradient_norm: f32,
    pub max_triplets_per_run: usize,
    pub max_runtime_ms: u64,
}

impl Default for GnnTrainingConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.001,
            batch_size: 32,
            max_epochs: 10,
            early_stopping_patience: 3,
            validation_split: 0.2,
            ewc_lambda: 0.1,
            margin: 0.5,
            triplet_loss_coefficient: 0.1,
            max_gradient_norm: 1.0,
            max_triplets_per_run: 256,
            max_runtime_ms: 300_000,
        }
    }
}

// ---------------------------------------------------------------------------
// GNN auto-trainer configuration — [learning.gnn.auto_trainer]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GnnAutoTrainerConfig {
    pub enabled: bool,
    /// Minimum time between training runs in ms (throttle).
    pub min_throttle_ms: u64,
    /// Trigger training after N new memories since last train.
    pub trigger_new_memories: u64,
    /// Trigger training after this many ms since last train.
    pub trigger_elapsed_ms: u64,
    /// Trigger training after N corrections since last train.
    pub trigger_corrections: u64,
    /// Memories needed before the first training run.
    pub first_run_threshold: u64,
    /// Max wall-clock time per training run in ms.
    pub max_runtime_ms: u64,
    /// Background tick interval in ms.
    pub tick_interval_ms: u64,
}

impl Default for GnnAutoTrainerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_throttle_ms: 86_400_000,
            trigger_new_memories: 20,
            trigger_elapsed_ms: 86_400_000,
            trigger_corrections: 3,
            first_run_threshold: 30,
            max_runtime_ms: 300_000,
            tick_interval_ms: 60_000,
        }
    }
}
