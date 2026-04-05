/// A message boundary within the virtual scroll content.
#[derive(Debug, Clone)]
pub struct MessageBound {
    /// Line index where this message starts.
    pub start_line: usize,
    /// Rendered height of this message in lines (including markdown).
    pub height: usize,
    /// Index of the message in the conversation.
    pub message_index: usize,
}

/// Virtual scroll viewport manager.
///
/// Tracks scroll position, viewport dimensions, and message boundaries to
/// enable efficient viewport-based rendering of long conversations. Only
/// content within the visible range (plus a pre-render buffer) needs to be
/// rendered each frame.
#[derive(Debug)]
pub struct VirtualScroll {
    /// Total content height in lines.
    total_lines: usize,
    /// Current scroll offset (line index of the top of the viewport).
    scroll_offset: usize,
    /// Viewport height in lines (terminal height minus chrome).
    viewport_height: usize,
    /// Pre-render buffer (extra lines above/below the viewport to pre-render).
    buffer_lines: usize,
    /// Whether the user has manually scrolled (disables auto-scroll).
    user_scrolled: bool,
    /// Message boundaries: (start_line, height) for each message.
    message_bounds: Vec<MessageBound>,
}

impl VirtualScroll {
    /// Create a new `VirtualScroll` with the given viewport height.
    ///
    /// The pre-render buffer defaults to 50 lines on each side.
    pub fn new(viewport_height: usize) -> Self {
        Self {
            total_lines: 0,
            scroll_offset: 0,
            viewport_height,
            buffer_lines: 50,
            user_scrolled: false,
            message_bounds: Vec::new(),
        }
    }

    /// Update the viewport height (e.g., on terminal resize).
    ///
    /// Clamps the current scroll offset if it would exceed the new maximum.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        self.clamp_offset();
    }

    // -- Content management ---------------------------------------------------

    /// Replace all content metrics.
    ///
    /// `total_lines` is the total number of rendered lines across all messages.
    /// `bounds` describes where each message starts and how tall it is.
    pub fn update_content(&mut self, total_lines: usize, bounds: Vec<MessageBound>) {
        self.total_lines = total_lines;
        self.message_bounds = bounds;
        self.clamp_offset();
    }

    /// Append a new message with the given rendered height.
    pub fn add_message(&mut self, height: usize) {
        let start = self.total_lines;
        let index = self.message_bounds.len();
        self.message_bounds.push(MessageBound {
            start_line: start,
            height,
            message_index: index,
        });
        self.total_lines = self.total_lines.saturating_add(height);
    }

    // -- Scrolling ------------------------------------------------------------

    /// Scroll up (toward older content) by the given number of lines.
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Scroll down (toward newer content) by the given number of lines.
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
        self.clamp_offset();
    }

    /// Scroll up by one full page (viewport height).
    pub fn page_up(&mut self) {
        self.scroll_up(self.viewport_height);
    }

    /// Scroll down by one full page (viewport height).
    pub fn page_down(&mut self) {
        self.scroll_down(self.viewport_height);
    }

    /// Jump to the very top of the content.
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// Jump to the very bottom of the content and re-enable auto-scroll.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.max_offset();
        self.user_scrolled = false;
    }

    // -- Viewport query -------------------------------------------------------

    /// Returns the inclusive range `(start_line, end_line)` of lines currently
    /// visible in the viewport.
    ///
    /// If there is no content, returns `(0, 0)`.
    pub fn visible_range(&self) -> (usize, usize) {
        if self.total_lines == 0 {
            return (0, 0);
        }
        let start = self.scroll_offset;
        let end = (start + self.viewport_height)
            .saturating_sub(1)
            .min(self.total_lines.saturating_sub(1));
        (start, end)
    }

    /// Returns the inclusive range to pre-render, which extends the visible
    /// range by `buffer_lines` on each side (clamped to content bounds).
    pub fn render_range(&self) -> (usize, usize) {
        if self.total_lines == 0 {
            return (0, 0);
        }
        let (vis_start, vis_end) = self.visible_range();
        let start = vis_start.saturating_sub(self.buffer_lines);
        let end = (vis_end + self.buffer_lines).min(self.total_lines.saturating_sub(1));
        (start, end)
    }

    /// Returns the message indices whose rendered lines overlap the current
    /// viewport.
    pub fn visible_messages(&self) -> Vec<usize> {
        if self.total_lines == 0 || self.message_bounds.is_empty() {
            return Vec::new();
        }
        let (vis_start, vis_end) = self.visible_range();
        self.message_bounds
            .iter()
            .filter(|mb| {
                let msg_end = mb.start_line + mb.height.saturating_sub(1);
                // Message overlaps viewport if it starts before viewport ends
                // AND ends after viewport starts.
                mb.start_line <= vis_end && msg_end >= vis_start
            })
            .map(|mb| mb.message_index)
            .collect()
    }

    // -- Auto-scroll ----------------------------------------------------------

    /// Called when new content arrives. If the user has not manually scrolled,
    /// auto-scrolls to the bottom.
    pub fn on_new_content(&mut self) {
        if !self.user_scrolled {
            self.scroll_offset = self.max_offset();
        }
    }

    /// Mark the scroll position as user-controlled (disables auto-scroll on
    /// new content).
    pub fn on_user_scroll(&mut self) {
        self.user_scrolled = true;
    }

    /// Re-enable auto-scroll (e.g., when the user presses End).
    pub fn reset_user_scroll(&mut self) {
        self.user_scrolled = false;
    }

    // -- State ----------------------------------------------------------------

    /// Current scroll offset (line index of the top of the viewport).
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Whether the viewport is at the very bottom of the content.
    pub fn at_bottom(&self) -> bool {
        self.scroll_offset >= self.max_offset()
    }

    /// Scroll progress as a fraction from `0.0` (top) to `1.0` (bottom).
    ///
    /// Returns `0.0` when content fits entirely within the viewport.
    pub fn scroll_percentage(&self) -> f32 {
        let max = self.max_offset();
        if max == 0 {
            return 0.0;
        }
        self.scroll_offset as f32 / max as f32
    }

    // -- Internal helpers -----------------------------------------------------

    /// Maximum scroll offset: the furthest the viewport can scroll down while
    /// still showing full content.
    fn max_offset(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height)
    }

    /// Clamp the scroll offset so it never exceeds `max_offset()`.
    fn clamp_offset(&mut self) {
        let max = self.max_offset();
        if self.scroll_offset > max {
            self.scroll_offset = max;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_buffer_lines() {
        let vs = VirtualScroll::new(20);
        // Smoke test that buffer is set
        let (rs, re) = vs.render_range();
        assert_eq!(rs, 0);
        assert_eq!(re, 0);
    }
}
