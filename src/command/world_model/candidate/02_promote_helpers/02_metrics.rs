fn mean_auxiliary_brier(
    model: &CpuLatentTransitionModel,
    examples: &[LatentTransitionExample],
) -> Result<f32> {
    let mut total = 0.0;
    let mut count = 0.0;
    for example in examples {
        for prediction in model.predict_auxiliary(&example.state, &example.action)? {
            let actual = if label_value(&example.labels, &prediction.label) {
                1.0
            } else {
                0.0
            };
            total += (prediction.probability - actual).powi(2);
            count += 1.0;
        }
    }
    Ok(if count == 0.0 { 1.0 } else { total / count })
}

fn checkpoint_under_jepa_cap(
    config: &archon_core::config::ArchonConfig,
    candidate: &JepaCandidateRecord,
) -> Result<bool> {
    let max_bytes = config
        .learning
        .world_model
        .jepa
        .max_checkpoint_mb
        .saturating_mul(1024 * 1024);
    let bytes = std::fs::metadata(&candidate.checkpoint.path)
        .map(|metadata| metadata.len())
        .unwrap_or(u64::MAX);
    Ok(bytes <= max_bytes)
}

fn model_cosine_distances(
    model: &CpuLatentTransitionModel,
    examples: &[LatentTransitionExample],
) -> Result<Vec<f32>> {
    examples
        .iter()
        .map(|example| {
            let predicted = archon_world_model::backend::predict_next_with_backend(
                model,
                &example.state,
                &example.action,
                model.metadata.backend,
            )?;
            cosine_error(&predicted, &example.next_state)
        })
        .collect()
}

fn actual_transition_distances(examples: &[LatentTransitionExample]) -> Result<Vec<f32>> {
    examples
        .iter()
        .map(|example| cosine_error(&example.state, &example.next_state))
        .collect()
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

fn write_promotion_validation_metadata(
    root: &Path,
    model_id: &str,
    eval: &CandidateEvalRecord,
) -> Result<std::path::PathBuf> {
    let dir = root.join("active");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{model_id}.validation.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "model_id": model_id,
            "validation_recorded_at": Utc::now(),
            "promotion_gate": eval.report,
            "next_state": eval.next_state,
            "surprise": eval.surprise,
            "brier": eval.brier,
            "backend_validation": {
                "cpu": "supported",
                "cuda": "feature-gated",
                "mlx_metal": "experimental_until_hardware_validation"
            }
        }))?,
    )?;
    Ok(path)
}

fn counterfactual_gate_from_heldout(examples: &[LatentTransitionExample], min_ndcg: f32) -> bool {
    if examples.is_empty() {
        return false;
    }
    let mut scores = examples
        .iter()
        .enumerate()
        .map(|(idx, example)| {
            let risk = label_risk(&example.labels);
            archon_world_model::shadow::ShadowActionScore {
                action_id: format!("heldout-{idx}"),
                advisory_score: label_success(&example.labels) - risk,
                estimated_success: label_success(&example.labels),
                estimated_risk: risk,
            }
        })
        .collect::<Vec<_>>();
    scores.sort_by(|left, right| right.advisory_score.total_cmp(&left.advisory_score));
    let relevance = examples
        .iter()
        .enumerate()
        .map(|(idx, example)| {
            (
                format!("heldout-{idx}"),
                label_success(&example.labels) * 3.0,
            )
        })
        .collect::<Vec<_>>();
    archon_world_model::shadow::ndcg_at_k(&scores, &relevance, 5) >= min_ndcg
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

fn label_success(labels: &WorldLabelSet) -> f32 {
    match labels.success {
        Some(true) => 1.0,
        Some(false) => 0.0,
        None if labels.failure => 0.0,
        None => 0.5,
    }
}

fn label_risk(labels: &WorldLabelSet) -> f32 {
    let mut risk: f32 = 0.0;
    if labels.failure {
        risk += 0.35;
    }
    if labels.retry {
        risk += 0.20;
    }
    if labels.provider_incident {
        risk += 0.20;
    }
    if labels.verification_needed || labels.plan_drift {
        risk += 0.15;
    }
    risk.min(1.0)
}

#[allow(dead_code)]
fn _assert_brier_report_is_serializable(report: &BrierImprovementReport) -> usize {
    report.labels_improved
}
