//! PaneLayout enum and rect computation for TASK-CLI-307.
//!
//! Uses `ratatui::layout::Layout::vertical()` / `Layout::horizontal()` API
//! (ratatui 0.27+ style, NOT deprecated `Layout::default().direction()` builder).

use ratatui::layout::{Constraint, Layout, Rect};

// ---------------------------------------------------------------------------
// PaneLayout
// ---------------------------------------------------------------------------

/// Layout mode for the split pane system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneLayout {
    /// Single pane (default, backward-compatible).
    Single,
    /// Two panes stacked top/bottom. Ratio = top pane percentage (0–100).
    HorizontalSplit(u16),
    /// Two panes side by side. Ratio = left pane percentage (0–100).
    VerticalSplit(u16),
    /// Grid of rows×cols panes.
    Grid(u16, u16),
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self::Single
    }
}

impl PaneLayout {
    /// Cycle through: Single → HorizontalSplit(50) → VerticalSplit(50) → Single.
    pub fn cycle(&self) -> Self {
        match self {
            Self::Single => Self::HorizontalSplit(50),
            Self::HorizontalSplit(_) => Self::VerticalSplit(50),
            Self::VerticalSplit(_) | Self::Grid(_, _) => Self::Single,
        }
    }

    /// Number of panes this layout requires.
    pub fn pane_count(&self) -> usize {
        match self {
            Self::Single => 1,
            Self::HorizontalSplit(_) | Self::VerticalSplit(_) => 2,
            Self::Grid(rows, cols) => (*rows as usize) * (*cols as usize),
        }
    }

    /// Resize the top/left pane by `delta` percentage points.
    ///
    /// Result is clamped to `[10, 90]`.
    pub fn resize(&self, delta: i16) -> Self {
        match self {
            Self::HorizontalSplit(r) => {
                Self::HorizontalSplit(clamp_ratio(*r as i16 + delta))
            }
            Self::VerticalSplit(r) => {
                Self::VerticalSplit(clamp_ratio(*r as i16 + delta))
            }
            other => other.clone(),
        }
    }
}

fn clamp_ratio(v: i16) -> u16 {
    v.clamp(10, 90) as u16
}

// ---------------------------------------------------------------------------
// Rect splitting helpers
// ---------------------------------------------------------------------------

/// Split `area` horizontally (top/bottom) at `top_pct` percent of the height.
///
/// Uses `Layout::vertical()` (ratatui 0.27+ API).
pub fn split_rect_horizontal(area: Rect, top_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::vertical([
        Constraint::Percentage(top_pct),
        Constraint::Percentage(100 - top_pct),
    ])
    .split(area);
    (chunks[0], chunks[1])
}

/// Split `area` vertically (left/right) at `left_pct` percent of the width.
///
/// Uses `Layout::horizontal()` (ratatui 0.27+ API).
pub fn split_rect_vertical(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::horizontal([
        Constraint::Percentage(left_pct),
        Constraint::Percentage(100 - left_pct),
    ])
    .split(area);
    (chunks[0], chunks[1])
}

/// Compute all pane rects for a given layout within `area`.
pub fn compute_pane_rects(layout: &PaneLayout, area: Rect) -> Vec<Rect> {
    match layout {
        PaneLayout::Single => vec![area],
        PaneLayout::HorizontalSplit(pct) => {
            let (top, bottom) = split_rect_horizontal(area, *pct);
            vec![top, bottom]
        }
        PaneLayout::VerticalSplit(pct) => {
            let (left, right) = split_rect_vertical(area, *pct);
            vec![left, right]
        }
        PaneLayout::Grid(rows, cols) => {
            let mut rects = Vec::new();
            let r = rows.max(&1);
            let c = cols.max(&1);
            let row_pct = 100 / r;
            let col_pct = 100 / c;
            let row_constraints: Vec<Constraint> =
                (0..*rows).map(|_| Constraint::Percentage(row_pct)).collect();
            let col_constraints: Vec<Constraint> =
                (0..*cols).map(|_| Constraint::Percentage(col_pct)).collect();

            let row_rects = Layout::vertical(row_constraints).split(area);
            for row_rect in row_rects.iter() {
                let col_rects = Layout::horizontal(col_constraints.clone()).split(*row_rect);
                rects.extend(col_rects.iter().copied());
            }
            rects
        }
    }
}
