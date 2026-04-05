//! Tests for TASK-CLI-314: verbosity rendering rules (brief mode)
//!
//! Covers: brief rendering of tool calls, tool results, thinking blocks;
//! StatusBar brief indicator; full-data preservation.

use archon_tui::status::StatusBar;
use archon_tui::verbosity::{
    format_thinking_brief, format_tool_call_brief, format_tool_result_brief,
};

// ---------------------------------------------------------------------------
// format_tool_call_brief
// ---------------------------------------------------------------------------

#[test]
fn brief_tool_call_shows_name_and_first_line() {
    let result = format_tool_call_brief("Read", "src/main.rs\nsome other content");
    assert!(result.contains("Read"));
    assert!(result.contains("src/main.rs"));
    // second line must not appear
    assert!(!result.contains("some other content"));
}

#[test]
fn brief_tool_call_single_line_input() {
    let result = format_tool_call_brief("Bash", "echo hello");
    assert!(result.contains("Bash"));
    assert!(result.contains("echo hello"));
}

#[test]
fn brief_tool_call_empty_input() {
    let result = format_tool_call_brief("Read", "");
    assert!(result.contains("Read"));
    // should not panic with empty input
}

#[test]
fn brief_tool_call_multiline_truncates() {
    let input = "line1\nline2\nline3\nline4";
    let result = format_tool_call_brief("Write", input);
    assert!(result.contains("Write"));
    assert!(result.contains("line1"));
    assert!(!result.contains("line2"));
    assert!(!result.contains("line3"));
}

// ---------------------------------------------------------------------------
// format_tool_result_brief
// ---------------------------------------------------------------------------

#[test]
fn brief_result_shows_line_count_and_first_line() {
    let content = "fn main() {\n    println!(\"hello\");\n}\n";
    let result = format_tool_result_brief(content);
    // Should mention line count
    assert!(
        result.contains('3') || result.contains("line"),
        "should show line count"
    );
    assert!(result.contains("fn main()"));
}

#[test]
fn brief_result_single_line() {
    let result = format_tool_result_brief("hello world");
    assert!(result.contains("hello world"));
}

#[test]
fn brief_result_empty_content() {
    let result = format_tool_result_brief("");
    // should not panic; may return empty marker
    let _ = result;
}

#[test]
fn brief_result_does_not_include_all_lines() {
    let lines: Vec<String> = (1..=50).map(|i| format!("line {i}")).collect();
    let content = lines.join("\n");
    let result = format_tool_result_brief(&content);
    // Should NOT contain line 50 (deep into the result)
    assert!(!result.contains("line 50"), "brief result must truncate");
}

// ---------------------------------------------------------------------------
// format_thinking_brief
// ---------------------------------------------------------------------------

#[test]
fn brief_thinking_returns_placeholder() {
    let result = format_thinking_brief("long thinking text that should be hidden");
    assert_eq!(
        result, "[thinking]",
        "must be exactly '[thinking]' — no ellipsis"
    );
}

#[test]
fn brief_thinking_empty_input_returns_placeholder() {
    let result = format_thinking_brief("");
    assert_eq!(result, "[thinking]");
}

#[test]
fn brief_thinking_no_ellipsis() {
    let result = format_thinking_brief("anything");
    assert!(!result.contains("..."), "must not contain ellipsis");
}

// ---------------------------------------------------------------------------
// StatusBar — verbose indicator
// ---------------------------------------------------------------------------

#[test]
fn status_bar_verbose_true_no_brief_indicator() {
    let mut bar = StatusBar::default();
    bar.verbose = true;
    let formatted = bar.format();
    assert!(
        !formatted.contains("[brief]"),
        "should not show [brief] in verbose mode"
    );
}

#[test]
fn status_bar_verbose_false_shows_brief_indicator() {
    let mut bar = StatusBar::default();
    bar.verbose = false;
    let formatted = bar.format();
    assert!(
        formatted.contains("[brief]"),
        "must show [brief] indicator in brief mode"
    );
}

#[test]
fn status_bar_default_is_verbose() {
    let bar = StatusBar::default();
    assert!(bar.verbose, "default should be verbose=true");
}
