//! Output state types for the Archon TUI.
//!
//! Relocated from `src/output.rs` → `src/output/` per REM-2h (REM-2 split
//! plan, docs/rem-2-split-plan.md section 4). Zero public-API change: every
//! `archon_tui::output::*` path is preserved via flat re-exports below.
//!
//! Submodules:
//! - `thinking` — `ThinkingState` (collapsible thinking display + dot animation).
//! - `tool_output` — `ToolDisplayStatus` + `ToolOutputState` (per-tool-call display state).
//! - `buffer` — `OutputBuffer` (append-only streaming buffer with scroll math).

mod buffer;
mod thinking;
mod tool_output;

pub use buffer::OutputBuffer;
pub use thinking::ThinkingState;
pub use tool_output::{ToolDisplayStatus, ToolOutputState};
