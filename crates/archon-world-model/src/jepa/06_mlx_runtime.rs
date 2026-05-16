#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_jepa_predict_runtime_on_device(
    model: &JepaTraceModel,
    window: &TraceWindow,
    action: &TraceAction,
) -> Result<JepaRuntimePrediction> {
    let started = Instant::now();
    let transition = model
        .transition_model
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("JepaCheckpointMissing: transition model missing"))?;
    let state = mlx_encode_window_array(&model.context_encoder, window)?;
    let action = mlx_encode_action_array(&model.action_encoder, action)?;
    let predicted_next_state = mlx_predict_transition_array(transition, &state, &action)?;
    let auxiliary_scores = mlx_predict_auxiliary_scores(&model.auxiliary_heads, &state, &action)?;
    let latency_ms = started.elapsed().as_millis() as u64;
    Ok(JepaRuntimePrediction {
        backend: BackendKind::Metal,
        predicted_next_state,
        guardrail_scores: jepa_guardrail_scores_from_auxiliary(&auxiliary_scores),
        auxiliary_scores,
        latency_ms,
        execution_report: JepaRuntimeBackendReport::new(
            BackendKind::Metal,
            "mlx-rs",
            observed_backend_device_name(BackendKind::Metal),
            true,
            latency_ms,
        ),
    })
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_encode_batch_on_device(
    encoders: &JepaEncoderSet,
    batch: &JepaFeatureBatch,
) -> Result<JepaEncodedBatch> {
    use mlx_rs::Device;

    Device::set_default(&Device::gpu());
    batch.validate()?;
    (0..batch.rows)
        .map(|row| {
            Ok(EncodedJepaTrainingExample {
                context_latent: mlx_project_encoder(
                    &encoders.context_encoder,
                    batch.context_feature_row(row)?,
                )?,
                action_latent: mlx_project_encoder(
                    &encoders.action_encoder,
                    batch.action_feature_row(row)?,
                )?,
                target_latent: mlx_project_encoder(
                    &encoders.target_encoder,
                    batch.target_feature_row(row)?,
                )?,
                horizon: batch.horizons[row],
                labels: batch.labels[row].clone(),
            })
        })
        .collect()
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_encode_window_array(encoder: &JepaTraceEncoder, window: &TraceWindow) -> Result<Vec<f32>> {
    mlx_project_encoder(
        encoder,
        &window_features(window, encoder.latent_dim, &encoder.role)?,
    )
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_encode_action_array(encoder: &JepaTraceEncoder, action: &TraceAction) -> Result<Vec<f32>> {
    mlx_project_encoder(
        encoder,
        &action_features(action, encoder.latent_dim, &encoder.role)?,
    )
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_project_encoder(encoder: &JepaTraceEncoder, features: &[f32]) -> Result<Vec<f32>> {
    use mlx_rs::nn::gelu_approximate;
    use mlx_rs::{Array, Device};

    if features.len() != encoder.latent_dim {
        bail!("jepa feature dimension mismatch");
    }
    Device::set_default(&Device::gpu());
    let dim = encoder.latent_dim as i32;
    let features = Array::from_slice(features, &[dim]);
    let input_weights = Array::from_slice(&encoder.input_weights, &[dim]);
    let hidden_bias = Array::from_slice(&encoder.hidden_bias, &[dim]);
    let output_weights = Array::from_slice(&encoder.output_weights, &[dim]);
    let output_bias = Array::from_slice(&encoder.output_bias, &[dim]);
    let residual = Array::from_slice(&vec![encoder.residual_weight; encoder.latent_dim], &[dim]);
    let hidden_scale = Array::from_slice(
        &vec![1.0 - encoder.residual_weight; encoder.latent_dim],
        &[dim],
    );
    let hidden = gelu_approximate(features.multiply(&input_weights)?.add(&hidden_bias)?)?;
    let projected = hidden.multiply(&output_weights)?.add(&output_bias)?;
    let output = features
        .multiply(&residual)?
        .add(&projected.multiply(&hidden_scale)?)?;
    let output = mlx_layer_norm_and_l2_normalize(output)?;
    output.eval()?;
    Ok(output.try_as_slice::<f32>()?.to_vec())
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_layer_norm_and_l2_normalize(tensor: mlx_rs::Array) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;

    let mean = tensor.mean(None)?;
    let centered = tensor.subtract(&mean)?;
    let variance = centered.square()?.mean(None)?;
    let normalized = centered.divide(&variance.add(&Array::from_f32(1e-6))?.sqrt()?)?;
    let l2 = normalized
        .square()?
        .sum(None)?
        .add(&Array::from_f32(1e-12))?
        .sqrt()?;
    Ok(normalized.divide(&l2)?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_layer_norm_rows_and_l2_normalize(tensor: mlx_rs::Array) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;

    let mean = tensor.mean_axis(1, Some(true))?;
    let centered = tensor.subtract(&mean)?;
    let variance = centered.square()?.mean_axis(1, Some(true))?;
    let normalized = centered.divide(&variance.add(&Array::from_f32(1e-6))?.sqrt()?)?;
    let l2 = normalized
        .square()?
        .sum_axis(1, Some(true))?
        .add(&Array::from_f32(1e-12))?
        .sqrt()?;
    Ok(normalized.divide(&l2)?)
}
