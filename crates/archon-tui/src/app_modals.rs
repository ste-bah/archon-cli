//! Modal overlay state types extracted from `app.rs` (L448-L489) per REM-2d
//! (docs/rem-2-split-plan.md §7, Option 7A).
//!
//! These types describe the transient state of interactive modal overlays
//! (session picker, MCP manager, splash config). They live as a sibling
//! module of `app` so `app.rs` stays comfortably under the 500-line ceiling
//! without introducing a subdirectory split.
//!
//! External consumers reach these types via `archon_tui::app::*` — the
//! stable path is preserved by a `pub use crate::app_modals::{...}` in
//! `app.rs`.
//!
//! Pure relocation: definitions are byte-equivalent to the originals.

use crate::events::{McpServerEntry, SessionPickerEntry};
use crate::splash::ActivityEntry;

/// Interactive session picker state (shown as modal overlay on /resume).
#[derive(Debug, Clone)]
pub struct SessionPicker {
    pub sessions: Vec<SessionPickerEntry>,
    pub selected: usize,
}

/// Which sub-view is active inside the MCP manager overlay.
#[derive(Debug, Clone)]
pub enum McpManagerView {
    ServerList {
        selected: usize,
    },
    ServerMenu {
        server_idx: usize,
        action_idx: usize,
    },
    /// Scrollable list of tool names for a specific server.
    ToolList {
        server_name: String,
        tools: Vec<String>,
        scroll: usize,
    },
}

/// Interactive MCP server manager state (shown as modal overlay on /mcp).
#[derive(Debug, Clone)]
pub struct McpManager {
    pub servers: Vec<McpServerEntry>,
    pub view: McpManagerView,
}

/// Configuration for the splash screen passed in from main.
#[derive(Debug, Clone, Default)]
pub struct SplashConfig {
    /// Model name to display.
    pub model: String,
    /// Working directory to display.
    pub working_dir: String,
    /// Recent session activity.
    pub activity: Vec<ActivityEntry>,
}
