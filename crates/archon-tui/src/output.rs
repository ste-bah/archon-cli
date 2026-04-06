use std::time::Instant;

// ---------------------------------------------------------------------------
// Thinking state
// ---------------------------------------------------------------------------

/// Tracks the collapsible thinking display.
#[derive(Debug, Clone)]
pub struct ThinkingState {
    /// Full accumulated thinking text.
    pub accumulated: String,
    /// Currently receiving thinking deltas.
    pub active: bool,
    /// User toggled to see the full text.
    pub expanded: bool,
    /// Animation frame for the dot shimmer (Knight Rider style).
    pub dot_offset: usize,
    /// When the current thinking run started (for elapsed time).
    pub start: Option<Instant>,
    /// Duration of the most recent completed thinking run, in milliseconds.
    pub last_duration_ms: u64,
}

impl Default for ThinkingState {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkingState {
    pub fn new() -> Self {
        Self {
            accumulated: String::new(),
            active: false,
            expanded: false,
            dot_offset: 0,
            start: None,
            last_duration_ms: 0,
        }
    }

    /// Append new thinking text. Activates the state if not already active.
    pub fn on_thinking_delta(&mut self, text: &str) {
        if !self.active {
            self.active = true;
            self.start = Some(Instant::now());
        }
        self.accumulated.push_str(text);
    }

    /// Mark the thinking phase as complete.
    pub fn on_thinking_complete(&mut self) {
        if self.active {
            self.last_duration_ms = self
                .start
                .map(|s| s.elapsed().as_millis() as u64)
                .unwrap_or(0);
            self.active = false;
            self.start = None;
        }
    }

    /// Advance the dot animation by one frame.
    pub fn tick_thinking(&mut self) {
        if self.active {
            // 3 dots, bounce cycle = 4 frames (0,1,2,1,0,…)
            self.dot_offset = self.dot_offset.wrapping_add(1);
        }
    }

    /// Toggle between expanded and collapsed views.
    pub fn toggle_expand(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Reset for the next thinking block.
    pub fn reset(&mut self) {
        self.accumulated.clear();
        self.active = false;
        self.expanded = false;
        self.dot_offset = 0;
        self.start = None;
        self.last_duration_ms = 0;
    }

    /// The bright-dot index for the 3-dot Knight Rider bounce (0,1,2,1,0,…).
    pub fn bright_dot_index(&self) -> usize {
        let cycle = 4; // 0→1→2→1 = 4 frames
        let phase = self.dot_offset % cycle;
        if phase < 3 { phase } else { cycle - phase }
    }

    /// Whether there is any accumulated thinking text worth showing.
    pub fn has_content(&self) -> bool {
        !self.accumulated.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tool output state
// ---------------------------------------------------------------------------

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
        format!("{arrow} Tool: {} -- {}{}", self.tool_name, self.status, preview)
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

// ---------------------------------------------------------------------------
// Output buffer
// ---------------------------------------------------------------------------

/// Output buffer -- append-only text buffer for streaming display.
#[derive(Debug, Default)]
pub struct OutputBuffer {
    lines: Vec<String>,
    current_line: String,
    /// Vertical scroll offset (lines from the top). When `scroll_locked` is
    /// false this is ignored and we auto-scroll to the bottom.
    pub scroll_offset: u16,
    /// When true the user has scrolled away from the bottom; new content does
    /// not auto-scroll.
    pub scroll_locked: bool,
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append text (may contain newlines).
    pub fn append(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\n' {
                self.lines.push(std::mem::take(&mut self.current_line));
            } else {
                self.current_line.push(ch);
            }
        }
    }

    /// Append a complete line.
    pub fn append_line(&mut self, line: &str) {
        if !self.current_line.is_empty() {
            self.lines.push(std::mem::take(&mut self.current_line));
        }
        self.lines.push(line.to_string());
    }

    /// Get all completed lines plus the current partial line.
    pub fn all_lines(&self) -> Vec<&str> {
        let mut result: Vec<&str> = self.lines.iter().map(|s| s.as_str()).collect();
        if !self.current_line.is_empty() {
            result.push(&self.current_line);
        }
        result
    }

    /// Total line count (including partial current line).
    pub fn line_count(&self) -> usize {
        self.lines.len() + if self.current_line.is_empty() { 0 } else { 1 }
    }

    /// Clear all content.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.current_line.clear();
        self.scroll_offset = 0;
        self.scroll_locked = false;
    }

    // -- scroll helpers -----------------------------------------------------

    /// Scroll up by `amount` lines (see earlier content). Locks auto-scroll.
    /// `scroll_offset` = lines scrolled UP from the bottom.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        self.scroll_locked = true;
    }

    /// Scroll down by `amount` lines (toward newer content).
    /// If offset reaches 0, unlocks auto-scroll.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        if self.scroll_offset == 0 {
            self.scroll_locked = false;
        }
    }

    /// Jump to the bottom and unlock auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
        self.scroll_locked = false;
    }

    /// Compute the actual scroll position for the `Paragraph::scroll()` call.
    ///
    /// `scroll_offset` = lines scrolled UP from the bottom.
    /// `Paragraph::scroll((y, 0))` expects physical rows from the TOP.
    /// NOTE: ratatui does NOT clamp — passing a value past content shows blank.
    ///
    /// When not scroll-locked: auto-scroll to bottom (return max_scroll).
    /// When locked: return max_scroll - scroll_offset (clamped to zero).
    pub fn effective_scroll(&self, total_wrapped_rows: u16, visible_height: u16) -> u16 {
        let max_scroll = total_wrapped_rows.saturating_sub(visible_height);
        if !self.scroll_locked {
            max_scroll
        } else {
            max_scroll.saturating_sub(self.scroll_offset)
        }
    }

    /// Count total wrapped rows given a terminal width using word-wrap simulation.
    ///
    /// Matches ratatui's `Wrap { trim: false }` behaviour: words are kept together
    /// where possible; only overlong single words are char-split. Simple
    /// `ceil(len/width)` underestimates when words don't align to line boundaries,
    /// causing the scroll position to fall short of the actual bottom.
    pub fn count_wrapped_rows(lines: &[&str], width: u16) -> u16 {
        if width == 0 {
            return lines.len() as u16;
        }
        let w = width as usize;
        let mut total: u32 = 0;

        for &line in lines {
            if line.is_empty() {
                total += 1;
                continue;
            }

            // Fast path: whole line fits on one row
            let char_count = line.chars().count();
            if char_count <= w {
                total += 1;
                continue;
            }

            // Word-wrap: split_inclusive keeps the space attached to the
            // preceding word ("hello ", "world"), mirroring ratatui's greedy fit.
            let mut row_width: usize = 0;
            let mut rows: u32 = 1;

            for token in line.split_inclusive(' ') {
                let token_w = token.chars().count();

                if row_width == 0 {
                    // Start of a new row
                    if token_w >= w {
                        // Single token wider than the row — char-wrap it
                        let extra = (token_w - 1) / w;
                        rows += extra as u32;
                        row_width = token_w - extra * w;
                    } else {
                        row_width = token_w;
                    }
                } else if row_width + token_w <= w {
                    row_width += token_w;
                } else {
                    // Token doesn't fit — start a new row
                    rows += 1;
                    if token_w >= w {
                        let extra = (token_w - 1) / w;
                        rows += extra as u32;
                        row_width = token_w - extra * w;
                    } else {
                        row_width = token_w;
                    }
                }
            }

            total += rows;
        }

        total.min(u16::MAX as u32) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_text_with_newlines() {
        let mut buf = OutputBuffer::new();
        buf.append("hello\nworld\n");
        assert_eq!(buf.all_lines(), vec!["hello", "world"]);
    }

    #[test]
    fn append_streaming_chars() {
        let mut buf = OutputBuffer::new();
        buf.append("H");
        buf.append("e");
        buf.append("l");
        buf.append("lo");
        assert_eq!(buf.all_lines(), vec!["Hello"]);
        assert_eq!(buf.line_count(), 1);
    }

    #[test]
    fn append_line() {
        let mut buf = OutputBuffer::new();
        buf.append_line("first");
        buf.append_line("second");
        assert_eq!(buf.all_lines(), vec!["first", "second"]);
    }

    // -- scroll tests -------------------------------------------------------

    #[test]
    fn scroll_up_locks_and_increases_offset() {
        let mut buf = OutputBuffer::new();
        // scroll_offset = lines scrolled UP from bottom. scroll_up adds.
        buf.scroll_up(5);
        assert!(buf.scroll_locked);
        assert_eq!(buf.scroll_offset, 5);
        buf.scroll_up(3);
        assert_eq!(buf.scroll_offset, 8);
    }

    #[test]
    fn scroll_down_decreases_offset() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 10;
        buf.scroll_down(3);
        assert_eq!(buf.scroll_offset, 7);
        assert!(buf.scroll_locked); // still locked, not at bottom
    }

    #[test]
    fn scroll_down_to_zero_unlocks() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 3;
        buf.scroll_down(5); // saturating_sub: 3 - 5 = 0
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.scroll_locked); // reached bottom, unlocked
    }

    #[test]
    fn scroll_to_bottom_resets() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 10;
        buf.scroll_to_bottom();
        assert_eq!(buf.scroll_offset, 0);
        assert!(!buf.scroll_locked);
    }

    #[test]
    fn effective_scroll_at_bottom() {
        let buf = OutputBuffer::new();
        // Not locked => auto-scroll to bottom => max_scroll = 30 - 10 = 20
        assert_eq!(buf.effective_scroll(30, 10), 20);
    }

    #[test]
    fn count_wrapped_rows_word_wrap_differs_from_char_wrap() {
        // "hi hello world" = 14 chars, width 7
        // Simple ceil(14/7) = 2 — WRONG for word-wrap
        // Word-wrap: "hi " (3) fits row1, "hello " (6) overflows → row2,
        //            "world" (5) overflows row2(6+5=11>7) → row3 = 3 rows
        let lines = ["hi hello world"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 7), 3);
    }

    #[test]
    fn count_wrapped_rows_long_word_char_splits() {
        // "abcdefghijklmnop" = 17 chars, width 5 → ceil(17/5) = 4 rows
        let lines = ["abcdefghijklmnop"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 5), 4);
    }

    #[test]
    fn count_wrapped_rows_fits_on_one_row() {
        let lines = ["hello world"];
        assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 20), 1);
    }

    #[test]
    fn effective_scroll_scrolled_up() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 5;
        // max_scroll = 30 - 10 = 20. effective = 20 - 5 = 15 (scrolled 5 lines up from bottom)
        assert_eq!(buf.effective_scroll(30, 10), 15);
    }

    #[test]
    fn effective_scroll_clamped_to_zero() {
        let mut buf = OutputBuffer::new();
        buf.scroll_locked = true;
        buf.scroll_offset = 100; // way past content
        // max_scroll = 30 - 10 = 20. effective = 20 - 100 = 0 (clamped via saturating_sub)
        assert_eq!(buf.effective_scroll(30, 10), 0);
    }

    // -- thinking state tests -----------------------------------------------

    #[test]
    fn thinking_delta_activates() {
        let mut ts = ThinkingState::new();
        assert!(!ts.active);
        ts.on_thinking_delta("hmm ");
        assert!(ts.active);
        assert_eq!(ts.accumulated, "hmm ");
    }

    #[test]
    fn thinking_complete_records_duration() {
        let mut ts = ThinkingState::new();
        ts.on_thinking_delta("thought");
        ts.on_thinking_complete();
        assert!(!ts.active);
        // duration should be >= 0 (near-instant in tests)
        assert!(ts.last_duration_ms < 1000);
    }

    #[test]
    fn thinking_toggle_expand() {
        let mut ts = ThinkingState::new();
        assert!(!ts.expanded);
        ts.toggle_expand();
        assert!(ts.expanded);
        ts.toggle_expand();
        assert!(!ts.expanded);
    }

    #[test]
    fn thinking_bright_dot_bounces() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        // frame 0 -> dot 0
        assert_eq!(ts.bright_dot_index(), 0);
        ts.dot_offset = 1;
        assert_eq!(ts.bright_dot_index(), 1);
        ts.dot_offset = 2;
        assert_eq!(ts.bright_dot_index(), 2);
        ts.dot_offset = 3;
        assert_eq!(ts.bright_dot_index(), 1); // bounce back
        ts.dot_offset = 4;
        assert_eq!(ts.bright_dot_index(), 0); // back to start
    }

    #[test]
    fn thinking_tick_advances() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        ts.tick_thinking();
        assert_eq!(ts.dot_offset, 1);
        ts.tick_thinking();
        assert_eq!(ts.dot_offset, 2);
    }

    #[test]
    fn thinking_tick_inactive_noop() {
        let mut ts = ThinkingState::new();
        ts.tick_thinking();
        assert_eq!(ts.dot_offset, 0);
    }

    #[test]
    fn thinking_reset_clears() {
        let mut ts = ThinkingState::new();
        ts.on_thinking_delta("some text");
        ts.on_thinking_complete();
        ts.reset();
        assert!(!ts.has_content());
        assert_eq!(ts.last_duration_ms, 0);
    }

    // -- tool output state tests -----------------------------------------------

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
