/// What a training run produced, plus the evidence needed to judge it.
#[derive(Debug, Clone)]
#[cfg(feature = "candle")]
pub(crate) struct EncoderTrainingOutcome {
    pub context: JepaTraceEncoder,
    pub target: JepaTraceEncoder,
    pub initial_loss: f32,
    pub final_loss: f32,
    pub epochs_run: usize,
    /// Latent variance fell below the collapse floor at the end of training.
    pub collapsed: bool,
}

/// Minimum per-dimension latent variance before a representation is considered
/// collapsed. A JEPA that maps every input to the same point drives the
/// prediction loss to zero while learning nothing, so a falling loss is not on
/// its own evidence of progress.
#[cfg(feature = "candle")]
const COLLAPSE_VARIANCE_FLOOR: f32 = 1e-4;

/// Weight on the variance term that pushes back against collapse.
#[cfg(feature = "candle")]
const VARIANCE_PENALTY_WEIGHT: f64 = 1.0;

/// Train the context encoder against an EMA target encoder.
///
/// Returns the trained pair. The caller keeps ownership of how they are used —
/// this deliberately does not promote anything.
#[cfg(feature = "candle")]
pub(crate) fn train_encoders(
    context: &JepaTraceEncoder,
    batch: &JepaFeatureBatch,
    config: &JepaTrainingConfig,
) -> Result<EncoderTrainingOutcome> {
    if batch.rows == 0 {
        bail!("jepa encoder training requires at least one example");
    }
    if batch.latent_dim != context.latent_dim {
        bail!(
            "jepa latent dimension mismatch: batch {} vs encoder {}",
            batch.latent_dim,
            context.latent_dim
        );
    }

    let device = candle_core::Device::Cpu;
    let dim = context.latent_dim;
    let rows = batch.rows;

    let context_features = matrix(&batch.context_features, rows, dim, &device)?;
    let action_features = matrix(&batch.action_features, rows, dim, &device)?;
    let target_features = matrix(&batch.target_features, rows, dim, &device)?;

    // Trainable context parameters, seeded from the deterministic encoder so a
    // zero-epoch run reproduces the previous behaviour exactly.
    let params = TrainableEncoder::seeded(context, &device)?;

    // The target starts as a copy and is never optimised — only EMA'd.
    let mut target = context.clone();
    target.role = "target".into();

    let mut optimizer = candle_nn::AdamW::new(
        params.vars(),
        candle_nn::ParamsAdamW {
            lr: config.learning_rate as f64,
            ..Default::default()
        },
    )?;

    let mut initial_loss = None;
    let mut final_loss = 0.0f32;
    let mut epochs_run = 0;

    for _ in 0..config.max_epochs {
        let context_latent = params.forward(&context_features)?;
        let action_latent = params.forward(&action_features)?;

        // Target branch: encoded with the EMA weights and DETACHED. Without the
        // detach, gradients would flow into both branches and the pair could
        // minimise the loss by agreeing on a constant.
        let target_latent = encode_with(&target, &target_features, &device)?.detach();

        // Predict the target representation from context + action. This is the
        // joint-embedding predictive objective: the comparison happens between
        // representations, never against the raw input.
        let predicted = (&context_latent + &action_latent)?;
        let prediction_loss = (&predicted - &target_latent)?.sqr()?.mean_all()?;

        // Variance term: penalise a representation whose dimensions carry no
        // spread. This is the pressure against collapse that the stop-gradient
        // alone does not guarantee.
        let variance = per_dimension_variance(&context_latent)?;
        let variance_penalty = variance
            .affine(-1.0, COLLAPSE_VARIANCE_FLOOR as f64)?
            .relu()?
            .mean_all()?;
        let loss = (&prediction_loss + &variance_penalty.affine(VARIANCE_PENALTY_WEIGHT, 0.0)?)?;

        let value = loss.to_scalar::<f32>()?;
        if !value.is_finite() {
            bail!("jepa encoder training diverged: non-finite loss");
        }
        initial_loss.get_or_insert(value);
        final_loss = value;

        optimizer.backward_step(&loss)?;
        epochs_run += 1;

        // EMA the target toward the freshly-updated context. Inside the loop,
        // which is what makes it a moving teacher; the previous code did this
        // once at construction, where it could not track anything.
        let trained = params.to_encoder(context)?;
        ema_into(&mut target, &trained, config.ema_decay);
    }

    let trained_context = params.to_encoder(context)?;
    let collapsed = {
        let latent = params.forward(&context_features)?;
        let variance = per_dimension_variance(&latent)?;
        variance.max(0)?.to_scalar::<f32>()? < COLLAPSE_VARIANCE_FLOOR
    };

    Ok(EncoderTrainingOutcome {
        context: trained_context,
        target,
        initial_loss: initial_loss.unwrap_or(final_loss),
        final_loss,
        epochs_run,
        collapsed,
    })
}

/// The encoder's parameters as candle `Var`s.
#[cfg(feature = "candle")]
struct TrainableEncoder {
    input_weights: candle_core::Var,
    hidden_bias: candle_core::Var,
    output_weights: candle_core::Var,
    output_bias: candle_core::Var,
    residual_weight: f32,
    dim: usize,
}

#[cfg(feature = "candle")]
impl TrainableEncoder {
    fn seeded(from: &JepaTraceEncoder, device: &candle_core::Device) -> Result<Self> {
        let dim = from.latent_dim;
        let matrix = (dim, dim);
        // Biases stay rank-1 so they broadcast across the batch dimension of a
        // `(rows, dim)` feature matrix.
        let vector = (dim,);
        Ok(Self {
            input_weights: seeded_var("input_weights", &from.input_weights, matrix, device)?,
            hidden_bias: seeded_var_1d("hidden_bias", &from.hidden_bias, vector, device)?,
            output_weights: seeded_var("output_weights", &from.output_weights, matrix, device)?,
            output_bias: seeded_var_1d("output_bias", &from.output_bias, vector, device)?,
            // Held fixed: it is a single scalar mixing weight, and letting it
            // drift lets the model trivially shrink the learned branch to zero
            // and fall back to passing the input straight through.
            residual_weight: from.residual_weight,
            dim,
        })
    }

    fn vars(&self) -> Vec<candle_core::Var> {
        vec![
            self.input_weights.clone(),
            self.hidden_bias.clone(),
            self.output_weights.clone(),
            self.output_bias.clone(),
        ]
    }

    /// Mirror of `JepaTraceEncoder::project`, in tensors.
    fn forward(&self, features: &candle_core::Tensor) -> Result<candle_core::Tensor> {
        // `features` is a batch matrix `(rows, dim)`. The weight matrices are
        // stored `(out, in)` to match `01_model.rs`, so they are transposed for
        // a right-multiply: (rows, in) x (in, out) -> (rows, out).
        let hidden = features
            .matmul(&self.input_weights.t()?)?
            .broadcast_add(&self.hidden_bias)?
            .gelu()?;
        let learned = hidden
            .matmul(&self.output_weights.t()?)?
            .broadcast_add(&self.output_bias)?;
        let residual = features.affine(self.residual_weight as f64, 0.0)?;
        let mixed = (&residual + &learned.affine(1.0 - self.residual_weight as f64, 0.0)?)?;
        tensor_layer_norm(&mixed)
    }

    fn to_encoder(&self, template: &JepaTraceEncoder) -> Result<JepaTraceEncoder> {
        let mut encoder = template.clone();
        encoder.input_weights = self.input_weights.flatten_all()?.to_vec1::<f32>()?;
        encoder.hidden_bias = self.hidden_bias.flatten_all()?.to_vec1::<f32>()?;
        encoder.output_weights = self.output_weights.flatten_all()?.to_vec1::<f32>()?;
        encoder.output_bias = self.output_bias.flatten_all()?.to_vec1::<f32>()?;
        encoder.residual_weight = self.residual_weight;
        debug_assert_eq!(encoder.input_weights.len(), self.dim * self.dim);
        debug_assert_eq!(encoder.hidden_bias.len(), self.dim);
        Ok(encoder)
    }
}

#[cfg(feature = "candle")]
fn seeded_var(
    name: &str,
    values: &[f32],
    shape: (usize, usize),
    device: &candle_core::Device,
) -> Result<candle_core::Var> {
    let expected = shape.0 * shape.1;
    if values.len() != expected {
        bail!(
            "jepa encoder parameter {name} has wrong length: expected {expected} for {:?}, got {}",
            shape,
            values.len()
        );
    }
    // Seeded from the deterministic weights so a zero-epoch run reproduces the
    // untrained encoder exactly, and any change is attributable to training.
    let initial = candle_core::Tensor::from_slice(values, shape, device)?;
    Ok(candle_core::Var::from_tensor(&initial)?)
}

#[cfg(feature = "candle")]
fn seeded_var_1d(
    name: &str,
    values: &[f32],
    shape: (usize,),
    device: &candle_core::Device,
) -> Result<candle_core::Var> {
    if values.len() != shape.0 {
        bail!(
            "jepa encoder parameter {name} has wrong length: expected {}, got {}",
            shape.0,
            values.len()
        );
    }
    let initial = candle_core::Tensor::from_slice(values, shape, device)?;
    Ok(candle_core::Var::from_tensor(&initial)?)
}

/// Row-wise layer norm, matching the scalar implementation in `01_model.rs`.
#[cfg(feature = "candle")]
fn tensor_layer_norm(values: &candle_core::Tensor) -> Result<candle_core::Tensor> {
    let mean = values.mean_keepdim(1)?;
    let centered = values.broadcast_sub(&mean)?;
    let variance = centered.sqr()?.mean_keepdim(1)?;
    let denom = (variance + 1e-5)?.sqrt()?;
    Ok(centered.broadcast_div(&denom)?)
}

/// Variance of each latent dimension across the batch.
#[cfg(feature = "candle")]
fn per_dimension_variance(latent: &candle_core::Tensor) -> Result<candle_core::Tensor> {
    let mean = latent.mean_keepdim(0)?;
    Ok(latent.broadcast_sub(&mean)?.sqr()?.mean(0)?)
}

/// Encode a feature matrix with fixed (non-trainable) encoder weights.
#[cfg(feature = "candle")]
fn encode_with(
    encoder: &JepaTraceEncoder,
    features: &candle_core::Tensor,
    device: &candle_core::Device,
) -> Result<candle_core::Tensor> {
    let dim = encoder.latent_dim;
    let expected = dim.saturating_mul(dim);
    if encoder.input_weights.len() != expected || encoder.output_weights.len() != expected {
        bail!(
            "jepa encoder weight shape mismatch: expected {expected} elements for a {dim}x{dim} matrix, \
             got input={} output={}",
            encoder.input_weights.len(),
            encoder.output_weights.len()
        );
    }
    let input_weights =
        candle_core::Tensor::from_slice(&encoder.input_weights, (dim, dim), device)?;
    let hidden_bias = candle_core::Tensor::from_slice(&encoder.hidden_bias, (dim,), device)?;
    let output_weights =
        candle_core::Tensor::from_slice(&encoder.output_weights, (dim, dim), device)?;
    let output_bias = candle_core::Tensor::from_slice(&encoder.output_bias, (dim,), device)?;

    // Same `(rows, dim) x (dim, dim)^T` form as `TrainableEncoder::forward`.
    let hidden = features
        .matmul(&input_weights.t()?)?
        .broadcast_add(&hidden_bias)?
        .gelu()?;
    let learned = hidden
        .matmul(&output_weights.t()?)?
        .broadcast_add(&output_bias)?;
    let residual = features.affine(encoder.residual_weight as f64, 0.0)?;
    let mixed = (&residual + &learned.affine(1.0 - encoder.residual_weight as f64, 0.0)?)?;
    tensor_layer_norm(&mixed)
}

