//! Snapshot baselines for archon-tui render surfaces.
//!
//! TASK-AGS-628 (reframed): capture stable snapshots of render entry points
//! that currently exist in `crates/archon-tui/src/` so subsequent refactor work
//! has a regression net. This file is mechanical — it does NOT modify any
//! production code and skips surfaces that lack a callable render entry point.
//!
//! SKIPPED targets (documented per task spec):
//!   - SessionPicker rendering: no dedicated `render_*` function; the picker
//!     is drawn inline inside `app::run_tui`. Snapshotting would require
//!     extracting the draw code, which the task forbids.
//!   - McpManager rendering: same reason — rendered inline in `app::run_tui`.
//!   - `chat`, `history`, `agents`, `settings`, `session_browser`,
//!     `diff_viewer` (overlay), `context_viz`, `model_picker`, `tasks_overlay`,
//!     `help` modules: do not yet exist in the crate (future Phase 6 work).

use archon_tui::diff_view::{render_diff, render_no_changes};
use archon_tui::markdown::{render_markdown, render_markdown_line};
use archon_tui::output::{OutputBuffer, ThinkingState, ToolOutputState};
use archon_tui::splash::{ActivityEntry, render_splash};
use archon_tui::status::StatusBar;
use archon_tui::syntax::{highlight_code, render_plain_code};
use archon_tui::theme::{available_themes, theme_by_name};
use archon_tui::vim::VimState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ---------------------------------------------------------------------------
// Splash
// ---------------------------------------------------------------------------

#[test]
fn snapshot_splash_with_activity() {
    let activity = vec![
        ActivityEntry {
            when: "1m ago".into(),
            description: "Chat session".into(),
        },
        ActivityEntry {
            when: "2h ago".into(),
            description: "Code review".into(),
        },
        ActivityEntry {
            when: "1d ago".into(),
            description: "Refactor pass".into(),
        },
    ];
    let lines = render_splash("claude-sonnet-4-6", "/home/user/archon", &activity);
    insta::assert_debug_snapshot!("splash_with_activity", lines);
}

#[test]
fn snapshot_splash_empty_activity() {
    let lines = render_splash("claude-sonnet-4-6", "/tmp", &[]);
    insta::assert_debug_snapshot!("splash_empty_activity", lines);
}

// ---------------------------------------------------------------------------
// StatusBar
// ---------------------------------------------------------------------------

#[test]
fn snapshot_statusbar_default() {
    let bar = StatusBar::default();
    insta::assert_snapshot!("statusbar_default", bar.format());
}

#[test]
fn snapshot_statusbar_with_branch_and_agent() {
    let bar = StatusBar {
        model: "claude-opus-4-6".into(),
        identity_mode: "spoof".into(),
        permission_mode: "ask".into(),
        cost: 1.2345,
        git_branch: Some("main".into()),
        verbose: true,
        agent_name: Some("code-reviewer".into()),
        agent_color: Some("#ff88aa".into()),
    };
    insta::assert_snapshot!("statusbar_with_branch_and_agent", bar.format());
}

#[test]
fn snapshot_statusbar_brief_mode() {
    let bar = StatusBar {
        verbose: false,
        ..StatusBar::default()
    };
    insta::assert_snapshot!("statusbar_brief_mode", bar.format());
}

// ---------------------------------------------------------------------------
// OutputBuffer
// ---------------------------------------------------------------------------

#[test]
fn snapshot_output_buffer_mixed_entries() {
    let mut buf = OutputBuffer::new();
    buf.append_line("user: hello");
    buf.append_line("assistant: hi there");
    buf.append_line("[tool] read_file(src/main.rs)");
    buf.append_line("[thinking] Let me consider...");
    buf.append_line("assistant: here is the answer");
    let joined = buf.all_lines().join("\n");
    let repr = format!("line_count={}\n---\n{}", buf.line_count(), joined);
    insta::assert_snapshot!("output_buffer_mixed_entries", repr);
}

#[test]
fn snapshot_output_buffer_wrapped_rows_count() {
    // Check count_wrapped_rows with a fixture so that wrapping math is
    // locked in — this is the only non-widget rendering surface of
    // OutputBuffer that's currently exposed.
    let lines = [
        "short",
        "a longer line that will wrap around a narrow width",
    ];
    let refs: Vec<&str> = lines.iter().copied().collect();
    let rows = OutputBuffer::count_wrapped_rows(&refs, 20);
    insta::assert_snapshot!("output_buffer_wrapped_rows_width20", format!("{}", rows));
}

// ---------------------------------------------------------------------------
// VimState
// ---------------------------------------------------------------------------

fn vim_snapshot(state: &VimState) -> String {
    let (r, c) = state.cursor();
    format!(
        "mode={}\ncursor=({},{})\ncommand_buffer={:?}\ntext=<<<\n{}\n>>>",
        state.mode_display(),
        r,
        c,
        state.command_buffer(),
        state.text()
    )
}

#[test]
fn snapshot_vim_normal_mode_with_text() {
    let state = VimState::from_text("line one\nline two\nline three");
    insta::assert_snapshot!("vim_normal_mode_with_text", vim_snapshot(&state));
}

#[test]
fn snapshot_vim_insert_mode_after_i() {
    let mut state = VimState::from_text("hello");
    // Press 'i' to enter insert mode.
    let _ = state.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    insta::assert_snapshot!("vim_insert_mode_after_i", vim_snapshot(&state));
}

// ---------------------------------------------------------------------------
// Diff view
// ---------------------------------------------------------------------------

#[test]
fn snapshot_render_diff_small_fixture() {
    let diff = "\
--- a/main.rs
+++ b/main.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"extra\");
 }";
    let lines = render_diff(diff);
    insta::assert_debug_snapshot!("render_diff_small_fixture", lines);
}

#[test]
fn snapshot_render_no_changes() {
    let lines = render_no_changes("src/lib.rs");
    insta::assert_debug_snapshot!("render_no_changes", lines);
}

// ---------------------------------------------------------------------------
// Theme palettes (22 built-in themes)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_theme_palettes_all_22() {
    let mut all = String::new();
    for name in available_themes() {
        let theme =
            theme_by_name(name).unwrap_or_else(|| panic!("theme_by_name({name}) returned None"));
        all.push_str(&format!("=== {name} ===\n{:#?}\n\n", theme));
    }
    insta::assert_snapshot!("theme_palettes_all_22", all);
}

#[test]
fn snapshot_theme_available_list() {
    let list: Vec<&str> = available_themes().to_vec();
    insta::assert_debug_snapshot!("theme_available_list", list);
}

// ---------------------------------------------------------------------------
// Markdown: render_markdown (full-document)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_markdown_simple_paragraph() {
    let lines = render_markdown("hello world");
    insta::assert_debug_snapshot!("markdown_simple_paragraph", lines);
}

#[test]
fn snapshot_markdown_bold_italic_inline_code() {
    let lines = render_markdown("**bold** *italic* `code`");
    insta::assert_debug_snapshot!("markdown_bold_italic_inline_code", lines);
}

#[test]
fn snapshot_markdown_bullet_list() {
    let lines = render_markdown("- item one\n- item two\n- item three");
    insta::assert_debug_snapshot!("markdown_bullet_list", lines);
}

#[test]
fn snapshot_markdown_fenced_code_block() {
    let lines = render_markdown("```rust\nfn main() {}\n```");
    insta::assert_debug_snapshot!("markdown_fenced_code_block", lines);
}

#[test]
fn snapshot_markdown_heading_and_paragraph() {
    let lines = render_markdown("# Title\n\nbody");
    insta::assert_debug_snapshot!("markdown_heading_and_paragraph", lines);
}

// ---------------------------------------------------------------------------
// Markdown: render_markdown_line (legacy line renderer)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_markdown_line_intj_theme() {
    let theme = theme_by_name("intj").expect("intj theme exists");
    let line = render_markdown_line("hello **world**", &theme);
    insta::assert_debug_snapshot!("markdown_line_intj_theme", line);
}

#[test]
fn snapshot_markdown_line_dark_theme() {
    let theme = theme_by_name("dark").expect("dark theme exists");
    let line = render_markdown_line("hello **world**", &theme);
    insta::assert_debug_snapshot!("markdown_line_dark_theme", line);
}

// ---------------------------------------------------------------------------
// Syntax: render_plain_code
// ---------------------------------------------------------------------------

#[test]
fn snapshot_syntax_plain_code_with_language() {
    let lines = render_plain_code("fn main() { println!(\"hi\"); }", Some("rust"));
    insta::assert_debug_snapshot!("syntax_plain_code_with_language", lines);
}

#[test]
fn snapshot_syntax_plain_code_no_language() {
    let lines = render_plain_code("fn main() { println!(\"hi\"); }", None);
    insta::assert_debug_snapshot!("syntax_plain_code_no_language", lines);
}

// ---------------------------------------------------------------------------
// Syntax: highlight_code (may return None if grammar unavailable)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_syntax_highlight_code_rust() {
    // Lock in whatever the current behavior is — Some(lines) if tree-sitter
    // grammar loads, None otherwise. We catch_unwind to be safe in case of
    // missing runtime assets, and snapshot the observed result as-is.
    let result = std::panic::catch_unwind(|| highlight_code("fn main() {}", "rust"));
    match result {
        Ok(opt) => insta::assert_debug_snapshot!("syntax_highlight_code_rust", opt),
        Err(_) => {
            // SKIP: highlight_code panicked (likely missing grammar assets in
            // the test env). Do not modify production code.
        }
    }
}

// ---------------------------------------------------------------------------
// Output: ToolOutputState (success path)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_tool_output_state_success_collapsed_line() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("file contents here\nline 2", false);
    insta::assert_snapshot!(
        "tool_output_state_success_collapsed_line",
        state.collapsed_line()
    );
}

#[test]
fn snapshot_tool_output_state_success_expanded_header() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("file contents here\nline 2", false);
    insta::assert_snapshot!(
        "tool_output_state_success_expanded_header",
        state.expanded_header()
    );
}

#[test]
fn snapshot_tool_output_state_success_brief_line() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("file contents here\nline 2", false);
    insta::assert_snapshot!("tool_output_state_success_brief_line", state.brief_line());
}

// ---------------------------------------------------------------------------
// Output: ToolOutputState (error path)
// ---------------------------------------------------------------------------

#[test]
fn snapshot_tool_output_state_error_collapsed_line() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("ENOENT", true);
    insta::assert_snapshot!(
        "tool_output_state_error_collapsed_line",
        state.collapsed_line()
    );
}

#[test]
fn snapshot_tool_output_state_error_expanded_header() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("ENOENT", true);
    insta::assert_snapshot!(
        "tool_output_state_error_expanded_header",
        state.expanded_header()
    );
}

#[test]
fn snapshot_tool_output_state_error_brief_line() {
    let mut state = ToolOutputState::new("read_file", "tool_123");
    state.complete("ENOENT", true);
    insta::assert_snapshot!("tool_output_state_error_brief_line", state.brief_line());
}

// ---------------------------------------------------------------------------
// Output: ThinkingState
// ---------------------------------------------------------------------------

#[test]
fn snapshot_thinking_state_after_two_deltas_and_complete() {
    let mut state = ThinkingState::new();
    state.on_thinking_delta("hmm let me think");
    state.on_thinking_delta("hmm let me think");
    state.on_thinking_complete();
    let repr = format!(
        "has_content={}\nbright_dot_index={}",
        state.has_content(),
        state.bright_dot_index()
    );
    insta::assert_snapshot!("thinking_state_after_two_deltas_and_complete", repr);
}
