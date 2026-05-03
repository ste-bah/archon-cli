//! Game-theory pipeline module.
//!
//! Provides the curated 84-agent game-theory arsenal, Tier 1 parallel
//! classification, 9-axis fingerprint, and Cozo persistence for runs
//! and fingerprints.

pub mod agents;
pub mod errors;
pub mod facade;
pub mod fingerprint;
pub mod registry;
pub mod schema;
pub mod spec;

pub use agents::{GameTheoryAgent, GameTheoryTier, GameTheoryToolAccess};
pub use errors::GameTheoryError;
pub use facade::classify;
pub use fingerprint::GameTheoryFingerprint;
pub use registry::{GAMETHEORY_AGENTS, GAMETHEORY_TIERS};
