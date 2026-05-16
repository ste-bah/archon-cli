fn fit_auxiliary_heads(
    latent_dim: usize,
    examples: &[EncodedJepaTrainingExample],
) -> Vec<JepaAuxiliaryHead> {
    auxiliary_labels()
        .into_iter()
        .map(|label| fit_auxiliary_head(label, latent_dim, examples))
        .collect()
}

fn fit_auxiliary_head(
    label: &'static str,
    latent_dim: usize,
    examples: &[EncodedJepaTrainingExample],
) -> JepaAuxiliaryHead {
    let positives = examples
        .iter()
        .filter(|example| label_value(&example.labels, label))
        .count() as f32;
    let prevalence = ((positives + 1.0) / (examples.len() as f32 + 2.0)).clamp(0.01, 0.99);
    let mut pos_context = vec![0.0; latent_dim];
    let mut neg_context = vec![0.0; latent_dim];
    let mut pos_action = vec![0.0; latent_dim];
    let mut neg_action = vec![0.0; latent_dim];
    let mut pos_count: f32 = 0.0;
    let mut neg_count: f32 = 0.0;
    for example in examples {
        let (context_target, action_target, count) = if label_value(&example.labels, label) {
            (&mut pos_context, &mut pos_action, &mut pos_count)
        } else {
            (&mut neg_context, &mut neg_action, &mut neg_count)
        };
        *count += 1.0;
        for idx in 0..latent_dim {
            context_target[idx] += example.context_latent[idx];
            action_target[idx] += example.action_latent[idx];
        }
    }
    normalize_mean(&mut pos_context, pos_count);
    normalize_mean(&mut neg_context, neg_count);
    normalize_mean(&mut pos_action, pos_count);
    normalize_mean(&mut neg_action, neg_count);
    JepaAuxiliaryHead {
        label: label.to_string(),
        bias: (prevalence / (1.0 - prevalence)).ln(),
        latent_weights: pos_context
            .iter()
            .zip(&neg_context)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
        action_weights: pos_action
            .iter()
            .zip(&neg_action)
            .map(|(pos, neg)| (pos - neg).clamp(-1.0, 1.0) * 0.25)
            .collect(),
    }
}

fn auxiliary_brier(model: &JepaTraceModel, examples: &[EncodedJepaTrainingExample]) -> f32 {
    let mut total = 0.0;
    let mut count = 0.0;
    for example in examples {
        for head in &model.auxiliary_heads {
            let target = if label_value(&example.labels, &head.label) {
                1.0
            } else {
                0.0
            };
            let probability =
                head.predict_probability(&example.context_latent, &example.action_latent);
            total += (probability - target).powi(2);
            count += 1.0;
        }
    }
    if count == 0.0 { 0.0 } else { total / count }
}

fn horizon_consistency_loss(errors: &BTreeMap<usize, (f32, usize)>) -> f32 {
    let mean = |horizon: usize| {
        errors.get(&horizon).and_then(|(sum, count)| {
            if *count == 0 {
                None
            } else {
                Some(*sum / *count as f32)
            }
        })
    };
    match (mean(1), mean(3), mean(5)) {
        (Some(e1), Some(e3), Some(e5)) => (e1 - e3).max(0.0) + (e3 - e5).max(0.0),
        _ => 0.0,
    }
}

fn latent_variance_loss(latents: &[&[f32]], floor: f32) -> f32 {
    if latents.is_empty() || latents[0].is_empty() {
        return 0.0;
    }
    let dim = latents[0].len();
    let mut total = 0.0;
    for idx in 0..dim {
        let mean = latents.iter().map(|latent| latent[idx]).sum::<f32>() / latents.len() as f32;
        let variance = latents
            .iter()
            .map(|latent| (latent[idx] - mean).powi(2))
            .sum::<f32>()
            / latents.len() as f32;
        total += (floor - variance.sqrt()).max(0.0);
    }
    total / dim as f32
}

#[derive(Debug, Clone, Copy)]
enum InputRole {
    Context,
    Action,
    Horizon,
}

fn covariance_weight(
    examples: &[EncodedJepaTrainingExample],
    idx: usize,
    input_mean: f32,
    target_mean: f32,
    role: InputRole,
) -> f32 {
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for example in examples {
        let input = match role {
            InputRole::Context => example.context_latent[idx],
            InputRole::Action => example.action_latent[idx],
            InputRole::Horizon => normalized_horizon(example.horizon),
        };
        numerator += (input - input_mean) * (example.target_latent[idx] - target_mean);
        denominator += (input - input_mean).powi(2);
    }
    if denominator <= f32::EPSILON {
        match role {
            InputRole::Context => 1.0,
            InputRole::Action | InputRole::Horizon => 0.0,
        }
    } else {
        (numerator / denominator).clamp(-2.0, 2.0)
    }
}

fn jepa_checkpoint_tensors(model: &JepaTraceModel) -> JepaCheckpointTensors {
    let auxiliary_bias = model
        .auxiliary_heads
        .iter()
        .map(|head| head.bias)
        .collect::<Vec<_>>();
    let auxiliary_latent_weights = model
        .auxiliary_heads
        .iter()
        .flat_map(|head| head.latent_weights.clone())
        .collect::<Vec<_>>();
    let auxiliary_action_weights = model
        .auxiliary_heads
        .iter()
        .flat_map(|head| head.action_weights.clone())
        .collect::<Vec<_>>();
    JepaCheckpointTensors {
        context_input_weights: model.context_encoder.input_weights.clone(),
        context_hidden_bias: model.context_encoder.hidden_bias.clone(),
        context_output_weights: model.context_encoder.output_weights.clone(),
        context_output_bias: model.context_encoder.output_bias.clone(),
        action_input_weights: model.action_encoder.input_weights.clone(),
        action_hidden_bias: model.action_encoder.hidden_bias.clone(),
        action_output_weights: model.action_encoder.output_weights.clone(),
        action_output_bias: model.action_encoder.output_bias.clone(),
        target_input_weights: model.target_encoder.input_weights.clone(),
        target_hidden_bias: model.target_encoder.hidden_bias.clone(),
        target_output_weights: model.target_encoder.output_weights.clone(),
        target_output_bias: model.target_encoder.output_bias.clone(),
        predictor_context_weights: model.predictor.context_weights.clone(),
        predictor_action_weights: model.predictor.action_weights.clone(),
        predictor_horizon_weights: model.predictor.horizon_weights.clone(),
        predictor_bias: model.predictor.bias.clone(),
        auxiliary_bias,
        auxiliary_latent_weights,
        auxiliary_action_weights,
    }
}

fn validate_latents(latent_dim: usize, example: &EncodedJepaTrainingExample) -> Result<()> {
    if example.context_latent.len() != latent_dim
        || example.action_latent.len() != latent_dim
        || example.target_latent.len() != latent_dim
    {
        bail!("jepa training example latent dimensions must match latent_dim");
    }
    Ok(())
}

fn mse(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("mse inputs must have matching dimensions");
    }
    Ok(left
        .iter()
        .zip(right)
        .map(|(left, right)| (left - right).powi(2))
        .sum::<f32>()
        / left.len().max(1) as f32)
}

fn cosine_error(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("cosine inputs must have matching dimensions");
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(1.0)
    } else {
        Ok(1.0 - (dot / (left_norm * right_norm)).clamp(-1.0, 1.0))
    }
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}

fn layer_norm(values: &mut [f32]) {
    if values.is_empty() {
        return;
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f32>()
        / values.len() as f32;
    let denom = (variance + 1e-6).sqrt();
    for value in &mut *values {
        *value = (*value - mean) / denom;
    }
    normalize(values);
}

fn normalize_mean(values: &mut [f32], count: f32) {
    if count > 0.0 {
        for value in values {
            *value /= count;
        }
    }
}

fn normalize_count(value: usize) -> f32 {
    (value as f32 / 16.0).clamp(0.0, 8.0)
}

fn normalized_horizon(horizon: usize) -> f32 {
    (horizon as f32 / 5.0).clamp(0.0, 4.0)
}

fn gelu(value: f32) -> f32 {
    0.5 * value * (1.0 + (0.797_884_6 * (value + 0.044_715 * value.powi(3))).tanh())
}

fn dot_prefix(weights: &[f32], values: &[f32]) -> f32 {
    weights
        .iter()
        .zip(values)
        .map(|(weight, value)| weight * value)
        .sum()
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-40.0, 40.0)).exp())
}

fn auxiliary_labels() -> Vec<&'static str> {
    vec![
        "failure",
        "retry",
        "provider_incident",
        "verification_needed",
        "user_correction",
        "plan_drift",
        "high_cost",
        "slow_run",
    ]
}

fn label_value(labels: &WorldLabelSet, label: &str) -> bool {
    match label {
        "failure" => labels.failure,
        "retry" => labels.retry,
        "provider_incident" => labels.provider_incident,
        "verification_needed" => labels.verification_needed,
        "user_correction" => labels.user_correction,
        "plan_drift" => labels.plan_drift,
        "high_cost" => labels.high_cost,
        "slow_run" => labels.slow_run,
        _ => false,
    }
}

fn jepa_guardrail_scores_from_auxiliary(scores: &[(String, f32)]) -> GuardrailRiskScores {
    let mut guardrail_scores = GuardrailRiskScores::default();
    for (label, probability) in scores {
        let probability = probability.is_finite().then(|| probability.clamp(0.0, 1.0));
        match label.as_str() {
            "failure" => guardrail_scores.predicted_failure = probability,
            "retry" => guardrail_scores.predicted_retry = probability,
            "provider_incident" => guardrail_scores.predicted_provider_incident = probability,
            "verification_needed" => guardrail_scores.predicted_verification_needed = probability,
            "user_correction" => guardrail_scores.predicted_user_correction = probability,
            "plan_drift" => guardrail_scores.predicted_plan_drift = probability,
            "high_cost" => guardrail_scores.predicted_high_cost = probability,
            "slow_run" => guardrail_scores.predicted_slow_run = probability,
            _ => {}
        }
    }
    guardrail_scores
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn tensor_f32(tensors: &safetensors::SafeTensors<'_>, name: &str) -> Result<Vec<f32>> {
    let tensor = tensors.tensor(name)?;
    Ok(tensor
        .data()
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

