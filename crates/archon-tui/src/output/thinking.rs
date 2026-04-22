//! Collapsible thinking-display state.
//!
//! Relocated from `src/output.rs` (ThinkingState section, L7-L97 + tests
//! L499-L569) per REM-2h.

use std::time::Instant;

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
}
