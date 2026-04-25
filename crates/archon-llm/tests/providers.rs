//! TASK-AGS-711: Phase 7 acceptance suite — single cargo test target that
//! binds the registry breadth, 31-compat-roundtrip, runtime switch, and
//! "no provider-id string branching" audit into one CI-enforceable gate.

#[path = "providers/all_compat.rs"]
mod all_compat;

#[path = "providers/breadth.rs"]
mod breadth;

#[path = "providers/runtime_switch.rs"]
mod runtime_switch;

#[path = "providers/audit.rs"]
mod audit;
