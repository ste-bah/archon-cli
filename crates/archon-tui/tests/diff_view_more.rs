//! TUI-328: coverage-oriented tests for `diff_view`.
//!
//! Covers `DiffView::handle_key` all branches, `render_current_hunk` for
//! unified + side-by-side + empty + no-hunks paths, and `render_no_changes`.

use archon_tui::diff_view::{
    render_current_hunk, render_no_changes, DiffLine, DiffState, DiffView, Hunk, LayoutMode,
};

fn mk_state(hunks: Vec<Hunk>) -> DiffState {
    DiffState {
        hunks,
        current_hunk: 0,
        show_whitespace: true,
        layout_mode: LayoutMode::Unified,
        search_active: false,
        search_query: String::new(),
    }
}

fn mk_hunk() -> Hunk {
    Hunk {
        start_line: 1,
        lines: vec![
            DiffLine::context("ctx line".into()),
            DiffLine::deletion("old line".into()),
            DiffLine::addition("new line".into()),
        ],
    }
}

// ───────────────────────────────────────────────────────────────────────
// handle_key — every branch
// ───────────────────────────────────────────────────────────────────────

#[test]
fn handle_key_n_advances_within_bounds() {
    let state = mk_state(vec![mk_hunk(), mk_hunk()]);
    let mut view = DiffView::new(state);
    assert_eq!(view.state.current_hunk, 0);
    assert_eq!(view.handle_key('n'), Some(()));
    assert_eq!(view.state.current_hunk, 1);
    // At last hunk, 'n' should not overflow.
    assert_eq!(view.handle_key('n'), Some(()));
    assert_eq!(view.state.current_hunk, 1);
}

#[test]
fn handle_key_p_rewinds_without_underflow() {
    let state = mk_state(vec![mk_hunk(), mk_hunk()]);
    let mut view = DiffView::new(state);
    view.state.current_hunk = 1;
    assert_eq!(view.handle_key('p'), Some(()));
    assert_eq!(view.state.current_hunk, 0);
    assert_eq!(view.handle_key('p'), Some(()));
    assert_eq!(view.state.current_hunk, 0);
}

#[test]
fn handle_key_w_toggles_whitespace() {
    let state = mk_state(vec![mk_hunk()]);
    let mut view = DiffView::new(state);
    let was = view.state.show_whitespace;
    assert_eq!(view.handle_key('w'), Some(()));
    assert_ne!(view.state.show_whitespace, was);
    assert_eq!(view.handle_key('w'), Some(()));
    assert_eq!(view.state.show_whitespace, was);
}

#[test]
fn handle_key_s_toggles_layout_mode() {
    let state = mk_state(vec![mk_hunk()]);
    let mut view = DiffView::new(state);
    assert!(matches!(view.state.layout_mode, LayoutMode::Unified));
    view.handle_key('s');
    assert!(matches!(view.state.layout_mode, LayoutMode::SideBySide));
    view.handle_key('s');
    assert!(matches!(view.state.layout_mode, LayoutMode::Unified));
}

#[test]
fn handle_key_slash_activates_search_and_esc_deactivates() {
    let state = mk_state(vec![mk_hunk()]);
    let mut view = DiffView::new(state);
    assert!(!view.state.search_active);
    view.handle_key('/');
    assert!(view.state.search_active);
    assert!(view.state.search_query.is_empty());
    view.handle_key('\x1B');
    assert!(!view.state.search_active);
}

#[test]
fn handle_key_typing_appends_to_search_query_when_active() {
    let state = mk_state(vec![mk_hunk()]);
    let mut view = DiffView::new(state);
    view.handle_key('/');
    view.handle_key('f');
    view.handle_key('o');
    view.handle_key('o');
    assert_eq!(view.state.search_query, "foo");
}

#[test]
fn handle_key_typing_when_not_searching_returns_none() {
    let state = mk_state(vec![mk_hunk()]);
    let mut view = DiffView::new(state);
    assert_eq!(view.handle_key('z'), None);
    // And search_query must stay empty — the 'z' must not have leaked in.
    assert!(view.state.search_query.is_empty());
}

// ───────────────────────────────────────────────────────────────────────
// render_current_hunk
// ───────────────────────────────────────────────────────────────────────

#[test]
fn render_current_hunk_empty_shows_placeholder() {
    let state = mk_state(vec![]);
    let view = DiffView::new(state);
    let lines = render_current_hunk(&view);
    assert_eq!(lines.len(), 1);
    let text: String = lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text.contains("No hunks"));
}

#[test]
fn render_current_hunk_unified_produces_one_line_per_entry() {
    let state = mk_state(vec![mk_hunk()]);
    let view = DiffView::new(state);
    let lines = render_current_hunk(&view);
    // mk_hunk has three lines; with show_whitespace=true all pass the filter.
    assert_eq!(lines.len(), 3);
}

#[test]
fn render_current_hunk_side_by_side_pairs_del_add() {
    let state = DiffState {
        hunks: vec![mk_hunk()],
        current_hunk: 0,
        show_whitespace: true,
        layout_mode: LayoutMode::SideBySide,
        search_active: false,
        search_query: String::new(),
    };
    let view = DiffView::new(state);
    let lines = render_current_hunk(&view);
    // 1 context + 1 paired del/add row = 2 lines.
    assert_eq!(lines.len(), 2);
}

#[test]
fn render_current_hunk_side_by_side_pure_context_no_pairs() {
    let state = DiffState {
        hunks: vec![Hunk {
            start_line: 1,
            lines: vec![
                DiffLine::context("a".into()),
                DiffLine::context("b".into()),
            ],
        }],
        current_hunk: 0,
        show_whitespace: true,
        layout_mode: LayoutMode::SideBySide,
        search_active: false,
        search_query: String::new(),
    };
    let view = DiffView::new(state);
    let lines = render_current_hunk(&view);
    // Two context lines, no pairs — should render two lines (not the
    // "(empty hunk)" placeholder since context lines push through).
    assert_eq!(lines.len(), 2);
}

#[test]
fn render_current_hunk_side_by_side_all_empty_shows_placeholder() {
    let state = DiffState {
        hunks: vec![Hunk {
            start_line: 1,
            lines: vec![],
        }],
        current_hunk: 0,
        show_whitespace: true,
        layout_mode: LayoutMode::SideBySide,
        search_active: false,
        search_query: String::new(),
    };
    let view = DiffView::new(state);
    let lines = render_current_hunk(&view);
    assert_eq!(lines.len(), 1);
    let text: String = lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text.contains("empty hunk"));
}

// ───────────────────────────────────────────────────────────────────────
// render_no_changes
// ───────────────────────────────────────────────────────────────────────

#[test]
fn render_no_changes_embeds_filename() {
    let lines = render_no_changes("src/foo.rs");
    assert_eq!(lines.len(), 1);
    let text: String = lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(text.contains("src/foo.rs"));
    assert!(text.contains("No changes"));
}
