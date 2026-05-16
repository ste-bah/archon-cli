pub trait JepaTensorBackend: Send + Sync {
    fn status(&self) -> BackendStatus;
    fn probe_jepa(&self) -> JepaBackendProbeReport;

    fn observed_device_name(&self) -> Option<String> {
        self.status().device_name
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaStageResult<JepaEncodedBatch>>;

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<JepaPredictor>>;

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<Vec<JepaAuxiliaryHead>>>;

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<CpuLatentTransitionModel>>;

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaStageResult<JepaTrainingLosses>>;

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
    ) -> Result<JepaStageResult<JepaRuntimePrediction>>;
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
    ) -> Result<JepaStageResult<JepaEncodedBatch>> {
        Ok(JepaStageResult::native(encode_examples(
            &encoders.context_encoder,
            &encoders.action_encoder,
            &encoders.target_encoder,
            batch,
        )?))
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<JepaPredictor>> {
        Ok(JepaStageResult::native(JepaPredictor::fit(
            latent_dim, encoded,
        )?))
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<Vec<JepaAuxiliaryHead>>> {
        Ok(JepaStageResult::native(fit_auxiliary_heads(
            latent_dim, encoded,
        )))
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<CpuLatentTransitionModel>> {
        Ok(JepaStageResult::native(CpuLatentTransitionModel::fit(
            latent_dim,
            &encoded_transition_examples(encoded),
        )?))
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaStageResult<JepaTrainingLosses>> {
        Ok(JepaStageResult::native(training_losses(
            model, encoded, config,
        )?))
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
    ) -> Result<JepaStageResult<JepaRuntimePrediction>> {
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
        let latency_ms = started.elapsed().as_millis() as u64;
        Ok(JepaStageResult::native(JepaRuntimePrediction {
            backend: BackendKind::Cpu,
            predicted_next_state,
            guardrail_scores: jepa_guardrail_scores_from_auxiliary(&auxiliary_scores),
            auxiliary_scores,
            latency_ms,
            execution_report: JepaRuntimeBackendReport::new(
                BackendKind::Cpu,
                "rust-vector",
                Some("cpu".into()),
                true,
                latency_ms,
            ),
        }))
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
            true,
        )
    }

    fn observed_device_name(&self) -> Option<String> {
        observed_backend_device_name(BackendKind::Cuda)
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaStageResult<JepaEncodedBatch>> {
        #[cfg(feature = "cuda")]
        {
            let device = cuda_jepa_device()?;
            Ok(JepaStageResult::native(candle_encode_batch_on_device(
                encoders, batch, &device,
            )?))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (encoders, batch);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<JepaPredictor>> {
        #[cfg(feature = "cuda")]
        {
            let device = cuda_jepa_device()?;
            Ok(JepaStageResult::native(candle_fit_predictor_on_device(
                latent_dim, encoded, &device,
            )?))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<Vec<JepaAuxiliaryHead>>> {
        #[cfg(feature = "cuda")]
        {
            let device = cuda_jepa_device()?;
            Ok(JepaStageResult::native(
                candle_fit_auxiliary_heads_on_device(latent_dim, encoded, &device)?,
            ))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<CpuLatentTransitionModel>> {
        #[cfg(feature = "cuda")]
        {
            let transition = crate::backend::candle::candle_cuda_fit_transition_model(
                latent_dim,
                &encoded_transition_examples(encoded),
            )?;
            let execution = if transition.metadata.backend == BackendKind::Cuda {
                JepaStageExecution::native()
            } else {
                JepaStageExecution::host_fallback(1)
            };
            Ok(JepaStageResult::new(transition, execution))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaStageResult<JepaTrainingLosses>> {
        #[cfg(feature = "cuda")]
        {
            let device = cuda_jepa_device()?;
            Ok(JepaStageResult::native(candle_training_losses_on_device(
                model, encoded, config, &device,
            )?))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (model, encoded, config);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
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
    ) -> Result<JepaStageResult<JepaRuntimePrediction>> {
        #[cfg(feature = "cuda")]
        {
            let device = cuda_jepa_device()?;
            let prediction = candle_jepa_predict_runtime_on_device(
                model,
                window,
                action,
                BackendKind::Cuda,
                &device,
            )?;
            let execution = JepaStageExecution {
                native: prediction.execution_report.native_runtime_prediction,
                host_fallback_count: prediction.execution_report.host_fallback_count,
            };
            Ok(JepaStageResult::new(prediction, execution))
        }
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (model, window, action);
            native_jepa_backend_unavailable(BackendKind::Cuda)
        }
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
            cfg!(all(
                feature = "mlx-metal",
                target_os = "macos",
                target_arch = "aarch64"
            )),
        )
    }

    fn observed_device_name(&self) -> Option<String> {
        observed_backend_device_name(BackendKind::Metal)
    }

    fn encode_batch(
        &self,
        encoders: &JepaEncoderSet,
        batch: &JepaFeatureBatch,
    ) -> Result<JepaStageResult<JepaEncodedBatch>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            Ok(JepaStageResult::native(mlx_encode_batch_on_device(
                encoders, batch,
            )?))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (encoders, batch);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
    }

    fn fit_predictor(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<JepaPredictor>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            Ok(JepaStageResult::native(mlx_fit_predictor_on_device(
                latent_dim, encoded,
            )?))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
    }

    fn fit_auxiliary_heads(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<Vec<JepaAuxiliaryHead>>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            Ok(JepaStageResult::native(mlx_fit_auxiliary_heads_on_device(
                latent_dim, encoded,
            )?))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
    }

    fn fit_transition(
        &self,
        latent_dim: usize,
        encoded: &JepaEncodedBatch,
    ) -> Result<JepaStageResult<CpuLatentTransitionModel>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            let transition = crate::backend::mlx::mlx_metal_fit_transition_model(
                latent_dim,
                &encoded_transition_examples(encoded),
            )?;
            let execution = if transition.metadata.backend == BackendKind::Metal {
                JepaStageExecution::native()
            } else {
                JepaStageExecution::host_fallback(1)
            };
            Ok(JepaStageResult::new(transition, execution))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (latent_dim, encoded);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
    }

    fn training_losses(
        &self,
        model: &JepaTraceModel,
        encoded: &JepaEncodedBatch,
        config: &JepaTrainingConfig,
    ) -> Result<JepaStageResult<JepaTrainingLosses>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            Ok(JepaStageResult::native(mlx_training_losses_on_device(
                model, encoded, config,
            )?))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (model, encoded, config);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
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
    ) -> Result<JepaStageResult<JepaRuntimePrediction>> {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            let prediction = mlx_jepa_predict_runtime_on_device(model, window, action)?;
            let execution = JepaStageExecution {
                native: prediction.execution_report.native_runtime_prediction,
                host_fallback_count: prediction.execution_report.host_fallback_count,
            };
            Ok(JepaStageResult::new(prediction, execution))
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            let _ = (model, window, action);
            native_jepa_backend_unavailable(BackendKind::Metal)
        }
    }
}

fn native_jepa_backend_unavailable<T>(backend: BackendKind) -> Result<T> {
    bail!("JepaBackendUnavailable: native {backend} JEPA tensor backend is not implemented")
}

