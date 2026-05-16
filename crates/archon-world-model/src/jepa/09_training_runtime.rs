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

    if !allow_cpu_fallback {
        if let Some(reason) = status.fallback_reason.as_deref() {
            bail!(
                "JepaBackendProbeFailed: requested JEPA backend {} unavailable: {reason}",
                status.selected
            );
        }
    }

    if status.selected == BackendKind::Cuda {
        #[cfg(feature = "cuda")]
        {
            match train_jepa_candidate_with_tensor_backend(
                rows,
                config,
                status.clone(),
                CandleCudaJepaBackend,
            ) {
                Ok(candidate) => return Ok(candidate),
                Err(error) if allow_cpu_fallback => {
                    let fallback = BackendStatus::cpu_fallback(
                        status.requested,
                        format!("jepa_native_backend_failed:{}:{error}", status.selected),
                    );
                    return train_jepa_candidate_cpu(rows, config, fallback);
                }
                Err(error) => {
                    bail!(
                        "JepaBackendNativeStageFailed: native JEPA backend for {} failed; refusing to write an accelerator-labelled candidate: {error}",
                        status.selected
                    );
                }
            }
        }
        #[cfg(not(feature = "cuda"))]
        {
            if allow_cpu_fallback {
                let fallback = BackendStatus::cpu_fallback(
                    status.requested,
                    "jepa_native_backend_not_compiled:cuda",
                );
                return train_jepa_candidate_cpu(rows, config, fallback);
            }
        }
    }

    if status.selected == BackendKind::Metal {
        #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
        {
            match train_jepa_candidate_with_tensor_backend(
                rows,
                config,
                status.clone(),
                MlxMetalJepaBackend,
            ) {
                Ok(candidate) => return Ok(candidate),
                Err(error) if allow_cpu_fallback => {
                    let fallback = BackendStatus::cpu_fallback(
                        status.requested,
                        format!("jepa_native_backend_failed:{}:{error}", status.selected),
                    );
                    return train_jepa_candidate_cpu(rows, config, fallback);
                }
                Err(error) => {
                    bail!(
                        "JepaBackendNativeStageFailed: native JEPA backend for {} failed; refusing to write an accelerator-labelled candidate: {error}",
                        status.selected
                    );
                }
            }
        }
        #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
        {
            if allow_cpu_fallback {
                let fallback = BackendStatus::cpu_fallback(
                    status.requested,
                    "jepa_native_backend_not_compiled:metal",
                );
                return train_jepa_candidate_cpu(rows, config, fallback);
            }
        }
    }

    if allow_cpu_fallback {
        let fallback = BackendStatus::cpu_fallback(
            status.requested,
            format!("jepa_native_backend_not_implemented:{}", status.selected),
        );
        return train_jepa_candidate_cpu(rows, config, fallback);
    }

    bail!(
        "JepaBackendNativeStageFailed: native JEPA backend for {} is not implemented; refusing to write an accelerator-labelled candidate",
        status.selected
    );
}

fn train_jepa_candidate_cpu(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    backend_status: BackendStatus,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_tensor_backend(rows, config, backend_status, CpuJepaBackend)
}

fn train_jepa_candidate_with_tensor_backend<B: JepaTensorBackend>(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    backend_status: BackendStatus,
    backend: B,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    config.validate()?;
    let examples = build_jepa_training_examples(rows, config)?;
    if examples.is_empty() {
        bail!("not enough rows to train JEPA: need future rows in the same session");
    }
    let (masked_examples, masking) = mask_jepa_training_examples(&examples, config.mask_ratio);
    let feature_batch = JepaFeatureBatch::from_examples(&masked_examples, config.latent_dim)?;

    let context_encoder = JepaTraceEncoder::new("context", config.latent_dim);
    let action_encoder = JepaTraceEncoder::new("action", config.latent_dim);
    let target_encoder = JepaTraceEncoder::ema_target_from(&context_encoder, config.ema_decay);
    let encoders = JepaEncoderSet {
        context_encoder: context_encoder.clone(),
        action_encoder: action_encoder.clone(),
        target_encoder: target_encoder.clone(),
    };
    let encoded_stage = backend.encode_batch(&encoders, &feature_batch)?;
    let encoded_execution = encoded_stage.execution;
    let encoded = encoded_stage.value;
    let initial_auxiliary_heads_stage = backend.fit_auxiliary_heads(config.latent_dim, &encoded)?;
    let initial_auxiliary_execution = initial_auxiliary_heads_stage.execution;
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
        auxiliary_heads: initial_auxiliary_heads_stage.value,
        transition_model: None,
    };
    let initial_losses_stage = backend.training_losses(&initial_model, &encoded, config)?;
    let initial_loss_execution = initial_losses_stage.execution;
    let initial_losses = initial_losses_stage.value;
    let predictor_stage = backend.fit_predictor(config.latent_dim, &encoded)?;
    let predictor_execution = predictor_stage.execution;
    let auxiliary_heads_stage = backend.fit_auxiliary_heads(config.latent_dim, &encoded)?;
    let auxiliary_execution = initial_auxiliary_execution.combine(auxiliary_heads_stage.execution);
    let transition_model_stage = backend.fit_transition(config.latent_dim, &encoded)?;
    let transition_execution = transition_model_stage.execution;
    let mut metadata =
        JepaTraceModelMetadata::candidate(config, rows.len() as u64, examples.len() as u64);
    metadata.backend = backend_status.selected;
    let mut model = JepaTraceModel {
        metadata: metadata.clone(),
        context_encoder,
        action_encoder,
        target_encoder,
        predictor: predictor_stage.value,
        auxiliary_heads: auxiliary_heads_stage.value,
        transition_model: Some(transition_model_stage.value),
    };
    let losses_stage = backend.training_losses(&model, &encoded, config)?;
    let loss_execution = initial_loss_execution.combine(losses_stage.execution);
    let runtime_execution = if backend_status.selected == BackendKind::Cpu {
        None
    } else {
        Some(
            backend
                .predict_runtime(&model, &examples[0].context, &examples[0].action)?
                .execution,
        )
    };
    metadata.parameter_count = model.parameter_count();
    metadata.backend_execution = if backend_status.selected == BackendKind::Cpu {
        JepaBackendExecutionReport::from_cpu_status(&backend_status, examples.len())
    } else {
        let probe = backend.probe_jepa();
        JepaBackendExecutionReport::native(
            &backend_status,
            examples.len(),
            JepaBackendExecutionEvidence {
                device_name: backend.observed_device_name(),
                tensor_self_test_passed: probe.tensor_self_test_passed,
                encode: encoded_execution,
                predictor_fit: predictor_execution,
                auxiliary_fit: auxiliary_execution,
                transition_fit: transition_execution,
                loss_eval: loss_execution,
                runtime_prediction: runtime_execution,
            },
        )
    };
    metadata.backend_execution.validation_example_count = examples.len();
    model.metadata = metadata;
    validate_jepa_backend_execution(&model.metadata)?;
    model.validate_finite()?;
    let losses = losses_stage.value;
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
            "JepaBackendHostFallbackRejected: jepa metadata backend {:?} does not match execution report selected backend {:?}",
            metadata.backend,
            report.selected_backend
        );
    }

    if matches!(metadata.backend, BackendKind::Cuda | BackendKind::Metal) {
        if !report.native_stage_proof_passes() {
            bail!(
                "JepaBackendNativeStageFailed: jepa {:?} candidate is missing native backend execution proof",
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
        bail!(
            "JepaBackendHostFallbackRejected: jepa CPU candidate cannot carry accelerator-selected execution metadata"
        );
    }

    Ok(())
}

pub fn jepa_backend_promotion_gate_failure(
    metadata: &JepaTraceModelMetadata,
    min_cuda_validation_examples: usize,
    min_metal_validation_examples: usize,
) -> Option<&'static str> {
    if let Err(error) = validate_jepa_backend_execution(metadata) {
        let message = error.to_string();
        if message.contains("JepaBackendHostFallbackRejected") {
            return Some("JepaBackendHostFallbackRejected");
        }
        return Some("JepaBackendNativeStageFailed");
    }

    let report = &metadata.backend_execution;
    match metadata.backend {
        BackendKind::Cuda => {
            if !report.native_stage_proof_passes() {
                Some("JepaBackendNativeStageFailed")
            } else if report.hardware_validation_captured_at.is_none()
                || report.validation_example_count < min_cuda_validation_examples
            {
                Some("JepaBackendHardwareValidationMissing")
            } else {
                None
            }
        }
        BackendKind::Metal => {
            if !report.native_stage_proof_passes() {
                Some("JepaBackendNativeStageFailed")
            } else if report.hardware_validation_captured_at.is_none()
                || report.validation_example_count < min_metal_validation_examples
            {
                Some("JepaBackendHardwareValidationMissing")
            } else {
                None
            }
        }
        BackendKind::Auto | BackendKind::Cpu => None,
    }
}

pub fn jepa_backend_promotion_gate(
    metadata: &JepaTraceModelMetadata,
    min_cuda_validation_examples: usize,
    min_metal_validation_examples: usize,
) -> bool {
    jepa_backend_promotion_gate_failure(
        metadata,
        min_cuda_validation_examples,
        min_metal_validation_examples,
    )
    .is_none()
}

pub fn jepa_backend_forward_parity(
    model: &JepaTraceModel,
    window: &TraceWindow,
    action: &TraceAction,
) -> Result<f32> {
    let cpu = CpuJepaBackend.predict_runtime(model, window, action)?.value;
    let backend = match model.metadata.backend {
        BackendKind::Cuda => {
            CandleCudaJepaBackend
                .predict_runtime(model, window, action)?
                .value
        }
        BackendKind::Metal => {
            MlxMetalJepaBackend
                .predict_runtime(model, window, action)?
                .value
        }
        BackendKind::Auto | BackendKind::Cpu => return Ok(1.0),
    };
    crate::backend::bridge::output_cosine_parity(
        &cpu.predicted_next_state,
        &backend.predicted_next_state,
    )
}

pub fn jepa_backend_forward_parity_gate(
    model: &JepaTraceModel,
    window: &TraceWindow,
    action: &TraceAction,
    cosine_floor: f32,
) -> bool {
    jepa_backend_forward_parity(model, window, action)
        .map(|cosine| cosine >= cosine_floor)
        .unwrap_or(false)
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
            Ok(CpuJepaBackend.predict_runtime(model, window, action)?.value)
        }
        BackendKind::Cuda => CandleCudaJepaBackend
            .predict_runtime(model, window, action)
            .map(|stage| stage.value)
            .map_err(|error| anyhow::anyhow!("JepaBackendUnavailable: {error}")),
        BackendKind::Metal => MlxMetalJepaBackend
            .predict_runtime(model, window, action)
            .map(|stage| stage.value)
            .map_err(|error| anyhow::anyhow!("JepaBackendUnavailable: {error}")),
    }
}
