//! GNN Enhancer - 3-layer graph attention network for embedding enhancement.
//!
//! Ported from root archon TS gnn-enhancer.ts. Round-trip architecture
//! preserves dimensionality: 1536->1024->1280->1536 with graph attention,
//! residual connections, and layer normalization.

mod accessors;
pub mod auto_trainer;
pub mod auto_trainer_one_shot;
// Reference: auto_trainer_runtime.rs (build/spawn helpers used by session.rs + pipeline.rs)
pub mod auto_trainer_runtime;
pub mod backprop;
pub mod cache;
mod enhancer;
pub mod ewc;
mod forward;
mod graph;
pub mod history;
mod legacy;
pub mod loss;
pub mod math;
pub mod optimizer;
#[cfg(test)]
mod tests;
pub mod trainer;
pub mod triplets_loss;
mod types;
mod weight_init;
pub mod weights;

pub use enhancer::GnnEnhancer;
pub use types::{
    ForwardResult, GnnConfig, GraphEnhancementResult, LayerActivationCache, LayerWeights,
    TrajectoryEdge, TrajectoryGraph, TrajectoryNode,
};

/// Legacy type alias for backward compatibility with trainer.rs.
#[allow(non_camel_case_types)]
pub type GNNEnhancer = GnnEnhancer;
