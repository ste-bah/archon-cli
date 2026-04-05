//! Tests for TASK-CLI-307: Split Pane TUI Layout.

use archon_tui::pane::{Pane, PaneContent};
use archon_tui::pane_layout::{PaneLayout, split_rect_horizontal, split_rect_vertical};
use archon_tui::pane_manager::{PaneManager, PANE_MIN_COLS, PANE_MIN_ROWS};
use ratatui::layout::Rect;

// ---------------------------------------------------------------------------
// PaneLayout enum
// ---------------------------------------------------------------------------

#[test]
fn layout_default_is_single() {
    let layout = PaneLayout::default();
    assert!(matches!(layout, PaneLayout::Single));
}

#[test]
fn layout_cycle_single_to_horizontal() {
    let layout = PaneLayout::Single;
    let next = layout.cycle();
    assert!(matches!(next, PaneLayout::HorizontalSplit(_)));
}

#[test]
fn layout_cycle_horizontal_to_vertical() {
    let layout = PaneLayout::HorizontalSplit(50);
    let next = layout.cycle();
    assert!(matches!(next, PaneLayout::VerticalSplit(_)));
}

#[test]
fn layout_cycle_vertical_to_single() {
    let layout = PaneLayout::VerticalSplit(50);
    let next = layout.cycle();
    assert!(matches!(next, PaneLayout::Single));
}

#[test]
fn split_ratio_stored() {
    let layout = PaneLayout::HorizontalSplit(40);
    match layout {
        PaneLayout::HorizontalSplit(r) => assert_eq!(r, 40),
        _ => panic!("expected HorizontalSplit"),
    }
}

// ---------------------------------------------------------------------------
// Rect splitting
// ---------------------------------------------------------------------------

#[test]
fn horizontal_split_divides_top_bottom() {
    let area = Rect::new(0, 0, 100, 40);
    let (top, bottom) = split_rect_horizontal(area, 50);
    assert_eq!(top.x, 0);
    assert_eq!(top.y, 0);
    assert_eq!(top.width, 100);
    assert_eq!(bottom.x, 0);
    assert_eq!(bottom.width, 100);
    // top + bottom heights sum to area height
    assert_eq!(top.height + bottom.height, 40);
}

#[test]
fn vertical_split_divides_left_right() {
    let area = Rect::new(0, 0, 100, 40);
    let (left, right) = split_rect_vertical(area, 50);
    assert_eq!(left.y, 0);
    assert_eq!(left.height, 40);
    assert_eq!(right.y, 0);
    assert_eq!(right.height, 40);
    // left + right widths sum to area width
    assert_eq!(left.width + right.width, 100);
}

#[test]
fn horizontal_split_at_30_percent() {
    let area = Rect::new(0, 0, 100, 100);
    let (top, bottom) = split_rect_horizontal(area, 30);
    // 30% of 100 = 30 rows
    assert!(top.height >= 29 && top.height <= 31, "top height ~30, got {}", top.height);
    assert!(bottom.height >= 69 && bottom.height <= 71, "bottom height ~70, got {}", bottom.height);
}

// ---------------------------------------------------------------------------
// Minimum pane size constants
// ---------------------------------------------------------------------------

#[test]
fn min_cols_is_20() {
    assert_eq!(PANE_MIN_COLS, 20);
}

#[test]
fn min_rows_is_5() {
    assert_eq!(PANE_MIN_ROWS, 5);
}

// ---------------------------------------------------------------------------
// PaneManager
// ---------------------------------------------------------------------------

#[test]
fn pane_manager_starts_with_single_pane() {
    let manager = PaneManager::new(100, 40);
    assert_eq!(manager.pane_count(), 1);
}

#[test]
fn pane_manager_default_layout_is_single() {
    let manager = PaneManager::new(100, 40);
    assert!(matches!(manager.layout(), PaneLayout::Single));
}

#[test]
fn pane_manager_initial_focus_is_zero() {
    let manager = PaneManager::new(100, 40);
    assert_eq!(manager.focused_idx(), 0);
}

#[test]
fn toggle_split_adds_second_pane() {
    let mut manager = PaneManager::new(100, 40);
    manager.toggle_split();
    assert_eq!(manager.pane_count(), 2, "after toggle, should have 2 panes");
}

#[test]
fn toggle_split_cycles_layout() {
    let mut manager = PaneManager::new(100, 40);
    assert!(matches!(manager.layout(), PaneLayout::Single));

    manager.toggle_split(); // → HorizontalSplit
    assert!(matches!(manager.layout(), PaneLayout::HorizontalSplit(_)));

    manager.toggle_split(); // → VerticalSplit
    assert!(matches!(manager.layout(), PaneLayout::VerticalSplit(_)));

    manager.toggle_split(); // → Single
    assert!(matches!(manager.layout(), PaneLayout::Single));
}

#[test]
fn cycle_focus_moves_to_next_pane() {
    let mut manager = PaneManager::new(100, 40);
    manager.toggle_split(); // add second pane
    assert_eq!(manager.focused_idx(), 0);
    manager.cycle_focus();
    assert_eq!(manager.focused_idx(), 1);
    manager.cycle_focus();
    assert_eq!(manager.focused_idx(), 0); // wraps
}

#[test]
fn close_pane_removes_second_pane() {
    let mut manager = PaneManager::new(100, 40);
    manager.toggle_split();
    assert_eq!(manager.pane_count(), 2);
    manager.close_focused().unwrap();
    assert_eq!(manager.pane_count(), 1);
}

#[test]
fn close_pane_single_returns_error() {
    let mut manager = PaneManager::new(100, 40);
    let result = manager.close_focused();
    assert!(result.is_err(), "closing the only pane must return error");
}

#[test]
fn ctrl_backslash_not_registered() {
    // Ctrl+\ sends SIGQUIT — must not be registered as a pane action
    let manager = PaneManager::new(100, 40);
    let bindings = manager.keybindings();
    let ctrl_backslash = bindings.iter().find(|(key, _)| {
        // Check for Ctrl+\ in any form
        key.contains("ctrl+\\") || key.contains("ctrl+backslash") || key.contains("ctrl-\\")
    });
    assert!(ctrl_backslash.is_none(), "Ctrl+\\ must NOT be registered: found {:?}", ctrl_backslash);
}

#[test]
fn meta_s_registered_for_split_toggle() {
    let manager = PaneManager::new(100, 40);
    let bindings = manager.keybindings();
    let has_meta_s = bindings.iter().any(|(key, action)| {
        (key.contains("meta+s") || key.contains("Meta+S") || key.to_lowercase().contains("meta+s"))
            && action.contains("split")
    });
    assert!(has_meta_s, "Meta+S must be registered for split toggle");
}

#[test]
fn resize_updates_dimensions() {
    let mut manager = PaneManager::new(100, 40);
    manager.resize(120, 50);
    let area = manager.total_area();
    assert_eq!(area.width, 120);
    assert_eq!(area.height, 50);
}

#[test]
fn split_below_minimum_size_refused() {
    // A 30x8 terminal with min pane 20 cols — horizontal split would give 4 rows (< 5 min)
    let mut manager = PaneManager::new(30, 10);
    // 10 rows / 2 = 5 rows each — exactly at min, should be allowed
    manager.toggle_split();

    // A terminal that's too small for split should refuse
    let mut small = PaneManager::new(30, 8);
    small.toggle_split();
    // After split of 8 rows: each pane gets 4 rows, which is < PANE_MIN_ROWS (5)
    // The manager should collapse or refuse the split
    let pane_count = small.pane_count();
    // Either the split was refused (still 1 pane) or collapsed back
    if pane_count == 2 {
        // If allowed, each pane must be at least min size after collapse
        let rects = small.compute_rects();
        for r in &rects {
            // rects may be adjusted to enforce minimum
            assert!(r.height >= PANE_MIN_ROWS as u16 || r.height == 0,
                "pane height must be >= minimum: {}", r.height);
        }
    }
}

// ---------------------------------------------------------------------------
// Pane struct
// ---------------------------------------------------------------------------

#[test]
fn pane_new_is_not_focused_by_default() {
    let pane = Pane::new(PaneContent::Conversation);
    assert!(!pane.is_focused());
}

#[test]
fn pane_set_focused() {
    let mut pane = Pane::new(PaneContent::Conversation);
    pane.set_focused(true);
    assert!(pane.is_focused());
}

#[test]
fn pane_content_types_exist() {
    let _ = Pane::new(PaneContent::Conversation);
    let _ = Pane::new(PaneContent::Log);
    let _ = Pane::new(PaneContent::ToolOutput);
    let _ = Pane::new(PaneContent::Terminal);
}
