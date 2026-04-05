//! Ultrathink feature: rainbow-colored keyword with Knight Rider shimmer animation.
//!
//! When the user types "ultrathink" (case-insensitive) in the input field:
//! - The keyword renders with per-character rainbow colors
//! - A shimmer sweeps back and forth across the characters (Knight Rider / KITT scanner)
//! - Effort level is bumped to "high" for that request only

use ratatui::style::Color;

/// Normal rainbow palette for the 10 characters of "ultrathink".
const RAINBOW: &[Color] = &[
    Color::Red,
    Color::Rgb(255, 165, 0), // orange
    Color::Yellow,
    Color::Green,
    Color::Blue,
    Color::Rgb(75, 0, 130),  // indigo
    Color::Rgb(148, 0, 211), // violet
];

/// Brighter shimmer variants (the "hot" char in the sweep).
const RAINBOW_SHIMMER: &[Color] = &[
    Color::Rgb(255, 100, 100), // bright red
    Color::Rgb(255, 200, 100), // bright orange
    Color::Rgb(255, 255, 150), // bright yellow
    Color::Rgb(100, 255, 100), // bright green
    Color::Rgb(100, 150, 255), // bright blue
    Color::Rgb(150, 100, 255), // bright indigo
    Color::Rgb(200, 100, 255), // bright violet
];

/// The keyword we search for (always compared lowercase).
const KEYWORD: &str = "ultrathink";

/// Tracks ultrathink animation state.
#[derive(Debug, Clone)]
pub struct UltrathinkState {
    /// Whether the keyword is currently detected in the input.
    pub active: bool,
    /// Animation frame counter (incremented by tick). Drives the shimmer sweep.
    pub shimmer_offset: usize,
    /// Byte-offset ranges of every "ultrathink" occurrence in the input.
    pub keyword_positions: Vec<(usize, usize)>,
}

impl Default for UltrathinkState {
    fn default() -> Self {
        Self::new()
    }
}

impl UltrathinkState {
    /// Create a new inactive state.
    pub fn new() -> Self {
        Self {
            active: false,
            shimmer_offset: 0,
            keyword_positions: Vec::new(),
        }
    }

    /// Scan input text for "ultrathink" keyword (case-insensitive).
    /// Updates `active` and `keyword_positions`.
    pub fn scan_input(&mut self, text: &str) {
        self.keyword_positions = find_ultrathink_positions(text);
        self.active = !self.keyword_positions.is_empty();
        if !self.active {
            self.shimmer_offset = 0;
        }
    }

    /// Advance the shimmer animation by one frame.
    ///
    /// The shimmer bounces back and forth across the keyword length (10 chars)
    /// like Knight Rider's KITT scanner.
    pub fn tick(&mut self) {
        if self.active {
            // The bounce cycle is 2 * (len - 1) frames for a 10-char keyword = 18
            self.shimmer_offset = self.shimmer_offset.wrapping_add(1);
        }
    }

    /// Get the color for a character at the given byte position in the input.
    ///
    /// Returns `None` if the position is not inside an ultrathink keyword.
    /// Returns `Some(color)` with rainbow + shimmer if it is.
    pub fn color_at(&self, byte_pos: usize) -> Option<Color> {
        for &(start, end) in &self.keyword_positions {
            if byte_pos >= start && byte_pos < end {
                let char_index = byte_pos - start;
                if char_index >= KEYWORD.len() {
                    return None;
                }
                let rainbow_idx = char_index % RAINBOW.len();

                // Knight Rider bounce: the "hot" position oscillates
                // For a 10-char keyword, the bounce cycle is 18 frames (0..9 then 8..1)
                let keyword_len = KEYWORD.len(); // 10
                let cycle_len = (keyword_len - 1) * 2; // 18
                let phase = self.shimmer_offset % cycle_len;
                let hot_pos = if phase < keyword_len {
                    phase
                } else {
                    cycle_len - phase
                };

                if char_index == hot_pos {
                    return Some(RAINBOW_SHIMMER[rainbow_idx]);
                }
                return Some(RAINBOW[rainbow_idx]);
            }
        }
        None
    }
}

/// Find all occurrences of "ultrathink" (case-insensitive) in `text`.
/// Returns `(start_byte, end_byte)` pairs.
pub fn find_ultrathink_positions(text: &str) -> Vec<(usize, usize)> {
    let needle = KEYWORD;
    let needle_len = needle.len(); // 10, ASCII
    let lower = text.to_ascii_lowercase(); // ASCII-only: preserves byte positions for non-ASCII
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(pos) = lower[start..].find(needle) {
        let abs_pos = start + pos;
        positions.push((abs_pos, abs_pos + needle_len));
        start = abs_pos + needle_len;
    }
    positions
}

/// Check if text contains the ultrathink keyword (case-insensitive).
pub fn has_ultrathink_keyword(text: &str) -> bool {
    text.to_lowercase().contains(KEYWORD)
}

/// Build a rainbow-colored `Line` for the status bar indicator "ULTRATHINK".
/// Each character cycles through the rainbow palette with the shimmer applied.
pub fn ultrathink_status_spans(shimmer_offset: usize) -> Vec<(char, Color)> {
    let label = "ULTRATHINK";
    let keyword_len = label.len(); // 10
    let cycle_len = if keyword_len > 1 {
        (keyword_len - 1) * 2
    } else {
        1
    };
    let phase = shimmer_offset % cycle_len;
    let hot_pos = if phase < keyword_len {
        phase
    } else {
        cycle_len - phase
    };

    label
        .chars()
        .enumerate()
        .map(|(i, ch)| {
            let rainbow_idx = i % RAINBOW.len();
            let color = if i == hot_pos {
                RAINBOW_SHIMMER[rainbow_idx]
            } else {
                RAINBOW[rainbow_idx]
            };
            (ch, color)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_positions_basic() {
        let positions = find_ultrathink_positions("hello ultrathink world");
        assert_eq!(positions, vec![(6, 16)]);
    }

    #[test]
    fn find_positions_case_insensitive() {
        let positions = find_ultrathink_positions("UltraThink");
        assert_eq!(positions, vec![(0, 10)]);

        let positions = find_ultrathink_positions("ULTRATHINK");
        assert_eq!(positions, vec![(0, 10)]);
    }

    #[test]
    fn find_positions_multiple() {
        let positions = find_ultrathink_positions("ultrathink and ultrathink");
        assert_eq!(positions, vec![(0, 10), (15, 25)]);
    }

    #[test]
    fn find_positions_empty_on_no_match() {
        let positions = find_ultrathink_positions("nothing here");
        assert!(positions.is_empty());
    }

    #[test]
    fn has_keyword_true() {
        assert!(has_ultrathink_keyword("please ultrathink about this"));
    }

    #[test]
    fn has_keyword_false() {
        assert!(!has_ultrathink_keyword("normal message"));
    }

    #[test]
    fn scan_input_activates() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        assert!(state.active);
        assert_eq!(state.keyword_positions, vec![(0, 10)]);
    }

    #[test]
    fn scan_input_deactivates() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        assert!(state.active);
        state.scan_input("normal text");
        assert!(!state.active);
        assert!(state.keyword_positions.is_empty());
    }

    #[test]
    fn color_at_outside_keyword_returns_none() {
        let mut state = UltrathinkState::new();
        state.scan_input("hello ultrathink world");
        // Position 0 is 'h' in "hello" — outside keyword
        assert!(state.color_at(0).is_none());
        // Position 20 is in "world" — outside keyword
        assert!(state.color_at(20).is_none());
    }

    #[test]
    fn color_at_inside_keyword_returns_some() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        // Every position 0..10 should return Some
        for i in 0..10 {
            assert!(
                state.color_at(i).is_some(),
                "expected color at position {i}"
            );
        }
        // Position 10 is past the keyword
        assert!(state.color_at(10).is_none());
    }

    #[test]
    fn tick_advances_shimmer() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        assert_eq!(state.shimmer_offset, 0);
        state.tick();
        assert_eq!(state.shimmer_offset, 1);
        state.tick();
        assert_eq!(state.shimmer_offset, 2);
    }

    #[test]
    fn shimmer_bounce_produces_bright_color() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        // At frame 0, char 0 should be the shimmer (bright) color
        let color0 = state.color_at(0);
        assert_eq!(color0, Some(RAINBOW_SHIMMER[0]));
        // char 1 should be normal rainbow
        let color1 = state.color_at(1);
        assert_eq!(color1, Some(RAINBOW[1]));

        // Advance to frame 3 — char 3 should be bright
        state.shimmer_offset = 3;
        let color3 = state.color_at(3);
        assert_eq!(color3, Some(RAINBOW_SHIMMER[3 % RAINBOW.len()]));
    }

    #[test]
    fn shimmer_bounce_reverses() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        // Frame 10 means the hot_pos should bounce: cycle_len=18, phase=10
        // hot_pos = 18 - 10 = 8
        state.shimmer_offset = 10;
        let color8 = state.color_at(8);
        assert_eq!(color8, Some(RAINBOW_SHIMMER[8 % RAINBOW.len()]));
        // char 9 should be normal
        let color9 = state.color_at(9);
        assert_eq!(color9, Some(RAINBOW[9 % RAINBOW.len()]));
    }

    #[test]
    fn status_spans_returns_ten_chars() {
        let spans = ultrathink_status_spans(0);
        assert_eq!(spans.len(), 10);
        assert_eq!(spans[0].0, 'U');
        assert_eq!(spans[9].0, 'K');
    }

    #[test]
    fn inactive_tick_does_not_advance() {
        let mut state = UltrathinkState::new();
        state.tick();
        assert_eq!(state.shimmer_offset, 0);
    }

    #[test]
    fn scan_resets_shimmer_when_deactivated() {
        let mut state = UltrathinkState::new();
        state.scan_input("ultrathink");
        state.tick();
        state.tick();
        assert_eq!(state.shimmer_offset, 2);
        state.scan_input("no keyword");
        assert_eq!(state.shimmer_offset, 0);
    }
}
