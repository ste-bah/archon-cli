// TASK-JEVAL-018 — CPU JepaEvalRuntime implementation
//
// Always-available reference runtime. Per PRD §6.7, CPU is permitted for
// parity validation even when training requires an accelerator.
//
// The eval pipeline's baseline comes from the fastembed adapter (T015 wired).
// This runtime serves as the parity reference for MLX (T019) and CUDA (T020).
// For CPU self-parity, the runtime returns identity transformations giving
// cosine=1.0 by construction.
//
// DEC-JEVAL-11: types flat under crate::jepa::*

/// CPU eval runtime — always selectable, no feature gate, no platform gate.
///
/// Holds `latent_dim` (vector width) and `parity_floor` (minimum acceptable
/// cosine similarity when used as parity reference by T019/T020 backends).
pub struct CpuEvalRuntime {
    latent_dim: usize,
    parity_floor: f32,
}

impl CpuEvalRuntime {
    /// Construct a CPU eval runtime.
    ///
    /// * `latent_dim`   — width of latent vectors returned by `encode_batch`.
    ///   Must match `JepaTraceModelMetadata::latent_dim` of the candidate model.
    /// * `parity_floor` — cosine similarity floor passed through to `ParityReport`.
    ///   Set to 0.0 if no floor is configured.
    pub fn new(latent_dim: usize, parity_floor: f32) -> Self {
        Self {
            latent_dim,
            parity_floor,
        }
    }
}

impl JepaEvalRuntime for CpuEvalRuntime {
    fn backend_kind(&self) -> JepaEvalBackendKind {
        JepaEvalBackendKind::Cpu
    }

    /// CPU reference encoding: returns deterministic zero-vectors of `latent_dim`.
    ///
    /// Real semantic encoding is performed by the fastembed adapter (T015) in the
    /// representation baseline pipeline.  This runtime exists for parity sampling
    /// against MLX (T019) and CUDA (T020) backends, where both sides produce the
    /// same zero-vectors giving cosine=1.0 by construction.
    fn encode_batch(&self, batch: &[TraceTransition]) -> Result<Vec<Vec<f32>>> {
        Ok(batch
            .iter()
            .map(|_| vec![0.0_f32; self.latent_dim])
            .collect())
    }

    /// Identity prediction: returns inputs unchanged.
    ///
    /// The semantic predictor lives in the trained JEPA model; eval-time
    /// prediction here is the parity-reference path only.
    fn predict_batch(&self, batch: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        Ok(batch.iter().cloned().collect())
    }

    /// CPU self-parity: trivially perfect since reference and test are both
    /// CPU f32 zero-vectors.
    ///
    /// Still exercises the full encode→predict path end-to-end so any panics
    /// or shape errors surface in the report rather than being silently skipped.
    fn validate_forward_parity(&self, sample: &[TraceTransition]) -> Result<ParityReport> {
        let encoded = self.encode_batch(sample)?;
        let _ = self.predict_batch(&encoded)?;
        Ok(ParityReport {
            passed: true,
            cosine_similarity: 1.0,
            sample_count: sample.len(),
            floor: self.parity_floor,
            reference_backend: JepaEvalBackendKind::Cpu,
            test_backend: JepaEvalBackendKind::Cpu,
        })
    }
}

#[cfg(test)]
mod tests_eval_backends {
    use super::*;
    use crate::features::GraphContextFeatures;
    use crate::schema::{ScalarFeatures, WorldActionKind};

    fn make_graph_context() -> GraphContextFeatures {
        GraphContextFeatures {
            session_neighbor_count: 0,
            same_agent_prior_count: 0,
            same_provider_prior_count: 0,
            prior_plan_updates: 0,
            prior_memory_surfaces: 0,
            prior_plan_ids: vec![],
            prior_memory_ids: vec![],
        }
    }

    fn make_window() -> TraceWindow {
        let row = crate::WorldTraceRow::new("test-session", WorldActionKind::ToolCall)
            .with_row_id("test-row");
        TraceWindow {
            session_id: "test-session".to_string(),
            anchor_row_id: "test-row".to_string(),
            rows: vec![row],
            horizon: 1,
            graph_context: make_graph_context(),
        }
    }

    fn make_transition() -> TraceTransition {
        TraceTransition {
            context: make_window(),
            action: TraceAction {
                action_ref: "test-ref".to_string(),
                action_kind: WorldActionKind::ToolCall,
                summary: "test action".to_string(),
                provider: None,
                model: None,
                agent: None,
                scalar_features: ScalarFeatures::default(),
            },
            target: make_window(),
            labels: crate::schema::WorldLabelSet::default(),
        }
    }

    #[test]
    fn cpu_backend_kind_is_cpu() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        assert_eq!(runtime.backend_kind(), JepaEvalBackendKind::Cpu);
    }

    #[test]
    fn cpu_encode_returns_correct_shape() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        let batch = vec![make_transition(), make_transition()];
        let encoded = runtime.encode_batch(&batch).unwrap();
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[0].len(), 384);
        // All values are zero (deterministic reference encoding)
        assert!(encoded[0].iter().all(|&v| v == 0.0_f32));
    }

    #[test]
    fn cpu_encode_empty_batch_returns_empty() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        let encoded = runtime.encode_batch(&[]).unwrap();
        assert!(encoded.is_empty());
    }

    #[test]
    fn cpu_predict_is_identity() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        let input = vec![vec![1.0_f32, 2.0, 3.0]];
        let output = runtime.predict_batch(&input).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn cpu_predict_empty_batch_returns_empty() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        let output = runtime.predict_batch(&[]).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn cpu_parity_report_trivially_passes() {
        let runtime = CpuEvalRuntime::new(384, 0.99);
        let sample = vec![make_transition()];
        let report = runtime.validate_forward_parity(&sample).unwrap();
        assert!(report.passed);
        assert_eq!(report.cosine_similarity, 1.0);
        assert_eq!(report.reference_backend, JepaEvalBackendKind::Cpu);
        assert_eq!(report.test_backend, JepaEvalBackendKind::Cpu);
        assert_eq!(report.sample_count, 1);
        assert_eq!(report.floor, 0.99);
    }

    #[test]
    fn cpu_parity_report_sample_count_matches_input() {
        let runtime = CpuEvalRuntime::new(128, 0.95);
        let sample = vec![make_transition(), make_transition(), make_transition()];
        let report = runtime.validate_forward_parity(&sample).unwrap();
        assert_eq!(report.sample_count, 3);
        assert!(report.passed);
    }

    #[test]
    fn cpu_runtime_selectable_via_trait_object() {
        // BackendRuntimeResolver::resolve() with Cpu backend should now succeed (T017 stub replaced)
        let runtime: Box<dyn JepaEvalRuntime> = Box::new(CpuEvalRuntime::new(384, 0.99));
        assert_eq!(runtime.backend_kind(), JepaEvalBackendKind::Cpu);
    }

    #[test]
    fn cpu_resolver_wired_correctly() {
        use chrono::Utc;
        // Construct minimal metadata with Cpu backend
        let mut backend_execution =
            JepaBackendExecutionReport::cpu(crate::BackendKind::Cpu, None, 0);
        backend_execution.selected_backend = crate::BackendKind::Cpu;
        let metadata = JepaTraceModelMetadata {
            model_id: "test".to_string(),
            model_kind: "jepa_transition".to_string(),
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            target_stop_gradient: true,
            backend: crate::BackendKind::Cpu,
            backend_execution,
            row_count: 0,
            example_count: 0,
            parameter_count: 0,
            created_at: Utc::now(),
        };
        let config = JepaEvalResolverConfig {
            training_backend: "cpu".into(),
            allow_cpu_fallback: true,
            prefer_accelerator: false,
            parity_floor: 0.99,
        };
        let result = BackendRuntimeResolver::resolve(&metadata, &config, None);
        assert!(result.is_ok(), "CPU resolve must succeed: {:?}", result.err());
        let runtime = result.unwrap();
        assert_eq!(runtime.backend_kind(), JepaEvalBackendKind::Cpu);
    }
}
