#[cfg(feature = "candle")]
fn ema_into(target: &mut JepaTraceEncoder, context: &JepaTraceEncoder, decay: f32) {
    ema_slice(&mut target.input_weights, &context.input_weights, decay);
    ema_slice(&mut target.hidden_bias, &context.hidden_bias, decay);
    ema_slice(&mut target.output_weights, &context.output_weights, decay);
    ema_slice(&mut target.output_bias, &context.output_bias, decay);
    target.residual_weight = decay * target.residual_weight + (1.0 - decay) * context.residual_weight;
}

#[cfg(feature = "candle")]
fn ema_slice(target: &mut [f32], context: &[f32], decay: f32) {
    for (slot, value) in target.iter_mut().zip(context.iter()) {
        *slot = decay * *slot + (1.0 - decay) * *value;
    }
}

#[cfg(feature = "candle")]
fn matrix(values: &[f32], rows: usize, dim: usize, device: &candle_core::Device) -> Result<candle_core::Tensor> {
    if values.len() != rows * dim {
        bail!(
            "jepa feature matrix has {} values, expected {}",
            values.len(),
            rows * dim
        );
    }
    Ok(candle_core::Tensor::from_slice(values, (rows, dim), device)?)
}

#[cfg(all(test, feature = "candle"))]
mod encoder_training_tests {
    use super::*;

    fn config(epochs: usize) -> JepaTrainingConfig {
        JepaTrainingConfig {
            max_epochs: epochs,
            learning_rate: 0.05,
            ema_decay: 0.9,
            ..JepaTrainingConfig::default()
        }
    }

    /// Deterministic but varied features, so the batch has real spread to learn
    /// from rather than a degenerate constant.
    fn batch(rows: usize, dim: usize) -> JepaFeatureBatch {
        let value = |r: usize, d: usize, salt: usize| {
            (((r * 31 + d * 17 + salt * 7) % 23) as f32 / 23.0) - 0.5
        };
        let build = |salt: usize| {
            (0..rows)
                .flat_map(|r| (0..dim).map(move |d| value(r, d, salt)))
                .collect::<Vec<f32>>()
        };
        JepaFeatureBatch {
            context_features: build(0),
            action_features: build(1),
            target_features: build(2),
            labels: vec![WorldLabelSet::default(); rows],
            horizons: vec![1; rows],
            rows,
            feature_dim: dim,
            latent_dim: dim,
        }
    }

    /// The whole point of Phase 2: representations must actually move. Before
    /// this existed the encoder was cloned into the model unchanged.
    #[test]
    fn training_changes_the_encoder_weights() {
        let encoder = JepaTraceEncoder::new("context", 8);
        let trained = train_encoders(&encoder, &batch(16, 8), &config(20)).unwrap();
        assert_ne!(trained.context.input_weights, encoder.input_weights);
        assert_ne!(trained.context.output_weights, encoder.output_weights);
        assert_eq!(trained.epochs_run, 20);
    }

    /// `max_epochs` and `learning_rate` were validated and then read by nothing.
    /// Zero epochs must reproduce the untrained encoder exactly, which is what
    /// makes any later difference attributable to training.
    #[test]
    fn zero_epochs_reproduces_the_untrained_encoder() {
        let encoder = JepaTraceEncoder::new("context", 8);
        let trained = train_encoders(&encoder, &batch(4, 8), &config(0)).unwrap();
        assert_eq!(trained.context.input_weights, encoder.input_weights);
        assert_eq!(trained.epochs_run, 0);
    }

    /// A JEPA is only learning if the objective improves. This is the weakest
    /// useful assertion — it does not claim the representation is good, only
    /// that gradient descent is connected to the parameters.
    #[test]
    fn the_objective_improves_over_training() {
        let trained = train_encoders(
            &JepaTraceEncoder::new("context", 8),
            &batch(16, 8),
            &config(40),
        )
        .unwrap();
        assert!(
            trained.final_loss < trained.initial_loss,
            "loss did not improve: {} -> {}",
            trained.initial_loss,
            trained.final_loss
        );
    }

    /// The target encoder must track the context encoder rather than stay a
    /// frozen copy — that is what makes it a moving teacher.
    #[test]
    fn the_target_encoder_follows_the_context_encoder() {
        let encoder = JepaTraceEncoder::new("context", 8);
        let trained = train_encoders(&encoder, &batch(16, 8), &config(20)).unwrap();
        assert_ne!(trained.target.input_weights, encoder.input_weights);
        // ...but lags it, because EMA decay makes it a slow follower.
        assert_ne!(trained.target.input_weights, trained.context.input_weights);
    }
}

/// Train the encoders for a runtime training pass, or fall back to the
/// untrained pair when the build has no tensor backend.
///
/// Split from `train_encoders` so the runtime has one call site regardless of
/// features. Training failure is not fatal: the untrained encoders are exactly
/// what the previous behaviour used, so a candidate is still produced and the
/// eval gates still decide whether it is worth anything.
#[cfg(feature = "candle")]
pub(crate) fn train_encoders_for_runtime(
    seed: &JepaTraceEncoder,
    batch: &JepaFeatureBatch,
    config: &JepaTrainingConfig,
    progress: JepaProgressObserver,
) -> Result<(JepaTraceEncoder, JepaTraceEncoder)> {
    match train_encoders(seed, batch, config) {
        Ok(outcome) => {
            if outcome.collapsed {
                // Loud, and it keeps the untrained encoders. A collapsed
                // representation maps every input to one point, which drives the
                // prediction loss toward zero while learning nothing — so a
                // falling loss must never be read as success on its own.
                emit_jepa_progress(
                    progress,
                    "jepa_encoder_training_collapsed",
                    "representation collapsed; keeping untrained encoders",
                );
                return Ok((
                    seed.clone(),
                    JepaTraceEncoder::ema_target_from(seed, config.ema_decay),
                ));
            }
            emit_jepa_progress(
                progress,
                "jepa_encoder_training_converged",
                &format!(
                    "loss {:.6} -> {:.6} over {} epoch(s)",
                    outcome.initial_loss, outcome.final_loss, outcome.epochs_run
                ),
            );
            Ok((outcome.context, outcome.target))
        }
        Err(error) => {
            emit_jepa_progress(
                progress,
                "jepa_encoder_training_failed",
                &format!("{error}; keeping untrained encoders"),
            );
            Ok((
                seed.clone(),
                JepaTraceEncoder::ema_target_from(seed, config.ema_decay),
            ))
        }
    }
}

/// Fallback for builds without a tensor backend: the previous behaviour, where
/// the encoders are deterministic and never move.
#[cfg(not(feature = "candle"))]
pub(crate) fn train_encoders_for_runtime(
    seed: &JepaTraceEncoder,
    _batch: &JepaFeatureBatch,
    config: &JepaTrainingConfig,
    progress: JepaProgressObserver,
) -> Result<(JepaTraceEncoder, JepaTraceEncoder)> {
    emit_jepa_progress(
        progress,
        "jepa_encoder_training_skipped",
        "no tensor backend in this build; encoders remain untrained",
    );
    Ok((
        seed.clone(),
        JepaTraceEncoder::ema_target_from(seed, config.ema_decay),
    ))
}
