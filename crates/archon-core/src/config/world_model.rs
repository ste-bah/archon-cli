use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelConfig {
    pub enabled: bool,
    pub model_kind: String,
    pub auto_promote_advisory: bool,
    pub require_approval_for_behavior_change: bool,
    pub state_dim: usize,
    pub max_checkpoint_mb: u64,
    pub max_prediction_latency_ms: u64,
    pub max_counterfactual_actions: usize,
    pub store_raw_text: bool,
    pub include_conversation_turns: bool,
    pub include_agent_outputs: bool,
    pub embeddings: WorldModelEmbeddingsConfig,
    pub labeler: WorldModelLabelerConfig,
    pub training: WorldModelTrainingConfig,
    pub jepa: WorldModelJepaConfig,
    pub eval: WorldModelEvalConfig,
    pub cold_start: WorldModelColdStartConfig,
    pub auto_trainer: WorldModelAutoTrainerConfig,
    pub replay: WorldModelReplayConfig,
    pub guardrails: WorldModelGuardrailsConfig,
    pub retention: WorldModelRetentionConfig,
}

impl Default for WorldModelConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_kind: "latent_transition".into(),
            auto_promote_advisory: true,
            require_approval_for_behavior_change: true,
            state_dim: 384,
            max_checkpoint_mb: 64,
            max_prediction_latency_ms: 100,
            max_counterfactual_actions: 5,
            store_raw_text: false,
            include_conversation_turns: true,
            include_agent_outputs: true,
            embeddings: WorldModelEmbeddingsConfig::default(),
            labeler: WorldModelLabelerConfig::default(),
            training: WorldModelTrainingConfig::default(),
            jepa: WorldModelJepaConfig::default(),
            eval: WorldModelEvalConfig::default(),
            cold_start: WorldModelColdStartConfig::default(),
            auto_trainer: WorldModelAutoTrainerConfig::default(),
            replay: WorldModelReplayConfig::default(),
            guardrails: WorldModelGuardrailsConfig::default(),
            retention: WorldModelRetentionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelEmbeddingsConfig {
    pub source: String,
    pub provider: String,
    pub model: String,
    pub dimensions: usize,
    pub projection_dim: usize,
    pub cache_enabled: bool,
    pub cache_max_mb: u64,
    pub redact_before_embedding: bool,
    pub allow_third_party: bool,
    pub external_base_url: String,
    pub external_api_key_env: String,
}

impl Default for WorldModelEmbeddingsConfig {
    fn default() -> Self {
        Self {
            source: "local".into(),
            provider: "fastembed".into(),
            model: "bge-base-en-v1.5".into(),
            dimensions: 768,
            projection_dim: 384,
            cache_enabled: true,
            cache_max_mb: 1_024,
            redact_before_embedding: true,
            allow_third_party: false,
            external_base_url: String::new(),
            external_api_key_env: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelLabelerConfig {
    pub analyzer: String,
    pub llm_enabled: bool,
    pub max_events_per_prompt: usize,
    pub max_prompt_chars: usize,
}

impl Default for WorldModelLabelerConfig {
    fn default() -> Self {
        Self {
            analyzer: "hybrid".into(),
            llm_enabled: true,
            max_events_per_prompt: 30,
            max_prompt_chars: 128_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelTrainingConfig {
    pub backend: String,
    pub allow_cpu_fallback: bool,
    pub prefer_accelerator: bool,
    pub precision: String,
    pub max_accelerator_memory_mb: u64,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub validation_split: f32,
    pub promotion_min_delta: f32,
    pub max_runtime_ms: u64,
}

impl Default for WorldModelTrainingConfig {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            allow_cpu_fallback: true,
            prefer_accelerator: true,
            precision: "fp32".into(),
            max_accelerator_memory_mb: 4_096,
            batch_size: 32,
            max_epochs: 10,
            validation_split: 0.2,
            promotion_min_delta: 0.02,
            max_runtime_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelJepaConfig {
    pub enabled: bool,
    pub latent_dim: usize,
    pub context_window_rows: usize,
    pub target_window_rows: usize,
    pub prediction_horizons: Vec<usize>,
    pub mask_ratio: f32,
    pub ema_decay: f32,
    pub latent_var_floor: f32,
    pub min_latent_std: f32,
    pub min_effective_rank_ratio: f32,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub learning_rate: f32,
    pub alpha_mse: f32,
    pub beta_aux: f32,
    pub gamma_horizon: f32,
    pub delta_var: f32,
    pub allow_generic_fallback: bool,
    pub max_runtime_ms: u64,
    pub max_prediction_latency_ms: u64,
    pub max_checkpoint_mb: u64,
    pub horizon_consistency_tol: f32,
    pub min_baseline_improvement: f32,
    pub min_heldout_examples: usize,
    pub min_training_examples: usize,
    pub require_native_accelerator_ops: bool,
    pub allow_accelerated_candidate_cpu_stage: bool,
    pub min_cuda_validation_examples: usize,
    pub min_metal_validation_examples: usize,
    pub backend_parity_cosine_floor: f32,
    pub max_backend_prediction_latency_ms: u64,
    pub max_backend_first_call_latency_ms: u64,
    /// Eval pipeline configuration (`[learning.world_model.jepa.eval]`). PRD-006C §12.
    #[serde(default)]
    pub eval: WorldModelJepaEvalConfig,
}

impl Default for WorldModelJepaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            latent_var_floor: 0.05,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            batch_size: 32,
            max_epochs: 10,
            learning_rate: 0.001,
            alpha_mse: 0.25,
            beta_aux: 0.50,
            gamma_horizon: 0.10,
            delta_var: 0.10,
            allow_generic_fallback: true,
            max_runtime_ms: 300_000,
            max_prediction_latency_ms: 50,
            max_checkpoint_mb: 64,
            horizon_consistency_tol: 0.02,
            min_baseline_improvement: 0.05,
            min_heldout_examples: 200,
            min_training_examples: 2_000,
            require_native_accelerator_ops: true,
            allow_accelerated_candidate_cpu_stage: false,
            min_cuda_validation_examples: 512,
            min_metal_validation_examples: 512,
            backend_parity_cosine_floor: 0.99,
            max_backend_prediction_latency_ms: 50,
            max_backend_first_call_latency_ms: 5_000,
            eval: WorldModelJepaEvalConfig::default(),
        }
    }
}

impl WorldModelJepaConfig {
    /// Returns the eval_schema_version used for config_fingerprint and cache_key
    /// computations.
    ///
    /// T025: reads from the nested `eval` sub-config (no longer hardcoded 1u32).
    /// Bump `learning.world_model.jepa.eval.eval_schema_version` in config to
    /// invalidate all existing embedding cache entries.
    pub fn eval_schema_version_or_default(&self) -> u32 {
        // T025: now reads from nested eval config, not hardcoded
        self.eval.eval_schema_version
    }
}

/// Eval pipeline configuration for `[learning.world_model.jepa.eval]`.
/// PRD-006C §12.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelJepaEvalConfig {
    /// Default eval mode: `"quick"` | `"full"` | `"promotion"`
    pub mode: String,
    /// JEPA encoding batch size (transitions per batch)
    pub batch_size: usize,
    /// Fastembed baseline embedding batch size
    pub embedding_batch_size: usize,
    /// Progress output interval in rows
    pub progress_interval_rows: usize,
    /// Quick-mode runtime budget in ms (must be > 0; quick must be fast)
    pub quick_max_runtime_ms: u64,
    /// Full-mode runtime budget in ms (0 = unlimited; handles 30+ min workloads)
    pub full_max_runtime_ms: u64,
    /// Promotion-mode runtime budget in ms (0 = unlimited)
    pub promotion_max_runtime_ms: u64,
    /// Heartbeat interval for stale lock detection when budget = 0
    pub stale_heartbeat_ms: u64,
    /// Enable embedding cache reads and writes
    pub cache_enabled: bool,
    /// Maximum embedding cache size in MB (LRU eviction when exceeded)
    pub cache_max_mb: u64,
    /// Reserved default for background execution once the eval worker is wired.
    pub background_default: bool,
    /// Schema version for embedding cache key invalidation.
    /// Bump this value to invalidate ALL existing cache entries.
    pub eval_schema_version: u32,
}

impl Default for WorldModelJepaEvalConfig {
    fn default() -> Self {
        Self {
            mode: "quick".to_string(),
            batch_size: 256,
            embedding_batch_size: 64,
            progress_interval_rows: 500,
            quick_max_runtime_ms: 30_000,
            full_max_runtime_ms: 0,
            promotion_max_runtime_ms: 0,
            stale_heartbeat_ms: 120_000,
            cache_enabled: true,
            cache_max_mb: 2048,
            background_default: false,
            eval_schema_version: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelEvalConfig {
    pub bootstrap_iterations: usize,
    pub confidence_level: f32,
    pub parity_precision: String,
    pub parity_min_cosine: f32,
    pub next_state_baseline_min_delta: f32,
    pub counterfactual_baseline_min_delta: f32,
    pub surprise_ks_min_p: f32,
    pub counterfactual_ndcg_min: f32,
}

impl Default for WorldModelEvalConfig {
    fn default() -> Self {
        Self {
            bootstrap_iterations: 1_000,
            confidence_level: 0.95,
            parity_precision: "fp32".into(),
            parity_min_cosine: 0.95,
            next_state_baseline_min_delta: 0.10,
            counterfactual_baseline_min_delta: 0.10,
            surprise_ks_min_p: 0.05,
            counterfactual_ndcg_min: 0.60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelColdStartConfig {
    pub min_rows: u64,
    pub min_sessions: u64,
    pub min_observed_days: u64,
}

impl Default for WorldModelColdStartConfig {
    fn default() -> Self {
        Self {
            min_rows: 1_000,
            min_sessions: 50,
            min_observed_days: 7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelAutoTrainerConfig {
    pub enabled: bool,
    pub min_throttle_ms: u64,
    pub idle_required_ms: u64,
    pub battery_suspend_below_percent: u8,
    pub trigger_new_rows: u64,
    pub trigger_surprises: u64,
    pub trigger_corrections: u64,
    pub trigger_elapsed_ms: u64,
    pub first_run_threshold: u64,
    pub max_runtime_ms: u64,
    pub tick_interval_ms: u64,
}

impl Default for WorldModelAutoTrainerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_throttle_ms: 86_400_000,
            idle_required_ms: 300_000,
            battery_suspend_below_percent: 30,
            trigger_new_rows: 100,
            trigger_surprises: 5,
            trigger_corrections: 3,
            trigger_elapsed_ms: 86_400_000,
            first_run_threshold: 300,
            max_runtime_ms: 300_000,
            tick_interval_ms: 60_000,
        }
    }
}

/// Surprise-weighted replay — `[learning.world_model.replay]`.
///
/// `prioritized_enabled` is the only key that changes what the trainer learns
/// from, and it is `false` by default: prioritised replay moves the training
/// distribution, so it stays a shadow computation until an operator turns it on
/// with matched baseline/canary evidence in hand. The other keys may only make
/// replay more conservative — `ReplayPolicy::clamped` narrows anything that
/// exceeds the crate's hard bounds, so a hand-edited value cannot widen one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelReplayConfig {
    pub prioritized_enabled: bool,
    pub held_out_fraction: f32,
    pub batch_size: usize,
    pub prioritized_fraction: f32,
    pub max_surprise_weight: f32,
    pub max_decile_share: f32,
    pub seed: u64,
    pub split_version: u32,
}

impl Default for WorldModelReplayConfig {
    fn default() -> Self {
        Self {
            prioritized_enabled: false,
            held_out_fraction: 0.2,
            batch_size: 512,
            prioritized_fraction: 0.5,
            max_surprise_weight: 4.0,
            max_decile_share: 0.40,
            seed: 0x5713_2C9E_A1B4_0F17,
            split_version: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelGuardrailsConfig {
    pub enabled: bool,
    pub interactive_mode: String,
    pub pipeline_mode: String,
    pub tool_run_mode: String,
    pub verification_run_mode: String,
    pub high_risk_threshold: f32,
    pub medium_risk_threshold: f32,
    pub critical_risk_threshold: f32,
    pub require_tests_for_coding_high_risk: bool,
    pub require_build_for_coding_high_risk: bool,
    pub require_lint_for_coding_high_risk: bool,
    pub require_typecheck_for_coding_high_risk: bool,
    pub require_plan_review_for_plan_drift: bool,
    pub require_source_check_for_research_high_risk: bool,
    pub require_manual_approval_for_critical: bool,
    pub max_guardrail_overhead_ms: u64,
    pub record_outcomes_without_prediction: bool,
    pub max_guardrail_events_per_session: usize,
}

impl Default for WorldModelGuardrailsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interactive_mode: "advisory".into(),
            pipeline_mode: "guarded".into(),
            tool_run_mode: "learn_only".into(),
            verification_run_mode: "learn_only".into(),
            high_risk_threshold: 0.70,
            medium_risk_threshold: 0.45,
            critical_risk_threshold: 0.85,
            require_tests_for_coding_high_risk: true,
            require_build_for_coding_high_risk: true,
            require_lint_for_coding_high_risk: false,
            require_typecheck_for_coding_high_risk: false,
            require_plan_review_for_plan_drift: true,
            require_source_check_for_research_high_risk: true,
            require_manual_approval_for_critical: false,
            max_guardrail_overhead_ms: 40,
            record_outcomes_without_prediction: true,
            max_guardrail_events_per_session: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldModelRetentionConfig {
    pub jsonl_rotate_mb: u64,
    pub raw_retention_days: u64,
    pub retain_cozo_summaries: bool,
    pub retain_checkpoint_count: usize,
}

impl Default for WorldModelRetentionConfig {
    fn default() -> Self {
        Self {
            jsonl_rotate_mb: 500,
            raw_retention_days: 90,
            retain_cozo_summaries: true,
            retain_checkpoint_count: 5,
        }
    }
}
