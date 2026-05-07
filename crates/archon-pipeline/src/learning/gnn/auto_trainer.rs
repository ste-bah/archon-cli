//! Auto-trainer - background tokio task wrapping synchronous GNN training.
//!
//! PR 3 delivers a configurable background loop that checks triggers
//! (memory count, correction count, time elapsed, first run) and delegates
//! the sync [`GnnTrainer`](super::trainer::GnnTrainer) to
//! [`tokio::task::spawn_blocking`].

mod control;
mod run_loop;
mod spawn;
#[cfg(test)]
mod tests;
mod triggers;
mod types;

pub use types::{AutoTrainer, AutoTrainerConfig, AutoTrainerStatus, TrainerState};
