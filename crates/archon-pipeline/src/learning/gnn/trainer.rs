//! Synchronous GNN training loop with Adam, EWC, early stopping, and timeout.
//!
//! PR 2 implementation - single-threaded, synchronous training. PR 3 wraps
//! this in a tokio background task.

mod batch;
mod gradients;
mod metrics;
mod run;
#[cfg(test)]
mod tests;
mod triplets;
mod types;

pub use types::{EpochMetrics, GnnTrainer, TrainingConfig, TrainingOutcome};
