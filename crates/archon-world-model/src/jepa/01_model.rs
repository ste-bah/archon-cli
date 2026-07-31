/// Two-layer encoder with a residual path.
///
/// `input_weights` and `output_weights` are dense `latent_dim x latent_dim`
/// matrices stored row-major: element `(row, col)` lives at
/// `row * latent_dim + col`.
///
/// They were per-dimension vectors until the matrix rewrite. That form could
/// only rescale each dimension independently — output `i` depended solely on
/// input `i` — so it could not combine two features, and could not rotate the
/// embedding basis into directions that predict anything. A general-purpose
/// sentence embedding's axes are not aligned to "this will need a retry"; that
/// direction is a combination of many axes, and only a matrix can form it.
/// Without one the "encoder" was a diagonal gain control, not an encoder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTraceEncoder {
    pub role: String,
    pub latent_dim: usize,
    pub input_weights: Vec<f32>,
    pub hidden_bias: Vec<f32>,
    pub output_weights: Vec<f32>,
    pub output_bias: Vec<f32>,
    pub residual_weight: f32,
}

impl JepaTraceEncoder {
    pub fn new(role: impl Into<String>, latent_dim: usize) -> Self {
        let role = role.into();
        // Xavier-style bound for a square layer: sqrt(6 / (fan_in + fan_out)).
        // Small weights keep the residual path dominant at initialisation, so
        // an untrained encoder stays close to the identity instead of scrambling
        // the embedding it was handed.
        let limit = (6.0f32 / (latent_dim as f32 + latent_dim as f32)).sqrt();
        let matrix_len = latent_dim.saturating_mul(latent_dim);
        let input_weights = deterministic_vector(&role, "input", matrix_len, -limit, limit);
        let hidden_bias = deterministic_vector(&role, "hidden_bias", latent_dim, -0.03, 0.03);
        let output_weights = deterministic_vector(&role, "output", matrix_len, -limit, limit);
        let output_bias = deterministic_vector(&role, "output_bias", latent_dim, -0.03, 0.03);
        Self {
            role,
            latent_dim,
            input_weights,
            hidden_bias,
            output_weights,
            output_bias,
            residual_weight: 0.20,
        }
    }

    pub fn ema_target_from(context: &Self, decay: f32) -> Self {
        let mut target = Self::new("target", context.latent_dim);
        target.input_weights = ema_values(&target.input_weights, &context.input_weights, decay);
        target.hidden_bias = ema_values(&target.hidden_bias, &context.hidden_bias, decay);
        target.output_weights = ema_values(&target.output_weights, &context.output_weights, decay);
        target.output_bias = ema_values(&target.output_bias, &context.output_bias, decay);
        target.residual_weight =
            decay * target.residual_weight + (1.0 - decay) * context.residual_weight;
        target
    }

    pub fn encode_window(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.project(window_features(window, self.latent_dim, &self.role)?)
    }

    pub fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>> {
        self.project(action_features(action, self.latent_dim, &self.role)?)
    }

    fn project(&self, features: Vec<f32>) -> Result<Vec<f32>> {
        let dim = self.latent_dim;
        if features.len() != dim {
            bail!("jepa feature dimension mismatch");
        }
        let expected = dim.saturating_mul(dim);
        if self.input_weights.len() != expected || self.output_weights.len() != expected {
            bail!(
                "jepa encoder weight shape mismatch: expected {expected} elements for a {dim}x{dim} matrix, \
                 got input={} output={}",
                self.input_weights.len(),
                self.output_weights.len()
            );
        }

        let hidden: Vec<f32> = (0..dim)
            .map(|row| {
                let offset = row * dim;
                let sum: f32 = (0..dim)
                    .map(|col| self.input_weights[offset + col] * features[col])
                    .sum();
                gelu(sum + self.hidden_bias[row])
            })
            .collect();

        let mut output: Vec<f32> = (0..dim)
            .map(|row| {
                let offset = row * dim;
                let sum: f32 = (0..dim)
                    .map(|col| self.output_weights[offset + col] * hidden[col])
                    .sum();
                // The residual carries the input through, so an untrained or
                // lightly trained encoder degrades to the embedding it was
                // given rather than to noise.
                self.residual_weight * features[row]
                    + (1.0 - self.residual_weight) * (sum + self.output_bias[row])
            })
            .collect();

        layer_norm(&mut output);
        Ok(output)
    }

    fn parameter_count(&self) -> u64 {
        (self.input_weights.len()
            + self.hidden_bias.len()
            + self.output_weights.len()
            + self.output_bias.len()) as u64
    }

    fn finite(&self) -> bool {
        self.input_weights.iter().all(|value| value.is_finite())
            && self.hidden_bias.iter().all(|value| value.is_finite())
            && self.output_weights.iter().all(|value| value.is_finite())
            && self.output_bias.iter().all(|value| value.is_finite())
            && self.residual_weight.is_finite()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaPredictor {
    pub latent_dim: usize,
    pub context_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
    pub horizon_weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl JepaPredictor {
    fn baseline(latent_dim: usize) -> Self {
        Self {
            latent_dim,
            context_weights: vec![1.0; latent_dim],
            action_weights: vec![0.0; latent_dim],
            horizon_weights: vec![0.0; latent_dim],
            bias: vec![0.0; latent_dim],
        }
    }

    fn fit(latent_dim: usize, examples: &[EncodedJepaTrainingExample]) -> Result<Self> {
        if examples.is_empty() {
            bail!("at least one JEPA example is required");
        }
        let mut context_mean = vec![0.0; latent_dim];
        let mut action_mean = vec![0.0; latent_dim];
        let mut target_mean = vec![0.0; latent_dim];
        let mut horizon_mean = 0.0;
        for example in examples {
            validate_latents(latent_dim, example)?;
            let horizon = normalized_horizon(example.horizon);
            horizon_mean += horizon;
            for idx in 0..latent_dim {
                context_mean[idx] += example.context_latent[idx];
                action_mean[idx] += example.action_latent[idx];
                target_mean[idx] += example.target_latent[idx];
            }
        }
        let denom = examples.len() as f32;
        horizon_mean /= denom;
        for idx in 0..latent_dim {
            context_mean[idx] /= denom;
            action_mean[idx] /= denom;
            target_mean[idx] /= denom;
        }

        let mut context_weights = vec![0.0; latent_dim];
        let mut action_weights = vec![0.0; latent_dim];
        let mut horizon_weights = vec![0.0; latent_dim];
        let mut bias = vec![0.0; latent_dim];
        for idx in 0..latent_dim {
            context_weights[idx] = covariance_weight(
                examples,
                idx,
                context_mean[idx],
                target_mean[idx],
                InputRole::Context,
            );
            action_weights[idx] = covariance_weight(
                examples,
                idx,
                action_mean[idx],
                target_mean[idx],
                InputRole::Action,
            );
            horizon_weights[idx] = covariance_weight(
                examples,
                idx,
                horizon_mean,
                target_mean[idx],
                InputRole::Horizon,
            );
            bias[idx] = target_mean[idx]
                - context_weights[idx] * context_mean[idx]
                - action_weights[idx] * action_mean[idx]
                - horizon_weights[idx] * horizon_mean;
        }

        Ok(Self {
            latent_dim,
            context_weights,
            action_weights,
            horizon_weights,
            bias,
        })
    }

    pub fn predict(&self, context: &[f32], action: &[f32], horizon: usize) -> Result<Vec<f32>> {
        if context.len() != self.latent_dim || action.len() != self.latent_dim {
            bail!("jepa predictor latent dimensions must match");
        }
        let horizon = normalized_horizon(horizon);
        let mut predicted = vec![0.0; self.latent_dim];
        for idx in 0..self.latent_dim {
            predicted[idx] = (self.bias[idx]
                + self.context_weights[idx] * context[idx]
                + self.action_weights[idx] * action[idx]
                + self.horizon_weights[idx] * horizon)
                .tanh();
        }
        layer_norm(&mut predicted);
        Ok(predicted)
    }

    fn parameter_count(&self) -> u64 {
        (self.context_weights.len()
            + self.action_weights.len()
            + self.horizon_weights.len()
            + self.bias.len()) as u64
    }

    fn finite(&self) -> bool {
        self.context_weights.iter().all(|value| value.is_finite())
            && self.action_weights.iter().all(|value| value.is_finite())
            && self.horizon_weights.iter().all(|value| value.is_finite())
            && self.bias.iter().all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaAuxiliaryHead {
    pub label: String,
    pub bias: f32,
    pub latent_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
}

impl JepaAuxiliaryHead {
    pub fn predict_probability(&self, context: &[f32], action: &[f32]) -> f32 {
        sigmoid(
            self.bias
                + dot_prefix(&self.latent_weights, context)
                + dot_prefix(&self.action_weights, action),
        )
    }

    fn parameter_count(&self) -> u64 {
        (1 + self.latent_weights.len() + self.action_weights.len()) as u64
    }

    fn finite(&self) -> bool {
        self.bias.is_finite()
            && self.latent_weights.iter().all(|value| value.is_finite())
            && self.action_weights.iter().all(|value| value.is_finite())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaTraceModel {
    pub metadata: JepaTraceModelMetadata,
    pub context_encoder: JepaTraceEncoder,
    pub action_encoder: JepaTraceEncoder,
    pub target_encoder: JepaTraceEncoder,
    pub predictor: JepaPredictor,
    pub auxiliary_heads: Vec<JepaAuxiliaryHead>,
    pub transition_model: Option<CpuLatentTransitionModel>,
}

impl JepaTraceModel {
    pub fn predict_training_target(
        &self,
        context: &[f32],
        action: &[f32],
        horizon: usize,
    ) -> Result<Vec<f32>> {
        self.predictor.predict(context, action, horizon)
    }

    pub fn predict_auxiliary(&self, context: &[f32], action: &[f32]) -> Result<Vec<(String, f32)>> {
        if context.len() != self.metadata.latent_dim || action.len() != self.metadata.latent_dim {
            bail!("jepa auxiliary latent dimensions must match");
        }
        Ok(self
            .auxiliary_heads
            .iter()
            .map(|head| {
                (
                    head.label.clone(),
                    head.predict_probability(context, action),
                )
            })
            .collect())
    }

    pub fn validate_finite(&self) -> Result<()> {
        if !self.context_encoder.finite()
            || !self.action_encoder.finite()
            || !self.target_encoder.finite()
            || !self.predictor.finite()
            || !self.auxiliary_heads.iter().all(JepaAuxiliaryHead::finite)
            || !self
                .transition_model
                .as_ref()
                .is_none_or(transition_model_finite)
        {
            bail!("jepa checkpoint contains non-finite values");
        }
        Ok(())
    }

    fn parameter_count(&self) -> u64 {
        self.context_encoder.parameter_count()
            + self.action_encoder.parameter_count()
            + self.target_encoder.parameter_count()
            + self.predictor.parameter_count()
            + self
                .auxiliary_heads
                .iter()
                .map(JepaAuxiliaryHead::parameter_count)
                .sum::<u64>()
            + self
                .transition_model
                .as_ref()
                .map(|model| model.metadata.parameter_count)
                .unwrap_or_default()
    }
}

impl WorldRepresentationAdapter for JepaTraceModel {
    fn dimensions(&self) -> usize {
        self.metadata.latent_dim
    }

    fn provider_name(&self) -> &str {
        "archon-jepa-inspired"
    }

    fn model_name(&self) -> &str {
        &self.metadata.model_id
    }

    fn encode_state(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.context_encoder.encode_window(window)
    }

    fn encode_action(&self, action: &TraceAction) -> Result<Vec<f32>> {
        self.action_encoder.encode_action(action)
    }

    fn encode_target(&self, window: &TraceWindow) -> Result<Vec<f32>> {
        self.target_encoder.encode_window(window)
    }
}
