//! Render coverage for the `/search` results overlay.
//!
//! Kept in its own file rather than inline in `search_results.rs` because
//! inlining pushed that module past the 500-line TUI ceiling. It ran as
//! `tests/screens_search_results_render.rs` until #189 Phase 0 made the screen
//! modules crate-private; the screen is genuinely wired — `app.rs` constructs
//! it — so the tests moved in-crate rather than the module being re-opened for
//! a test's benefit.
//!
//! The `build_highlighted_spans` direct-call tests STAY inline in the screen:
//! they exercise a `fn`-private helper this module cannot reach either.

use super::search_results::SearchResults;
use crate::events::FileEntry;
use crate::theme::intj_theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::PathBuf;

/// Render a `SearchResults` overlay into a `TestBackend` and return
/// the flattened buffer string (newline-joined rows).
fn render_to_string(sr: &SearchResults, w: u16, h: u16) -> String {
    let backend = TestBackend::new(w, h);
    let mut terminal = Terminal::new(backend).expect("TestBackend");
    let theme = intj_theme();
    terminal
        .draw(|f| sr.render(f, f.area(), &theme))
        .expect("draw");
    let buffer = terminal.backend().buffer().clone();
    let area = buffer.area;
    let mut s = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            s.push_str(buffer[(x, y)].symbol());
        }
        s.push('\n');
    }
    s
}

#[test]
fn render_empty_results_shows_no_matches_placeholder() {
    let sr = SearchResults::new("nothingmatches".into(), vec![]);
    let body = render_to_string(&sr, 80, 24);
    assert!(
        body.contains("no matches for"),
        "expected `no matches for` placeholder; got:\n{body}"
    );
    assert!(
        body.contains("nothingmatches"),
        "title bar must surface the query; got:\n{body}"
    );
}

#[test]
fn render_single_result_shows_path_and_count() {
    let sr = SearchResults::new(
        "main".into(),
        vec![FileEntry {
            name: "main.rs".into(),
            path: PathBuf::from("/proj/src/main.rs"),
            is_dir: false,
        }],
    );
    let body = render_to_string(&sr, 100, 24);
    assert!(body.contains("/proj/src/main.rs"), "path missing: {body}");
    assert!(
        body.contains("1 match"),
        "title must show match count; got:\n{body}"
    );
}

#[test]
fn render_multi_result_shows_all_paths() {
    let sr = SearchResults::new(
        "rs".into(),
        vec![
            FileEntry {
                name: "a.rs".into(),
                path: PathBuf::from("/p/a.rs"),
                is_dir: false,
            },
            FileEntry {
                name: "b.rs".into(),
                path: PathBuf::from("/p/b.rs"),
                is_dir: false,
            },
            FileEntry {
                name: "c.rs".into(),
                path: PathBuf::from("/p/c.rs"),
                is_dir: false,
            },
        ],
    );
    let body = render_to_string(&sr, 100, 24);
    assert!(body.contains("/p/a.rs"));
    assert!(body.contains("/p/b.rs"));
    assert!(body.contains("/p/c.rs"));
    assert!(
        body.contains("3 match"),
        "title must show 3 matches; got:\n{body}"
    );
}

#[test]
fn render_with_selection_at_middle() {
    // 5 entries, selection at middle — verifies select_index >0
    // path through the visible-slice arithmetic.
    let entries: Vec<FileEntry> = (0..5)
        .map(|i| FileEntry {
            name: format!("f{i}.rs"),
            path: PathBuf::from(format!("/p/f{i}.rs")),
            is_dir: false,
        })
        .collect();
    let mut sr = SearchResults::new("rs".into(), entries);
    sr.selected_index = 2;
    let body = render_to_string(&sr, 100, 24);
    for i in 0..5 {
        assert!(body.contains(&format!("/p/f{i}.rs")), "row {i} missing");
    }
}

#[test]
fn render_scrolls_to_keep_selection_on_screen() {
    // 50 entries in a small overlay forces the visible-slice scroll
    // path: when selected_index >= body_rows the slice shifts so the
    // selection stays on-screen.
    let entries: Vec<FileEntry> = (0..50)
        .map(|i| FileEntry {
            name: format!("file{i:02}.rs"),
            path: PathBuf::from(format!("/p/file{i:02}.rs")),
            is_dir: false,
        })
        .collect();
    let mut sr = SearchResults::new("file".into(), entries);
    sr.selected_index = 40; // near the end
    let body = render_to_string(&sr, 80, 14);
    assert!(
        body.contains("/p/file40.rs"),
        "selected entry must be on-screen; got:\n{body}"
    );
    assert!(
        !body.contains("/p/file00.rs"),
        "early entries must scroll off-screen; got:\n{body}"
    );
}

#[test]
fn render_small_terminal_does_not_panic() {
    // Defensive: tiny terminal still renders without panic. Exercises
    // the `.max(70)` / `.saturating_sub(2)` clamps.
    let sr = SearchResults::new(
        "x".into(),
        vec![FileEntry {
            name: "x.rs".into(),
            path: PathBuf::from("/x.rs"),
            is_dir: false,
        }],
    );
    let body = render_to_string(&sr, 30, 8);
    assert!(!body.is_empty());
}
