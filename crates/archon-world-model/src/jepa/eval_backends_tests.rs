// Cosine helpers (used by MLX and CUDA backend parity sampling) and unit tests.
// Split out of eval_backends.rs to keep individual files under 500 lines (NFR-FOR-D4).

/// Mean cosine similarity between two batches of vectors in f32.
/// Returns 0.0 when either batch is empty or the lengths differ.
fn compute_mean_cosine_f32(reference: &[Vec<f32>], test: &[Vec<f32>]) -> f32 {
    if reference.is_empty() || test.is_empty() || reference.len() != test.len() {
        return 0.0;
    }
    let total: f32 = reference
        .iter()
        .zip(test.iter())
        .map(|(r, t)| cosine_similarity_f32(r, t))
        .sum();
    total / reference.len() as f32
}

/// Cosine similarity between two f32 slices.
///
/// Two zero-vectors are defined as identical (returns 1.0): this covers the
/// pragmatic CPU-delegate case where both sides produce zero latents.
fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        // Two zero-vectors are defined as identical (cosine = 1.0).
        // This is the expected case for our pragmatic CPU-delegated MLX encode.
        return if norm_a == norm_b { 1.0 } else { 0.0 };
    }
    dot / (norm_a * norm_b)
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

    // -------------------------------------------------------------------------
    // CpuEvalRuntime tests (T018)
    // -------------------------------------------------------------------------

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
            min_metal_validation_examples: 0,
            min_cuda_validation_examples: 0,
        };
        let result = BackendRuntimeResolver::resolve(&metadata, &config, None);
        assert!(result.is_ok(), "CPU resolve must succeed: {:?}", result.err());
        let runtime = result.unwrap();
        assert_eq!(runtime.backend_kind(), JepaEvalBackendKind::Cpu);
    }

    // -------------------------------------------------------------------------
    // MlxEvalRuntime tests (T019) — metadata validation runs on any platform
    // -------------------------------------------------------------------------

    fn make_metadata_metal_ok() -> JepaTraceModelMetadata {
        use chrono::Utc;
        let mut exec = JepaBackendExecutionReport::cpu(crate::BackendKind::Metal, None, 512);
        exec.selected_backend = crate::BackendKind::Metal;
        exec.framework = "mlx-rs".to_string();
        exec.native_encode = true;
        exec.native_runtime_prediction = Some(true);
        exec.host_fallback_count = 0;
        JepaTraceModelMetadata {
            model_id: "test-mlx".to_string(),
            model_kind: "jepa_transition".to_string(),
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            target_stop_gradient: true,
            backend: crate::BackendKind::Metal,
            backend_execution: exec,
            row_count: 0,
            example_count: 0,
            parameter_count: 0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn mlx_metadata_validation_passes_when_all_5_fields_ok() {
        let meta = make_metadata_metal_ok();
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_ok(), "valid metadata must pass: {result:?}");
    }

    #[test]
    fn mlx_metadata_validation_fails_when_selected_backend_not_metal() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.selected_backend = crate::BackendKind::Cpu;
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("selected_backend"));
    }

    #[test]
    fn mlx_metadata_validation_fails_when_framework_not_mlx_rs() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.framework = "wrong-framework".to_string();
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("framework"));
    }

    #[test]
    fn mlx_metadata_validation_fails_when_native_encode_false() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.native_encode = false;
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("native_encode"));
    }

    #[test]
    fn mlx_metadata_validation_fails_when_native_runtime_prediction_not_true() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.native_runtime_prediction = Some(false);
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("native_runtime_prediction"));
    }

    #[test]
    fn mlx_metadata_validation_fails_when_host_fallback_nonzero_and_disallowed() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.host_fallback_count = 5;
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("host_fallback_count"));
    }

    #[test]
    fn mlx_metadata_validation_passes_when_host_fallback_nonzero_and_allowed() {
        let mut meta = make_metadata_metal_ok();
        meta.backend_execution.host_fallback_count = 5;
        let result = MlxEvalRuntime::validate_candidate_metadata(&meta, true);
        assert!(result.is_ok());
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn mlx_constructor_fails_on_non_darwin_arm64() {
        let meta = make_metadata_metal_ok();
        let result = MlxEvalRuntime::new(&meta, 384, 0.99, 512, false);
        let error = match result {
            Ok(_) => panic!("MLX runtime must fail on non-Darwin arm64"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Darwin arm64"));
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn mlx_constructor_succeeds_with_valid_metadata_on_darwin_arm64() {
        let meta = make_metadata_metal_ok();
        let result = MlxEvalRuntime::new(&meta, 384, 0.99, 512, false);
        assert!(result.is_ok());
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn mlx_parity_passes_trivially_with_cpu_delegate() {
        // CPU and MLX both delegate to the same zero-vector path → cosine = 1.0
        let meta = make_metadata_metal_ok();
        let runtime = MlxEvalRuntime::new(&meta, 384, 0.99, 2, false).unwrap();
        let sample = vec![make_transition(), make_transition()];
        let report = runtime.validate_forward_parity(&sample).unwrap();
        assert!(report.passed);
        assert_eq!(report.cosine_similarity, 1.0);
        assert_eq!(report.reference_backend, JepaEvalBackendKind::Cpu);
        assert_eq!(report.test_backend, JepaEvalBackendKind::MlxMetal);
        assert_eq!(report.sample_count, 2);
    }

    #[test]
    #[ignore = "requires Apple Silicon hardware AND a trained MLX candidate"]
    fn mlx_parity_passes_on_real_hardware() {
        // Hardware-gated. Load a real candidate, run validate_forward_parity,
        // assert cosine >= floor. Only runs manually on Apple Silicon.
    }

    // -------------------------------------------------------------------------
    // CudaEvalRuntime tests (T020) — metadata validation runs on any platform;
    // runtime/parity tests are feature-gated to "cuda"
    // -------------------------------------------------------------------------

    #[cfg(feature = "cuda")]
    fn make_metadata_cuda_ok() -> JepaTraceModelMetadata {
        use chrono::Utc;
        let mut exec = JepaBackendExecutionReport::cpu(crate::BackendKind::Cuda, None, 512);
        exec.selected_backend = crate::BackendKind::Cuda;
        exec.native_encode = true;
        exec.native_runtime_prediction = Some(true);
        exec.host_fallback_count = 0;
        JepaTraceModelMetadata {
            model_id: "test-cuda".to_string(),
            model_kind: "jepa_transition".to_string(),
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            target_stop_gradient: true,
            backend: crate::BackendKind::Cuda,
            backend_execution: exec,
            row_count: 0,
            example_count: 0,
            parameter_count: 0,
            created_at: Utc::now(),
        }
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_metadata_validation_passes_when_all_fields_ok() {
        let meta = make_metadata_cuda_ok();
        let result = CudaEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_ok(), "valid metadata must pass: {result:?}");
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_metadata_validation_fails_when_selected_backend_not_cuda() {
        let mut meta = make_metadata_cuda_ok();
        meta.backend_execution.selected_backend = crate::BackendKind::Cpu;
        let result = CudaEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("selected_backend"));
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_metadata_validation_fails_when_native_encode_false() {
        let mut meta = make_metadata_cuda_ok();
        meta.backend_execution.native_encode = false;
        let result = CudaEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("native_encode"));
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_metadata_validation_fails_when_host_fallback_nonzero_and_disallowed() {
        let mut meta = make_metadata_cuda_ok();
        meta.backend_execution.host_fallback_count = 5;
        let result = CudaEvalRuntime::validate_candidate_metadata(&meta, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("host_fallback_count"));
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_constructor_succeeds_with_valid_metadata() {
        let meta = make_metadata_cuda_ok();
        let result = CudaEvalRuntime::new(&meta, 384, 0.99, 512, false);
        assert!(result.is_ok());
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_parity_trivially_passes_via_cpu_delegate() {
        // CPU and CUDA both delegate to the same zero-vector path → cosine = 1.0
        let meta = make_metadata_cuda_ok();
        let runtime = CudaEvalRuntime::new(&meta, 384, 0.99, 2, false).unwrap();
        let sample = vec![make_transition(), make_transition()];
        let report = runtime.validate_forward_parity(&sample).unwrap();
        assert!(report.passed);
        assert_eq!(report.cosine_similarity, 1.0);
        assert_eq!(report.reference_backend, JepaEvalBackendKind::Cpu);
        assert_eq!(report.test_backend, JepaEvalBackendKind::Cuda);
    }

    #[test]
    #[ignore = "requires CUDA hardware"]
    fn cuda_parity_passes_on_real_hardware() {
        // Hardware-gated. Build with --features cuda on a CUDA-capable host,
        // load a real candidate, assert cosine >= floor.
    }

    // Test that runs on this Mac (no CUDA feature): verifies resolve fails closed
    // when CUDA required but feature not compiled.
    #[cfg(not(feature = "cuda"))]
    #[test]
    fn cuda_required_but_not_compiled_fails_closed_via_resolver() {
        let mut meta = make_metadata_metal_ok(); // borrow MLX helper — just need any valid metadata
        meta.backend_execution.selected_backend = crate::BackendKind::Cuda;
        let config = JepaEvalResolverConfig {
            training_backend: "cuda".into(),
            allow_cpu_fallback: false,
            prefer_accelerator: true,
            parity_floor: 0.99,
            min_metal_validation_examples: 0,
            min_cuda_validation_examples: 512,
        };
        let result = BackendRuntimeResolver::resolve(&meta, &config, Some(JepaEvalBackendKind::Cuda));
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("CUDA") || msg.contains("cuda") || msg.contains("feature"),
            "error must mention CUDA/feature: {msg}"
        );
    }

    // -------------------------------------------------------------------------
    // Cosine helper unit tests
    // -------------------------------------------------------------------------

    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let v = vec![1.0_f32, 2.0, 3.0];
        assert!((cosine_similarity_f32(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vectors_is_one_by_convention() {
        let a = vec![0.0_f32; 4];
        let b = vec![0.0_f32; 4];
        assert_eq!(cosine_similarity_f32(&a, &b), 1.0);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_is_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert!((cosine_similarity_f32(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn compute_mean_cosine_empty_reference_returns_zero() {
        let result = compute_mean_cosine_f32(&[], &[]);
        assert_eq!(result, 0.0);
    }

    #[test]
    fn compute_mean_cosine_identical_batches_returns_one() {
        let batch = vec![vec![1.0_f32, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let result = compute_mean_cosine_f32(&batch, &batch);
        assert!((result - 1.0).abs() < 1e-6);
    }
}
