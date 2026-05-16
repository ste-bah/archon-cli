fn predict_with_checkpoint(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    model_id: &str,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<PredictionInference> {
    let registry = ModelRegistry::open(root)?;
    let active_kind = registry
        .active_model_kind()?
        .unwrap_or_else(|| LATENT_TRANSITION_MODEL_KIND.into());
    if config.learning.world_model.model_kind == JEPA_MODEL_KIND {
        if active_kind != JEPA_MODEL_KIND {
            anyhow::bail!("JepaCheckpointMissing: active model is not a JEPA checkpoint");
        }
        return predict_with_jepa_checkpoint(
            config, &registry, model_id, session_id, action_ref, summary,
        );
    }

    let candidate = registry.load_cpu_candidate(model_id)?;
    let adapter = build_embedding_adapter(config)?;
    let state = embed(adapter.as_ref(), model_id, summary)?;
    let action = embed(adapter.as_ref(), model_id, &format!("action={summary}"))?;
    let next = archon_world_model::backend::predict_next_with_backend(
        &candidate.model,
        &state,
        &action,
        candidate.model.metadata.backend,
    )?;
    let guardrail_scores = Some(guardrail_scores_from_auxiliary(
        candidate
            .model
            .predict_auxiliary(&state, &action)?
            .iter()
            .map(|prediction| (prediction.label.as_str(), prediction.probability)),
    ));
    Ok(PredictionInference {
        summary: format!(
            "next-state dim={} norm={:.4}",
            next.len(),
            vector_norm(&next)
        ),
        vector: next,
        model_kind: LATENT_TRANSITION_MODEL_KIND.into(),
        representation_source: format!("{}:{}", adapter.provider_name(), adapter.model_name()),
        guardrail_scores,
        jepa_runtime_backend_report: None,
    })
}

fn predict_with_jepa_checkpoint(
    config: &archon_core::config::ArchonConfig,
    registry: &ModelRegistry,
    model_id: &str,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<PredictionInference> {
    let started = Instant::now();
    let candidate = registry
        .load_jepa_candidate(model_id)
        .map_err(|error| anyhow::anyhow!("JepaCheckpointMissing: {error}"))?;
    candidate
        .model
        .validate_finite()
        .map_err(|error| anyhow::anyhow!("JepaCheckpointInvalid: {error}"))?;
    archon_world_model::jepa::validate_jepa_backend_execution(&candidate.model.metadata)
        .map_err(|error| anyhow::anyhow!("JepaBackendUnavailable: {error}"))?;
    if candidate.model.metadata.latent_dim != config.learning.world_model.state_dim {
        anyhow::bail!(
            "JepaDimensionMismatch: expected {}, got {}",
            config.learning.world_model.state_dim,
            candidate.model.metadata.latent_dim
        );
    }
    let (window, action) = synthetic_runtime_window(session_id, action_ref, summary)?;
    let runtime = archon_world_model::jepa::predict_jepa_with_backend(
        &candidate.model,
        &window,
        &action,
        candidate.model.metadata.backend,
    )?;
    let guardrail_scores = Some(runtime.guardrail_scores);
    let runtime_backend_report = runtime.execution_report.clone();
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let (measured_latency_ms, latency_cap_ms) = jepa_prediction_latency_budget(
        config,
        candidate.model.metadata.backend,
        runtime.latency_ms,
        elapsed_ms,
    );
    if measured_latency_ms > latency_cap_ms {
        anyhow::bail!(
            "JepaLatencyExceeded: prediction took {measured_latency_ms}ms, cap={latency_cap_ms}ms"
        );
    }
    Ok(PredictionInference {
        summary: format!(
            "next-state dim={} norm={:.4}",
            runtime.predicted_next_state.len(),
            vector_norm(&runtime.predicted_next_state)
        ),
        vector: runtime.predicted_next_state,
        model_kind: JEPA_MODEL_KIND.into(),
        representation_source: format!("archon-jepa:{}", candidate.model.metadata.model_id),
        guardrail_scores,
        jepa_runtime_backend_report: Some(runtime_backend_report),
    })
}

pub(crate) fn jepa_prediction_latency_budget(
    config: &archon_core::config::ArchonConfig,
    backend: archon_world_model::BackendKind,
    runtime_latency_ms: u64,
    elapsed_ms: u64,
) -> (u64, u64) {
    if matches!(
        backend,
        archon_world_model::BackendKind::Cuda | archon_world_model::BackendKind::Metal
    ) {
        (
            runtime_latency_ms,
            config
                .learning
                .world_model
                .jepa
                .max_backend_prediction_latency_ms,
        )
    } else {
        (
            elapsed_ms,
            config.learning.world_model.jepa.max_prediction_latency_ms,
        )
    }
}

fn encode_jepa_actual_outcome(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    prediction: &PersistedPrediction,
    actual_summary: &str,
) -> anyhow::Result<Vec<f32>> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry
        .load_jepa_candidate(&prediction.model_id)
        .map_err(|error| anyhow::anyhow!("JepaCheckpointMissing: {error}"))?;
    if candidate.model.metadata.latent_dim != config.learning.world_model.state_dim {
        anyhow::bail!(
            "JepaDimensionMismatch: expected {}, got {}",
            config.learning.world_model.state_dim,
            candidate.model.metadata.latent_dim
        );
    }
    let (window, _) = synthetic_runtime_window(
        &prediction.session_id,
        &format!("{}-actual", prediction.action_ref),
        actual_summary,
    )?;
    candidate
        .model
        .encode_target(&window)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))
}

fn synthetic_runtime_window(
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> anyhow::Result<(archon_world_model::TraceWindow, TraceAction)> {
    let mut row =
        WorldTraceRow::new(session_id, WorldActionKind::AgentAttempt).with_row_id(action_ref);
    row.redacted_excerpt = Some(summary.to_string());
    row.created_at = Utc::now();
    let action = TraceAction::from_row(&row);
    let builder = TraceWindowBuilder::new(&[row]);
    let window = builder
        .context_window(action_ref, 1)
        .map_err(|error| anyhow::anyhow!("JepaEncoderFailed: {error}"))?;
    Ok((window, action))
}

fn unavailable_reason_from_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    for reason in [
        "JepaCheckpointMissing",
        "JepaCheckpointInvalid",
        "JepaEncoderFailed",
        "JepaDimensionMismatch",
        "JepaLatencyExceeded",
        "JepaBackendUnavailable",
        "JepaBackendProbeFailed",
        "JepaBackendNativeStageFailed",
        "JepaBackendHostFallbackRejected",
        "JepaBackendParityFailed",
        "JepaBackendHardwareValidationMissing",
    ] {
        if message.contains(reason) {
            return reason;
        }
    }
    "StoreUnavailable"
}

fn embed(
    adapter: &dyn WorldEmbeddingAdapter,
    source_hash: &str,
    text: &str,
) -> anyhow::Result<Vec<f32>> {
    Ok(adapter
        .embed(&EmbeddingRequest {
            text: text.to_string(),
            source_hash: source_hash.to_string(),
            redaction_policy: "world-model-default-redacted".into(),
        })?
        .values)
}

fn vector_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn guardrail_scores_from_auxiliary<'a>(
    predictions: impl IntoIterator<Item = (&'a str, f32)>,
) -> GuardrailRiskScores {
    let mut scores = GuardrailRiskScores::default();
    for (label, probability) in predictions {
        let probability = finite_probability(probability);
        match label {
            "failure" => scores.predicted_failure = probability,
            "retry" => scores.predicted_retry = probability,
            "provider_incident" => scores.predicted_provider_incident = probability,
            "verification_needed" => scores.predicted_verification_needed = probability,
            "user_correction" => scores.predicted_user_correction = probability,
            "plan_drift" => scores.predicted_plan_drift = probability,
            "high_cost" => scores.predicted_high_cost = probability,
            "slow_run" => scores.predicted_slow_run = probability,
            _ => {}
        }
    }
    scores
}

fn finite_probability(probability: f32) -> Option<f32> {
    probability.is_finite().then(|| probability.clamp(0.0, 1.0))
}

fn jepa_runtime_backend_report_lines(prediction: &PersistedPrediction) -> String {
    let Some(report) = &prediction.jepa_runtime_backend_report else {
        return String::new();
    };
    format!(
        "Runtime backend: {}\n\
             Runtime framework: {}\n\
             Runtime device: {}\n\
             Runtime native prediction: {}\n\
             Runtime host fallback count: {}\n",
        report.backend,
        report.framework,
        report.device_name.as_deref().unwrap_or("unknown"),
        report.native_runtime_prediction,
        report.host_fallback_count
    )
}

fn cosine_error(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = vector_norm(left);
    let right_norm = vector_norm(right);
    if left_norm == 0.0 || right_norm == 0.0 {
        1.0
    } else {
        1.0 - (dot / (left_norm * right_norm)).clamp(-1.0, 1.0)
    }
}
