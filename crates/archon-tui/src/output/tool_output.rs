//! Per-tool-call display state.
//!
//! Relocated from `src/output.rs` (ToolOutputState section, L103-L204 +
//! tests L573-L632) per REM-2h.

/// Display status of a tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolDisplayStatus {
    Running,
    Success,
    Error,
}

impl std::fmt::Display for ToolDisplayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Success => write!(f, "ok"),
            Self::Error => write!(f, "error"),
        }
    }
}

/// Tracks the collapsible display state for a single tool invocation.
#[derive(Debug, Clone)]
pub struct ToolOutputState {
    /// Name of the tool (e.g., "Read", "Write", "Bash").
    pub tool_name: String,
    /// Unique tool_use ID from the API.
    pub tool_id: String,
    /// Current execution status.
    pub status: ToolDisplayStatus,
    /// Full tool output text.
    pub output: String,
    /// Whether the user has expanded this tool block.
    pub expanded: bool,
    /// First 3 lines of output, precomputed for collapsed view.
    pub truncated_preview: String,
}

impl ToolOutputState {
    /// Create a new tool output state (starts as Running, collapsed).
    pub fn new(tool_name: &str, tool_id: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_id: tool_id.to_string(),
            status: ToolDisplayStatus::Running,
            output: String::new(),
            expanded: false,
            truncated_preview: String::new(),
        }
    }

    /// Mark the tool as complete and set output.
    pub fn complete(&mut self, output: &str, is_error: bool) {
        self.status = if is_error {
            ToolDisplayStatus::Error
        } else {
            ToolDisplayStatus::Success
        };
        self.output = output.to_string();
        self.truncated_preview = Self::compute_preview(output);
    }

    /// Toggle between expanded and collapsed views.
    pub fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Compute a preview from the first 3 lines of output.
    fn compute_preview(output: &str) -> String {
        let lines: Vec<&str> = output.lines().take(3).collect();
        let preview = lines.join("\n");
        if output.lines().count() > 3 {
            format!("{preview}\n...")
        } else {
            preview
        }
    }

    /// Format for collapsed display: "arrow Tool: name -- status (preview)"
    pub fn collapsed_line(&self) -> String {
        let arrow = "\u{25b6}"; // ▶
        let first_line = self.output.lines().next().unwrap_or("").trim();
        let preview = if first_line.is_empty() {
            String::new()
        } else {
            format!(" {first_line}")
        };
        format!(
            "{arrow} Tool: {} -- {}{}",
            self.tool_name, self.status, preview
        )
    }

    /// Format for expanded display header: "arrow Tool: name -- status"
    pub fn expanded_header(&self) -> String {
        let arrow = "\u{25bc}"; // ▼
        format!("{arrow} Tool: {} -- {}", self.tool_name, self.status)
    }

    /// Format for brief mode: tool name only, no preview.
    pub fn brief_line(&self) -> String {
        let arrow = "\u{25b6}"; // ▶
        format!("{arrow} {} -- {}", self.tool_name, self.status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_output_new_starts_running_collapsed() {
        let state = ToolOutputState::new("Read", "tool_123");
        assert_eq!(state.tool_name, "Read");
        assert_eq!(state.tool_id, "tool_123");
        assert_eq!(state.status, ToolDisplayStatus::Running);
        assert!(!state.expanded);
        assert!(state.output.is_empty());
    }

    #[test]
    fn tool_output_complete_sets_status() {
        let mut state = ToolOutputState::new("Write", "tool_456");
        state.complete("file written successfully", false);
        assert_eq!(state.status, ToolDisplayStatus::Success);
        assert_eq!(state.output, "file written successfully");

        let mut state2 = ToolOutputState::new("Bash", "tool_789");
        state2.complete("command failed", true);
        assert_eq!(state2.status, ToolDisplayStatus::Error);
    }

    #[test]
    fn tool_output_preview_truncates() {
        let mut state = ToolOutputState::new("Grep", "tool_abc");
        state.complete("line1\nline2\nline3\nline4\nline5", false);
        assert!(state.truncated_preview.contains("line1"));
        assert!(state.truncated_preview.contains("line3"));
        assert!(state.truncated_preview.contains("..."));
        assert!(!state.truncated_preview.contains("line4"));
    }

    #[test]
    fn tool_output_toggle_expand() {
        let mut state = ToolOutputState::new("Read", "tool_def");
        assert!(!state.expanded);
        state.toggle_expand();
        assert!(state.expanded);
        state.toggle_expand();
        assert!(!state.expanded);
    }

    #[test]
    fn tool_output_collapsed_line_format() {
        let mut state = ToolOutputState::new("Bash", "tool_ghi");
        state.complete("hello world\nsecond line", false);
        let line = state.collapsed_line();
        assert!(line.contains("Bash"));
        assert!(line.contains("ok"));
        assert!(line.contains("hello world"));
    }

    #[test]
    fn tool_output_brief_line_no_preview() {
        let mut state = ToolOutputState::new("Read", "tool_jkl");
        state.complete("lots of content here", false);
        let line = state.brief_line();
        assert!(line.contains("Read"));
        assert!(line.contains("ok"));
        assert!(!line.contains("lots of content"));
    }
}
