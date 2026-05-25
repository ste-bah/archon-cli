//! Integration Wiring - orchestrates SONA, ReasoningBank, DESC, Sherlock,
//! and GNN auto-training subsystems into a unified pipeline-facing API.
//!
//! Implements REQ-LEARN-F09.

mod autonomy;
mod core;
mod memory;
mod phd;
mod sherlock;
#[cfg(test)]
mod tests;
mod types;

pub use core::LearningIntegration;
pub use memory::{PendingStore, PipelineMemoryCoordinator};
pub use phd::{PhDLearningIntegration, StyleFeedback};
pub use sherlock::{SherlockLearningIntegration, SherlockVerdict};
pub use types::{LearningContext, LearningIntegrationConfig};
