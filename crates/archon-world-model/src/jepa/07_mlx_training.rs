#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_fit_predictor_on_device(
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
) -> Result<JepaPredictor> {
    use mlx_rs::{Array, Device};

    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
    Device::set_default(&Device::gpu());
    let contexts = mlx_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Context)?;
    let actions = mlx_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Action)?;
    let targets = mlx_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Target)?;
    let horizons = mlx_horizon_column(encoded);
    let context_mean = contexts.mean_axis(0, None)?;
    let action_mean = actions.mean_axis(0, None)?;
    let target_mean = targets.mean_axis(0, None)?;
    let horizon_mean = horizons.mean(None)?;
    let centered_contexts = contexts.subtract(&context_mean)?;
    let centered_actions = actions.subtract(&action_mean)?;
    let centered_targets = targets.subtract(&target_mean)?;
    let centered_horizons = horizons.subtract(&horizon_mean)?;
    let context_var = centered_contexts
        .square()?
        .mean_axis(0, None)?
        .add(&Array::from_f32(1e-6))?;
    let action_var = centered_actions
        .square()?
        .mean_axis(0, None)?
        .add(&Array::from_f32(1e-6))?;
    let horizon_var = centered_horizons
        .square()?
        .mean(None)?
        .add(&Array::from_f32(1e-6))?;
    let context_weights = mlx_clamp(
        &centered_contexts
            .multiply(&centered_targets)?
            .mean_axis(0, None)?
            .divide(&context_var)?,
        -2.0,
        2.0,
    )?;
    let action_weights = mlx_clamp(
        &centered_actions
            .multiply(&centered_targets)?
            .mean_axis(0, None)?
            .divide(&action_var)?,
        -2.0,
        2.0,
    )?;
    let horizon_weights = mlx_clamp(
        &centered_horizons
            .multiply(&centered_targets)?
            .mean_axis(0, None)?
            .divide(&horizon_var)?,
        -2.0,
        2.0,
    )?;
    let bias = target_mean
        .subtract(&context_weights.multiply(&context_mean)?)?
        .subtract(&action_weights.multiply(&action_mean)?)?
        .subtract(&horizon_weights.multiply(&horizon_mean)?)?;
    Ok(JepaPredictor {
        latent_dim,
        context_weights: mlx_to_vec(&context_weights)?,
        action_weights: mlx_to_vec(&action_weights)?,
        horizon_weights: mlx_to_vec(&horizon_weights)?,
        bias: mlx_to_vec(&bias)?,
    })
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_fit_auxiliary_heads_on_device(
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
) -> Result<Vec<JepaAuxiliaryHead>> {
    use mlx_rs::Device;

    if encoded.is_empty() {
        return Ok(fit_auxiliary_heads(latent_dim, encoded));
    }
    Device::set_default(&Device::gpu());
    let contexts = mlx_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Context)?;
    let actions = mlx_encoded_matrix(encoded, latent_dim, EncodedLatentRole::Action)?;
    auxiliary_labels()
        .into_iter()
        .map(|label| {
            mlx_fit_auxiliary_head_on_device(label, latent_dim, encoded, &contexts, &actions)
        })
        .collect()
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_fit_auxiliary_head_on_device(
    label: &'static str,
    latent_dim: usize,
    encoded: &JepaEncodedBatch,
    contexts: &mlx_rs::Array,
    actions: &mlx_rs::Array,
) -> Result<JepaAuxiliaryHead> {
    use mlx_rs::Array;

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
                1.0
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    let pos_mask = Array::from_slice(&mask_values, &[encoded.len() as i32, 1]);
    let neg_mask = Array::from_slice(
        &mask_values
            .into_iter()
            .map(|value| 1.0 - value)
            .collect::<Vec<_>>(),
        &[encoded.len() as i32, 1],
    );
    let pos_context = mlx_masked_mean(contexts, &pos_mask, positives, latent_dim)?;
    let neg_context = mlx_masked_mean(contexts, &neg_mask, negatives, latent_dim)?;
    let pos_action = mlx_masked_mean(actions, &pos_mask, positives, latent_dim)?;
    let neg_action = mlx_masked_mean(actions, &neg_mask, negatives, latent_dim)?;
    let latent_weights = mlx_affine(
        &mlx_clamp(&pos_context.subtract(&neg_context)?, -1.0, 1.0)?,
        0.25,
        0.0,
    )?;
    let action_weights = mlx_affine(
        &mlx_clamp(&pos_action.subtract(&neg_action)?, -1.0, 1.0)?,
        0.25,
        0.0,
    )?;
    Ok(JepaAuxiliaryHead {
        label: label.to_string(),
        bias: (prevalence / (1.0 - prevalence)).ln(),
        latent_weights: mlx_to_vec(&latent_weights)?,
        action_weights: mlx_to_vec(&action_weights)?,
    })
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_training_losses_on_device(
    model: &JepaTraceModel,
    encoded: &JepaEncodedBatch,
    config: &JepaTrainingConfig,
) -> Result<JepaTrainingLosses> {
    use mlx_rs::Device;

    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
    Device::set_default(&Device::gpu());
    let contexts = mlx_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Context,
    )?;
    let actions = mlx_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Action,
    )?;
    let targets = mlx_encoded_matrix(
        encoded,
        model.metadata.latent_dim,
        EncodedLatentRole::Target,
    )?;
    let predicted =
        mlx_predict_training_targets_on_device(&model.predictor, &contexts, &actions, encoded)?;
    let cosine_errors = mlx_cosine_errors_on_device(&predicted, &targets)?;
    let loss_jepa = cosine_errors.iter().sum::<f32>() / cosine_errors.len().max(1) as f32;
    let loss_mse = predicted
        .subtract(&targets)?
        .square()?
        .mean(None)?
        .item::<f32>();
    let loss_aux =
        mlx_auxiliary_brier_on_device(&model.auxiliary_heads, encoded, &contexts, &actions)?;
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for (example, error) in encoded.iter().zip(&cosine_errors) {
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += *error;
        entry.1 += 1;
    }
    let loss_horizon = horizon_consistency_loss(&horizon_errors);
    let loss_var =
        mlx_latent_variance_loss_on_device(&contexts, config.latent_var_floor)?.item::<f32>();
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

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_predict_training_targets_on_device(
    predictor: &JepaPredictor,
    contexts: &mlx_rs::Array,
    actions: &mlx_rs::Array,
    encoded: &JepaEncodedBatch,
) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;
    use mlx_rs::ops::tanh;

    let dim = predictor.latent_dim as i32;
    let context_weights = Array::from_slice(&predictor.context_weights, &[1, dim]);
    let action_weights = Array::from_slice(&predictor.action_weights, &[1, dim]);
    let horizon_weights = Array::from_slice(&predictor.horizon_weights, &[1, dim]);
    let bias = Array::from_slice(&predictor.bias, &[1, dim]);
    let horizons = mlx_horizon_column(encoded);
    let raw = contexts
        .multiply(&context_weights)?
        .add(&actions.multiply(&action_weights)?)?
        .add(&horizons.multiply(&horizon_weights)?)?
        .add(&bias)?;
    mlx_layer_norm_rows_and_l2_normalize(tanh(&raw)?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_predict_transition_array(
    transition: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    crate::backend::mlx::mlx_metal_predict_next(transition, state, action)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_predict_auxiliary_scores(
    heads: &[JepaAuxiliaryHead],
    state: &[f32],
    action: &[f32],
) -> Result<Vec<(String, f32)>> {
    use mlx_rs::Array;
    use mlx_rs::ops::sigmoid;

    let dim = state.len() as i32;
    let state = Array::from_slice(state, &[1, dim]);
    let action = Array::from_slice(action, &[1, dim]);
    heads
        .iter()
        .map(|head| {
            let latent_weights = Array::from_slice(&head.latent_weights, &[1, dim]);
            let action_weights = Array::from_slice(&head.action_weights, &[1, dim]);
            let score = state
                .multiply(&latent_weights)?
                .sum(None)?
                .add(&action.multiply(&action_weights)?.sum(None)?)?
                .add(&Array::from_f32(head.bias))?;
            let probability = sigmoid(&score)?;
            Ok((head.label.clone(), probability.item::<f32>()))
        })
        .collect()
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_cosine_errors_on_device(
    predicted: &mlx_rs::Array,
    targets: &mlx_rs::Array,
) -> Result<Vec<f32>> {
    use mlx_rs::Array;

    let dot = predicted.multiply(targets)?.sum_axis(1, Some(true))?;
    let predicted_norm = predicted
        .square()?
        .sum_axis(1, Some(true))?
        .add(&Array::from_f32(1e-12))?
        .sqrt()?;
    let target_norm = targets
        .square()?
        .sum_axis(1, Some(true))?
        .add(&Array::from_f32(1e-12))?
        .sqrt()?;
    let cosine = dot.divide(&predicted_norm.multiply(&target_norm)?)?;
    Ok(mlx_to_vec(&mlx_affine(&cosine, -1.0, 1.0)?)?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_auxiliary_brier_on_device(
    heads: &[JepaAuxiliaryHead],
    encoded: &JepaEncodedBatch,
    contexts: &mlx_rs::Array,
    actions: &mlx_rs::Array,
) -> Result<f32> {
    use mlx_rs::Array;
    use mlx_rs::ops::sigmoid;

    if heads.is_empty() {
        return Ok(0.0);
    }
    let rows = encoded.len() as i32;
    let dim = contexts.dim(1);
    let mut total = 0.0;
    for head in heads {
        let latent_weights = Array::from_slice(&head.latent_weights, &[1, dim]);
        let action_weights = Array::from_slice(&head.action_weights, &[1, dim]);
        let scores = contexts
            .multiply(&latent_weights)?
            .sum_axis(1, Some(true))?
            .add(&actions.multiply(&action_weights)?.sum_axis(1, Some(true))?)?
            .add(&Array::from_slice(
                &vec![head.bias; rows as usize],
                &[rows, 1],
            ))?;
        let probabilities = sigmoid(&scores)?;
        let targets = Array::from_slice(
            &encoded
                .iter()
                .map(|example| {
                    if label_value(&example.labels, &head.label) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect::<Vec<_>>(),
            &[rows, 1],
        );
        total += probabilities
            .subtract(&targets)?
            .square()?
            .mean(None)?
            .item::<f32>();
    }
    Ok(total / heads.len() as f32)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_latent_variance_loss_on_device(
    contexts: &mlx_rs::Array,
    floor: f32,
) -> Result<mlx_rs::Array> {
    let mean = contexts.mean_axis(0, None)?;
    let std = contexts
        .subtract(&mean)?
        .square()?
        .mean_axis(0, None)?
        .sqrt()?;
    Ok(mlx_clamp(&mlx_affine(&std, -1.0, floor)?, 0.0, f32::MAX)?.mean(None)?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_masked_mean(
    values: &mlx_rs::Array,
    mask: &mlx_rs::Array,
    count: usize,
    latent_dim: usize,
) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;

    if count == 0 {
        return Ok(Array::from_slice(
            &vec![0.0; latent_dim],
            &[latent_dim as i32],
        ));
    }
    Ok(values
        .multiply(mask)?
        .sum_axis(0, None)?
        .divide(&Array::from_f32(count as f32))?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_encoded_matrix(
    encoded: &JepaEncodedBatch,
    latent_dim: usize,
    role: EncodedLatentRole,
) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;

    if encoded.is_empty() {
        bail!("at least one JEPA example is required");
    }
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
    Ok(Array::from_slice(
        &values,
        &[encoded.len() as i32, latent_dim as i32],
    ))
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_horizon_column(encoded: &JepaEncodedBatch) -> mlx_rs::Array {
    mlx_rs::Array::from_slice(
        &encoded
            .iter()
            .map(|example| normalized_horizon(example.horizon))
            .collect::<Vec<_>>(),
        &[encoded.len() as i32, 1],
    )
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_affine(array: &mlx_rs::Array, mul: f32, add: f32) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;

    Ok(array
        .multiply(&Array::from_f32(mul))?
        .add(&Array::from_f32(add))?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_clamp(array: &mlx_rs::Array, min: f32, max: f32) -> Result<mlx_rs::Array> {
    use mlx_rs::Array;
    use mlx_rs::ops::{maximum, minimum};

    Ok(minimum(
        &maximum(array, &Array::from_f32(min))?,
        &Array::from_f32(max),
    )?)
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
#[allow(dead_code)]
fn mlx_to_vec(array: &mlx_rs::Array) -> Result<Vec<f32>> {
    array.eval()?;
    Ok(array.try_as_slice::<f32>()?.to_vec())
}
