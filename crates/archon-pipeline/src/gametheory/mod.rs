//! Game-theory pipeline module.
//!
//! Provides the curated 84-agent game-theory arsenal, Tier 1 parallel
//! classification, 9-axis fingerprint, routing, specialist execution,
//! final-stage report synthesis, and Cozo persistence for all artefacts.

pub mod agents;
pub mod errors;
pub mod facade;
pub mod final_stage;
pub mod fingerprint;
pub mod prompt_builder;
pub mod quality;
pub mod registry;
pub mod routing;
pub mod schema;
pub mod sections;
pub mod spec;

pub use agents::{GameTheoryAgent, GameTheoryTier, GameTheoryToolAccess};
pub use errors::GameTheoryError;
pub use facade::{
    FullPipelineResult, GameTheoryMemoryContext, GameTheoryRunOptions, MemoryRecallAudit, classify,
    run_full_pipeline, run_full_pipeline_with_memory, run_full_pipeline_with_options,
};
pub use fingerprint::GameTheoryFingerprint;
pub use registry::{GAMETHEORY_AGENTS, GAMETHEORY_TIERS};
pub use routing::{
    GameTheorySpec, RoutingDecision, evaluate_routing, load_spec, resolve_spec_path,
};
