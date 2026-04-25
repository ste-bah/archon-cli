//! Integration tests for DiffView keyboard navigation.
//!
//! Tests the 5 keybindings from REQ-TUI-MOD-008:
//! - `n` — next hunk
//! - `p` — previous hunk
//! - `w` — toggle whitespace visibility
//! - `s` — toggle side-by-side / unified layout
//! - `/` — search within diff

use archon_tui::diff_view::{DiffLine, DiffState, DiffView, Hunk, LayoutMode};

fn sample_diff_state() -> DiffState {
    DiffState {
        hunks: vec![
            Hunk {
                start_line: 1,
                lines: vec![
                    DiffLine::context("fn main() {".into()),
                    DiffLine::deletion("-    old();".into()),
                    DiffLine::addition("+    new();".into()),
                    DiffLine::context("}".into()),
                ],
            },
            Hunk {
                start_line: 10,
                lines: vec![
                    DiffLine::context("fn helper() {".into()),
                    DiffLine::deletion("-    println!(\"deleted\");".into()),
                    DiffLine::addition("+    println!(\"added\");".into()),
                    DiffLine::addition("+    extra();".into()),
                    DiffLine::context("}".into()),
                ],
            },
        ],
        current_hunk: 0,
        show_whitespace: false,
        layout_mode: LayoutMode::Unified,
        search_active: false,
        search_query: String::new(),
    }
}

fn sample_diff_view() -> DiffView {
    DiffView::new(sample_diff_state())
}

#[test]
fn test_n_key_advances_hunk() {
    let mut view = sample_diff_view();
    assert_eq!(view.state.current_hunk, 0);

    view.handle_key('n');

    assert_eq!(
        view.state.current_hunk, 1,
        "n key should advance to next hunk"
    );
}

#[test]
fn test_n_key_at_last_hunk_stays() {
    let mut view = sample_diff_view();
    view.state.current_hunk = 1; // start at last hunk

    view.handle_key('n');

    assert_eq!(view.state.current_hunk, 1, "n key at last hunk should stay");
}

#[test]
fn test_p_key_goes_to_previous_hunk() {
    let mut view = sample_diff_view();
    view.state.current_hunk = 1; // start at second hunk

    view.handle_key('p');

    assert_eq!(
        view.state.current_hunk, 0,
        "p key should go to previous hunk"
    );
}

#[test]
fn test_p_key_at_first_hunk_stays() {
    let mut view = sample_diff_view();
    assert_eq!(view.state.current_hunk, 0);

    view.handle_key('p');

    assert_eq!(
        view.state.current_hunk, 0,
        "p key at first hunk should stay"
    );
}

#[test]
fn test_w_key_toggles_whitespace() {
    let mut view = sample_diff_view();
    assert!(
        !view.state.show_whitespace,
        "whitespace should be hidden by default"
    );

    view.handle_key('w');

    assert!(
        view.state.show_whitespace,
        "w key should toggle whitespace visibility on"
    );

    view.handle_key('w');

    assert!(
        !view.state.show_whitespace,
        "w key should toggle whitespace visibility off"
    );
}

#[test]
fn test_s_key_toggles_layout_mode() {
    let mut view = sample_diff_view();
    assert_eq!(view.state.layout_mode, LayoutMode::Unified);

    view.handle_key('s');

    assert_eq!(
        view.state.layout_mode,
        LayoutMode::SideBySide,
        "s key should toggle to side-by-side"
    );

    view.handle_key('s');

    assert_eq!(
        view.state.layout_mode,
        LayoutMode::Unified,
        "s key should toggle back to unified"
    );
}

#[test]
fn test_slash_key_activates_search() {
    let mut view = sample_diff_view();
    assert!(!view.state.search_active);

    view.handle_key('/');

    assert!(
        view.state.search_active,
        "/ key should activate search mode"
    );
}

#[test]
fn test_search_query_accumulates_chars() {
    let mut view = sample_diff_view();
    view.handle_key('/');

    view.handle_key('f');
    view.handle_key('o');
    view.handle_key('o');

    assert_eq!(
        view.state.search_query, "foo",
        "search should accumulate characters"
    );
}

#[test]
fn test_escape_deactivates_search() {
    let mut view = sample_diff_view();
    view.handle_key('/');
    view.handle_key('f');
    view.handle_key('o');
    view.handle_key('o');
    assert!(view.state.search_active);

    view.handle_key('\x1B'); // Escape

    assert!(!view.state.search_active, "Escape should deactivate search");
}

#[test]
fn test_all_keybindings_from_req() {
    // This test documents the complete set of keybindings from REQ-TUI-MOD-008
    let mut view = sample_diff_view();

    // n - next hunk
    view.handle_key('n');
    assert_eq!(view.state.current_hunk, 1);

    // p - previous hunk
    view.handle_key('p');
    assert_eq!(view.state.current_hunk, 0);

    // w - toggle whitespace
    view.handle_key('w');
    assert!(view.state.show_whitespace);
    view.handle_key('w');
    assert!(!view.state.show_whitespace);

    // s - toggle layout
    view.handle_key('s');
    assert_eq!(view.state.layout_mode, LayoutMode::SideBySide);
    view.handle_key('s');
    assert_eq!(view.state.layout_mode, LayoutMode::Unified);

    // / - search
    view.handle_key('/');
    assert!(view.state.search_active);
    view.handle_key('\x1B'); // escape
    assert!(!view.state.search_active);
}
