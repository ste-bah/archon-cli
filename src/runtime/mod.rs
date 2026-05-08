//! Runtime composition helpers extracted from `main.rs`.
//!
//! Each submodule owns one cross-cutting construction concern so
//! `main.rs` stays a thin orchestrator. TASK-AGS-699.

pub(crate) mod agent_profile_overlay;
pub(crate) mod llm;
pub(crate) mod permission_events;
pub(crate) mod sandbox_events;
