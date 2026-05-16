#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_encode_batch_on_device(
    encoders: &JepaEncoderSet,
    batch: &JepaFeatureBatch,
    device: &candle_core::Device,
) -> Result<JepaEncodedBatch> {
    batch.validate()?;
    (0..batch.rows)
        .map(|row| {
            Ok(EncodedJepaTrainingExample {
                context_latent: candle_project_encoder(
                    &encoders.context_encoder,
                    batch.context_feature_row(row)?,
                    device,
                )?
                .to_vec1::<f32>()?,
                action_latent: candle_project_encoder(
                    &encoders.action_encoder,
                    batch.action_feature_row(row)?,
                    device,
                )?
                .to_vec1::<f32>()?,
                target_latent: candle_project_encoder(
                    &encoders.target_encoder,
                    batch.target_feature_row(row)?,
                    device,
                )?
                .to_vec1::<f32>()?,
                horizon: batch.horizons[row],
                labels: batch.labels[row].clone(),
            })
        })
        .collect()
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_fit_predictor_on_device(
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
    device: &candle_core::Device,
) -> Result<JepaPredictor> {
    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
    let contexts = candle_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Context, device)?;
    let actions = candle_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Action, device)?;
    let targets = candle_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Target, device)?;
    let horizons = candle_horizon_column(encoded, device)?;

    let context_mean = contexts.mean(0)?;
    let action_mean = actions.mean(0)?;
    let target_mean = targets.mean(0)?;
    let horizon_mean = horizons.mean_all()?;
    let centered_contexts = contexts.broadcast_sub(&context_mean)?;
    let centered_actions = actions.broadcast_sub(&action_mean)?;
    let centered_targets = targets.broadcast_sub(&target_mean)?;
    let centered_horizons = horizons.broadcast_sub(&horizon_mean)?;
    let context_var = (centered_contexts.sqr()?.mean(0)? + 1e-6f64)?;
    let action_var = (centered_actions.sqr()?.mean(0)? + 1e-6f64)?;
    let horizon_var = (centered_horizons.sqr()?.mean_all()? + 1e-6f64)?;
    let context_weights = (centered_contexts
        .broadcast_mul(&centered_targets)?
        .mean(0)?
        / context_var)?
        .clamp(-2.0f64, 2.0f64)?;
    let action_weights = (centered_actions.broadcast_mul(&centered_targets)?.mean(0)?
        / action_var)?
        .clamp(-2.0f64, 2.0f64)?;
    let horizon_weights = centered_horizons
        .broadcast_mul(&centered_targets)?
        .mean(0)?
        .broadcast_div(&horizon_var)?
        .clamp(-2.0f64, 2.0f64)?;
    let bias = target_mean
        .broadcast_sub(&context_weights.broadcast_mul(&context_mean)?)?
        .broadcast_sub(&action_weights.broadcast_mul(&action_mean)?)?
        .broadcast_sub(&horizon_weights.broadcast_mul(&horizon_mean)?)?;

    Ok(JepaPredictor {
        latent_dim,
        context_weights: context_weights.to_vec1::<f32>()?,
        action_weights: action_weights.to_vec1::<f32>()?,
        horizon_weights: horizon_weights.to_vec1::<f32>()?,
        bias: bias.to_vec1::<f32>()?,
    })
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_fit_auxiliary_heads_on_device(
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
    device: &candle_core::Device,
) -> Result<Vec<JepaAuxiliaryHead>> {
    if encoded.is_empty() {
        return Ok(fit_auxiliary_heads(latent_dim, encoded));
    }
    let contexts = candle_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Context, device)?;
    let actions = candle_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Action, device)?;
    auxiliary_labels()
        .into_iter()
        .map(|label| {
            candle_fit_auxiliary_head_on_device(
                label, latent_dim, encoded, &contexts, &actions, device,
            )
        })
        .collect()
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_fit_auxiliary_head_on_device(
    label: &'static str,
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
    contexts: &candle_core::Tensor,
    actions: &candle_core::Tensor,
    device: &candle_core::Device,
) -> Result<JepaAuxiliaryHead> {
    let positives = encoded
        .iter()
        .filter(|example| label_value(&example.labels, label))
        .count();
    let negatives = encoded.len().saturating_sub(positives);
    let prevalence = ((positives as f32 + 1.0) / (encoded.len() as f32 + 2.0)).clamp(0.01, 0.99);
    let mask_values = encoded
        .iter()
        .map(|example| {
            if label_value(&example.labels, label) {
                1.0f32
            } else {
                0.0f32
            }
        })
        .collect::<Vec<_>>();
    let pos_mask = candle_core::Tensor::from_vec(mask_values.clone(), (encoded.len(), 1), device)?;
    let neg_mask = candle_core::Tensor::from_vec(
        mask_values
            .into_iter()
            .map(|value| 1.0f32 - value)
            .collect::<Vec<_>>(),
        (encoded.len(), 1),
        device,
    )?;
    let pos_context = candle_masked_mean(contexts, &pos_mask, positives, latent_dim)?;
    let neg_context = candle_masked_mean(contexts, &neg_mask, negatives, latent_dim)?;
    let pos_action = candle_masked_mean(actions, &pos_mask, positives, latent_dim)?;
    let neg_action = candle_masked_mean(actions, &neg_mask, negatives, latent_dim)?;
    let latent_weights = pos_context
        .broadcast_sub(&neg_context)?
        .clamp(-1.0f64, 1.0f64)?
        .affine(0.25, 0.0)?;
    let action_weights = pos_action
        .broadcast_sub(&neg_action)?
        .clamp(-1.0f64, 1.0f64)?
        .affine(0.25, 0.0)?;
    Ok(JepaAuxiliaryHead {
        label: label.to_string(),
        bias: (prevalence / (1.0 - prevalence)).ln(),
        latent_weights: latent_weights.to_vec1::<f32>()?,
        action_weights: action_weights.to_vec1::<f32>()?,
    })
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_training_losses_on_device(
    model: &JepaTraceModel,
    encoded: &JepaEncodedBatch,
    config: &JepaTrainingConfig,
    device: &candle_core::Device,
) -> Result<JepaTrainingLosses> {
    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
    let contexts = candle_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Context,
        device,
    )?;
    let actions = candle_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Action,
        device,
    )?;
    let targets = candle_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Target,
        device,
    )?;
    let predicted = candle_predict_training_targets_on_device(
        &model.predictor,
        &contexts,
        &actions,
        encoded,
        device,
    )?;
    let cosine_errors = candle_cosine_errors_on_device(&predicted, &targets)?;
    let loss_jepa = cosine_errors.iter().sum::<f32>() / cosine_errors.len().max(1) as f32;
    let loss_mse = predicted
        .broadcast_sub(&targets)?
        .sqr()?
        .mean_all()?
        .to_scalar::<f32>()?;
    let loss_aux = candle_auxiliary_brier_on_device(
        &model.auxiliary_heads,
        encoded,
        &contexts,
        &actions,
        device,
    )?;
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for (example, error) in encoded.iter().zip(&cosine_errors) {
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += *error;
        entry.1 += 1;
    }
    let loss_horizon = horizon_consistency_loss(&horizon_errors);
    let loss_var = candle_latent_variance_loss_on_device(&contexts, config.latent_var_floor)?
        .to_scalar::<f32>()?;
    let loss_total = loss_jepa
        + config.alpha_mse * loss_mse
        + config.beta_aux * loss_aux
        + config.gamma_horizon * loss_horizon
        + config.delta_var * loss_var;
    Ok(JepaTrainingLosses {
        loss_jepa,
        loss_mse,
        loss_aux,
        loss_horizon,
        loss_var,
        loss_total,
    })
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_predict_training_targets_on_device(
    predictor: &JepaPredictor,
    contexts: &candle_core::Tensor,
    actions: &candle_core::Tensor,
    encoded: &JepaEncodedBatch,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    let dim = predictor.latent_dim;
    let context_weights = candle_core::Tensor::from_slice(&predictor.context_weights, dim, device)?
        .reshape((1, dim))?;
    let action_weights = candle_core::Tensor::from_slice(&predictor.action_weights, dim, device)?
        .reshape((1, dim))?;
    let horizon_weights = candle_core::Tensor::from_slice(&predictor.horizon_weights, dim, device)?
        .reshape((1, dim))?;
    let bias = candle_core::Tensor::from_slice(&predictor.bias, dim, device)?.reshape((1, dim))?;
    let horizons = candle_horizon_column(encoded, device)?;
    let raw = contexts
        .broadcast_mul(&context_weights)?
        .broadcast_add(&actions.broadcast_mul(&action_weights)?)?
        .broadcast_add(&horizons.broadcast_mul(&horizon_weights)?)?
        .broadcast_add(&bias)?
        .tanh()?;
    candle_layer_norm_rows_and_l2_normalize(raw)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_layer_norm_rows_and_l2_normalize(
    tensor: candle_core::Tensor,
) -> Result<candle_core::Tensor> {
    let mean = tensor.mean_keepdim(1)?;
    let centered = tensor.broadcast_sub(&mean)?;
    let variance = centered.sqr()?.mean_keepdim(1)?;
    let normalized = centered.broadcast_div(&(variance + 1e-6f64)?.sqrt()?)?;
    let l2 = (normalized.sqr()?.sum_keepdim(1)? + 1e-12f64)?.sqrt()?;
    Ok(normalized.broadcast_div(&l2)?)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_cosine_errors_on_device(
    predicted: &candle_core::Tensor,
    targets: &candle_core::Tensor,
) -> Result<Vec<f32>> {
    let dot = predicted.broadcast_mul(targets)?.sum_keepdim(1)?;
    let predicted_norm = predicted.sqr()?.sum_keepdim(1)?.sqrt()?;
    let target_norm = targets.sqr()?.sum_keepdim(1)?.sqrt()?;
    let denom = (predicted_norm.broadcast_mul(&target_norm)? + 1e-12f64)?;
    let cosine = dot.broadcast_div(&denom)?;
    Ok(cosine
        .affine(-1.0, 1.0)?
        .to_vec2::<f32>()?
        .into_iter()
        .map(|row| row.first().copied().unwrap_or_default())
        .collect())
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_auxiliary_brier_on_device(
    heads: &[JepaAuxiliaryHead],
    encoded: &JepaEncodedBatch,
    contexts: &candle_core::Tensor,
    actions: &candle_core::Tensor,
    device: &candle_core::Device,
) -> Result<f32> {
    if heads.is_empty() {
        return Ok(0.0);
    }
    let rows = encoded.len();
    let dim = contexts.dim(1)?;
    let mut total = 0.0;
    for head in heads {
        let latent_weights = candle_core::Tensor::from_slice(&head.latent_weights, dim, device)?
            .reshape((1, dim))?;
        let action_weights = candle_core::Tensor::from_slice(&head.action_weights, dim, device)?
            .reshape((1, dim))?;
        let latent_score = contexts.broadcast_mul(&latent_weights)?.sum_keepdim(1)?;
        let action_score = actions.broadcast_mul(&action_weights)?.sum_keepdim(1)?;
        let scores = latent_score.broadcast_add(&action_score)?.broadcast_add(
            &candle_core::Tensor::from_vec(vec![head.bias; rows], (rows, 1), device)?,
        )?;
        let probabilities = (scores.neg()?.exp()? + 1.0f64)?.recip()?;
        let targets = candle_core::Tensor::from_vec(
            encoded
                .iter()
                .map(|example| {
                    if label_value(&example.labels, &head.label) {
                        1.0f32
                    } else {
                        0.0f32
                    }
                })
                .collect::<Vec<_>>(),
            (rows, 1),
            device,
        )?;
        total += probabilities
            .broadcast_sub(&targets)?
            .sqr()?
            .mean_all()?
            .to_scalar::<f32>()?;
    }
    Ok(total / heads.len() as f32)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_latent_variance_loss_on_device(
    contexts: &candle_core::Tensor,
    floor: f32,
) -> Result<candle_core::Tensor> {
    let mean = contexts.mean(0)?;
    let std = contexts.broadcast_sub(&mean)?.sqr()?.mean(0)?.sqrt()?;
    Ok(std
        .affine(-1.0, f64::from(floor))?
        .clamp(0.0f64, f64::MAX)?
        .mean_all()?)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_masked_mean(
    values: &candle_core::Tensor,
    mask: &candle_core::Tensor,
    count: usize,
    latent_dim: usize,
) -> Result<candle_core::Tensor> {
    if count == 0 {
        return candle_core::Tensor::from_vec(
            vec![0.0f32; latent_dim],
            latent_dim,
            values.device(),
        )
        .map_err(Into::into);
    }
    Ok(values
        .broadcast_mul(mask)?
        .sum(0)?
        .affine(1.0 / count as f64, 0.0)?)
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum EncodedLatentRole {
    Context,
    Action,
    Target,
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_encoded_matrix(
    encoded: &JepaEncodedBatch,
    latent_dim: usize,
    role: EncodedLatentRole,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
    let rows = encoded.len();
    let values = encoded
        .iter()
        .flat_map(|example| {
            let latent = match role {
                EncodedLatentRole::Context => &example.context_latent,
                EncodedLatentRole::Action => &example.action_latent,
                EncodedLatentRole::Target => &example.target_latent,
            };
            (0..latent_dim).map(move |idx| latent.get(idx).copied().unwrap_or_default())
        })
        .collect::<Vec<_>>();
    Ok(candle_core::Tensor::from_vec(
        values,
        (rows, latent_dim),
        device,
    )?)
}

#[cfg(feature = "candle")]
#[allow(dead_code)]
fn candle_horizon_column(
    encoded: &JepaEncodedBatch,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    Ok(candle_core::Tensor::from_vec(
        encoded
            .iter()
            .map(|example| normalized_horizon(example.horizon))
            .collect::<Vec<_>>(),
        (encoded.len(), 1),
        device,
    )?)
}

