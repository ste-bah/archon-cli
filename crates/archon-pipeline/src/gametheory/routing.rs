//! Conditional specialist routing via expression evaluation.
//!
//! Parses `.archon/specs/gametheory.yaml`, evaluates per-agent condition
//! expressions against the Tier 1 fingerprint, and produces a deterministic
//! [`RoutingDecision`] with enabled/skipped specialists and cycle detection.

mod conditions;
mod dag;
mod planner;
mod spec;
#[cfg(test)]
mod tests;
mod types;

#[cfg(test)]
pub(crate) use conditions::parse_condition;
pub use planner::evaluate_routing;
pub use spec::{load_spec, resolve_spec_path};
pub use types::{AgentEntry, GameTheorySpec, RoutingDecision, TierEntry};
