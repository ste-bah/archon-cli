#[cfg(feature = "cuda")]
fn cuda_jepa_device() -> Result<candle_core::Device> {
    static CUDA_JEPA_DEVICE: std::sync::OnceLock<std::result::Result<candle_core::Device, String>> =
        std::sync::OnceLock::new();
    CUDA_JEPA_DEVICE
        .get_or_init(|| candle_core::Device::new_cuda(0).map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| anyhow::anyhow!("cuda_device_unavailable:{error}"))
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_jepa_predict_runtime_on_device(
    model: &JepaTraceModel,
    window: &TraceWindow,
    action: &TraceAction,
    backend: BackendKind,
    device: &candle_core::Device,
) -> Result<JepaRuntimePrediction> {
    let started = Instant::now();
    let transition = model
        .transition_model
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("JepaCheckpointMissing: transition model missing"))?;
    let state = candle_encode_window_tensor(&model.context_encoder, window, device)?;
    let action = candle_encode_action_tensor(&model.action_encoder, action, device)?;
    let predicted_next_state =
        candle_predict_transition_tensor(transition, &state, &action, device)?.to_vec1::<f32>()?;
    let auxiliary_scores =
        candle_predict_auxiliary_scores(&model.auxiliary_heads, &state, &action, device)?;
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(JepaRuntimePrediction {
        backend,
        predicted_next_state,
        guardrail_scores: jepa_guardrail_scores_from_auxiliary(&auxiliary_scores),
        auxiliary_scores,
        latency_ms,
        execution_report: JepaRuntimeBackendReport::new(
            backend,
            "candle",
            observed_backend_device_name(backend),
            true,
            latency_ms,
        ),
    })
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_encode_window_tensor(
    encoder: &JepaTraceEncoder,
    window: &TraceWindow,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    let features = window_features(window, encoder.latent_dim, &encoder.role)?;
    candle_project_encoder(encoder, &features, device)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_encode_action_tensor(
    encoder: &JepaTraceEncoder,
    action: &TraceAction,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    let features = action_features(action, encoder.latent_dim, &encoder.role)?;
    candle_project_encoder(encoder, &features, device)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_project_encoder(
    encoder: &JepaTraceEncoder,
    features: &[f32],
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    if features.len() != encoder.latent_dim {
        bail!("jepa feature dimension mismatch");
    }
    let dim = encoder.latent_dim;
    let features = candle_core::Tensor::from_slice(features, dim, device)?;
    let input_weights = candle_core::Tensor::from_slice(&encoder.input_weights, dim, device)?;
    let hidden_bias = candle_core::Tensor::from_slice(&encoder.hidden_bias, dim, device)?;
    let output_weights = candle_core::Tensor::from_slice(&encoder.output_weights, dim, device)?;
    let output_bias = candle_core::Tensor::from_slice(&encoder.output_bias, dim, device)?;
    let residual = candle_core::Tensor::from_vec(vec![encoder.residual_weight; dim], dim, device)?;
    let hidden_scale =
        candle_core::Tensor::from_vec(vec![1.0 - encoder.residual_weight; dim], dim, device)?;

    let hidden = features
        .broadcast_mul(&input_weights)?
        .broadcast_add(&hidden_bias)?
        .gelu()?;
    let projected = hidden
        .broadcast_mul(&output_weights)?
        .broadcast_add(&output_bias)?;
    let output = features
        .broadcast_mul(&residual)?
        .broadcast_add(&projected.broadcast_mul(&hidden_scale)?)?;
    candle_layer_norm_and_l2_normalize(output)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_layer_norm_and_l2_normalize(tensor: candle_core::Tensor) -> Result<candle_core::Tensor> {
    let mean = tensor.mean_all()?;
    let centered = tensor.broadcast_sub(&mean)?;
    let variance = centered.sqr()?.mean_all()?;
    let denom = (variance + 1e-6f64)?.sqrt()?;
    let normalized = centered.broadcast_div(&denom)?;
    let l2 = (normalized.sqr()?.sum_all()? + 1e-12f64)?.sqrt()?;
    Ok(normalized.broadcast_div(&l2)?)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_predict_transition_tensor(
    transition: &CpuLatentTransitionModel,
    state: &candle_core::Tensor,
    action: &candle_core::Tensor,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    let state_dim = transition.metadata.state_dim;
    if state.elem_count() != state_dim || action.elem_count() != state_dim {
        bail!("jepa runtime tensor dimensions must match transition state_dim");
    }
    let state_weights =
        candle_core::Tensor::from_slice(&transition.state_weights, state_dim, device)?;
    let action_weights =
        candle_core::Tensor::from_slice(&transition.action_weights, state_dim, device)?;
    let bias = candle_core::Tensor::from_slice(&transition.transition_bias, state_dim, device)?;
    Ok(state
        .broadcast_mul(&state_weights)?
        .broadcast_add(&action.broadcast_mul(&action_weights)?)?
        .broadcast_add(&bias)?)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_predict_auxiliary_scores(
    heads: &[JepaAuxiliaryHead],
    state: &candle_core::Tensor,
    action: &candle_core::Tensor,
    device: &candle_core::Device,
) -> Result<Vec<(String, f32)>> {
    let dim = state.elem_count();
    if action.elem_count() != dim {
        bail!("jepa runtime auxiliary tensor dimensions must match");
    }
    heads
        .iter()
        .map(|head| {
            let latent_weights =
                candle_core::Tensor::from_slice(&head.latent_weights, dim, device)?;
            let action_weights =
                candle_core::Tensor::from_slice(&head.action_weights, dim, device)?;
            let latent = state
                .broadcast_mul(&latent_weights)?
                .sum_all()?
                .to_scalar::<f32>()?;
            let action = action
                .broadcast_mul(&action_weights)?
                .sum_all()?
                .to_scalar::<f32>()?;
            Ok((head.label.clone(), sigmoid(head.bias + latent + action)))
        })
        .collect()
}
