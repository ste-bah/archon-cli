//! JEPA-inspired trace representation candidate model.
//!
//! M2 keeps the implementation intentionally local and deterministic: the
//! encoders consume structured trace features plus deterministic lexical
//! hashing, and the CPU trainer fits the predictor and auxiliary heads without
//! calling semantic embedding providers.

include!("jepa/00_config_metadata.rs");
include!("jepa/01_model.rs");
include!("jepa/02_records_backend_types.rs");
include!("jepa/03_backend_impls.rs");
include!("jepa/04_candle_runtime.rs");
include!("jepa/05_candle_training.rs");
include!("jepa/06_mlx_runtime.rs");
include!("jepa/07_mlx_training.rs");
include!("jepa/08_examples_eval.rs");
include!("jepa/09_training_runtime.rs");
include!("jepa/10_checkpoint_io.rs");
include!("jepa/11_mask_encode_loss.rs");
include!("jepa/12_features.rs");
include!("jepa/13_aux_math_utils.rs");
include!("jepa/eval_planner.rs");
include!("jepa/eval_run_store.rs");
include!("jepa/eval_progress.rs");
include!("jepa/eval_runtime.rs");
include!("jepa/eval_backends.rs");

#[cfg(test)]
mod tests {
    include!("jepa/14_tests_support.rs");
    include!("jepa/15_tests_core.rs");
    include!("jepa/16_tests_fake_backend.rs");
    include!("jepa/17_tests_cpu_accel.rs");
    include!("jepa/18_tests_cuda.rs");
    include!("jepa/19_tests_mlx.rs");
    include!("jepa/20_tests_gates_checkpoint.rs");
}

#[cfg(test)]
mod tests_eval_pipeline {
    include!("jepa/21_tests_eval_pipeline.rs");
}
