use candle_nn::Optimizer as _;

// Gradient training for the JEPA encoders.
//
// Before this existed, `JepaTraceEncoder::new` produced deterministic weights
// that were cloned into the model unchanged: `max_epochs` and `learning_rate`
// were validated and then read by nothing, `ema_target_from` ran once outside
// any loop, and the "training" that followed only fitted heads on frozen
// features. Representations never moved, which is the one thing a JEPA exists
// to do.
//
// `input_weights` and `output_weights` are dense `latent_dim x latent_dim`
// matrices, stored row-major and flattened into the existing `Vec<f32>` fields,
// so serialisation and the checkpoint format are untouched. They were 1-D
// vectors until the matrix rewrite; that form could only rescale each dimension
// in place, so it could not combine features or rotate the embedding basis
// toward a direction worth predicting.
//
// Feature batches are `(rows, dim)`, so the forward pass right-multiplies by
// the transpose: `(rows, in) x (in, out)`. The single-vector inference paths in
// the candle and mlx runtimes use column vectors and multiply on the left
// instead; both agree with `JepaTraceEncoder::project`.
//
// Three properties that make this a JEPA rather than an autoencoder:
//
// * the loss is computed in **representation space**, never against raw input;
// * the target branch is **detached**, so no gradient flows through it — this
//   is what stops the trivial solution where both encoders collapse together;
// * the target encoder follows the context encoder by **EMA inside the loop**,
//   which is what makes the target a slowly-moving teacher rather than a copy.
//
// Collapse is still possible even with a stop-gradient, so the loss carries an
// explicit variance term and the caller is told whether the representation
// degenerated.




include!("22_encoder_training_a.rs");
include!("22_encoder_training_b.rs");
