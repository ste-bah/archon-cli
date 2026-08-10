//! Shadow observation of the executive loop on live turns.

pub mod observer;
mod store;
pub mod types;

pub use observer::{
    SHADOW_AGREEMENT_METRIC, SHADOW_DEGRADED_MARKER, ShadowTurnInput, ShadowTurnObserver,
};
pub use types::{
    ACTION_MISMATCH_WEIGHT, LiveTurnOutcome, MAX_COUNTED_TOOL_FAILURES, OUTCOME_MISMATCH_WEIGHT,
    ShadowComparison, ShadowObservation, TOOL_FAILURE_WEIGHT, observed_action_from_tools,
    surprise_of,
};
