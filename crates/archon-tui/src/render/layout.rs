//! Layout computation for the TUI render pipeline.
//!
//! `Layout` holds the computed `Rect` regions for each UI area.
//! `compute_layout()` produces a `Layout` from the terminal size.

use ratatui::layout::{Constraint, Direction, Rect};

/// Computed layout regions for the TUI.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Output area (top, takes remaining space).
    pub output: Rect,
    /// Input area (3 rows, just below output).
    pub input: Rect,
    /// Permission indicator (1 row).
    pub permission: Rect,
    /// Status bar (1 row, bottom).
    pub status: Rect,
}

/// Compute the layout regions for a terminal of the given size.
///
/// The layout is always:
///
/// ```text
/// ┌──────────────────────────────────────┐
/// │           OUTPUT AREA                │
/// │         (min 3 rows)                  │
/// ├──────────────┬───────────────────────┤
/// │  INPUT AREA  │                       │
/// │  (3 rows)    │                       │
/// ├──────────────┴───────────────────────┤
/// │  PERMISSION (1 row)                  │
/// ├───────────────────────────────────────┤
/// │  STATUS (1 row)                      │
/// └───────────────────────────────────────┘
/// ```
pub fn compute_layout(size: Rect) -> Layout {
    let chunks = ratatui::layout::Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),    // output area
            Constraint::Length(3), // input area
            Constraint::Length(1), // permission indicator
            Constraint::Length(1), // status bar
        ])
        .split(size);

    Layout {
        output: chunks[0],
        input: chunks[1],
        permission: chunks[2],
        status: chunks[3],
    }
}
