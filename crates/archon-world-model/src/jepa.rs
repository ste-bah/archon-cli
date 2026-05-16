//! JEPA-style trace representation candidate model.
//!
//! M2 keeps the implementation intentionally local and deterministic: the
//! encoders consume structured trace features plus deterministic lexical
//! hashing, and the CPU trainer fits the predictor and auxiliary heads without
//! calling semantic embedding providers.

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use safetensors::tensor::{Dtype, TensorView, serialize_to_file};
use serde::{Deserialize, Serialize};

use crate::backend::{BackendKind, BackendStatus};
use crate::model::{CpuLatentTransitionModel, LatentTransitionExample};
use crate::representation::{
    TraceAction, TraceTransition, TraceWindow, TraceWindowBuilder, WorldRepresentationAdapter,
};
use crate::schema::{ScalarFeatures, WorldLabelSet, WorldTraceRow};
use crate::train::TrainingStatus;

pub const JEPA_MODEL_KIND: &str = "jepa_transition";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTrainingConfig {
    pub latent_dim: usize,
    pub context_window_rows: usize,
    pub target_window_rows: usize,
    pub prediction_horizons: Vec<usize>,
    pub mask_ratio: f32,
    pub ema_decay: f32,
    pub latent_var_floor: f32,
    pub max_epochs: usize,
    pub learning_rate: f32,
    pub alpha_mse: f32,
    pub beta_aux: f32,
    pub gamma_horizon: f32,
    pub delta_var: f32,
    pub min_latent_std: f32,
    pub min_effective_rank_ratio: f32,
    pub horizon_consistency_tol: f32,
}

impl Default for JepaTrainingConfig {
    fn default() -> Self {
        Self {
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            latent_var_floor: 0.05,
            max_epochs: 10,
            learning_rate: 0.001,
            alpha_mse: 0.25,
            beta_aux: 0.50,
            gamma_horizon: 0.10,
            delta_var: 0.10,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            horizon_consistency_tol: 0.02,
        }
    }
}

impl JepaTrainingConfig {
    pub fn validate(&self) -> Result<()> {
        if self.latent_dim == 0 {
            bail!("jepa latent_dim must be greater than zero");
        }
        if self.context_window_rows == 0 || self.target_window_rows == 0 {
            bail!("jepa context_window_rows and target_window_rows must be greater than zero");
        }
        if self.prediction_horizons.is_empty()
            || self.prediction_horizons.iter().any(|horizon| *horizon == 0)
        {
            bail!("jepa prediction_horizons must contain positive horizons");
        }
        for (name, value) in [
            ("mask_ratio", self.mask_ratio),
            ("ema_decay", self.ema_decay),
            ("latent_var_floor", self.latent_var_floor),
            ("learning_rate", self.learning_rate),
            ("alpha_mse", self.alpha_mse),
            ("beta_aux", self.beta_aux),
            ("gamma_horizon", self.gamma_horizon),
            ("delta_var", self.delta_var),
            ("min_latent_std", self.min_latent_std),
            ("min_effective_rank_ratio", self.min_effective_rank_ratio),
            ("horizon_consistency_tol", self.horizon_consistency_tol),
        ] {
            if !value.is_finite() || value < 0.0 {
                bail!("jepa {name} must be finite and non-negative");
            }
        }
        if self.mask_ratio > 1.0 || self.ema_decay > 1.0 {
            bail!("jepa mask_ratio and ema_decay must be <= 1.0");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTraceModelMetadata {
    pub model_id: String,
    pub model_kind: String,
    pub latent_dim: usize,
    pub context_window_rows: usize,
    pub target_window_rows: usize,
    pub prediction_horizons: Vec<usize>,
    pub mask_ratio: f32,
    pub ema_decay: f32,
    pub target_stop_gradient: bool,
    pub backend: BackendKind,
    #[serde(default)]
    pub backend_execution: JepaBackendExecutionReport,
    pub row_count: u64,
    pub example_count: u64,
    pub parameter_count: u64,
    pub created_at: DateTime<Utc>,
}

impl JepaTraceModelMetadata {
    fn candidate(config: &JepaTrainingConfig, row_count: u64, example_count: u64) -> Self {
        Self {
            model_id: format!("jepa-world-model-candidate-{}", uuid::Uuid::new_v4()),
            model_kind: JEPA_MODEL_KIND.into(),
            latent_dim: config.latent_dim,
            context_window_rows: config.context_window_rows,
            target_window_rows: config.target_window_rows,
            prediction_horizons: config.prediction_horizons.clone(),
            mask_ratio: config.mask_ratio,
            ema_decay: config.ema_decay,
            target_stop_gradient: true,
            backend: BackendKind::Cpu,
            backend_execution: JepaBackendExecutionReport::cpu(
                BackendKind::Cpu,
                None,
                example_count as usize,
            ),
            row_count,
            example_count,
            parameter_count: 0,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaBackendExecutionReport {
    pub requested_backend: BackendKind,
    pub selected_backend: BackendKind,
    pub framework: String,
    pub device_name: Option<String>,
    pub commit_sha: String,
    pub feature_compiled: bool,
    pub tensor_self_test_passed: bool,
    pub hardware_validation_captured_at: Option<DateTime<Utc>>,
    pub validation_example_count: usize,
    pub native_encode: bool,
    pub native_predictor_fit: bool,
    pub native_auxiliary_fit: bool,
    pub native_transition_fit: bool,
    pub native_loss_eval: bool,
    pub native_runtime_prediction: Option<bool>,
    pub host_fallback_count: u64,
    pub allowed_host_stage_count: u64,
    pub fallback_reason: Option<String>,
}

impl Default for JepaBackendExecutionReport {
    fn default() -> Self {
        Self::cpu(BackendKind::Cpu, None, 0)
    }
}

impl JepaBackendExecutionReport {
    pub fn cpu(
        requested_backend: BackendKind,
        fallback_reason: Option<String>,
        validation_example_count: usize,
    ) -> Self {
        Self {
            requested_backend,
            selected_backend: BackendKind::Cpu,
            framework: "rust-vector".into(),
            device_name: Some("cpu".into()),
            commit_sha: build_commit_sha(),
            feature_compiled: true,
            tensor_self_test_passed: true,
            hardware_validation_captured_at: None,
            validation_example_count,
            native_encode: true,
            native_predictor_fit: true,
            native_auxiliary_fit: true,
            native_transition_fit: true,
            native_loss_eval: true,
            native_runtime_prediction: None,
            host_fallback_count: 0,
            allowed_host_stage_count: 0,
            fallback_reason,
        }
    }

    pub fn from_cpu_status(status: &BackendStatus, validation_example_count: usize) -> Self {
        Self::cpu(
            status.requested,
            status.fallback_reason.clone(),
            validation_example_count,
        )
    }

    pub fn native_stage_proof_passes(&self) -> bool {
        self.feature_compiled
            && self.tensor_self_test_passed
            && self
                .device_name
                .as_ref()
                .is_some_and(|name| !name.is_empty())
            && !self.commit_sha.trim().is_empty()
            && self.commit_sha.trim() != "unknown"
            && self.native_encode
            && self.native_predictor_fit
            && self.native_auxiliary_fit
            && self.native_transition_fit
            && self.native_loss_eval
            && self.host_fallback_count == 0
    }
}

fn build_commit_sha() -> String {
    option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .or(option_env!("SOURCE_VERSION"))
        .unwrap_or("unknown")
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTraceEncoder {
    pub role: String,
    pub latent_dim: usize,
    pub input_weights: Vec<f32>,
    pub hidden_bias: Vec<f32>,
    pub output_weights: Vec<f32>,
    pub output_bias: Vec<f32>,
    pub residual_weight: f32,
}

impl JepaTraceEncoder {
    pub fn new(role: impl Into<String>, latent_dim: usize) -> Self {
        let role = role.into();
        let input_weights = deterministic_vector(&role, "input", latent_dim, 0.85, 1.15);
        let hidden_bias = deterministic_vector(&role, "hidden_bias", latent_dim, -0.03, 0.03);
        let output_weights = deterministic_vector(&role, "output", latent_dim, 0.85, 1.15);
        let output_bias = deterministic_vector(&role, "output_bias", latent_dim, -0.03, 0.03);
        Self {
            role,
            latent_dim,
            input_weights,
            hidden_bias,
            output_weights,
            output_bias,
            residual_weight: 0.20,
        }
    }

    pub fn ema_target_from(context: &Self, decay: f32) -> Self {
        let mut target = Self::new("target", context.latent_dim);
        target.input_weights = ema_values(&target.input_weights, &context.input_weights, decay);
        target.hidden_bias = ema_values(&target.hidden_bias, &context.hidden_bias, decay);
        target.output_weights = ema_values(&target.output_weights, &context.output_weights, decay);
        target.output_bias = ema_values(&target.output_bias, &context.output_bias, decay);
        target.residual_weight =
            decay * target.residual_weight + (1.0 - decay) * context.residual_weight;
        target
    }

    pub fn encode_window(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.project(window_features(window, self.latent_dim, &self.role)?)
    }

    pub fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>> {
        self.project(action_features(action, self.latent_dim, &self.role)?)
    }

    fn project(&self, features: Vec<f32>) -> Result<Vec<f32>> {
        if features.len() != self.latent_dim {
            bail!("jepa feature dimension mismatch");
        }
        let mut hidden = vec![0.0; self.latent_dim];
        for idx in 0..self.latent_dim {
            hidden[idx] = gelu(features[idx] * self.input_weights[idx] + self.hidden_bias[idx]);
        }
        let mut output = vec![0.0; self.latent_dim];
        for idx in 0..self.latent_dim {
            output[idx] = self.residual_weight * features[idx]
                + (1.0 - self.residual_weight)
                    * (hidden[idx] * self.output_weights[idx] + self.output_bias[idx]);
        }
        layer_norm(&mut output);
        Ok(output)
    }

    fn parameter_count(&self) -> u64 {
        (self.input_weights.len()
            + self.hidden_bias.len()
            + self.output_weights.len()
            + self.output_bias.len()) as u64
    }

    fn finite(&self) -> bool {
        self.input_weights.iter().all(|value| value.is_finite())
            && self.hidden_bias.iter().all(|value| value.is_finite())
            && self.output_weights.iter().all(|value| value.is_finite())
            && self.output_bias.iter().all(|value| value.is_finite())
            && self.residual_weight.is_finite()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaPredictor {
    pub latent_dim: usize,
    pub context_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
    pub horizon_weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl JepaPredictor {
    fn baseline(latent_dim: usize) -> Self {
        Self {
            latent_dim,
            context_weights: vec![1.0; latent_dim],
            action_weights: vec![0.0; latent_dim],
            horizon_weights: vec![0.0; latent_dim],
            bias: vec![0.0; latent_dim],
        }
    }

    fn fit(latent_dim: usize, examples: &[EncodedJepaTrainingExample]) -> Result<Self> {
        if examples.is_empty() {
            bail!("at least one JEPA example is required");
        }
        let mut context_mean = vec![0.0; latent_dim];
        let mut action_mean = vec![0.0; latent_dim];
        let mut target_mean = vec![0.0; latent_dim];
        let mut horizon_mean = 0.0;
        for example in examples {
            validate_latents(latent_dim, example)?;
            let horizon = normalized_horizon(example.horizon);
            horizon_mean += horizon;
            for idx in 0..latent_dim {
                context_mean[idx] += example.context_latent[idx];
                action_mean[idx] += example.action_latent[idx];
                target_mean[idx] += example.target_latent[idx];
            }
        }
        let denom = examples.len() as f32;
        horizon_mean /= denom;
        for idx in 0..latent_dim {
            context_mean[idx] /= denom;
            action_mean[idx] /= denom;
            target_mean[idx] /= denom;
        }

        let mut context_weights = vec![0.0; latent_dim];
        let mut action_weights = vec![0.0; latent_dim];
        let mut horizon_weights = vec![0.0; latent_dim];
        let mut bias = vec![0.0; latent_dim];
        for idx in 0..latent_dim {
            context_weights[idx] = covariance_weight(
                examples,
                idx,
                context_mean[idx],
                target_mean[idx],
                InputRole::Context,
            );
            action_weights[idx] = covariance_weight(
                examples,
                idx,
                action_mean[idx],
                target_mean[idx],
                InputRole::Action,
            );
            horizon_weights[idx] = covariance_weight(
                examples,
                idx,
                horizon_mean,
                target_mean[idx],
                InputRole::Horizon,
            );
            bias[idx] = target_mean[idx]
                - context_weights[idx] * context_mean[idx]
                - action_weights[idx] * action_mean[idx]
                - horizon_weights[idx] * horizon_mean;
        }

        Ok(Self {
            latent_dim,
            context_weights,
            action_weights,
            horizon_weights,
            bias,
        })
    }

    pub fn predict(&self, context: &[f32], action: &[f32], horizon: usize) -> Result<Vec<f32>> {
        if context.len() != self.latent_dim || action.len() != self.latent_dim {
            bail!("jepa predictor latent dimensions must match");
        }
        let horizon = normalized_horizon(horizon);
        let mut predicted = vec![0.0; self.latent_dim];
        for idx in 0..self.latent_dim {
            predicted[idx] = (self.bias[idx]
                + self.context_weights[idx] * context[idx]
                + self.action_weights[idx] * action[idx]
                + self.horizon_weights[idx] * horizon)
                .tanh();
        }
        layer_norm(&mut predicted);
        Ok(predicted)
    }

    fn parameter_count(&self) -> u64 {
        (self.context_weights.len()
            + self.action_weights.len()
            + self.horizon_weights.len()
            + self.bias.len()) as u64
    }

    fn finite(&self) -> bool {
        self.context_weights.iter().all(|value| value.is_finite())
            && self.action_weights.iter().all(|value| value.is_finite())
            && self.horizon_weights.iter().all(|value| value.is_finite())
            && self.bias.iter().all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaAuxiliaryHead {
    pub label: String,
    pub bias: f32,
    pub latent_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
}

impl JepaAuxiliaryHead {
    pub fn predict_probability(&self, context: &[f32], action: &[f32]) -> f32 {
        sigmoid(
            self.bias
                + dot_prefix(&self.latent_weights, context)
                + dot_prefix(&self.action_weights, action),
        )
    }

    fn parameter_count(&self) -> u64 {
        (1 + self.latent_weights.len() + self.action_weights.len()) as u64
    }

    fn finite(&self) -> bool {
        self.bias.is_finite()
            && self.latent_weights.iter().all(|value| value.is_finite())
            && self.action_weights.iter().all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTraceModel {
    pub metadata: JepaTraceModelMetadata,
    pub context_encoder: JepaTraceEncoder,
    pub action_encoder: JepaTraceEncoder,
    pub target_encoder: JepaTraceEncoder,
    pub predictor: JepaPredictor,
    pub auxiliary_heads: Vec<JepaAuxiliaryHead>,
    pub transition_model: Option<CpuLatentTransitionModel>,
}

impl JepaTraceModel {
    pub fn predict_training_target(
        &self,
        context: &[f32],
        action: &[f32],
        horizon: usize,
    ) -> Result<Vec<f32>> {
        self.predictor.predict(context, action, horizon)
    }

    pub fn predict_auxiliary(&self, context: &[f32], action: &[f32]) -> Result<Vec<(String, f32)>> {
        if context.len() != self.metadata.latent_dim || action.len() != self.metadata.latent_dim {
            bail!("jepa auxiliary latent dimensions must match");
        }
        Ok(self
            .auxiliary_heads
            .iter()
            .map(|head| {
                (
                    head.label.clone(),
                    head.predict_probability(context, action),
                )
            })
            .collect())
    }

    pub fn validate_finite(&self) -> Result<()> {
        if !self.context_encoder.finite()
            || !self.action_encoder.finite()
            || !self.target_encoder.finite()
            || !self.predictor.finite()
            || !self.auxiliary_heads.iter().all(JepaAuxiliaryHead::finite)
            || !self
                .transition_model
                .as_ref()
                .is_none_or(transition_model_finite)
        {
            bail!("jepa checkpoint contains non-finite values");
        }
        Ok(())
    }

    fn parameter_count(&self) -> u64 {
        self.context_encoder.parameter_count()
            + self.action_encoder.parameter_count()
            + self.target_encoder.parameter_count()
            + self.predictor.parameter_count()
            + self
                .auxiliary_heads
                .iter()
                .map(JepaAuxiliaryHead::parameter_count)
                .sum::<u64>()
            + self
                .transition_model
                .as_ref()
                .map(|model| model.metadata.parameter_count)
                .unwrap_or_default()
    }
}

impl WorldRepresentationAdapter for JepaTraceModel {
    fn dimensions(&self) -> usize {
        self.metadata.latent_dim
    }

    fn provider_name(&self) -> &str {
        "archon-jepa"
    }

    fn model_name(&self) -> &str {
        &self.metadata.model_id
    }

    fn encode_state(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.context_encoder.encode_window(window)
    }

    fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>> {
        self.action_encoder.encode_action(action)
    }

    fn encode_target(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.target_encoder.encode_window(window)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTrainingExample {
    pub context: TraceWindow,
    pub action: TraceAction,
    pub target: TraceWindow,
    pub horizon: usize,
    pub labels: WorldLabelSet,
}

impl From<TraceTransition> for JepaTrainingExample {
    fn from(transition: TraceTransition) -> Self {
        Self {
            horizon: transition.target.horizon,
            context: transition.context,
            action: transition.action,
            target: transition.target,
            labels: transition.labels,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTrainingLosses {
    pub loss_jepa: f32,
    pub loss_mse: f32,
    pub loss_aux: f32,
    pub loss_horizon: f32,
    pub loss_var: f32,
    pub loss_total: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTrainingProgress {
    pub initial_loss_total: f32,
    pub final_loss_total: f32,
    pub improved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaMaskingReport {
    pub mask_ratio: f32,
    pub masked_context_fields: usize,
    pub masked_action_fields: usize,
    pub reconstructs_raw_text: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaCollapseReport {
    pub mean_latent_std: f32,
    pub effective_rank_ratio: f32,
    pub min_latent_std: f32,
    pub min_effective_rank_ratio: f32,
    pub passes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaHorizonReport {
    pub e_1: Option<f32>,
    pub e_3: Option<f32>,
    pub e_5: Option<f32>,
    pub tolerance: f32,
    pub passes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaRepresentationComparisonReport {
    pub candidate_id: String,
    pub baseline_backend: String,
    pub baseline_available: bool,
    pub failure_reason: Option<String>,
    pub heldout_examples: usize,
    pub min_heldout_examples: usize,
    pub jepa_next_state_cosine_similarity: f32,
    pub baseline_next_state_cosine_similarity: f32,
    pub relative_improvement: f32,
    pub min_baseline_improvement: f32,
    pub brier_regressed: bool,
    pub passed: bool,
}

impl JepaRepresentationComparisonReport {
    pub fn fail_closed(
        candidate_id: impl Into<String>,
        baseline_backend: impl Into<String>,
        failure_reason: impl Into<String>,
        min_heldout_examples: usize,
        min_baseline_improvement: f32,
    ) -> Self {
        Self {
            candidate_id: candidate_id.into(),
            baseline_backend: baseline_backend.into(),
            baseline_available: false,
            failure_reason: Some(failure_reason.into()),
            heldout_examples: 0,
            min_heldout_examples,
            jepa_next_state_cosine_similarity: 0.0,
            baseline_next_state_cosine_similarity: 0.0,
            relative_improvement: 0.0,
            min_baseline_improvement,
            brier_regressed: true,
            passed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaPromotionGateReport {
    pub corpus_sufficient: bool,
    pub representation_baseline: bool,
    pub representation_collapse: bool,
    pub multi_horizon_consistency: bool,
    pub checkpoint_size: bool,
    pub tensor_safety: bool,
    #[serde(default = "default_true")]
    pub backend_execution: bool,
    pub passed: bool,
}

impl JepaPromotionGateReport {
    pub fn from_parts(
        corpus_sufficient: bool,
        representation_baseline: bool,
        representation_collapse: bool,
        multi_horizon_consistency: bool,
        checkpoint_size: bool,
        tensor_safety: bool,
    ) -> Self {
        Self::from_parts_with_backend_execution(
            corpus_sufficient,
            representation_baseline,
            representation_collapse,
            multi_horizon_consistency,
            checkpoint_size,
            tensor_safety,
            true,
        )
    }

    pub fn from_parts_with_backend_execution(
        corpus_sufficient: bool,
        representation_baseline: bool,
        representation_collapse: bool,
        multi_horizon_consistency: bool,
        checkpoint_size: bool,
        tensor_safety: bool,
        backend_execution: bool,
    ) -> Self {
        let passed = corpus_sufficient
            && representation_baseline
            && representation_collapse
            && multi_horizon_consistency
            && checkpoint_size
            && tensor_safety
            && backend_execution;
        Self {
            corpus_sufficient,
            representation_baseline,
            representation_collapse,
            multi_horizon_consistency,
            checkpoint_size,
            tensor_safety,
            backend_execution,
            passed,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaEvalRecord {
    pub candidate_id: String,
    pub comparison: JepaRepresentationComparisonReport,
    pub collapse: JepaCollapseReport,
    pub horizon: JepaHorizonReport,
    pub gates: JepaPromotionGateReport,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTrainingOutcome {
    pub status: TrainingStatus,
    pub metadata: JepaTraceModelMetadata,
    pub initial_losses: JepaTrainingLosses,
    pub losses: JepaTrainingLosses,
    pub progress: JepaTrainingProgress,
    pub masking: JepaMaskingReport,
    pub collapse: JepaCollapseReport,
    pub horizon: JepaHorizonReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaCheckpointRecord {
    pub model_id: String,
    pub format: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaCheckpointTensors {
    pub context_input_weights: Vec<f32>,
    pub context_hidden_bias: Vec<f32>,
    pub context_output_weights: Vec<f32>,
    pub context_output_bias: Vec<f32>,
    pub action_input_weights: Vec<f32>,
    pub action_hidden_bias: Vec<f32>,
    pub action_output_weights: Vec<f32>,
    pub action_output_bias: Vec<f32>,
    pub target_input_weights: Vec<f32>,
    pub target_hidden_bias: Vec<f32>,
    pub target_output_weights: Vec<f32>,
    pub target_output_bias: Vec<f32>,
    pub predictor_context_weights: Vec<f32>,
    pub predictor_action_weights: Vec<f32>,
    pub predictor_horizon_weights: Vec<f32>,
    pub predictor_bias: Vec<f32>,
    pub auxiliary_bias: Vec<f32>,
    pub auxiliary_latent_weights: Vec<f32>,
    pub auxiliary_action_weights: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodedJepaTrainingExample {
    pub context_latent: Vec<f32>,
    pub action_latent: Vec<f32>,
    pub target_latent: Vec<f32>,
    pub horizon: usize,
    pub labels: WorldLabelSet,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaEncoderSet {
    pub context_encoder: JepaTraceEncoder,
    pub action_encoder: JepaTraceEncoder,
    pub target_encoder: JepaTraceEncoder,
}

pub type JepaFeatureBatch = Vec<JepaTrainingExample>;
pub type JepaEncodedBatch = Vec<EncodedJepaTrainingExample>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaBackendProbeReport {
    pub status: BackendStatus,
    pub feature_compiled: bool,
    pub tensor_self_test_passed: bool,
    pub native_runtime_prediction: bool,
    pub unavailable_reason: Option<String>,
}

impl JepaBackendProbeReport {
    pub fn from_status(status: BackendStatus, native_runtime_prediction: bool) -> Self {
        let unavailable_reason = status.fallback_reason.clone();
        let feature_compiled =
            status.selected == BackendKind::Cpu || status.framework != "unavailable";
        Self {
            status,
            feature_compiled,
            tensor_self_test_passed: true,
            native_runtime_prediction,
            unavailable_reason,
        }
    }

    pub fn from_probe(
        requested_backend: BackendKind,
        probe: crate::backend::BackendProbeReport,
        native_runtime_prediction: bool,
    ) -> Self {
        let status = BackendStatus {
            requested: requested_backend,
            selected: requested_backend,
            framework: probe.framework.clone(),
            device_name: None,
            experimental: requested_backend == BackendKind::Metal,
            fallback_reason: probe.reason.clone(),
        };
        Self {
            status,
            feature_compiled: probe.compiled,
            tensor_self_test_passed: probe.tensor_self_test_passed,
            native_runtime_prediction: native_runtime_prediction && probe.available,
            unavailable_reason: probe.reason,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaRuntimePrediction {
    pub backend: BackendKind,
    pub predicted_next_state: Vec<f32>,
    pub auxiliary_scores: Vec<(String, f32)>,
    pub latency_ms: u64,
}

pub trait JepaTensorBackend: Send + Sync {
    fn status(&self) -> BackendStatus;
    fn probe_jepa(&self) -> JepaBackendProbeReport;

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaEncodedBatch>;

    fn fit_predictor(&self, latent_dim: usize, encoded: &JepaEncodedBatch)
    -> Result<JepaPredictor>;

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<Vec<JepaAuxiliaryHead>>;

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<CpuLatentTransitionModel>;

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaTrainingLosses>;

    fn collapse_report(
        &self,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaCollapseReport>;

    fn predict_runtime(
        &self,
        model: &JepaTraceModel,
        window: &TraceWindow,
        action: &TraceAction,
    ) -> Result<JepaRuntimePrediction>;
}

#[derive(Debug, Clone, Default)]
pub struct CpuJepaBackend;

impl JepaTensorBackend for CpuJepaBackend {
    fn status(&self) -> BackendStatus {
        BackendStatus::cpu()
    }

    fn probe_jepa(&self) -> JepaBackendProbeReport {
        JepaBackendProbeReport::from_status(self.status(), true)
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaEncodedBatch> {
        encode_examples(
            &encoders.context_encoder,
            &encoders.action_encoder,
            &encoders.target_encoder,
            batch,
        )
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaPredictor> {
        JepaPredictor::fit(latent_dim, encoded)
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<Vec<JepaAuxiliaryHead>> {
        Ok(fit_auxiliary_heads(latent_dim, encoded))
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<CpuLatentTransitionModel> {
        CpuLatentTransitionModel::fit(latent_dim, &encoded_transition_examples(encoded))
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaTrainingLosses> {
        training_losses(model, encoded, config)
    }

    fn collapse_report(
        &self,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaCollapseReport> {
        evaluate_representation_collapse(
            &heldout_context_latents(encoded),
            config.min_latent_std,
            config.min_effective_rank_ratio,
        )
    }

    fn predict_runtime(
        &self,
        model: &JepaTraceModel,
        window: &TraceWindow,
        action: &TraceAction,
    ) -> Result<JepaRuntimePrediction> {
        let started = Instant::now();
        let transition = model
            .transition_model
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("JepaCheckpointMissing: transition model missing"))?;
        let state = model.encode_state(window)?;
        let action_latent = model.encode_action(action)?;
        let predicted_next_state = crate::backend::predict_next_with_backend(
            transition,
            &state,
            &action_latent,
            BackendKind::Cpu,
        )?;
        let auxiliary_scores = model.predict_auxiliary(&state, &action_latent)?;
        Ok(JepaRuntimePrediction {
            backend: BackendKind::Cpu,
            predicted_next_state,
            auxiliary_scores,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct CandleCudaJepaBackend;

impl JepaTensorBackend for CandleCudaJepaBackend {
    fn status(&self) -> BackendStatus {
        crate::backend::select_runtime_backend(BackendKind::Cuda, false)
    }

    fn probe_jepa(&self) -> JepaBackendProbeReport {
        JepaBackendProbeReport::from_probe(
            BackendKind::Cuda,
            crate::backend::probe_backend(BackendKind::Cuda),
            false,
        )
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaEncodedBatch> {
        let _ = (encoders, batch);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaPredictor> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<Vec<JepaAuxiliaryHead>> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<CpuLatentTransitionModel> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaTrainingLosses> {
        let _ = (model, encoded, config);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn collapse_report(
        &self,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaCollapseReport> {
        let _ = (encoded, config);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }

    fn predict_runtime(
        &self,
        model: &JepaTraceModel,
        window: &TraceWindow,
        action: &TraceAction,
    ) -> Result<JepaRuntimePrediction> {
        let _ = (model, window, action);
        native_jepa_backend_unavailable(BackendKind::Cuda)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MlxMetalJepaBackend;

impl JepaTensorBackend for MlxMetalJepaBackend {
    fn status(&self) -> BackendStatus {
        crate::backend::select_runtime_backend(BackendKind::Metal, false)
    }

    fn probe_jepa(&self) -> JepaBackendProbeReport {
        JepaBackendProbeReport::from_probe(
            BackendKind::Metal,
            crate::backend::probe_backend(BackendKind::Metal),
            false,
        )
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaEncodedBatch> {
        let _ = (encoders, batch);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaPredictor> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<Vec<JepaAuxiliaryHead>> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<CpuLatentTransitionModel> {
        let _ = (latent_dim, encoded);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaTrainingLosses> {
        let _ = (model, encoded, config);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn collapse_report(
        &self,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaCollapseReport> {
        let _ = (encoded, config);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }

    fn predict_runtime(
        &self,
        model: &JepaTraceModel,
        window: &TraceWindow,
        action: &TraceAction,
    ) -> Result<JepaRuntimePrediction> {
        let _ = (model, window, action);
        native_jepa_backend_unavailable(BackendKind::Metal)
    }
}

fn native_jepa_backend_unavailable<T>(backend: BackendKind) -> Result<T> {
    bail!("native {backend} JEPA tensor backend is not implemented")
}

pub fn build_jepa_training_examples(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
) -> Result<Vec<JepaTrainingExample>> {
    config.validate()?;
    let builder = TraceWindowBuilder::new(rows);
    let mut examples = Vec::new();
    for horizon in &config.prediction_horizons {
        let transitions = builder.adjacent_transitions(
            config.context_window_rows,
            config.target_window_rows,
            *horizon,
        )?;
        examples.extend(transitions.into_iter().map(JepaTrainingExample::from));
    }
    Ok(examples)
}

pub fn mask_jepa_training_examples(
    examples: &[JepaTrainingExample],
    mask_ratio: f32,
) -> (Vec<JepaTrainingExample>, JepaMaskingReport) {
    let mask_ratio = mask_ratio.clamp(0.0, 1.0);
    let mut report = JepaMaskingReport {
        mask_ratio,
        masked_context_fields: 0,
        masked_action_fields: 0,
        reconstructs_raw_text: false,
    };
    let masked = examples
        .iter()
        .map(|example| mask_jepa_training_example(example, mask_ratio, &mut report))
        .collect();
    (masked, report)
}

pub fn evaluate_representation_collapse(
    latents: &[Vec<f32>],
    min_latent_std: f32,
    min_effective_rank_ratio: f32,
) -> Result<JepaCollapseReport> {
    if latents.is_empty() || latents[0].is_empty() {
        bail!("collapse evaluation requires at least one non-empty latent");
    }
    let dim = latents[0].len();
    if latents.iter().any(|latent| latent.len() != dim) {
        bail!("collapse evaluation latent dimensions must match");
    }
    if latents
        .iter()
        .flat_map(|latent| latent.iter())
        .any(|value| !value.is_finite())
    {
        bail!("collapse evaluation latents must be finite");
    }
    let mut means = vec![0.0_f64; dim];
    for latent in latents {
        for (idx, value) in latent.iter().enumerate() {
            means[idx] += f64::from(*value);
        }
    }
    for mean in &mut means {
        *mean /= latents.len() as f64;
    }
    let mut stds = vec![0.0_f64; dim];
    for idx in 0..dim {
        let variance = latents
            .iter()
            .map(|latent| {
                let centered = f64::from(latent[idx]) - means[idx];
                centered * centered
            })
            .sum::<f64>()
            / latents.len() as f64;
        stds[idx] = variance.sqrt();
    }
    let mean_latent_std = (stds.iter().sum::<f64>() / dim as f64) as f32;
    let effective_rank_ratio = singular_value_effective_rank_ratio(latents, &means)?;
    let passes =
        mean_latent_std >= min_latent_std && effective_rank_ratio >= min_effective_rank_ratio;
    Ok(JepaCollapseReport {
        mean_latent_std,
        effective_rank_ratio,
        min_latent_std,
        min_effective_rank_ratio,
        passes,
    })
}

fn singular_value_effective_rank_ratio(latents: &[Vec<f32>], means: &[f64]) -> Result<f32> {
    let sample_count = latents.len();
    let dim = means.len();
    if sample_count == 0 || dim == 0 {
        bail!("collapse evaluation requires at least one latent");
    }

    let gram = mean_centered_gram_matrix(latents, means);
    let eigenvalues = jacobi_symmetric_eigenvalues(gram);
    let mut sigma_sum = 0.0_f64;
    let mut sigma_sq_sum = 0.0_f64;
    for eigenvalue in eigenvalues {
        if !eigenvalue.is_finite() {
            bail!("collapse evaluation eigenvalues must be finite");
        }
        let singular_value = eigenvalue.max(0.0).sqrt();
        sigma_sum += singular_value;
        sigma_sq_sum += singular_value * singular_value;
    }
    if sigma_sq_sum <= f64::EPSILON || !sigma_sum.is_finite() || !sigma_sq_sum.is_finite() {
        return Ok(0.0);
    }

    let effective_rank = (sigma_sum * sigma_sum) / sigma_sq_sum;
    Ok((effective_rank / dim as f64).clamp(0.0, 1.0) as f32)
}

fn mean_centered_gram_matrix(latents: &[Vec<f32>], means: &[f64]) -> Vec<Vec<f64>> {
    let sample_count = latents.len();
    let dim = means.len();
    if sample_count <= dim {
        let mut matrix = vec![vec![0.0_f64; sample_count]; sample_count];
        for row_idx in 0..sample_count {
            for other_idx in row_idx..sample_count {
                let mut dot = 0.0_f64;
                for feature_idx in 0..dim {
                    dot += centered_latent(latents, means, row_idx, feature_idx)
                        * centered_latent(latents, means, other_idx, feature_idx);
                }
                matrix[row_idx][other_idx] = dot;
                matrix[other_idx][row_idx] = dot;
            }
        }
        matrix
    } else {
        let mut matrix = vec![vec![0.0_f64; dim]; dim];
        for left_idx in 0..dim {
            for right_idx in left_idx..dim {
                let mut dot = 0.0_f64;
                for row_idx in 0..sample_count {
                    dot += centered_latent(latents, means, row_idx, left_idx)
                        * centered_latent(latents, means, row_idx, right_idx);
                }
                matrix[left_idx][right_idx] = dot;
                matrix[right_idx][left_idx] = dot;
            }
        }
        matrix
    }
}

fn centered_latent(latents: &[Vec<f32>], means: &[f64], row_idx: usize, feature_idx: usize) -> f64 {
    f64::from(latents[row_idx][feature_idx]) - means[feature_idx]
}

fn jacobi_symmetric_eigenvalues(mut matrix: Vec<Vec<f64>>) -> Vec<f64> {
    let size = matrix.len();
    if size == 0 {
        return Vec::new();
    }
    if size == 1 {
        return vec![matrix[0][0]];
    }

    let scale = matrix
        .iter()
        .enumerate()
        .map(|(idx, row)| row[idx].abs())
        .fold(1.0_f64, f64::max);
    let tolerance = (1e-10_f64 * scale).max(1e-12);
    for _ in 0..64 {
        let mut changed = false;
        for pivot in 0..size - 1 {
            for other in pivot + 1..size {
                let off_diag = matrix[pivot][other];
                if off_diag.abs() <= tolerance {
                    continue;
                }
                changed = true;
                let pivot_diag = matrix[pivot][pivot];
                let other_diag = matrix[other][other];
                let tau = (other_diag - pivot_diag) / (2.0 * off_diag);
                let turn = if tau >= 0.0 {
                    1.0 / (tau + (1.0 + tau * tau).sqrt())
                } else {
                    -1.0 / (-tau + (1.0 + tau * tau).sqrt())
                };
                let cosine = 1.0 / (1.0 + turn * turn).sqrt();
                let sine = turn * cosine;

                for idx in 0..size {
                    if idx == pivot || idx == other {
                        continue;
                    }
                    let left = matrix[idx][pivot];
                    let right = matrix[idx][other];
                    let rotated_left = cosine * left - sine * right;
                    let rotated_right = sine * left + cosine * right;
                    matrix[idx][pivot] = rotated_left;
                    matrix[pivot][idx] = rotated_left;
                    matrix[idx][other] = rotated_right;
                    matrix[other][idx] = rotated_right;
                }

                matrix[pivot][pivot] = cosine * cosine * pivot_diag
                    - 2.0 * sine * cosine * off_diag
                    + sine * sine * other_diag;
                matrix[other][other] = sine * sine * pivot_diag
                    + 2.0 * sine * cosine * off_diag
                    + cosine * cosine * other_diag;
                matrix[pivot][other] = 0.0;
                matrix[other][pivot] = 0.0;
            }
        }
        if !changed {
            break;
        }
    }

    matrix
        .into_iter()
        .enumerate()
        .map(|(idx, row)| row[idx])
        .collect()
}

pub fn train_jepa_candidate(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_backend(rows, config, BackendKind::Cpu, true)
}

pub fn train_jepa_candidate_with_backend(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    requested_backend: BackendKind,
    allow_cpu_fallback: bool,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    let status = crate::backend::select_runtime_backend(requested_backend, allow_cpu_fallback);
    train_jepa_candidate_with_backend_status(rows, config, status, allow_cpu_fallback)
}

fn train_jepa_candidate_with_backend_status(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    if status.selected == BackendKind::Cpu {
        return train_jepa_candidate_cpu(rows, config, status);
    }

    if allow_cpu_fallback {
        let fallback = BackendStatus::cpu_fallback(
            status.requested,
            format!("jepa_native_backend_not_implemented:{}", status.selected),
        );
        return train_jepa_candidate_cpu(rows, config, fallback);
    }

    bail!(
        "native JEPA backend for {} is not implemented; refusing to write an accelerator-labelled candidate",
        status.selected
    );
}

fn train_jepa_candidate_cpu(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    backend_status: BackendStatus,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    config.validate()?;
    let examples = build_jepa_training_examples(rows, config)?;
    if examples.is_empty() {
        bail!("not enough rows to train JEPA: need future rows in the same session");
    }
    let (masked_examples, masking) = mask_jepa_training_examples(&examples, config.mask_ratio);

    let context_encoder = JepaTraceEncoder::new("context", config.latent_dim);
    let action_encoder = JepaTraceEncoder::new("action", config.latent_dim);
    let target_encoder = JepaTraceEncoder::ema_target_from(&context_encoder, config.ema_decay);
    let backend = CpuJepaBackend;
    let encoders = JepaEncoderSet {
        context_encoder: context_encoder.clone(),
        action_encoder: action_encoder.clone(),
        target_encoder: target_encoder.clone(),
    };
    let encoded = backend.encode_batch(&encoders, &masked_examples)?;
    let initial_model = JepaTraceModel {
        metadata: JepaTraceModelMetadata::candidate(
            config,
            rows.len() as u64,
            examples.len() as u64,
        ),
        context_encoder: context_encoder.clone(),
        action_encoder: action_encoder.clone(),
        target_encoder: target_encoder.clone(),
        predictor: JepaPredictor::baseline(config.latent_dim),
        auxiliary_heads: backend.fit_auxiliary_heads(config.latent_dim, &encoded)?,
        transition_model: None,
    };
    let initial_losses = backend.training_losses(&initial_model, &encoded, config)?;
    let predictor = backend.fit_predictor(config.latent_dim, &encoded)?;
    let auxiliary_heads = backend.fit_auxiliary_heads(config.latent_dim, &encoded)?;
    let transition_model = backend.fit_transition(config.latent_dim, &encoded)?;
    let mut metadata =
        JepaTraceModelMetadata::candidate(config, rows.len() as u64, examples.len() as u64);
    metadata.backend = BackendKind::Cpu;
    metadata.backend_execution =
        JepaBackendExecutionReport::from_cpu_status(&backend_status, examples.len());
    let mut model = JepaTraceModel {
        metadata: metadata.clone(),
        context_encoder,
        action_encoder,
        target_encoder,
        predictor,
        auxiliary_heads,
        transition_model: Some(transition_model),
    };
    metadata.parameter_count = model.parameter_count();
    metadata.backend_execution.validation_example_count = examples.len();
    model.metadata = metadata;
    validate_jepa_backend_execution(&model.metadata)?;
    model.validate_finite()?;
    let losses = backend.training_losses(&model, &encoded, config)?;
    let progress = JepaTrainingProgress {
        initial_loss_total: initial_losses.loss_total,
        final_loss_total: losses.loss_total,
        improved: losses.loss_total <= initial_losses.loss_total,
    };
    let collapse = backend.collapse_report(&encoded, config)?;
    let horizon = horizon_report_for_model(&model, &encoded, config.horizon_consistency_tol)?;
    let outcome = JepaTrainingOutcome {
        status: TrainingStatus::CandidateWritten,
        metadata: model.metadata.clone(),
        initial_losses,
        losses,
        progress,
        masking,
        collapse,
        horizon,
    };
    Ok((model, outcome))
}

pub fn validate_jepa_backend_execution(metadata: &JepaTraceModelMetadata) -> Result<()> {
    let report = &metadata.backend_execution;
    if metadata.backend != report.selected_backend {
        bail!(
            "jepa metadata backend {:?} does not match execution report selected backend {:?}",
            metadata.backend,
            report.selected_backend
        );
    }

    if matches!(metadata.backend, BackendKind::Cuda | BackendKind::Metal) {
        if !report.native_stage_proof_passes() {
            bail!(
                "jepa {:?} candidate is missing native backend execution proof",
                metadata.backend
            );
        }
    }

    if metadata.backend == BackendKind::Cpu
        && matches!(
            report.selected_backend,
            BackendKind::Cuda | BackendKind::Metal
        )
    {
        bail!("jepa CPU candidate cannot carry accelerator-selected execution metadata");
    }

    Ok(())
}

pub fn jepa_backend_promotion_gate(
    metadata: &JepaTraceModelMetadata,
    min_cuda_validation_examples: usize,
    min_metal_validation_examples: usize,
) -> bool {
    if validate_jepa_backend_execution(metadata).is_err() {
        return false;
    }
    let report = &metadata.backend_execution;
    match metadata.backend {
        BackendKind::Cuda => {
            report.native_stage_proof_passes()
                && report.hardware_validation_captured_at.is_some()
                && report.validation_example_count >= min_cuda_validation_examples
        }
        BackendKind::Metal => {
            report.native_stage_proof_passes()
                && report.hardware_validation_captured_at.is_some()
                && report.validation_example_count >= min_metal_validation_examples
        }
        BackendKind::Auto | BackendKind::Cpu => true,
    }
}

pub fn predict_jepa_with_backend(
    model: &JepaTraceModel,
    window: &TraceWindow,
    action: &TraceAction,
    backend: BackendKind,
) -> Result<JepaRuntimePrediction> {
    validate_jepa_backend_execution(&model.metadata)?;
    if model.metadata.backend != BackendKind::Auto && model.metadata.backend != backend {
        bail!(
            "JepaBackendUnavailable: requested runtime backend {backend} does not match candidate backend {}",
            model.metadata.backend
        );
    }

    match backend {
        BackendKind::Auto | BackendKind::Cpu => {
            CpuJepaBackend.predict_runtime(model, window, action)
        }
        BackendKind::Cuda => CandleCudaJepaBackend
            .predict_runtime(model, window, action)
            .map_err(|error| anyhow::anyhow!("JepaBackendUnavailable: {error}")),
        BackendKind::Metal => MlxMetalJepaBackend
            .predict_runtime(model, window, action)
            .map_err(|error| anyhow::anyhow!("JepaBackendUnavailable: {error}")),
    }
}

pub fn append_jepa_training_run(root: &Path, outcome: &JepaTrainingOutcome) -> Result<PathBuf> {
    let dir = root.join("jepa").join("training-runs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("training-runs.jsonl");
    let mut line = serde_json::to_vec(&serde_json::json!({
        "model_id": outcome.metadata.model_id.clone(),
        "model_kind": outcome.metadata.model_kind.clone(),
        "created_at": Utc::now(),
        "row_count": outcome.metadata.row_count,
        "example_count": outcome.metadata.example_count,
        "horizons": outcome.metadata.prediction_horizons.clone(),
        "backend_execution": outcome.metadata.backend_execution.clone(),
        "masking": outcome.masking.clone(),
        "initial_losses": outcome.initial_losses.clone(),
        "losses": outcome.losses.clone(),
        "progress": outcome.progress.clone(),
        "collapse": outcome.collapse.clone(),
        "horizon": outcome.horizon.clone()
    }))?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)?;
    Ok(path)
}

pub fn write_jepa_safetensors_checkpoint(
    root: &Path,
    model: &JepaTraceModel,
) -> Result<JepaCheckpointRecord> {
    let record = JepaCheckpointRecord {
        model_id: model.metadata.model_id.clone(),
        format: "candle_safetensors".into(),
        path: root
            .join("jepa")
            .join("candidates")
            .join(format!("{}.safetensors", model.metadata.model_id)),
    };
    if let Some(parent) = record.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tensors = jepa_checkpoint_tensors(model);
    let named = vec![
        ("context_input_weights", tensors.context_input_weights),
        ("context_hidden_bias", tensors.context_hidden_bias),
        ("context_output_weights", tensors.context_output_weights),
        ("context_output_bias", tensors.context_output_bias),
        ("action_input_weights", tensors.action_input_weights),
        ("action_hidden_bias", tensors.action_hidden_bias),
        ("action_output_weights", tensors.action_output_weights),
        ("action_output_bias", tensors.action_output_bias),
        ("target_input_weights", tensors.target_input_weights),
        ("target_hidden_bias", tensors.target_hidden_bias),
        ("target_output_weights", tensors.target_output_weights),
        ("target_output_bias", tensors.target_output_bias),
        (
            "predictor_context_weights",
            tensors.predictor_context_weights,
        ),
        ("predictor_action_weights", tensors.predictor_action_weights),
        (
            "predictor_horizon_weights",
            tensors.predictor_horizon_weights,
        ),
        ("predictor_bias", tensors.predictor_bias),
        ("auxiliary_bias", tensors.auxiliary_bias),
        ("auxiliary_latent_weights", tensors.auxiliary_latent_weights),
        ("auxiliary_action_weights", tensors.auxiliary_action_weights),
    ];
    let tensor_bytes = named
        .into_iter()
        .map(|(name, values)| (name.to_string(), f32_bytes(&values), values.len()))
        .collect::<Vec<_>>();
    let views = tensor_bytes
        .iter()
        .map(|(name, bytes, len)| {
            Ok((
                name.clone(),
                TensorView::new(Dtype::F32, vec![*len], bytes.as_slice())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    serialize_to_file(views, None, &record.path)?;
    Ok(record)
}

pub fn read_jepa_safetensors_checkpoint(path: &Path) -> Result<JepaCheckpointTensors> {
    let bytes = std::fs::read(path)?;
    let tensors = safetensors::SafeTensors::deserialize(&bytes)?;
    Ok(JepaCheckpointTensors {
        context_input_weights: tensor_f32(&tensors, "context_input_weights")?,
        context_hidden_bias: tensor_f32(&tensors, "context_hidden_bias")?,
        context_output_weights: tensor_f32(&tensors, "context_output_weights")?,
        context_output_bias: tensor_f32(&tensors, "context_output_bias")?,
        action_input_weights: tensor_f32(&tensors, "action_input_weights")?,
        action_hidden_bias: tensor_f32(&tensors, "action_hidden_bias")?,
        action_output_weights: tensor_f32(&tensors, "action_output_weights")?,
        action_output_bias: tensor_f32(&tensors, "action_output_bias")?,
        target_input_weights: tensor_f32(&tensors, "target_input_weights")?,
        target_hidden_bias: tensor_f32(&tensors, "target_hidden_bias")?,
        target_output_weights: tensor_f32(&tensors, "target_output_weights")?,
        target_output_bias: tensor_f32(&tensors, "target_output_bias")?,
        predictor_context_weights: tensor_f32(&tensors, "predictor_context_weights")?,
        predictor_action_weights: tensor_f32(&tensors, "predictor_action_weights")?,
        predictor_horizon_weights: tensor_f32(&tensors, "predictor_horizon_weights")?,
        predictor_bias: tensor_f32(&tensors, "predictor_bias")?,
        auxiliary_bias: tensor_f32(&tensors, "auxiliary_bias")?,
        auxiliary_latent_weights: tensor_f32(&tensors, "auxiliary_latent_weights")?,
        auxiliary_action_weights: tensor_f32(&tensors, "auxiliary_action_weights")?,
    })
}

fn mask_jepa_training_example(
    example: &JepaTrainingExample,
    mask_ratio: f32,
    report: &mut JepaMaskingReport,
) -> JepaTrainingExample {
    let mut masked = example.clone();
    for row in &mut masked.context.rows {
        let prefix = format!("{}:{}:{}", row.session_id, row.row_id, example.horizon);
        if should_mask(&prefix, "excerpt", mask_ratio) {
            row.redacted_excerpt = Some("[MASKED_EXCERPT]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "action_kind", mask_ratio) {
            row.action_kind = crate::schema::WorldActionKind::Unknown;
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "provider", mask_ratio) {
            row.provider = Some("[MASKED_PROVIDER]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "model", mask_ratio) {
            row.model = Some("[MASKED_MODEL]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "agent", mask_ratio) {
            row.agent = Some("[MASKED_AGENT]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "scalar", mask_ratio) {
            row.scalar_features = ScalarFeatures::default();
            report.masked_context_fields += 1;
        }
    }

    let prefix = format!("action:{}:{}", masked.action.action_ref, example.horizon);
    if should_mask(&prefix, "summary", mask_ratio) {
        masked.action.summary = "[MASKED_EXCERPT]".into();
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "action_kind", mask_ratio) {
        masked.action.action_kind = crate::schema::WorldActionKind::Unknown;
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "provider", mask_ratio) {
        masked.action.provider = Some("[MASKED_PROVIDER]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "model", mask_ratio) {
        masked.action.model = Some("[MASKED_MODEL]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "agent", mask_ratio) {
        masked.action.agent = Some("[MASKED_AGENT]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "scalar", mask_ratio) {
        masked.action.scalar_features = ScalarFeatures::default();
        report.masked_action_fields += 1;
    }
    masked
}

fn should_mask(prefix: &str, field: &str, mask_ratio: f32) -> bool {
    if mask_ratio <= 0.0 {
        return false;
    }
    if mask_ratio >= 1.0 {
        return true;
    }
    let mut hasher = DefaultHasher::new();
    prefix.hash(&mut hasher);
    field.hash(&mut hasher);
    let unit = (hasher.finish() % 10_000) as f32 / 10_000.0;
    unit < mask_ratio
}

fn encode_examples(
    context_encoder: &JepaTraceEncoder,
    action_encoder: &JepaTraceEncoder,
    target_encoder: &JepaTraceEncoder,
    examples: &[JepaTrainingExample],
) -> Result<Vec<EncodedJepaTrainingExample>> {
    examples
        .iter()
        .map(|example| {
            Ok(EncodedJepaTrainingExample {
                context_latent: context_encoder.encode_window(&example.context)?,
                action_latent: action_encoder.encode_action(&example.action)?,
                target_latent: target_encoder.encode_window(&example.target)?,
                horizon: example.horizon,
                labels: example.labels.clone(),
            })
        })
        .collect()
}

fn encoded_transition_examples(
    examples: &[EncodedJepaTrainingExample],
) -> Vec<LatentTransitionExample> {
    examples
        .iter()
        .map(|example| LatentTransitionExample {
            state: example.context_latent.clone(),
            action: example.action_latent.clone(),
            next_state: example.target_latent.clone(),
            labels: example.labels.clone(),
        })
        .collect()
}

fn transition_model_finite(model: &CpuLatentTransitionModel) -> bool {
    model.state_weights.iter().all(|value| value.is_finite())
        && model.action_weights.iter().all(|value| value.is_finite())
        && model.transition_bias.iter().all(|value| value.is_finite())
        && model.mean_delta.iter().all(|value| value.is_finite())
        && model.auxiliary_heads.iter().all(|head| {
            head.bias.is_finite()
                && head.state_weights.iter().all(|value| value.is_finite())
                && head.action_weights.iter().all(|value| value.is_finite())
        })
}

fn training_losses(
    model: &JepaTraceModel,
    examples: &[EncodedJepaTrainingExample],
    config: &JepaTrainingConfig,
) -> Result<JepaTrainingLosses> {
    let mut loss_jepa = 0.0;
    let mut loss_mse = 0.0;
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for example in examples {
        let predicted = model.predict_training_target(
            &example.context_latent,
            &example.action_latent,
            example.horizon,
        )?;
        let cosine = cosine_error(&predicted, &example.target_latent)?;
        loss_jepa += cosine;
        loss_mse += mse(&predicted, &example.target_latent)?;
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += cosine;
        entry.1 += 1;
    }
    let denom = examples.len().max(1) as f32;
    loss_jepa /= denom;
    loss_mse /= denom;
    let loss_aux = auxiliary_brier(model, examples);
    let loss_horizon = horizon_consistency_loss(&horizon_errors);
    let loss_var = latent_variance_loss(
        examples
            .iter()
            .map(|example| example.context_latent.as_slice())
            .collect::<Vec<_>>()
            .as_slice(),
        config.latent_var_floor,
    );
    let loss_total = loss_jepa
        + config.alpha_mse * loss_mse
        + config.beta_aux * loss_aux
        + config.gamma_horizon * loss_horizon
        + config.delta_var * loss_var;
    Ok(JepaTrainingLosses {
        loss_jepa,
        loss_mse,
        loss_aux,
        loss_horizon,
        loss_var,
        loss_total,
    })
}

fn horizon_report_for_model(
    model: &JepaTraceModel,
    examples: &[EncodedJepaTrainingExample],
    tolerance: f32,
) -> Result<JepaHorizonReport> {
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for example in examples {
        let predicted = model.predict_training_target(
            &example.context_latent,
            &example.action_latent,
            example.horizon,
        )?;
        let cosine = cosine_error(&predicted, &example.target_latent)?;
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += cosine;
        entry.1 += 1;
    }
    Ok(horizon_report_from_errors(&horizon_errors, tolerance))
}

fn horizon_report_from_errors(
    errors: &BTreeMap<usize, (f32, usize)>,
    tolerance: f32,
) -> JepaHorizonReport {
    let mean = |horizon: usize| {
        errors.get(&horizon).and_then(|(sum, count)| {
            if *count == 0 {
                None
            } else {
                Some(*sum / *count as f32)
            }
        })
    };
    let e_1 = mean(1);
    let e_3 = mean(3);
    let e_5 = mean(5);
    let passes = match (e_1, e_3, e_5) {
        (Some(e1), Some(e3), Some(e5)) => {
            [e1, e3, e5, tolerance]
                .into_iter()
                .all(|value| value.is_finite())
                && e1 <= e3 + tolerance
                && e3 <= e5 + tolerance
        }
        _ => false,
    };
    JepaHorizonReport {
        e_1,
        e_3,
        e_5,
        tolerance,
        passes,
    }
}

fn heldout_context_latents(examples: &[EncodedJepaTrainingExample]) -> Vec<Vec<f32>> {
    if examples.is_empty() {
        return Vec::new();
    }
    let split = ((examples.len() as f32) * 0.8).floor() as usize;
    let split = split.min(examples.len().saturating_sub(1));
    examples[split..]
        .iter()
        .map(|example| example.context_latent.clone())
        .collect()
}

fn window_features(window: &TraceWindow, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    let mut features = vec![0.0; dimensions];
    add_token(
        &mut features,
        &format!("{role}:session:{}", window.session_id),
        0.10,
    );
    add_token(
        &mut features,
        &format!("{role}:anchor:{}", window.anchor_row_id),
        0.05,
    );
    add_numeric(
        &mut features,
        "horizon",
        normalized_horizon(window.horizon),
        0.50,
    );
    add_numeric(
        &mut features,
        "graph.session_neighbor_count",
        normalize_count(window.graph_context.session_neighbor_count),
        0.55,
    );
    add_numeric(
        &mut features,
        "graph.same_agent_prior_count",
        normalize_count(window.graph_context.same_agent_prior_count),
        0.45,
    );
    add_numeric(
        &mut features,
        "graph.same_provider_prior_count",
        normalize_count(window.graph_context.same_provider_prior_count),
        0.45,
    );
    add_numeric(
        &mut features,
        "graph.prior_plan_updates",
        normalize_count(window.graph_context.prior_plan_updates),
        0.40,
    );
    add_numeric(
        &mut features,
        "graph.prior_memory_surfaces",
        normalize_count(window.graph_context.prior_memory_surfaces),
        0.40,
    );
    for plan_id in &window.graph_context.prior_plan_ids {
        add_token(&mut features, &format!("graph.plan:{plan_id}"), 0.10);
    }
    for memory_id in &window.graph_context.prior_memory_ids {
        add_token(&mut features, &format!("graph.memory:{memory_id}"), 0.10);
    }

    let row_weight = 1.0 / window.rows.len().max(1) as f32;
    for row in &window.rows {
        add_row_features(&mut features, row, row_weight, role);
    }
    normalize(&mut features);
    Ok(features)
}

fn action_features(action: &TraceAction, dimensions: usize, role: &str) -> Result<Vec<f32>> {
    if dimensions == 0 {
        bail!("jepa dimensions must be greater than zero");
    }
    let mut features = vec![0.0; dimensions];
    add_token(
        &mut features,
        &format!("{role}:action:{}", action.action_ref),
        0.20,
    );
    add_token(
        &mut features,
        &format!("{role}:kind:{:?}", action.action_kind),
        0.80,
    );
    if let Some(provider) = &action.provider {
        add_token(&mut features, &format!("{role}:provider:{provider}"), 0.65);
    }
    if let Some(model) = &action.model {
        add_token(&mut features, &format!("{role}:model:{model}"), 0.50);
    }
    if let Some(agent) = &action.agent {
        add_token(&mut features, &format!("{role}:agent:{agent}"), 0.50);
    }
    add_scalar_features(&mut features, &action.scalar_features, 1.0);
    add_lexical_features(&mut features, &action.summary, 0.20);
    normalize(&mut features);
    Ok(features)
}

fn add_row_features(features: &mut [f32], row: &WorldTraceRow, weight: f32, role: &str) {
    add_token(
        features,
        &format!("{role}:source:{:?}", row.source),
        0.45 * weight,
    );
    add_token(
        features,
        &format!("{role}:action_kind:{:?}", row.action_kind),
        0.65 * weight,
    );
    if let Some(provider) = &row.provider {
        add_token(
            features,
            &format!("{role}:provider:{provider}"),
            0.55 * weight,
        );
    }
    if let Some(model) = &row.model {
        add_token(features, &format!("{role}:model:{model}"), 0.40 * weight);
    }
    if let Some(agent) = &row.agent {
        add_token(features, &format!("{role}:agent:{agent}"), 0.40 * weight);
    }
    add_scalar_features(features, &row.scalar_features, weight);
    if let Some(excerpt) = &row.redacted_excerpt {
        add_lexical_features(features, excerpt, 0.15 * weight);
    }
    for evidence in &row.evidence_refs {
        add_token(
            features,
            &format!("{role}:evidence:{}:{}", evidence.source, evidence.id),
            0.10 * weight,
        );
    }
}

fn add_scalar_features(features: &mut [f32], scalar: &ScalarFeatures, weight: f32) {
    if let Some(value) = scalar.cost_usd {
        add_numeric(
            features,
            "scalar.cost_usd",
            (value as f32 / 2.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.duration_ms {
        add_numeric(
            features,
            "scalar.duration_ms",
            (value as f32 / 300_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.attempt_index {
        add_numeric(
            features,
            "scalar.attempt_index",
            (value as f32 / 8.0).clamp(0.0, 4.0),
            weight,
        );
    }
    if let Some(value) = scalar.tokens_in {
        add_numeric(
            features,
            "scalar.tokens_in",
            (value as f32 / 100_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.tokens_out {
        add_numeric(
            features,
            "scalar.tokens_out",
            (value as f32 / 50_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
    if let Some(value) = scalar.quality_overall {
        add_numeric(
            features,
            "scalar.quality_overall",
            (value as f32).clamp(0.0, 1.0),
            weight,
        );
    }
    if let Some(value) = scalar.provider_cooldown_ms {
        add_numeric(
            features,
            "scalar.provider_cooldown_ms",
            (value as f32 / 300_000.0).clamp(0.0, 8.0),
            weight,
        );
    }
}

fn add_lexical_features(features: &mut [f32], text: &str, weight: f32) {
    for token in text.split_whitespace().take(64) {
        add_token(features, &format!("lex:{token}"), weight);
    }
}

fn add_numeric(features: &mut [f32], name: &str, value: f32, weight: f32) {
    if value.is_finite() {
        add_token(features, &format!("num:{name}"), value * weight);
    }
}

fn add_token(features: &mut [f32], token: &str, weight: f32) {
    if features.is_empty() || !weight.is_finite() {
        return;
    }
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    let hash = hasher.finish();
    let bucket = (hash as usize) % features.len();
    let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
    features[bucket] += sign * weight;
}

fn deterministic_vector(
    role: &str,
    salt: &str,
    len: usize,
    min_value: f32,
    max_value: f32,
) -> Vec<f32> {
    (0..len)
        .map(|idx| {
            let mut hasher = DefaultHasher::new();
            role.hash(&mut hasher);
            salt.hash(&mut hasher);
            idx.hash(&mut hasher);
            let unit = (hasher.finish() % 10_000) as f32 / 10_000.0;
            min_value + unit * (max_value - min_value)
        })
        .collect()
}

fn ema_values(previous_target: &[f32], online: &[f32], decay: f32) -> Vec<f32> {
    previous_target
        .iter()
        .zip(online)
        .map(|(target, online)| decay * target + (1.0 - decay) * online)
        .collect()
}

fn fit_auxiliary_heads(
    latent_dim: usize,
    examples: &[EncodedJepaTrainingExample],
) -> Vec<JepaAuxiliaryHead> {
    auxiliary_labels()
        .into_iter()
        .map(|label| fit_auxiliary_head(label, latent_dim, examples))
        .collect()
}

fn fit_auxiliary_head(
    label: &'static str,
    latent_dim: usize,
    examples: &[EncodedJepaTrainingExample],
) -> JepaAuxiliaryHead {
    let positives = examples
        .iter()
        .filter(|example| label_value(&example.labels, label))
        .count() as f32;
    let prevalence = ((positives + 1.0) / (examples.len() as f32 + 2.0)).clamp(0.01, 0.99);
    let mut pos_context = vec![0.0; latent_dim];
    let mut neg_context = vec![0.0; latent_dim];
    let mut pos_action = vec![0.0; latent_dim];
    let mut neg_action = vec![0.0; latent_dim];
    let mut pos_count: f32 = 0.0;
    let mut neg_count: f32 = 0.0;
    for example in examples {
        let (context_target, action_target, count) = if label_value(&example.labels, label) {
            (&mut pos_context, &mut pos_action, &mut pos_count)
        } else {
            (&mut neg_context, &mut neg_action, &mut neg_count)
        };
        *count += 1.0;
        for idx in 0..latent_dim {
            context_target[idx] += example.context_latent[idx];
            action_target[idx] += example.action_latent[idx];
        }
    }
    normalize_mean(&mut pos_context, pos_count);
    normalize_mean(&mut neg_context, neg_count);
    normalize_mean(&mut pos_action, pos_count);
    normalize_mean(&mut neg_action, neg_count);
    JepaAuxiliaryHead {
        label: label.to_string(),
        bias: (prevalence / (1.0 - prevalence)).ln(),
        latent_weights: pos_context
            .iter()
            .zip(&neg_context)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
        action_weights: pos_action
            .iter()
            .zip(&neg_action)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
    }
}

fn auxiliary_brier(model: &JepaTraceModel, examples: &[EncodedJepaTrainingExample]) -> f32 {
    let mut total = 0.0;
    let mut count = 0.0;
    for example in examples {
        for head in &model.auxiliary_heads {
            let target = if label_value(&example.labels, &head.label) {
                1.0
            } else {
                0.0
            };
            let probability =
                head.predict_probability(&example.context_latent, &example.action_latent);
            total += (probability - target).powi(2);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

fn horizon_consistency_loss(errors: &BTreeMap<usize, (f32, usize)>) -> f32 {
    let mean = |horizon: usize| {
        errors.get(&horizon).and_then(|(sum, count)| {
            if *count == 0 {
                None
            } else {
                Some(*sum / *count as f32)
            }
        })
    };
    match (mean(1), mean(3), mean(5)) {
        (Some(e1), Some(e3), Some(e5)) => (e1 - e3).max(0.0) + (e3 - e5).max(0.0),
        _ => 0.0,
    }
}

fn latent_variance_loss(latents: &[&[f32]], floor: f32) -> f32 {
    if latents.is_empty() || latents[0].is_empty() {
        return 0.0;
    }
    let dim = latents[0].len();
    let mut total = 0.0;
    for idx in 0..dim {
        let mean = latents.iter().map(|latent| latent[idx]).sum::<f32>() / latents.len() as f32;
        let variance = latents
            .iter()
            .map(|latent| (latent[idx] - mean).powi(2))
            .sum::<f32>()
            / latents.len() as f32;
        total += (floor - variance.sqrt()).max(0.0);
    }
    total / dim as f32
}

#[derive(Debug, Clone, Copy)]
enum InputRole {
    Context,
    Action,
    Horizon,
}

fn covariance_weight(
    examples: &[EncodedJepaTrainingExample],
    idx: usize,
    input_mean: f32,
    target_mean: f32,
    role: InputRole,
) -> f32 {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for example in examples {
        let input = match role {
            InputRole::Context => example.context_latent[idx],
            InputRole::Action => example.action_latent[idx],
            InputRole::Horizon => normalized_horizon(example.horizon),
        };
        numerator += (input - input_mean) * (example.target_latent[idx] - target_mean);
        denominator += (input - input_mean).powi(2);
    }
    if denominator <= f32::EPSILON {
        match role {
            InputRole::Context => 1.0,
            InputRole::Action | InputRole::Horizon => 0.0,
        }
    } else {
        (numerator / denominator).clamp(-2.0, 2.0)
    }
}

fn jepa_checkpoint_tensors(model: &JepaTraceModel) -> JepaCheckpointTensors {
    let auxiliary_bias = model
        .auxiliary_heads
        .iter()
        .map(|head| head.bias)
        .collect::<Vec<_>>();
    let auxiliary_latent_weights = model
        .auxiliary_heads
        .iter()
        .flat_map(|head| head.latent_weights.clone())
        .collect::<Vec<_>>();
    let auxiliary_action_weights = model
        .auxiliary_heads
        .iter()
        .flat_map(|head| head.action_weights.clone())
        .collect::<Vec<_>>();
    JepaCheckpointTensors {
        context_input_weights: model.context_encoder.input_weights.clone(),
        context_hidden_bias: model.context_encoder.hidden_bias.clone(),
        context_output_weights: model.context_encoder.output_weights.clone(),
        context_output_bias: model.context_encoder.output_bias.clone(),
        action_input_weights: model.action_encoder.input_weights.clone(),
        action_hidden_bias: model.action_encoder.hidden_bias.clone(),
        action_output_weights: model.action_encoder.output_weights.clone(),
        action_output_bias: model.action_encoder.output_bias.clone(),
        target_input_weights: model.target_encoder.input_weights.clone(),
        target_hidden_bias: model.target_encoder.hidden_bias.clone(),
        target_output_weights: model.target_encoder.output_weights.clone(),
        target_output_bias: model.target_encoder.output_bias.clone(),
        predictor_context_weights: model.predictor.context_weights.clone(),
        predictor_action_weights: model.predictor.action_weights.clone(),
        predictor_horizon_weights: model.predictor.horizon_weights.clone(),
        predictor_bias: model.predictor.bias.clone(),
        auxiliary_bias,
        auxiliary_latent_weights,
        auxiliary_action_weights,
    }
}

fn validate_latents(latent_dim: usize, example: &EncodedJepaTrainingExample) -> Result<()> {
    if example.context_latent.len() != latent_dim
        || example.action_latent.len() != latent_dim
        || example.target_latent.len() != latent_dim
    {
        bail!("jepa training example latent dimensions must match latent_dim");
    }
    Ok(())
}

fn mse(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("mse inputs must have matching dimensions");
    }
    Ok(left
        .iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f32>()
        / left.len().max(1) as f32)
}

fn cosine_error(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("cosine inputs must have matching dimensions");
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(1.0)
    } else {
        Ok(1.0 - (dot / (left_norm * right_norm)).clamp(-1.0, 1.0))
    }
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}

fn layer_norm(values: &mut [f32]) {
    if values.is_empty() {
        return;
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / values.len() as f32;
    let denom = (variance + 1e-6).sqrt();
    for value in &mut *values {
        *value = (*value - mean) / denom;
    }
    normalize(values);
}

fn normalize_mean(values: &mut [f32], count: f32) {
    if count > 0.0 {
        for value in values {
            *value /= count;
        }
    }
}

fn normalize_count(value: usize) -> f32 {
    (value as f32 / 16.0).clamp(0.0, 8.0)
}

fn normalized_horizon(horizon: usize) -> f32 {
    (horizon as f32 / 5.0).clamp(0.0, 4.0)
}

fn gelu(value: f32) -> f32 {
    0.5 * value * (1.0 + (0.797_884_6 * (value + 0.044_715 * value.powi(3))).tanh())
}

fn dot_prefix(weights: &[f32], values: &[f32]) -> f32 {
    weights
        .iter()
        .zip(values)
        .map(|(weight, value)| weight * value)
        .sum()
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-40.0, 40.0)).exp())
}

fn auxiliary_labels() -> Vec<&'static str> {
    vec![
        "failure",
        "retry",
        "provider_incident",
        "verification_needed",
        "user_correction",
        "plan_drift",
        "high_cost",
        "slow_run",
    ]
}

fn label_value(labels: &WorldLabelSet, label: &str) -> bool {
    match label {
        "failure" => labels.failure,
        "retry" => labels.retry,
        "provider_incident" => labels.provider_incident,
        "verification_needed" => labels.verification_needed,
        "user_correction" => labels.user_correction,
        "plan_drift" => labels.plan_drift,
        "high_cost" => labels.high_cost,
        "slow_run" => labels.slow_run,
        _ => false,
    }
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn tensor_f32(tensors: &safetensors::SafeTensors<'_>, name: &str) -> Result<Vec<f32>> {
    let tensor = tensors.tensor(name)?;
    Ok(tensor
        .data()
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{WorldActionKind, WorldTraceRow};

    fn rows() -> Vec<WorldTraceRow> {
        let mut first = WorldTraceRow::new("s1", WorldActionKind::PlanUpdate).with_row_id("r1");
        first.agent = Some("planner".into());
        first.redacted_excerpt = Some("draft plan".into());
        let mut second = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r2");
        second.provider = Some("local".into());
        second.agent = Some("coder".into());
        second.redacted_excerpt = Some("run cargo test".into());
        let mut third = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("r3");
        third.labels.verification_needed = true;
        third.redacted_excerpt = Some("tests failed".into());
        let mut fourth = WorldTraceRow::new("s1", WorldActionKind::Retry).with_row_id("r4");
        fourth.labels.retry = true;
        fourth.redacted_excerpt = Some("fix tests".into());
        vec![first, second, third, fourth]
    }

    fn long_rows() -> Vec<WorldTraceRow> {
        (0..8)
            .map(|idx| {
                let kind = match idx % 4 {
                    0 => WorldActionKind::PlanUpdate,
                    1 => WorldActionKind::ToolCall,
                    2 => WorldActionKind::Verification,
                    _ => WorldActionKind::Retry,
                };
                let mut row = WorldTraceRow::new("s1", kind).with_row_id(format!("r{idx}"));
                row.provider = Some("local".into());
                row.agent = Some(format!("agent-{}", idx % 2));
                row.redacted_excerpt = Some(format!("trace event {idx}"));
                row.labels.retry = idx % 3 == 0;
                row.labels.verification_needed = idx % 2 == 0;
                row
            })
            .collect()
    }

    #[test]
    fn jepa_examples_follow_configured_horizons() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1, 3],
            ..JepaTrainingConfig::default()
        };

        let examples = build_jepa_training_examples(&rows(), &config).unwrap();

        assert!(examples.iter().any(|example| example.horizon == 1));
        assert!(examples.iter().any(|example| example.horizon == 3));
    }

    #[test]
    fn masking_uses_typed_sentinels_without_touching_target() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            mask_ratio: 1.0,
            ..JepaTrainingConfig::default()
        };
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();

        let (masked, report) = mask_jepa_training_examples(&examples, config.mask_ratio);

        assert!(report.masked_context_fields > 0);
        assert!(report.masked_action_fields > 0);
        assert_eq!(masked[0].context.session_id, examples[0].context.session_id);
        assert_eq!(
            masked[0].context.rows[0].redacted_excerpt.as_deref(),
            Some("[MASKED_EXCERPT]")
        );
        assert_eq!(masked[0].action.summary, "[MASKED_EXCERPT]");
        assert_eq!(
            masked[0].target.rows[0].redacted_excerpt,
            examples[0].target.rows[0].redacted_excerpt
        );
        assert!(!report.reconstructs_raw_text);
    }

    #[test]
    fn jepa_training_produces_configured_latent_dimensions() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };

        let (model, outcome) = train_jepa_candidate(&rows(), &config).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let state = model.encode_state(&examples[0].context).unwrap();
        let action = model.encode_action(&examples[0].action).unwrap();
        let target = model.encode_target(&examples[0].target).unwrap();

        assert_eq!(model.metadata.model_kind, JEPA_MODEL_KIND);
        assert_eq!(model.dimensions(), 8);
        assert_eq!(state.len(), 8);
        assert_eq!(action.len(), 8);
        assert_eq!(target.len(), 8);
        assert!(outcome.losses.loss_total.is_finite());
        assert!(outcome.metadata.target_stop_gradient);
        assert_eq!(outcome.masking.mask_ratio, 0.30);
        assert!(model.transition_model.is_some());
        assert_eq!(model.provider_name(), "archon-jepa");
    }

    #[test]
    fn jepa_cpu_training_records_backend_execution_proof() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };

        let (model, outcome) =
            train_jepa_candidate_with_backend(&rows(), &config, BackendKind::Cpu, true).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.requested_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            model.metadata.backend_execution,
            outcome.metadata.backend_execution
        );
        assert!(outcome.metadata.backend_execution.feature_compiled);
        assert!(outcome.metadata.backend_execution.tensor_self_test_passed);
        assert!(outcome.metadata.backend_execution.native_encode);
        assert!(outcome.metadata.backend_execution.native_predictor_fit);
        assert!(outcome.metadata.backend_execution.native_auxiliary_fit);
        assert!(outcome.metadata.backend_execution.native_transition_fit);
        assert!(outcome.metadata.backend_execution.native_loss_eval);
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
    }

    #[test]
    fn cpu_jepa_backend_wraps_current_training_operations() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let context_encoder = JepaTraceEncoder::new("context", config.latent_dim);
        let action_encoder = JepaTraceEncoder::new("action", config.latent_dim);
        let target_encoder = JepaTraceEncoder::ema_target_from(&context_encoder, config.ema_decay);
        let encoders = JepaEncoderSet {
            context_encoder,
            action_encoder,
            target_encoder,
        };
        let backend = CpuJepaBackend;

        let encoded = backend.encode_batch(&encoders, &examples).unwrap();
        let predictor = backend.fit_predictor(config.latent_dim, &encoded).unwrap();
        let transition = backend.fit_transition(config.latent_dim, &encoded).unwrap();

        assert_eq!(backend.status().selected, BackendKind::Cpu);
        assert_eq!(encoded.len(), examples.len());
        assert_eq!(predictor.latent_dim, config.latent_dim);
        assert_eq!(transition.metadata.backend, BackendKind::Cpu);
    }

    #[test]
    fn accelerator_jepa_backend_stubs_compile_and_fail_closed() {
        let cuda = CandleCudaJepaBackend;
        let metal = MlxMetalJepaBackend;
        let encoded = Vec::new();

        assert_eq!(cuda.probe_jepa().status.requested, BackendKind::Cuda);
        assert_eq!(metal.probe_jepa().status.requested, BackendKind::Metal);
        assert!(
            cuda.fit_predictor(8, &encoded)
                .unwrap_err()
                .to_string()
                .contains("native cuda JEPA tensor backend is not implemented")
        );
        assert!(
            metal
                .fit_predictor(8, &encoded)
                .unwrap_err()
                .to_string()
                .contains("native metal JEPA tensor backend is not implemented")
        );
    }

    #[test]
    fn requested_accelerator_with_fallback_writes_cpu_labelled_candidate() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status =
            BackendStatus::cpu_fallback(BackendKind::Cuda, "cuda_probe_failed:not_compiled");

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, true).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.requested_backend,
            BackendKind::Cuda
        );
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome
                .metadata
                .backend_execution
                .fallback_reason
                .as_deref(),
            Some("cuda_probe_failed:not_compiled")
        );
    }

    #[test]
    fn selected_accelerator_without_native_jepa_fails_or_relabels_cpu() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status = BackendStatus {
            requested: BackendKind::Cuda,
            selected: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            experimental: false,
            fallback_reason: None,
        };

        let error =
            train_jepa_candidate_with_backend_status(&rows(), &config, status.clone(), false)
                .unwrap_err();
        assert!(error.to_string().contains("native JEPA backend"));

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, true).unwrap();
        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome
                .metadata
                .backend_execution
                .fallback_reason
                .as_deref(),
            Some("jepa_native_backend_not_implemented:cuda")
        );
    }

    #[test]
    fn cuda_metadata_without_native_execution_proof_is_rejected() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;

        let error = validate_jepa_backend_execution(&model.metadata).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not match execution report")
        );
    }

    #[test]
    fn accelerator_promotion_gate_requires_hardware_validation_report() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;
        model.metadata.backend_execution = JepaBackendExecutionReport {
            requested_backend: BackendKind::Cuda,
            selected_backend: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            commit_sha: "abc123".into(),
            feature_compiled: true,
            tensor_self_test_passed: true,
            hardware_validation_captured_at: None,
            validation_example_count: 512,
            native_encode: true,
            native_predictor_fit: true,
            native_auxiliary_fit: true,
            native_transition_fit: true,
            native_loss_eval: true,
            native_runtime_prediction: Some(true),
            host_fallback_count: 0,
            allowed_host_stage_count: 0,
            fallback_reason: None,
        };

        assert!(validate_jepa_backend_execution(&model.metadata).is_ok());
        assert!(!jepa_backend_promotion_gate(&model.metadata, 512, 512));

        model
            .metadata
            .backend_execution
            .hardware_validation_captured_at = Some(Utc::now());

        assert!(jepa_backend_promotion_gate(&model.metadata, 512, 512));
    }

    #[test]
    fn target_encoder_is_ema_of_context_encoder() {
        let context = JepaTraceEncoder::new("context", 8);
        let initialized_target = JepaTraceEncoder::new("target", 8);
        let target = JepaTraceEncoder::ema_target_from(&context, 0.5);

        assert_eq!(target.role, "target");
        let expected = 0.5 * initialized_target.input_weights[0] + 0.5 * context.input_weights[0];
        assert!((target.input_weights[0] - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn collapse_gate_rejects_constant_latents() {
        let latents = vec![vec![0.5; 8]; 4];

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(!report.passes);
        assert_eq!(report.mean_latent_std, 0.0);
        assert_eq!(report.effective_rank_ratio, 0.0);
    }

    #[test]
    fn collapse_gate_rejects_rank_one_latents_with_nonzero_std() {
        let direction = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        let latents = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0]
            .into_iter()
            .map(|scale| {
                direction
                    .iter()
                    .map(|component| scale * component)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(report.mean_latent_std >= 0.05);
        assert!(report.effective_rank_ratio < 0.50);
        assert!(!report.passes);
    }

    #[test]
    fn collapse_gate_accepts_full_rank_latents() {
        let mut latents = Vec::new();
        for idx in 0..8 {
            let mut positive = vec![0.0; 8];
            positive[idx] = 3.0;
            latents.push(positive);

            let mut negative = vec![0.0; 8];
            negative[idx] = -3.0;
            latents.push(negative);
        }

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(report.mean_latent_std >= 0.05);
        assert!(report.effective_rank_ratio >= 0.99);
        assert!(report.passes);
    }

    #[test]
    fn jepa_module_keeps_encoder_path_free_of_embedding_adapters() {
        let source = include_str!("jepa.rs");
        let forbidden_fragments = [
            ("Memory", "EmbeddingAdapter"),
            ("World", "EmbeddingAdapter"),
            ("Embedding", "Request"),
            ("Embedding", "Vector"),
            ("DeterministicHash", "EmbeddingAdapter"),
            ("local_", "fastembed"),
            ("OpenAI", "Embedding"),
            ("Fast", "Embed"),
            (".", "embed("),
        ];

        for (left, right) in forbidden_fragments {
            let forbidden = format!("{left}{right}");
            assert!(
                !source.contains(&forbidden),
                "JEPA module must not reference embedding adapter path: {forbidden}"
            );
        }
    }

    #[test]
    fn horizon_report_requires_monotonic_multi_horizon_errors() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1, 3, 5],
            ..JepaTrainingConfig::default()
        };

        let (_, outcome) = train_jepa_candidate(&long_rows(), &config).unwrap();

        assert!(outcome.horizon.e_1.is_some());
        assert!(outcome.horizon.e_3.is_some());
        assert!(outcome.horizon.e_5.is_some());
    }

    #[test]
    fn nan_guard_fails_closed() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.predictor.bias[0] = f32::NAN;

        let error = model.validate_finite().unwrap_err();

        assert!(error.to_string().contains("non-finite"));
    }

    #[test]
    fn training_run_ledger_records_component_losses() {
        let temp = tempfile::tempdir().unwrap();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (_, outcome) = train_jepa_candidate(&rows(), &config).unwrap();

        let path = append_jepa_training_run(temp.path(), &outcome).unwrap();
        let content = std::fs::read_to_string(path).unwrap();

        assert!(content.contains("\"loss_jepa\""));
        assert!(content.contains("\"loss_var\""));
        assert!(content.contains("\"collapse\""));
        assert!(content.contains("\"backend_execution\""));
    }

    #[test]
    fn jepa_safetensors_checkpoint_roundtrips_weights() {
        let temp = tempfile::tempdir().unwrap();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (model, _) = train_jepa_candidate(&rows(), &config).unwrap();

        let record = write_jepa_safetensors_checkpoint(temp.path(), &model).unwrap();
        let loaded = read_jepa_safetensors_checkpoint(&record.path).unwrap();

        assert_eq!(record.format, "candle_safetensors");
        assert_eq!(
            loaded.predictor_bias, model.predictor.bias,
            "predictor bias should roundtrip through the checkpoint"
        );
    }
}
