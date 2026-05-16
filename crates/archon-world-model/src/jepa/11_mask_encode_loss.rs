fn mask_jepa_training_example(
    example: &JepaTrainingExample,
    mask_ratio: f32,
    report: &mut JepaMaskingReport,
) -> JepaTrainingExample {
    let mut masked = example.clone();
    for row in &mut masked.context.rows {
        let prefix = format!("{}:{}:{}", row.session_id, row.row_id, example.horizon);
        if should_mask(&prefix, "excerpt", mask_ratio) {
            row.redacted_excerpt = Some("[MASKED_EXCERPT]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "action_kind", mask_ratio) {
            row.action_kind = crate::schema::WorldActionKind::Unknown;
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "provider", mask_ratio) {
            row.provider = Some("[MASKED_PROVIDER]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "model", mask_ratio) {
            row.model = Some("[MASKED_MODEL]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "agent", mask_ratio) {
            row.agent = Some("[MASKED_AGENT]".into());
            report.masked_context_fields += 1;
        }
        if should_mask(&prefix, "scalar", mask_ratio) {
            row.scalar_features = ScalarFeatures::default();
            report.masked_context_fields += 1;
        }
    }

    let prefix = format!("action:{}:{}", masked.action.action_ref, example.horizon);
    if should_mask(&prefix, "summary", mask_ratio) {
        masked.action.summary = "[MASKED_EXCERPT]".into();
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "action_kind", mask_ratio) {
        masked.action.action_kind = crate::schema::WorldActionKind::Unknown;
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "provider", mask_ratio) {
        masked.action.provider = Some("[MASKED_PROVIDER]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "model", mask_ratio) {
        masked.action.model = Some("[MASKED_MODEL]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "agent", mask_ratio) {
        masked.action.agent = Some("[MASKED_AGENT]".into());
        report.masked_action_fields += 1;
    }
    if should_mask(&prefix, "scalar", mask_ratio) {
        masked.action.scalar_features = ScalarFeatures::default();
        report.masked_action_fields += 1;
    }
    masked
}

fn should_mask(prefix: &str, field: &str, mask_ratio: f32) -> bool {
    if mask_ratio <= 0.0 {
        return false;
    }
    if mask_ratio >= 1.0 {
        return true;
    }
    let mut hasher = DefaultHasher::new();
    prefix.hash(&mut hasher);
    field.hash(&mut hasher);
    let unit = (hasher.finish() % 10_000) as f32 / 10_000.0;
    unit < mask_ratio
}

fn encode_examples(
    context_encoder: &JepaTraceEncoder,
    action_encoder: &JepaTraceEncoder,
    target_encoder: &JepaTraceEncoder,
    batch: &JepaFeatureBatch,
) -> Result<Vec<EncodedJepaTrainingExample>> {
    batch.validate()?;
    (0..batch.rows)
        .map(|row| {
            Ok(EncodedJepaTrainingExample {
                context_latent: context_encoder
                    .project(batch.context_feature_row(row)?.to_vec())?,
                action_latent: action_encoder.project(batch.action_feature_row(row)?.to_vec())?,
                target_latent: target_encoder.project(batch.target_feature_row(row)?.to_vec())?,
                horizon: batch.horizons[row],
                labels: batch.labels[row].clone(),
            })
        })
        .collect()
}

fn encoded_transition_examples(
    examples: &[EncodedJepaTrainingExample],
) -> Vec<LatentTransitionExample> {
    examples
        .iter()
        .map(|example| LatentTransitionExample {
            state: example.context_latent.clone(),
            action: example.action_latent.clone(),
            next_state: example.target_latent.clone(),
            labels: example.labels.clone(),
        })
        .collect()
}

fn transition_model_finite(model: &CpuLatentTransitionModel) -> bool {
    model.state_weights.iter().all(|value| value.is_finite())
        && model.action_weights.iter().all(|value| value.is_finite())
        && model.transition_bias.iter().all(|value| value.is_finite())
        && model.mean_delta.iter().all(|value| value.is_finite())
        && model.auxiliary_heads.iter().all(|head| {
            head.bias.is_finite()
                && head.state_weights.iter().all(|value| value.is_finite())
                && head.action_weights.iter().all(|value| value.is_finite())
        })
}

fn training_losses(
    model: &JepaTraceModel,
    examples: &[EncodedJepaTrainingExample],
    config: &JepaTrainingConfig,
) -> Result<JepaTrainingLosses> {
    let mut loss_jepa = 0.0;
    let mut loss_mse = 0.0;
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for example in examples {
        let predicted = model.predict_training_target(
            &example.context_latent,
            &example.action_latent,
            example.horizon,
        )?;
        let cosine = cosine_error(&predicted, &example.target_latent)?;
        loss_jepa += cosine;
        loss_mse += mse(&predicted, &example.target_latent)?;
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += cosine;
        entry.1 += 1;
    }
    let denom = examples.len().max(1) as f32;
    loss_jepa /= denom;
    loss_mse /= denom;
    let loss_aux = auxiliary_brier(model, examples);
    let loss_horizon = horizon_consistency_loss(&horizon_errors);
    let loss_var = latent_variance_loss(
        examples
            .iter()
            .map(|example| example.context_latent.as_slice())
            .collect::<Vec<_>>()
            .as_slice(),
        config.latent_var_floor,
    );
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

fn horizon_report_for_model(
    model: &JepaTraceModel,
    examples: &[EncodedJepaTrainingExample],
    tolerance: f32,
) -> Result<JepaHorizonReport> {
    let mut horizon_errors: BTreeMap<usize, (f32, usize)> = BTreeMap::new();
    for example in examples {
        let predicted = model.predict_training_target(
            &example.context_latent,
            &example.action_latent,
            example.horizon,
        )?;
        let cosine = cosine_error(&predicted, &example.target_latent)?;
        let entry = horizon_errors.entry(example.horizon).or_default();
        entry.0 += cosine;
        entry.1 += 1;
    }
    Ok(horizon_report_from_errors(&horizon_errors, tolerance))
}

fn horizon_report_from_errors(
    errors: &BTreeMap<usize, (f32, usize)>,
    tolerance: f32,
) -> JepaHorizonReport {
    let mean = |horizon: usize| {
        errors.get(&horizon).and_then(|(sum, count)| {
            if *count == 0 {
                None
            } else {
                Some(*sum / *count as f32)
            }
        })
    };
    let e_1 = mean(1);
    let e_3 = mean(3);
    let e_5 = mean(5);
    let passes = match (e_1, e_3, e_5) {
        (Some(e1), Some(e3), Some(e5)) => {
            [e1, e3, e5, tolerance]
                .into_iter()
                .all(|value| value.is_finite())
                && e1 <= e3 + tolerance
                && e3 <= e5 + tolerance
        }
        _ => false,
    };
    JepaHorizonReport {
        e_1,
        e_3,
        e_5,
        tolerance,
        passes,
    }
}

fn heldout_context_latents(examples: &[EncodedJepaTrainingExample]) -> Vec<Vec<f32>> {
    if examples.is_empty() {
        return Vec::new();
    }
    let split = ((examples.len() as f32) * 0.8).floor() as usize;
    let split = split.min(examples.len().saturating_sub(1));
    examples[split..]
        .iter()
        .map(|example| example.context_latent.clone())
        .collect()
}
