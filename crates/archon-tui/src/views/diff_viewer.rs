//! Diff viewer view (TASK-AGS-617 stub scaffold).
//!
//! This is the initial scaffolding for the diff viewer overlay. The full
//! migration — pulling in the `similar` crate for proper line/word diffing,
//! introducing an `AgentEvent::FileEdit { path, before, after }` variant,
//! wiring keybindings through `input.rs`, and integrating syntect
//! highlighting — is intentionally deferred to a later phase. See
//! `DIFF_VIEWER_PLACEHOLDER`.
//!
//! For now this module exposes the minimal API surface
//! (`DiffLineKind`, `DiffLine`, `DiffHunk`, `DiffViewerState`, `open`,
//! `draw`, `on_key`, `on_agent_event`) so subsequent work and tests can
//! depend on it. The diff itself is computed via a deliberately naive
//! per-line "all-removed-then-all-added" fallback rather than using the
//! `similar` crate, which is NOT yet a dependency of `archon-tui`.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module. It also MUST NOT import any new
//! crate-level dependencies (no `similar`, no `syntect`) — those arrive
//! with the full migration alongside the `AgentEvent::FileEdit` variant
//! and the corresponding `app.rs` / `input.rs` wiring.

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full diff viewer view content.
///
/// The real diff rendering, `similar`-backed hunk computation,
/// `AgentEvent::FileEdit` plumbing, and syntect highlighting are deferred
/// to a later task. This constant exists so future modules and tests have
/// a single symbol to point at when wiring diff viewer content.
pub const DIFF_VIEWER_PLACEHOLDER: &str =
    "Diff viewer (placeholder — full migration with similar crate + AgentEvent::FileEdit + syntect highlighting in later phase)";

/// Kind of a single line inside a diff hunk.
///
/// Mirrors the three classes the eventual `similar`-backed implementation
/// will produce: unchanged context, an inserted line, and a deleted line.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    /// A line present in both `before` and `after` (unchanged).
    Context,
    /// A line present only in `after` (inserted).
    Added,
    /// A line present only in `before` (deleted).
    Removed,
}

/// A single line within a diff hunk.
///
/// The stub renderer does not yet display these lines — only an empty
/// bordered block is drawn — but they are stored on `DiffViewerState` so
/// tests and downstream callers can inspect the computed diff today.
#[derive(Debug, Clone)]
pub struct DiffLine {
    /// Whether the line is context, an insertion, or a deletion.
    pub kind: DiffLineKind,
    /// Raw text of the line (with the trailing newline stripped).
    pub text: String,
}

/// A contiguous block of diff lines, mirroring a unified-diff hunk.
///
/// The full migration will populate `header` with a real `@@ -a,b +c,d @@`
/// hunk header. The stub leaves it empty but keeps the field so call sites
/// remain stable.
#[derive(Debug, Default, Clone)]
pub struct DiffHunk {
    /// Hunk header (e.g. `@@ -1,3 +1,4 @@`). Empty in the stub.
    pub header: String,
    /// Lines belonging to this hunk, in display order.
    pub lines: Vec<DiffLine>,
}

/// State for the diff viewer overlay.
///
/// Tracks the computed hunks, cursor position over hunks, the path the
/// diff was opened against, and an optional language hint for future
/// syntect highlighting. The stub does not yet use `language`; it exists
/// so call sites stay stable across the full migration.
#[derive(Debug, Default, Clone)]
pub struct DiffViewerState {
    /// Computed hunks for the currently open diff. The stub always
    /// produces at most a single hunk via the naive line diff in `open`.
    pub hunks: Vec<DiffHunk>,
    /// Index of the currently highlighted hunk. Always clamped to
    /// `0..hunks.len()` (or `0` when `hunks` is empty).
    pub cursor: usize,
    /// Path the diff was opened against, if any. Set by `open` and
    /// cleared on `Default`.
    pub path: Option<PathBuf>,
    /// Optional language hint for syntect highlighting. Always `None` in
    /// the stub; populated by the full migration.
    pub language: Option<String>,
}

/// Open (or re-open) the diff viewer against `path` with the given
/// `before` and `after` contents.
///
/// Stub behaviour: computes a naive line diff by splitting both inputs on
/// `'\n'` and emitting every `before` line as `Removed` followed by every
/// `after` line as `Added`, all wrapped in a single hunk. The real
/// implementation will use the `similar` crate to produce proper
/// unified-diff hunks with context, but `similar` is NOT yet a dependency
/// of `archon-tui` (Path A defers the dep bump to a later phase).
///
/// Always sets `state.path = Some(path)`, resets `state.cursor` to `0`,
/// and clears `state.language` (still no syntect integration in the
/// stub).
pub fn open(state: &mut DiffViewerState, path: PathBuf, before: &str, after: &str) {
    let mut hunk = DiffHunk::default();

    for line in before.split('\n') {
        hunk.lines.push(DiffLine {
            kind: DiffLineKind::Removed,
            text: line.to_string(),
        });
    }
    for line in after.split('\n') {
        hunk.lines.push(DiffLine {
            kind: DiffLineKind::Added,
            text: line.to_string(),
        });
    }

    state.hunks = vec![hunk];
    state.cursor = 0;
    state.path = Some(path);
    state.language = None;
}

/// Draw the diff viewer view into `area`.
///
/// Renders an empty bordered block titled "Diff" — actual hunk rendering,
/// gutter glyphs, and syntect highlighting are deferred to the full
/// migration.
pub fn draw(frame: &mut Frame, area: Rect, _state: &DiffViewerState) {
    let block = Block::default().borders(Borders::ALL).title("Diff");
    frame.render_widget(block, area);
}

/// Handle a key event for the diff viewer view.
///
/// Returns `false` because this scaffold does not yet consume input via
/// the parent app event loop in any meaningful way. Cursor movement is
/// clamped to `0..hunks.len()`:
///
/// * `Down` / `j` / `n` — advance cursor by one hunk (clamped at the end)
/// * `Up`   / `k` / `p` — retreat cursor by one hunk (saturating at zero)
/// * `Esc`              — reset cursor to the top of the diff
///
/// All other keys are ignored. The full migration will replace this with
/// the real keymap dispatched through `input.rs`.
pub fn on_key(state: &mut DiffViewerState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('n') => {
            let max_index = state.hunks.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('p') => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Esc => {
            state.cursor = 0;
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will react to the eventual
/// `AgentEvent::FileEdit { path, before, after }` variant by calling
/// `open` with the carried payload. That variant does NOT yet exist on
/// `crate::app::AgentEvent` — Path A explicitly defers adding it.
pub fn on_agent_event(_state: &mut DiffViewerState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;
    use std::path::PathBuf;

    fn hunk() -> DiffHunk {
        DiffHunk::default()
    }

    #[test]
    fn diff_viewer_state_default_empty() {
        let state = DiffViewerState::default();
        assert!(state.hunks.is_empty(), "default hunks should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
        assert!(state.path.is_none(), "default path should be None");
        assert!(state.language.is_none(), "default language should be None");
    }

    #[test]
    fn open_naive_diff_produces_added_and_removed() {
        let mut state = DiffViewerState::default();
        open(&mut state, PathBuf::from("foo.rs"), "a\nb", "a\nc");
        assert!(
            !state.hunks.is_empty(),
            "open should populate at least one hunk"
        );
        let lines: Vec<&DiffLine> = state.hunks.iter().flat_map(|h| h.lines.iter()).collect();
        assert!(
            lines.iter().any(|l| l.kind == DiffLineKind::Added),
            "naive diff should include at least one Added line"
        );
        assert!(
            lines.iter().any(|l| l.kind == DiffLineKind::Removed),
            "naive diff should include at least one Removed line"
        );
    }

    #[test]
    fn open_sets_path_and_resets_cursor() {
        let mut state = DiffViewerState {
            cursor: 99,
            ..Default::default()
        };
        let path = PathBuf::from("bar.rs");
        open(&mut state, path.clone(), "old", "new");
        assert_eq!(state.cursor, 0, "open should reset cursor to 0");
        assert_eq!(
            state.path,
            Some(path),
            "open should record the provided path"
        );
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = DiffViewerState {
            hunks: vec![hunk(), hunk()],
            cursor: 0,
            ..Default::default()
        };
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = DiffViewerState {
            hunks: vec![hunk()],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor, 0,
            "Down on single-hunk diff should keep cursor clamped at 0"
        );
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = DiffViewerState {
            hunks: vec![hunk()],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");
    }

    #[test]
    fn on_key_n_advances_like_down() {
        let mut state = DiffViewerState {
            hunks: vec![hunk(), hunk()],
            cursor: 0,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Char('n'));
        assert_eq!(state.cursor, 1, "n should advance cursor like Down");
    }

    #[test]
    fn on_key_p_decrements_like_up() {
        let mut state = DiffViewerState {
            hunks: vec![hunk(), hunk()],
            cursor: 1,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Char('p'));
        assert_eq!(state.cursor, 0, "p should decrement cursor like Up");
    }

    #[test]
    fn on_key_esc_resets_cursor() {
        let mut state = DiffViewerState {
            hunks: vec![hunk(), hunk(), hunk()],
            cursor: 2,
            ..Default::default()
        };
        on_key(&mut state, KeyCode::Esc);
        assert_eq!(state.cursor, 0, "Esc should reset cursor to 0");
    }

    #[test]
    fn draw_does_not_panic() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = DiffViewerState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on default state");
    }
}
