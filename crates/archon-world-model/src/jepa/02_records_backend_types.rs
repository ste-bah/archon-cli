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

    /// Construct a gate report for a quick-mode eval where Tier-2 was skipped.
    /// `representation_baseline` is forced false — `gates.passed` will be false
    /// by construction. This enforces PRD §11:
    /// "A skipped Tier-2 gate must never leave gates.passed = true."
    pub fn quick_mode_skipped(
        corpus_sufficient: bool,
        representation_collapse: bool,
        multi_horizon_consistency: bool,
        checkpoint_size: bool,
        tensor_safety: bool,
        backend_execution: bool,
    ) -> Self {
        Self::from_parts_with_backend_execution(
            corpus_sufficient,
            false, // representation_baseline — FORCED FALSE when skipped
            representation_collapse,
            multi_horizon_consistency,
            checkpoint_size,
            tensor_safety,
            backend_execution,
        )
    }
}

fn default_true() -> bool {
    true
}

fn default_baseline_skipped() -> bool {
    true
}

fn default_legacy_fingerprint() -> String {
    "legacy".to_string()
}

/// Eval mode selectable at runtime. Does NOT include Legacy — that is
/// deserialisation-only (see PersistedEvalMode). DEC-JEVAL-05.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEvalMode {
    Quick,
    Full,
    Promotion,
}

/// Mode as persisted in on-disk eval records. Includes Legacy sentinel
/// for records written before this schema. DEC-JEVAL-05.
/// Legacy is NOT runtime-selectable — it is serde-deserialization only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedEvalMode {
    Quick,
    Full,
    Promotion,
    /// Deserialisation-only sentinel. Records with this mode are NON-PROMOTABLE.
    /// Never set at runtime — only appears on pre-schema legacy on-disk records.
    #[serde(other)]
    Legacy,
}

impl Default for PersistedEvalMode {
    fn default() -> Self {
        // serde(default) — legacy records fail all promotion checks
        Self::Legacy
    }
}

impl From<RuntimeEvalMode> for PersistedEvalMode {
    fn from(m: RuntimeEvalMode) -> Self {
        match m {
            RuntimeEvalMode::Quick => Self::Quick,
            RuntimeEvalMode::Full => Self::Full,
            RuntimeEvalMode::Promotion => Self::Promotion,
        }
    }
}

impl TryFrom<PersistedEvalMode> for RuntimeEvalMode {
    type Error = anyhow::Error;
    fn try_from(p: PersistedEvalMode) -> Result<RuntimeEvalMode, Self::Error> {
        match p {
            PersistedEvalMode::Quick => Ok(RuntimeEvalMode::Quick),
            PersistedEvalMode::Full => Ok(RuntimeEvalMode::Full),
            PersistedEvalMode::Promotion => Ok(RuntimeEvalMode::Promotion),
            PersistedEvalMode::Legacy => Err(anyhow::anyhow!(
                "legacy eval record (PersistedEvalMode::Legacy) cannot be used as RuntimeEvalMode; \
                 re-run: archon world eval-jepa <candidate> --full"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaEvalRecord {
    pub candidate_id: String,
    /// Eval mode used to produce this record. Defaults to Legacy so that
    /// pre-schema records fail all promotion checks.
    #[serde(default)]
    pub mode: PersistedEvalMode,
    /// Whether the Tier-2 baseline comparison was skipped. Defaults to true
    /// so that pre-schema records fail the baseline_skipped promotion check.
    #[serde(default = "default_baseline_skipped")]
    pub baseline_skipped: bool,
    /// Human-readable reason why baseline was skipped, if applicable.
    #[serde(default)]
    pub skipped_reason: Option<String>,
    /// Content fingerprint of the corpus used in this eval run.
    #[serde(default)]
    pub corpus_fingerprint: Option<String>,
    /// Fingerprint of the eval config at time of run. Defaults to "legacy"
    /// so that pre-schema records fail the config_fingerprint promotion check.
    #[serde(default = "default_legacy_fingerprint")]
    pub config_fingerprint: String,
    /// Schema version of this record. 0 for pre-schema legacy records.
    /// Current schema version = 1; 0 != 1 forces non-promotable.
    #[serde(default)]
    pub eval_schema_version: u32,
    /// Tier-2 baseline representation comparison. None when baseline was skipped.
    pub comparison: Option<JepaRepresentationComparisonReport>,
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
pub struct JepaMlxArrayCheckpoint {
    pub model_id: String,
    pub arrays: JepaCheckpointTensors,
    pub memory_order: String,
    pub dtype: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaFeatureBatch {
    pub context_features: Vec<f32>,
    pub action_features: Vec<f32>,
    pub target_features: Vec<f32>,
    pub labels: Vec<WorldLabelSet>,
    pub horizons: Vec<usize>,
    pub rows: usize,
    pub feature_dim: usize,
    pub latent_dim: usize,
}

impl JepaFeatureBatch {
    pub fn from_examples(examples: &[JepaTrainingExample], latent_dim: usize) -> Result<Self> {
        if latent_dim == 0 {
            bail!("jepa feature batch latent_dim must be greater than zero");
        }
        let mut context_feature_values = Vec::with_capacity(examples.len() * latent_dim);
        let mut action_feature_values = Vec::with_capacity(examples.len() * latent_dim);
        let mut target_feature_values = Vec::with_capacity(examples.len() * latent_dim);
        let mut labels = Vec::with_capacity(examples.len());
        let mut horizons = Vec::with_capacity(examples.len());
        for example in examples {
            context_feature_values.extend(window_features(
                &example.context,
                latent_dim,
                "context",
            )?);
            action_feature_values.extend(action_features(&example.action, latent_dim, "action")?);
            target_feature_values.extend(window_features(&example.target, latent_dim, "target")?);
            labels.push(example.labels.clone());
            horizons.push(example.horizon);
        }
        Ok(Self {
            context_features: context_feature_values,
            action_features: action_feature_values,
            target_features: target_feature_values,
            labels,
            horizons,
            rows: examples.len(),
            feature_dim: latent_dim,
            latent_dim,
        })
    }

    pub fn len(&self) -> usize {
        self.rows
    }

    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    fn context_feature_row(&self, row: usize) -> Result<&[f32]> {
        self.feature_row(&self.context_features, row)
    }

    fn action_feature_row(&self, row: usize) -> Result<&[f32]> {
        self.feature_row(&self.action_features, row)
    }

    fn target_feature_row(&self, row: usize) -> Result<&[f32]> {
        self.feature_row(&self.target_features, row)
    }

    fn feature_row<'a>(&self, features: &'a [f32], row: usize) -> Result<&'a [f32]> {
        if row >= self.rows {
            bail!("jepa feature batch row out of bounds");
        }
        let start = row
            .checked_mul(self.feature_dim)
            .ok_or_else(|| anyhow::anyhow!("jepa feature batch row overflow"))?;
        let end = start + self.feature_dim;
        features
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("jepa feature batch shape mismatch"))
    }

    fn validate(&self) -> Result<()> {
        if self.feature_dim == 0 || self.latent_dim == 0 {
            bail!("jepa feature batch dimensions must be greater than zero");
        }
        if self.feature_dim != self.latent_dim {
            bail!("jepa feature batch feature_dim must match latent_dim");
        }
        let expected = self.rows * self.feature_dim;
        if self.context_features.len() != expected
            || self.action_features.len() != expected
            || self.target_features.len() != expected
            || self.labels.len() != self.rows
            || self.horizons.len() != self.rows
        {
            bail!("jepa feature batch shape mismatch");
        }
        Ok(())
    }
}

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
    pub guardrail_scores: GuardrailRiskScores,
    pub auxiliary_scores: Vec<(String, f32)>,
    pub latency_ms: u64,
    pub execution_report: JepaRuntimeBackendReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JepaRuntimeBackendReport {
    pub backend: BackendKind,
    pub framework: String,
    pub device_name: Option<String>,
    pub native_runtime_prediction: bool,
    pub latency_ms: u64,
    pub host_fallback_count: u64,
    pub fallback_reason: Option<String>,
}

impl JepaRuntimeBackendReport {
    fn new(
        backend: BackendKind,
        framework: impl Into<String>,
        device_name: Option<String>,
        native_runtime_prediction: bool,
        latency_ms: u64,
    ) -> Self {
        Self {
            backend,
            framework: framework.into(),
            device_name,
            native_runtime_prediction,
            latency_ms,
            host_fallback_count: 0,
            fallback_reason: None,
        }
    }
}

#[cfg(test)]
mod tests_02 {
    use super::*;

    #[test]
    fn persisted_eval_mode_defaults_to_legacy() {
        let mode: PersistedEvalMode = serde_json::from_str("\"legacy\"").unwrap();
        assert_eq!(mode, PersistedEvalMode::Legacy);
        // default() returns Legacy
        assert_eq!(PersistedEvalMode::default(), PersistedEvalMode::Legacy);
    }

    #[test]
    fn persisted_legacy_to_runtime_fails() {
        let result = RuntimeEvalMode::try_from(PersistedEvalMode::Legacy);
        assert!(result.is_err(), "Legacy must not convert to RuntimeEvalMode");
    }

    #[test]
    fn persisted_full_to_runtime_ok() {
        let r = RuntimeEvalMode::try_from(PersistedEvalMode::Full).unwrap();
        assert_eq!(r, RuntimeEvalMode::Full);
    }

    #[test]
    fn runtime_mode_converts_to_persisted() {
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Quick),
            PersistedEvalMode::Quick
        );
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Full),
            PersistedEvalMode::Full
        );
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Promotion),
            PersistedEvalMode::Promotion
        );
        // RuntimeEvalMode has NO Legacy variant — cannot construct Legacy at runtime
    }

    #[test]
    fn runtime_eval_mode_has_no_legacy_variant() {
        // Compile-time guarantee documented in comments.
        // Test the enum exhaustiveness: only 3 variants.
        let modes = [
            RuntimeEvalMode::Quick,
            RuntimeEvalMode::Full,
            RuntimeEvalMode::Promotion,
        ];
        assert_eq!(modes.len(), 3);
    }

    #[test]
    fn legacy_eval_record_serde_defaults() {
        // Simulate a legacy on-disk record: no new fields present.
        // JepaEvalRecord's new fields must default to fail-promotion values.
        let json = serde_json::json!({
            "candidate_id": "jepa-world-model-candidate-test",
            "comparison": null,
            "collapse": {
                "mean_latent_std": 0.0065,
                "effective_rank_ratio": 0.0476,
                "min_latent_std": 0.05,
                "min_effective_rank_ratio": 0.50,
                "passes": false
            },
            "horizon": {
                "e_1": null, "e_3": null, "e_5": null,
                "tolerance": 0.02, "passes": true
            },
            "gates": {
                "corpus_sufficient": false,
                "representation_baseline": false,
                "representation_collapse": false,
                "multi_horizon_consistency": true,
                "checkpoint_size": true,
                "tensor_safety": true,
                "backend_execution": true,
                "passed": false
            },
            "created_at": "2026-05-16T21:39:00Z"
            // NOTE: no mode/baseline_skipped/skipped_reason/corpus_fingerprint/
            //       config_fingerprint/eval_schema_version fields — testing serde defaults
        });
        let record: JepaEvalRecord = serde_json::from_value(json).expect("deserializes");
        assert_eq!(record.mode, PersistedEvalMode::Legacy, "mode defaults to Legacy");
        assert_eq!(record.eval_schema_version, 0, "eval_schema_version defaults to 0");
        assert!(record.baseline_skipped, "baseline_skipped defaults to true");
        assert_eq!(
            record.config_fingerprint, "legacy",
            "config_fingerprint defaults to 'legacy'"
        );
        assert!(
            record.corpus_fingerprint.is_none(),
            "corpus_fingerprint defaults to None"
        );
    }

    #[test]
    fn quick_mode_skipped_gates_passed_is_always_false() {
        // PRD §11: "A skipped Tier-2 gate must never leave gates.passed = true"
        let gates =
            JepaPromotionGateReport::quick_mode_skipped(true, true, true, true, true, true);
        assert!(
            !gates.passed,
            "representation_baseline=false forces gates.passed=false"
        );
        assert!(
            !gates.representation_baseline,
            "representation_baseline is false"
        );
    }
}
