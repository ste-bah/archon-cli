//! Decomposed-PRD lifecycle policy: the value-level decisions the lifecycle
//! driver consults.
//!
//! Everything here is a pure function over `serde_json::Value` and a
//! [`LifecycleContract`](crate::generated_lifecycle_support::LifecycleContract).
//! Nothing in this module reaches a host, a store, an agent client, or the
//! run's event log — which is exactly why it can live beside the contract and
//! the generated lifecycle rather than in the binary.
//!
//! What stayed behind: `LifecycleDriver` and its stage methods. The driver
//! holds an `Arc<WorkflowScriptHost>` and is written as an inherent
//! `impl WorkflowV2ScriptRunner`, so coherence pins it to the crate that owns
//! that type until the host is behind a port trait.

pub mod adversarial;
pub mod assignment_invalid;
pub mod boundary_repair;
pub mod cross_cutting;
pub mod drain_gate;
pub mod inventory_items;
pub mod noop_routing;
pub mod terminal_gate;
pub mod triage_outcomes;
pub mod verify_invariants;
pub mod verify_merge;
pub mod verify_options;
pub mod verify_outcome_repair;
pub mod verify_overreach;
pub mod verify_retriage;
pub mod verify_routing;
/// Only `verify_options` reads manifest scopes; it stays inside the cluster.
pub(crate) mod verify_scope;
pub mod verify_supersede;

#[cfg(test)]
mod verify_options_tests;
#[cfg(test)]
mod verify_outcome_repair_tests;
