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
    /// First 3 lines of output, precomputed for non-transcript views.
    pub truncated_preview: String,
    /// Safe summary of validated tool input, if the tool permits it.
    pub summary: Option<String>,
    /// Transcript line containing the collapsed success marker.
    pub marker_line: Option<usize>,
    /// Number of inline excerpt lines currently inserted after the marker.
    pub expanded_line_count: usize,
    started_at: std::time::Instant,
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
            summary: None,
            marker_line: None,
            expanded_line_count: 0,
            started_at: std::time::Instant::now(),
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

    fn compute_preview(output: &str) -> String {
        let lines: Vec<&str> = output.lines().take(3).collect();
        let preview = lines.join("\n");
        if output.lines().count() > 3 {
            format!("{preview}\n...")
        } else {
            preview
        }
    }

    /// Elapsed lifecycle duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Inline transcript excerpt: the first three lines, capped safely at 600 characters.
    pub fn excerpt_lines(&self) -> Vec<String> {
        let mut excerpt = self.output.lines().take(3).collect::<Vec<_>>().join("\n");
        let has_more = self.output.lines().count() > 3 || excerpt.chars().count() > 600;
        if excerpt.chars().count() > 600 {
            let end = excerpt
                .char_indices()
                .nth(599)
                .map(|(index, _)| index)
                .unwrap_or(excerpt.len());
            excerpt.truncate(end);
        }
        if has_more {
            excerpt.push('…');
        }
        excerpt.lines().map(ToString::to_string).collect()
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
