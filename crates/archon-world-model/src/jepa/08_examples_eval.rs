pub fn build_jepa_training_examples(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
) -> Result<Vec<JepaTrainingExample>> {
    build_jepa_training_examples_controlled(rows, config, None)
}

pub fn build_jepa_training_examples_controlled(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    should_stop: Option<&dyn Fn() -> bool>,
) -> Result<Vec<JepaTrainingExample>> {
    config.validate()?;
    let builder = TraceWindowBuilder::new(rows);
    let mut examples = Vec::new();
    for horizon in &config.prediction_horizons {
        check_jepa_stop(should_stop, "jepa transition scan")?;
        let transitions = builder.adjacent_transitions(
            config.context_window_rows,
            config.target_window_rows,
            *horizon,
        )?;
        examples.extend(transitions.into_iter().map(JepaTrainingExample::from));
        check_jepa_stop(should_stop, "jepa transition append")?;
    }
    Ok(examples)
}

fn check_jepa_stop(should_stop: Option<&dyn Fn() -> bool>, stage: &str) -> Result<()> {
    if should_stop.is_some_and(|check| check()) {
        bail!("jepa training stopped or timed out during {stage}");
    }
    Ok(())
}

pub fn mask_jepa_training_examples(
    examples: &[JepaTrainingExample],
    mask_ratio: f32,
) -> (Vec<JepaTrainingExample>, JepaMaskingReport) {
    let mask_ratio = mask_ratio.clamp(0.0, 1.0);
    let mut report = JepaMaskingReport {
        mask_ratio,
        masked_context_fields: 0,
        masked_action_fields: 0,
        reconstructs_raw_text: false,
    };
    let masked = examples
        .iter()
        .map(|example| mask_jepa_training_example(example, mask_ratio, &mut report))
        .collect();
    (masked, report)
}

pub fn evaluate_representation_collapse(
    latents: &[Vec<f32>],
    min_latent_std: f32,
    min_effective_rank_ratio: f32,
) -> Result<JepaCollapseReport> {
    if latents.is_empty() || latents[0].is_empty() {
        bail!("collapse evaluation requires at least one non-empty latent");
    }
    let dim = latents[0].len();
    if latents.iter().any(|latent| latent.len() != dim) {
        bail!("collapse evaluation latent dimensions must match");
    }
    if latents
        .iter()
        .flat_map(|latent| latent.iter())
        .any(|value| !value.is_finite())
    {
        bail!("collapse evaluation latents must be finite");
    }
    let mut means = vec![0.0_f64; dim];
    for latent in latents {
        for (idx, value) in latent.iter().enumerate() {
            means[idx] += f64::from(*value);
        }
    }
    for mean in &mut means {
        *mean /= latents.len() as f64;
    }
    let mut stds = vec![0.0_f64; dim];
    for idx in 0..dim {
        let variance = latents
            .iter()
            .map(|latent| {
                let centered = f64::from(latent[idx]) - means[idx];
                centered * centered
            })
            .sum::<f64>()
            / latents.len() as f64;
        stds[idx] = variance.sqrt();
    }
    let mean_latent_std = (stds.iter().sum::<f64>() / dim as f64) as f32;
    let effective_rank_ratio = singular_value_effective_rank_ratio(latents, &means)?;
    let passes =
        mean_latent_std >= min_latent_std && effective_rank_ratio >= min_effective_rank_ratio;
    Ok(JepaCollapseReport {
        mean_latent_std,
        effective_rank_ratio,
        min_latent_std,
        min_effective_rank_ratio,
        passes,
    })
}

fn singular_value_effective_rank_ratio(latents: &[Vec<f32>], means: &[f64]) -> Result<f32> {
    let sample_count = latents.len();
    let dim = means.len();
    if sample_count == 0 || dim == 0 {
        bail!("collapse evaluation requires at least one latent");
    }

    let gram = mean_centered_gram_matrix(latents, means);
    let eigenvalues = jacobi_symmetric_eigenvalues(gram);
    let mut sigma_sum = 0.0_f64;
    let mut sigma_sq_sum = 0.0_f64;
    for eigenvalue in eigenvalues {
        if !eigenvalue.is_finite() {
            bail!("collapse evaluation eigenvalues must be finite");
        }
        let singular_value = eigenvalue.max(0.0).sqrt();
        sigma_sum += singular_value;
        sigma_sq_sum += singular_value * singular_value;
    }
    if sigma_sq_sum <= f64::EPSILON || !sigma_sum.is_finite() || !sigma_sq_sum.is_finite() {
        return Ok(0.0);
    }

    let effective_rank = (sigma_sum * sigma_sum) / sigma_sq_sum;
    Ok((effective_rank / dim as f64).clamp(0.0, 1.0) as f32)
}

fn mean_centered_gram_matrix(latents: &[Vec<f32>], means: &[f64]) -> Vec<Vec<f64>> {
    let sample_count = latents.len();
    let dim = means.len();
    if sample_count <= dim {
        let mut matrix = vec![vec![0.0_f64; sample_count]; sample_count];
        for row_idx in 0..sample_count {
            for other_idx in row_idx..sample_count {
                let mut dot = 0.0_f64;
                for feature_idx in 0..dim {
                    dot += centered_latent(latents, means, row_idx, feature_idx)
                        * centered_latent(latents, means, other_idx, feature_idx);
                }
                matrix[row_idx][other_idx] = dot;
                matrix[other_idx][row_idx] = dot;
            }
        }
        matrix
    } else {
        let mut matrix = vec![vec![0.0_f64; dim]; dim];
        for left_idx in 0..dim {
            for right_idx in left_idx..dim {
                let mut dot = 0.0_f64;
                for row_idx in 0..sample_count {
                    dot += centered_latent(latents, means, row_idx, left_idx)
                        * centered_latent(latents, means, row_idx, right_idx);
                }
                matrix[left_idx][right_idx] = dot;
                matrix[right_idx][left_idx] = dot;
            }
        }
        matrix
    }
}

fn centered_latent(latents: &[Vec<f32>], means: &[f64], row_idx: usize, feature_idx: usize) -> f64 {
    f64::from(latents[row_idx][feature_idx]) - means[feature_idx]
}

fn jacobi_symmetric_eigenvalues(mut matrix: Vec<Vec<f64>>) -> Vec<f64> {
    let size = matrix.len();
    if size == 0 {
        return Vec::new();
    }
    if size == 1 {
        return vec![matrix[0][0]];
    }

    let scale = matrix
        .iter()
        .enumerate()
        .map(|(idx, row)| row[idx].abs())
        .fold(1.0_f64, f64::max);
    let tolerance = (1e-10_f64 * scale).max(1e-12);
    for _ in 0..64 {
        let mut changed = false;
        for pivot in 0..size - 1 {
            for other in pivot + 1..size {
                let off_diag = matrix[pivot][other];
                if off_diag.abs() <= tolerance {
                    continue;
                }
                changed = true;
                let pivot_diag = matrix[pivot][pivot];
                let other_diag = matrix[other][other];
                let tau = (other_diag - pivot_diag) / (2.0 * off_diag);
                let turn = if tau >= 0.0 {
                    1.0 / (tau + (1.0 + tau * tau).sqrt())
                } else {
                    -1.0 / (-tau + (1.0 + tau * tau).sqrt())
                };
                let cosine = 1.0 / (1.0 + turn * turn).sqrt();
                let sine = turn * cosine;

                for idx in 0..size {
                    if idx == pivot || idx == other {
                        continue;
                    }
                    let left = matrix[idx][pivot];
                    let right = matrix[idx][other];
                    let rotated_left = cosine * left - sine * right;
                    let rotated_right = sine * left + cosine * right;
                    matrix[idx][pivot] = rotated_left;
                    matrix[pivot][idx] = rotated_left;
                    matrix[idx][other] = rotated_right;
                    matrix[other][idx] = rotated_right;
                }

                matrix[pivot][pivot] = cosine * cosine * pivot_diag
                    - 2.0 * sine * cosine * off_diag
                    + sine * sine * other_diag;
                matrix[other][other] = sine * sine * pivot_diag
                    + 2.0 * sine * cosine * off_diag
                    + cosine * cosine * other_diag;
                matrix[pivot][other] = 0.0;
                matrix[other][pivot] = 0.0;
            }
        }
        if !changed {
            break;
        }
    }

    matrix
        .into_iter()
        .enumerate()
        .map(|(idx, row)| row[idx])
        .collect()
}
