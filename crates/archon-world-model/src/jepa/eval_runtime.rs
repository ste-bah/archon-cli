// TASK-JEVAL-017 — JepaEvalRuntime trait + BackendRuntimeResolver
//
// Backend-neutral evaluation contract per PRD-006C §6.4.
// Three implementations land in T018 (CPU), T019 (MLX), T020 (CUDA).
// This file defines only the trait, resolver, and stubs that return Err.
//
// DEC-JEVAL-11: types are flat under crate::jepa::*. Like eval_planner.rs,
// the resolver uses a crate-local config mirror (JepaEvalResolverConfig) to
// avoid depending on archon-core.
//
// NOTE: `selected_backend` lives on `metadata.backend_execution.selected_backend`
// (JepaBackendExecutionReport), NOT directly on JepaTraceModelMetadata.

/// Mirrors the relevant fields of `archon_core::config::WorldModelTrainingConfig`
/// needed by the backend resolver. Crate-local to keep archon-world-model free
/// of an archon-core dependency.
#[derive(Debug, Clone, Default)]
pub struct JepaEvalResolverConfig {
    /// Training backend: "auto" | "cpu" | "metal" | "cuda"
    pub training_backend: String,
    pub allow_cpu_fallback: bool,
    pub prefer_accelerator: bool,
    /// Minimum cosine similarity floor for backend parity validation.
    /// Maps to `WorldModelJepaConfig::backend_parity_cosine_floor` in archon-core.
    /// Set to 0.0 (the Default) when no floor is configured.
    pub parity_floor: f32,
}

/// CPU-vs-accelerator parity sample report.
#[derive(Debug, Clone)]
pub struct ParityReport {
    pub passed: bool,
    pub cosine_similarity: f32,
    pub sample_count: usize,
    pub floor: f32,
    pub reference_backend: JepaEvalBackendKind,
    pub test_backend: JepaEvalBackendKind,
}

/// Backend-neutral evaluation contract.
/// CPU (T018), MLX Metal (T019), CUDA (T020) all implement this.
pub trait JepaEvalRuntime: Send + Sync {
    fn backend_kind(&self) -> JepaEvalBackendKind;

    /// Encode a batch of trace transitions into JEPA latent representations.
    fn encode_batch(&self, batch: &[TraceTransition]) -> Result<Vec<Vec<f32>>>;

    /// Run prediction on a batch of encoded latents.
    fn predict_batch(&self, batch: &[Vec<f32>]) -> Result<Vec<Vec<f32>>>;

    /// CPU-vs-accelerator parity sample. CPU impl returns trivially OK.
    fn validate_forward_parity(&self, sample: &[TraceTransition]) -> Result<ParityReport>;
}

/// Resolves the appropriate `JepaEvalRuntime` based on candidate metadata,
/// resolver config, and CLI override (§6.4 priority).
pub struct BackendRuntimeResolver;

impl BackendRuntimeResolver {
    /// Priority: CLI override > candidate metadata selected_backend > training config
    pub fn determine_preferred_backend(
        metadata: &JepaTraceModelMetadata,
        resolver_config: &JepaEvalResolverConfig,
        cli_override: Option<JepaEvalBackendKind>,
    ) -> JepaEvalBackendKind {
        if let Some(cli) = cli_override {
            return cli;
        }
        match metadata.backend_execution.selected_backend {
            crate::BackendKind::Metal => JepaEvalBackendKind::MlxMetal,
            crate::BackendKind::Cuda => JepaEvalBackendKind::Cuda,
            crate::BackendKind::Cpu | crate::BackendKind::Auto => {
                // Fall back to training config
                match resolver_config.training_backend.as_str() {
                    "metal" => JepaEvalBackendKind::MlxMetal,
                    "cuda" => JepaEvalBackendKind::Cuda,
                    _ => JepaEvalBackendKind::Cpu,
                }
            }
        }
    }

    /// CUDA is "required" per §6.6 when ANY of:
    /// (a) candidate metadata selected_backend = Cuda
    /// (b) training_backend = "cuda" AND (!allow_cpu_fallback OR prefer_accelerator)
    /// (c) CLI override = Cuda
    pub fn cuda_is_required(
        metadata: &JepaTraceModelMetadata,
        resolver_config: &JepaEvalResolverConfig,
        cli_override: Option<JepaEvalBackendKind>,
    ) -> bool {
        cli_override == Some(JepaEvalBackendKind::Cuda)
            || matches!(metadata.backend_execution.selected_backend, crate::BackendKind::Cuda)
            || (resolver_config.training_backend == "cuda"
                && (!resolver_config.allow_cpu_fallback || resolver_config.prefer_accelerator))
    }

    /// Resolve to a concrete runtime. T018 (CPU) is wired; T019/T020 are stubs.
    pub fn resolve(
        metadata: &JepaTraceModelMetadata,
        resolver_config: &JepaEvalResolverConfig,
        cli_override: Option<JepaEvalBackendKind>,
    ) -> Result<Box<dyn JepaEvalRuntime>> {
        let preferred = Self::determine_preferred_backend(metadata, resolver_config, cli_override);
        match preferred {
            JepaEvalBackendKind::Cpu => {
                Self::resolve_cpu(metadata.latent_dim, resolver_config.parity_floor)
            }
            JepaEvalBackendKind::MlxMetal => Self::resolve_mlx(),
            JepaEvalBackendKind::Cuda => {
                Self::resolve_cuda(metadata, resolver_config, cli_override)
            }
        }
    }

    fn resolve_cpu(latent_dim: usize, parity_floor: f32) -> Result<Box<dyn JepaEvalRuntime>> {
        Ok(Box::new(CpuEvalRuntime::new(latent_dim, parity_floor)))
    }

    fn resolve_mlx() -> Result<Box<dyn JepaEvalRuntime>> {
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            anyhow::bail!(
                "MLX Metal backend requires Darwin arm64. Current platform: {}/{}.",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            anyhow::bail!("MLX Metal JepaEvalRuntime not yet implemented — see TASK-JEVAL-019")
        }
    }

    fn resolve_cuda(
        metadata: &JepaTraceModelMetadata,
        resolver_config: &JepaEvalResolverConfig,
        cli_override: Option<JepaEvalBackendKind>,
    ) -> Result<Box<dyn JepaEvalRuntime>> {
        let required = Self::cuda_is_required(metadata, resolver_config, cli_override);
        #[cfg(not(feature = "cuda"))]
        if required {
            anyhow::bail!(
                "CUDA backend required but not available (binary not built with feature=cuda). \
                 Rebuild with --features cuda, or use --backend cpu if policy/config allow fallback."
            );
        }
        // Stub until T020
        anyhow::bail!("CUDA JepaEvalRuntime not yet implemented — see TASK-JEVAL-020")
    }
}

#[cfg(test)]
mod tests_eval_runtime {
    use super::*;
    use chrono::Utc;

    fn make_metadata(backend: crate::BackendKind) -> JepaTraceModelMetadata {
        let mut backend_execution = JepaBackendExecutionReport::cpu(backend, None, 0);
        // Force selected_backend to match the test scenario; the `cpu()` helper
        // always sets selected_backend=Cpu but we need to test other backends.
        backend_execution.selected_backend = backend;
        JepaTraceModelMetadata {
            model_id: "test".to_string(),
            model_kind: "jepa_transition".to_string(),
            latent_dim: 384,
            context_window_rows: 8,
            target_window_rows: 3,
            prediction_horizons: vec![1, 3, 5],
            mask_ratio: 0.30,
            ema_decay: 0.996,
            target_stop_gradient: true,
            backend,
            backend_execution,
            row_count: 0,
            example_count: 0,
            parameter_count: 0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn cli_override_takes_priority_over_metadata() {
        let metadata = make_metadata(crate::BackendKind::Metal);
        let config = JepaEvalResolverConfig {
            training_backend: "cuda".into(),
            ..Default::default()
        };
        let preferred = BackendRuntimeResolver::determine_preferred_backend(
            &metadata,
            &config,
            Some(JepaEvalBackendKind::Cpu),
        );
        assert_eq!(preferred, JepaEvalBackendKind::Cpu);
    }

    #[test]
    fn cuda_required_when_metadata_says_cuda() {
        let metadata = make_metadata(crate::BackendKind::Cuda);
        let config = JepaEvalResolverConfig::default();
        assert!(BackendRuntimeResolver::cuda_is_required(
            &metadata, &config, None
        ));
    }

    #[test]
    fn cuda_required_when_training_cuda_no_fallback() {
        let metadata = make_metadata(crate::BackendKind::Cpu);
        let config = JepaEvalResolverConfig {
            training_backend: "cuda".into(),
            allow_cpu_fallback: false,
            prefer_accelerator: false,
            parity_floor: 0.0,
        };
        assert!(BackendRuntimeResolver::cuda_is_required(
            &metadata, &config, None
        ));
    }

    #[test]
    fn cuda_required_when_training_cuda_prefer_accelerator() {
        let metadata = make_metadata(crate::BackendKind::Cpu);
        let config = JepaEvalResolverConfig {
            training_backend: "cuda".into(),
            allow_cpu_fallback: true,
            prefer_accelerator: true,
            parity_floor: 0.0,
        };
        assert!(BackendRuntimeResolver::cuda_is_required(
            &metadata, &config, None
        ));
    }

    #[test]
    fn cuda_not_required_when_cpu_training_and_fallback_allowed() {
        let metadata = make_metadata(crate::BackendKind::Cpu);
        let config = JepaEvalResolverConfig {
            training_backend: "cpu".into(),
            allow_cpu_fallback: true,
            prefer_accelerator: false,
            parity_floor: 0.0,
        };
        assert!(!BackendRuntimeResolver::cuda_is_required(
            &metadata, &config, None
        ));
    }

    #[test]
    fn cli_cuda_override_makes_cuda_required() {
        let metadata = make_metadata(crate::BackendKind::Cpu);
        let config = JepaEvalResolverConfig::default();
        assert!(BackendRuntimeResolver::cuda_is_required(
            &metadata,
            &config,
            Some(JepaEvalBackendKind::Cuda)
        ));
    }

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn mlx_fails_on_non_darwin_arm64() {
        let result = BackendRuntimeResolver::resolve_mlx();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Darwin arm64"),
            "error must mention Darwin arm64"
        );
    }
}
