//! Thin re-export shim for the lifted tracing primitives.
//!
//! The TASK-TUI-802 tracing surface (`init_tracing`, `RedactionLayer`,
//! `span_agent_turn`, `span_slash_dispatch`, `span_channel_send`) was lifted
//! to the `archon-observability` crate in TASK-AGS-OBS-905 as part of the
//! Stage 10 LIFT sequence. Existing call sites continue to import from
//! `archon_tui::observability::…` because `observability.rs` re-exports the
//! symbols below — and those in turn come from the new crate.
//!
//! This shim is intentionally ~10 lines. When OBS-906 carves
//! `RedactionLayer` into `archon_observability::redaction`, the re-export
//! here stays unchanged because `archon_observability` keeps a crate-root
//! re-export so the short path `archon_observability::RedactionLayer`
//! remains stable across internal refactors.
//!
//! Plan for removal: once every caller inside `archon-tui` and the
//! top-level `src/` has been flipped to import directly from
//! `archon_observability`, this file becomes a redundant hop and can be
//! deleted (tracked informally under the OBS wiring subtask; no ticket
//! yet because the external surface must stay on `archon_tui::observability`
//! until all call sites are migrated in follow-up PRs).

pub use archon_observability::{
    RedactionLayer, init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch,
};
