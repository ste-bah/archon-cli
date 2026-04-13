//! View modules for the archon TUI.
//!
//! Each submodule owns rendering for a single screen or overlay. Per the
//! per-view isolation rule, view modules MUST NOT import from each other
//! (`crate::views::*`); shared helpers belong in `crate::theme`,
//! `crate::markdown`, etc.

pub mod agents;
pub mod context_viz;
pub mod diff_viewer;
pub mod help;
pub mod history;
pub mod model_picker;
pub mod session_browser;
pub mod settings;
pub mod tasks_overlay;
