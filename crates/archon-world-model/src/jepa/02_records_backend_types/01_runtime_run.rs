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

/// Which hardware backend produced the embeddings / ran the eval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JepaEvalBackendKind {
    MlxMetal,
    Cuda,
    Cpu,
}

/// Lifecycle status of a single eval run persisted under eval-runs/<run-id>.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Stale,
}

/// Which pipeline stage the run is currently executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStage {
    Tier0,
    Tier1,
    BaselineEmbed,
    TransitionFit,
    Report,
}

/// Full on-disk run record schema (PRD-006C §6.2).
/// Written atomically by JepaEvalRunStore (temp + rename).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JepaEvalRunRecord {
    pub run_id: String,
    pub candidate_id: String,
    pub corpus_fingerprint: Option<String>,
    /// Always None in 006C (training corpus separate from eval corpus).
    pub training_corpus_fingerprint: Option<String>,
    /// Quick | Full | Promotion — never Legacy (runtime selection only).
    pub mode: RuntimeEvalMode,
    pub backend: JepaEvalBackendKind,
    pub status: EvalRunStatus,
    pub current_stage: EvalRunStage,
    /// Wall-clock ms per stage name.
    pub stage_timings: std::collections::HashMap<String, u64>,
    pub baseline_skipped: bool,
    pub skipped_reason: Option<String>,
    pub pid: u32,
    pub host: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub rows_total: usize,
    /// F-CRIT-01: incremented at every progress_interval_rows boundary.
    pub rows_completed: usize,
    pub transitions_total: usize,
    /// F-CRIT-01: incremented at every progress_interval_rows boundary.
    pub transitions_completed: usize,
    pub embeddings_total: usize,
    pub embeddings_completed: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub backend_parity_examples: usize,
    pub failure_reason: Option<String>,
    pub partial_gates: serde_json::Value,
    pub result_paths: std::collections::HashMap<String, String>,
}
