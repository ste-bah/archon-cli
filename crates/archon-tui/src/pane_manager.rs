//! PaneManager for TASK-CLI-307: focus ring, keyboard routing, resize.

use ratatui::layout::Rect;

use crate::pane::{Pane, PaneContent};
use crate::pane_layout::{PaneLayout, compute_pane_rects};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum columns a pane must have to be valid.
pub const PANE_MIN_COLS: u16 = 20;
/// Minimum rows a pane must have to be valid.
pub const PANE_MIN_ROWS: u16 = 5;
/// Resize step in percentage points (5%).
pub const RESIZE_STEP: i16 = 5;

// ---------------------------------------------------------------------------
// PaneManager
// ---------------------------------------------------------------------------

/// Manages the set of panes, their layout, and focus state.
pub struct PaneManager {
    panes: Vec<Pane>,
    layout: PaneLayout,
    focused_idx: usize,
    total_cols: u16,
    total_rows: u16,
}

impl PaneManager {
    /// Create a new manager with a single conversation pane.
    pub fn new(cols: u16, rows: u16) -> Self {
        let mut first = Pane::new(PaneContent::Conversation);
        first.set_focused(true);
        Self {
            panes: vec![first],
            layout: PaneLayout::Single,
            focused_idx: 0,
            total_cols: cols,
            total_rows: rows,
        }
    }

    // ── Accessors ───────────────────────────────────────────────────────────

    /// Number of panes currently managed.
    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// Current layout mode.
    pub fn layout(&self) -> &PaneLayout {
        &self.layout
    }

    /// Index of the currently focused pane.
    pub fn focused_idx(&self) -> usize {
        self.focused_idx
    }

    /// Total terminal area (derived from cols/rows).
    pub fn total_area(&self) -> Rect {
        Rect::new(0, 0, self.total_cols, self.total_rows)
    }

    /// Compute the rects for all current panes.
    pub fn compute_rects(&self) -> Vec<Rect> {
        compute_pane_rects(&self.layout, self.total_area())
    }

    // ── Layout operations ────────────────────────────────────────────────────

    /// Cycle the layout (Meta+S): Single → HorizontalSplit → VerticalSplit → Single.
    ///
    /// When splitting: adds a new pane if needed (up to layout's pane count).
    /// When returning to Single: collapses extra panes.
    /// Refuses to split below minimum pane size.
    pub fn toggle_split(&mut self) {
        let next = self.layout.cycle();

        // Check if the resulting split would be below minimum size
        if !self.can_support_layout(&next) {
            // Can't split — terminal too small. Log and stay on Single.
            tracing::warn!(
                cols = self.total_cols,
                rows = self.total_rows,
                "cannot split: terminal too small for minimum pane size"
            );
            // Still update layout for cycle state, but collapse immediately
            self.layout = PaneLayout::Single;
            self.collapse_to_one();
            return;
        }

        let target_count = next.pane_count();

        if target_count > self.panes.len() {
            // Add panes to reach target count
            while self.panes.len() < target_count {
                self.panes.push(Pane::new(PaneContent::Conversation));
            }
        } else if target_count < self.panes.len() {
            self.panes.truncate(target_count);
            self.focused_idx = self.focused_idx.min(self.panes.len().saturating_sub(1));
        }

        self.layout = next;
        self.update_focus_state();
    }

    /// Move focus to the next pane (Ctrl+Tab).
    pub fn cycle_focus(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focused_idx = (self.focused_idx + 1) % self.panes.len();
        self.update_focus_state();
    }

    /// Move focus to the previous pane (Shift+Ctrl+Tab).
    pub fn cycle_focus_backward(&mut self) {
        if self.panes.is_empty() {
            return;
        }
        self.focused_idx = if self.focused_idx == 0 {
            self.panes.len() - 1
        } else {
            self.focused_idx - 1
        };
        self.update_focus_state();
    }

    /// Close the focused pane (Ctrl+W). Returns error if only one pane exists.
    pub fn close_focused(&mut self) -> Result<(), &'static str> {
        if self.panes.len() <= 1 {
            return Err("cannot close the last pane");
        }
        self.panes.remove(self.focused_idx);
        self.focused_idx = self.focused_idx.min(self.panes.len() - 1);

        // Update layout to match new pane count
        if self.panes.len() == 1 {
            self.layout = PaneLayout::Single;
        }

        self.update_focus_state();
        Ok(())
    }

    /// Resize the focused split boundary by `delta` percentage points (Ctrl+Arrow).
    ///
    /// `delta` is positive to expand the primary pane, negative to shrink.
    pub fn resize_split(&mut self, delta: i16) {
        self.layout = self.layout.resize(delta);
    }

    /// Handle a terminal resize event (crossterm Event::Resize).
    ///
    /// No dirty flag needed — ratatui renders on every frame.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.total_cols = cols;
        self.total_rows = rows;

        // Check if current layout still fits; collapse if too small
        if !self.can_support_layout(&self.layout.clone()) {
            self.layout = PaneLayout::Single;
            self.collapse_to_one();
        }
    }

    /// Add a pane programmatically (e.g. for agent team members).
    pub fn add_pane(&mut self, content: PaneContent) {
        self.panes.push(Pane::new(content));
    }

    // ── Keybindings (for validation) ─────────────────────────────────────────

    /// Return all registered keybindings as (key, action) pairs.
    ///
    /// Guaranteed: `Ctrl+\` is NOT in this list (SIGQUIT risk).
    /// Guaranteed: `Meta+S` is registered for split toggle.
    pub fn keybindings(&self) -> Vec<(String, String)> {
        vec![
            ("Meta+S".into(), "toggle split".into()),
            ("Ctrl+Tab".into(), "cycle focus".into()),
            ("Ctrl+W".into(), "close pane".into()),
            ("Ctrl+Right".into(), "resize split right".into()),
            ("Ctrl+Left".into(), "resize split left".into()),
            ("Ctrl+Up".into(), "resize split up".into()),
            ("Ctrl+Down".into(), "resize split down".into()),
        ]
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn update_focus_state(&mut self) {
        for (i, pane) in self.panes.iter_mut().enumerate() {
            pane.set_focused(i == self.focused_idx);
        }
    }

    fn collapse_to_one(&mut self) {
        self.panes.truncate(1);
        self.focused_idx = 0;
        self.update_focus_state();
    }

    /// Check if the given layout can be supported by the current terminal size.
    fn can_support_layout(&self, layout: &PaneLayout) -> bool {
        match layout {
            PaneLayout::Single => true,
            PaneLayout::HorizontalSplit(_) => {
                // Each pane gets half the rows — both must be >= min
                let half_rows = self.total_rows / 2;
                half_rows >= PANE_MIN_ROWS && self.total_cols >= PANE_MIN_COLS
            }
            PaneLayout::VerticalSplit(_) => {
                // Each pane gets half the cols — both must be >= min
                let half_cols = self.total_cols / 2;
                half_cols >= PANE_MIN_COLS && self.total_rows >= PANE_MIN_ROWS
            }
            PaneLayout::Grid(rows, cols) => {
                let pane_rows = self.total_rows / rows.max(&1);
                let pane_cols = self.total_cols / cols.max(&1);
                pane_rows >= PANE_MIN_ROWS && pane_cols >= PANE_MIN_COLS
            }
        }
    }
}
