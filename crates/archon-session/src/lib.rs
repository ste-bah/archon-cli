pub mod attach;
pub mod background;
pub mod checkpoint;
pub mod export;
/// Per-message human feedback, stored beside the log rather than in it (#193).
pub mod feedback;
pub mod fork;
pub mod history {}
pub mod listing;
pub mod metadata;
pub mod naming;
pub mod plan;
mod plan_authority_secret;
mod plan_models;
mod plan_store;
/// Derived session state, folded once and cached (#193 Phase B).
pub mod projection;
/// The migrated consumer that proves the projection machinery (#193 Phase B).
pub mod projection_stats;
pub mod registry;
pub mod resume;
pub mod search;
pub mod storage;
