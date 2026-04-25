//! Runtime composition helpers extracted from `main.rs`.
//!
//! Each submodule owns one cross-cutting construction concern so
//! `main.rs` stays a thin orchestrator. TASK-AGS-699.

pub(crate) mod llm;
