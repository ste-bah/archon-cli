use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use safetensors::tensor::{Dtype, TensorView, serialize_to_file};
use serde::{Deserialize, Serialize};

use crate::backend::{BackendKind, BackendStatus};
use crate::guardrail::GuardrailRiskScores;
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
            || self.prediction_horizons.contains(&0)
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

    fn native(
        status: &BackendStatus,
        validation_example_count: usize,
        evidence: JepaBackendExecutionEvidence,
    ) -> Self {
        let feature_compiled = jepa_backend_feature_compiled(status.selected);
        let tensor_self_test_passed = evidence.tensor_self_test_passed;
        let hardware_validation_captured_at =
            (feature_compiled && tensor_self_test_passed && evidence.device_name.is_some())
                .then(Utc::now);
        let host_fallback_count = evidence.host_fallback_count();
        Self {
            requested_backend: status.requested,
            selected_backend: status.selected,
            framework: status.framework.clone(),
            device_name: evidence.device_name,
            commit_sha: build_commit_sha(),
            feature_compiled,
            tensor_self_test_passed,
            hardware_validation_captured_at,
            validation_example_count,
            native_encode: evidence.encode.native,
            native_predictor_fit: evidence.predictor_fit.native,
            native_auxiliary_fit: evidence.auxiliary_fit.native,
            native_transition_fit: evidence.transition_fit.native,
            native_loss_eval: evidence.loss_eval.native,
            native_runtime_prediction: evidence.runtime_prediction.map(|stage| stage.native),
            host_fallback_count,
            allowed_host_stage_count: 0,
            fallback_reason: None,
        }
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
            && self.native_runtime_prediction == Some(true)
            && self.host_fallback_count == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JepaStageExecution {
    pub native: bool,
    pub host_fallback_count: u64,
}

impl JepaStageExecution {
    pub fn native() -> Self {
        Self {
            native: true,
            host_fallback_count: 0,
        }
    }

    pub fn host_fallback(count: u64) -> Self {
        Self {
            native: false,
            host_fallback_count: count.max(1),
        }
    }

    fn combine(self, other: Self) -> Self {
        Self {
            native: self.native && other.native,
            host_fallback_count: self.host_fallback_count + other.host_fallback_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JepaStageResult<T> {
    pub value: T,
    pub execution: JepaStageExecution,
}

impl<T> JepaStageResult<T> {
    pub fn native(value: T) -> Self {
        Self {
            value,
            execution: JepaStageExecution::native(),
        }
    }

    pub fn new(value: T, execution: JepaStageExecution) -> Self {
        Self { value, execution }
    }
}

#[derive(Debug, Clone)]
struct JepaBackendExecutionEvidence {
    device_name: Option<String>,
    tensor_self_test_passed: bool,
    encode: JepaStageExecution,
    predictor_fit: JepaStageExecution,
    auxiliary_fit: JepaStageExecution,
    transition_fit: JepaStageExecution,
    loss_eval: JepaStageExecution,
    runtime_prediction: Option<JepaStageExecution>,
}

impl JepaBackendExecutionEvidence {
    fn host_fallback_count(&self) -> u64 {
        self.encode.host_fallback_count
            + self.predictor_fit.host_fallback_count
            + self.auxiliary_fit.host_fallback_count
            + self.transition_fit.host_fallback_count
            + self.loss_eval.host_fallback_count
            + self
                .runtime_prediction
                .map(|stage| stage.host_fallback_count)
                .unwrap_or_default()
    }
}

fn jepa_backend_feature_compiled(backend: BackendKind) -> bool {
    match backend {
        BackendKind::Cpu | BackendKind::Auto => true,
        BackendKind::Cuda => cfg!(feature = "cuda"),
        BackendKind::Metal => cfg!(all(
            feature = "mlx-metal",
            target_os = "macos",
            target_arch = "aarch64"
        )),
    }
}

fn build_commit_sha() -> String {
    option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .or(option_env!("SOURCE_VERSION"))
        .map(str::to_string)
        .or_else(runtime_git_sha)
        .unwrap_or_else(|| "unknown".to_string())
}

fn runtime_git_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

fn observed_backend_device_name(backend: BackendKind) -> Option<String> {
    match backend {
        BackendKind::Cpu => Some("cpu".into()),
        BackendKind::Cuda => {
            #[cfg(feature = "cuda")]
            {
                cuda_jepa_device().ok().map(|_| "cuda:0".to_string())
            }
            #[cfg(not(feature = "cuda"))]
            {
                None
            }
        }
        BackendKind::Metal => {
            #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
            {
                crate::backend::metal_runtime_available().then(|| "metal:0".to_string())
            }
            #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
            {
                None
            }
        }
        BackendKind::Auto => None,
    }
}

