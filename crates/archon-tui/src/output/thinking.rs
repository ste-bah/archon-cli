//! Collapsible thinking-display state.
//!
//! Relocated from `src/output.rs` (ThinkingState section, L7-L97 + tests
//! L499-L569) per REM-2h.

use std::cell::Cell;
use std::time::Instant;

/// A completed, bounded thinking block retained for the current session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThinkingBlock {
    pub text: String,
    pub duration_ms: u64,
    pub marker_line: usize,
    pub expanded: bool,
}

/// Tracks the collapsible thinking display.
#[derive(Debug, Clone)]
pub struct ThinkingState {
    /// Full accumulated thinking text for the active block.
    pub accumulated: String,
    /// Currently receiving thinking deltas.
    pub active: bool,
    /// Active content is an unapproved interactive preview.
    pub transient: bool,
    /// User toggled to see the full text.
    pub expanded: bool,
    /// Animation frame for the dot shimmer (Knight Rider style).
    pub dot_offset: usize,
    /// When the current thinking run started (for elapsed time).
    pub start: Option<Instant>,
    /// Duration of the most recent completed thinking run, in milliseconds.
    pub last_duration_ms: u64,
    /// Wrapped rows scrolled up from the bottom of the expanded active block.
    pub scroll_offset: usize,
    /// Absolute wrapped-row position used after jumping to the top.
    scroll_from_top: Option<usize>,
    /// Last rendered maximum scroll, used to restore follow mode from top-origin navigation.
    last_max_scroll: Cell<usize>,
    /// Whether expanded thinking is detached from auto-follow.
    pub scroll_locked: bool,
}

impl Default for ThinkingState {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkingState {
    /// Maximum UTF-8 bytes retained for one active thinking block.
    pub const MAX_CAPTURE_BYTES: usize = 256 * 1024;

    pub fn new() -> Self {
        Self {
            accumulated: String::new(),
            active: false,
            transient: false,
            expanded: false,
            dot_offset: 0,
            start: None,
            last_duration_ms: 0,
            scroll_offset: 0,
            scroll_from_top: None,
            last_max_scroll: Cell::new(0),
            scroll_locked: false,
        }
    }

    /// Append new thinking text. Activates the state if not already active.
    pub fn on_thinking_delta(&mut self, text: &str) {
        if !self.active {
            self.active = true;
            self.expanded = false;
            self.reset_scroll();
            self.start = Some(Instant::now());
        }
        self.accumulated.push_str(text);
        self.retain_bounded_tail();
    }

    /// Append an unapproved thinking preview for interactive display.
    pub fn on_transient_thinking_delta(&mut self, text: &str) {
        if !self.active {
            self.transient = true;
        }
        self.on_thinking_delta(text);
    }

    /// Mark the active preview as approved for normal completion/archive.
    pub fn commit_preview(&mut self) {
        self.transient = false;
    }

    /// Clear an unapproved preview without creating history.
    pub fn discard_preview(&mut self) {
        if self.transient {
            self.reset();
        }
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
            self.reset_scroll();
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
        self.reset_scroll();
    }

    pub fn scroll_up(&mut self, amount: u16) {
        if let Some(position) = self.scroll_from_top.as_mut() {
            *position = position.saturating_sub(amount as usize);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_add(amount as usize);
        }
        self.scroll_locked = true;
    }

    pub fn scroll_down(&mut self, amount: u16) {
        if let Some(position) = self.scroll_from_top {
            let next = position.saturating_add(amount as usize);
            if next >= self.last_max_scroll.get() {
                self.reset_scroll();
            } else {
                self.scroll_from_top = Some(next);
            }
            return;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(amount as usize);
        if self.scroll_offset == 0 {
            self.scroll_locked = false;
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.scroll_from_top = Some(0);
        self.scroll_locked = true;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.reset_scroll();
    }

    pub fn effective_scroll(&self, total_rows: usize, visible_height: u16) -> usize {
        let max_scroll = total_rows.saturating_sub(visible_height as usize);
        self.last_max_scroll.set(max_scroll);
        if let Some(position) = self.scroll_from_top {
            return position.min(max_scroll);
        }
        if self.scroll_locked {
            max_scroll.saturating_sub(self.scroll_offset)
        } else {
            max_scroll
        }
    }

    fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
        self.scroll_from_top = None;
        self.scroll_locked = false;
    }

    /// Reset for the next thinking block.
    pub fn reset(&mut self) {
        self.accumulated.clear();
        self.active = false;
        self.transient = false;
        self.expanded = false;
        self.dot_offset = 0;
        self.start = None;
        self.last_duration_ms = 0;
        self.reset_scroll();
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

    fn retain_bounded_tail(&mut self) {
        if self.accumulated.len() <= Self::MAX_CAPTURE_BYTES {
            return;
        }
        let mut start = self.accumulated.len() - Self::MAX_CAPTURE_BYTES;
        while !self.accumulated.is_char_boundary(start) {
            start += 1;
        }
        self.accumulated.drain(..start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn page_down_to_tail_restores_follow_for_later_thinking() {
        let mut ts = ThinkingState::new();
        ts.on_thinking_delta("initial");
        ts.scroll_to_top();
        assert_eq!(ts.effective_scroll(20, 5), 0);

        ts.scroll_down(100);
        assert_eq!(ts.effective_scroll(20, 5), 15);
        assert!(!ts.scroll_locked);

        assert_eq!(ts.effective_scroll(30, 5), 25);
    }

    #[test]
    fn thinking_bright_dot_bounces() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        assert_eq!(ts.bright_dot_index(), 0);
        ts.dot_offset = 1;
        assert_eq!(ts.bright_dot_index(), 1);
        ts.dot_offset = 2;
        assert_eq!(ts.bright_dot_index(), 2);
        ts.dot_offset = 3;
        assert_eq!(ts.bright_dot_index(), 1);
        ts.dot_offset = 4;
        assert_eq!(ts.bright_dot_index(), 0);
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
    fn thinking_scroll_state_resets_on_collapse_completion_and_new_block() {
        let mut ts = ThinkingState::new();
        ts.on_thinking_delta("first block");
        ts.toggle_expand();
        ts.scroll_up(10);
        assert_eq!(ts.scroll_offset, 10);
        assert!(ts.scroll_locked);

        ts.toggle_expand();
        assert_eq!(ts.scroll_offset, 0);
        assert!(!ts.scroll_locked);

        ts.toggle_expand();
        ts.scroll_up(4);
        ts.on_thinking_complete();
        assert_eq!(ts.scroll_offset, 0);
        assert!(!ts.scroll_locked);

        ts.accumulated.clear();
        ts.on_thinking_delta("second block");
        assert_eq!(ts.scroll_offset, 0);
        assert!(!ts.scroll_locked);
    }

    #[test]
    fn thinking_scroll_effective_position_clamps_from_bottom() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        ts.expanded = true;
        assert_eq!(ts.effective_scroll(30, 10), 20);

        ts.scroll_up(6);
        assert_eq!(ts.effective_scroll(30, 10), 14);
        ts.scroll_to_top();
        assert_eq!(ts.effective_scroll(30, 10), 0);
        ts.scroll_to_bottom();
        assert_eq!(ts.effective_scroll(30, 10), 20);
    }

    #[test]
    fn thinking_scroll_supports_more_than_u16_wrapped_rows() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        ts.expanded = true;

        assert_eq!(ts.effective_scroll(70_000, 10), 69_990);
        ts.scroll_up(60_000);
        ts.scroll_up(5_540);
        assert_eq!(ts.effective_scroll(70_000, 10), 4_450);
        ts.scroll_to_top();
        assert_eq!(ts.effective_scroll(70_000, 10), 0);
    }

    #[test]
    fn thinking_can_scroll_down_after_jumping_to_top() {
        let mut ts = ThinkingState::new();
        ts.active = true;
        ts.expanded = true;
        ts.scroll_to_top();
        assert_eq!(ts.effective_scroll(70_000, 10), 0);

        ts.scroll_down(10);
        assert_eq!(ts.effective_scroll(70_000, 10), 10);
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
}
