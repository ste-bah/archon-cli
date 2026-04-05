//! Brief-mode rendering rules for TASK-CLI-314.
//!
//! These are pure functions applied at render time.  Full data is never
//! discarded; brief mode is a display filter only.

// ---------------------------------------------------------------------------
// Tool call formatting
// ---------------------------------------------------------------------------

/// Format a tool call for brief display: `"ToolName: first-line-of-input"`.
///
/// Only the tool name and the first non-empty line of the input are shown.
/// Additional lines are silently elided.
pub fn format_tool_call_brief(tool_name: &str, input: &str) -> String {
    let first_line = input.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        tool_name.to_owned()
    } else {
        format!("{tool_name}: {first_line}")
    }
}

// ---------------------------------------------------------------------------
// Tool result formatting
// ---------------------------------------------------------------------------

/// Format a tool result for brief display: `"[N lines] first-line"`.
///
/// Empty content returns an empty string.
pub fn format_tool_result_brief(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();
    let first = lines.first().copied().unwrap_or("").trim();

    if line_count == 1 {
        first.to_owned()
    } else {
        format!("[{line_count} lines] {first}")
    }
}

// ---------------------------------------------------------------------------
// Thinking formatting
// ---------------------------------------------------------------------------

/// Return the brief placeholder for a thinking block.
///
/// Always returns `"[thinking]"` — no ellipsis, consistent with TUI conventions.
pub fn format_thinking_brief(_thinking_text: &str) -> &'static str {
    "[thinking]"
}
