//! Context window visualization widget (REQ-MOD-018).
//!
//! Displays a horizontal gauge + sparkline of model context usage,
//! with a warn color band when usage >= 90%.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    symbols::block,
    widgets::Widget,
};
use std::collections::VecDeque;

const HISTORY_MAX: usize = 60;

/// Visualizes model context window token usage.
pub struct ContextViz {
    /// Currently used tokens (non-reserved).
    pub used: usize,
    /// Reserved tokens (already committed, not yet generated).
    pub reserved: usize,
    /// Maximum context window size.
    pub max: usize,
    history: VecDeque<usize>,
}

impl ContextViz {
    /// Create a new ContextViz with the given max context window size.
    pub fn new(max: usize) -> Self {
        Self {
            used: 0,
            reserved: 0,
            max,
            history: VecDeque::with_capacity(HISTORY_MAX),
        }
    }

    /// Update the context usage values.
    ///
    /// - `used`: non-reserved tokens consumed
    /// - `reserved`: tokens committed via tool calls
    /// - `max`: total context window capacity
    pub fn update(&mut self, used: usize, reserved: usize, max: usize) {
        self.used = used;
        self.reserved = reserved;
        self.max = max;
        self.history.push_back(used);
        while self.history.len() > HISTORY_MAX {
            self.history.pop_front();
        }
    }

    /// Fill percentage of the context window (0.0 ..= 100.0).
    /// Combines `used` + `reserved` as both count against the window.
    pub fn fill_percent(&self) -> f64 {
        if self.max == 0 {
            return 0.0;
        }
        ((self.used + self.reserved) as f64 / self.max as f64) * 100.0
    }

    /// True when context usage is at or above the 90% warn threshold.
    pub fn is_warn(&self) -> bool {
        self.fill_percent() >= 90.0
    }

    /// Number of history entries currently stored.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Human-readable usage summary for debugging/logs.
    pub fn usage_string(&self) -> String {
        format!(
            "context: {}/{} ({:.1}%)",
            self.used + self.reserved,
            self.max,
            self.fill_percent()
        )
    }
}

impl Widget for ContextViz {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 1 {
            return;
        }

        let pct = self.fill_percent();
        let warn = pct >= 90.0;

        // ---- Gauge bar ----
        let gauge_width = (area.width - 3).max(1) as f64;
        let fill_cells = ((pct / 100.0) * gauge_width).ceil() as u16;

        let base_style = if warn {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Yellow)
        };

        let label_style = if warn {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::Green)
        };

        // Background cells (empty part of bar)
        let bg_style = Style::default().fg(Color::DarkGray);
        for x in 0..area.width {
            buf.set_string(area.x + x, area.y, " ", bg_style);
        }

        // Filled gauge cells
        for x in 0..fill_cells {
            let cell = if x as u16 == fill_cells - 1 && pct < 100.0 {
                "▓"
            } else {
                "█"
            };
            buf.set_string(area.x + x, area.y, cell, base_style);
        }

        // ---- Percentage label ----
        let label = format!(" {:5.1}%", pct);
        let label_x = area.x + 1;
        buf.set_string(label_x, area.y, label.as_str(), label_style);

        // ---- Warn badge ----
        if warn {
            let badge = "WARN";
            let badge_x = area.x + area.width.saturating_sub(badge.len() as u16 + 1);
            for (i, c) in badge.chars().enumerate() {
                let badge_str = c.to_string();
                buf.set_string(
                    badge_x + i as u16,
                    area.y,
                    &badge_str,
                    Style::default().fg(Color::Red),
                );
            }
        }

        // ---- Sparkline (row below gauge) ----
        if area.height >= 2 {
            let spark_chars = ['░', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
            let spark_len = (area.width - 1) as usize;
            let history_vec: Vec<usize> = self.history.iter().copied().collect();
            let base = if self.max == 0 { 0 } else { self.max };

            for (i, cell_x) in (0..spark_len as u16).enumerate() {
                let bucket = if !history_vec.is_empty() {
                    let idx = (i * history_vec.len()) / spark_len.max(1);
                    history_vec.get(idx).copied().unwrap_or(0)
                } else {
                    0
                };
                let level = if base == 0 {
                    0
                } else {
                    ((bucket as f64 / base as f64) * 8.0).round() as usize
                }
                .min(spark_chars.len() - 1);

                let spark_char = spark_chars[level];
                let spark_style = if warn {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                let spark_str = spark_char.to_string();
                buf.set_string(area.x + cell_x, area.y + 1, &spark_str, spark_style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ninety_percent_fill_triggers_warn() {
        let mut viz = ContextViz::new(100_000);
        viz.update(90_000, 5_000, 100_000);
        assert!(viz.is_warn());
    }

    #[test]
    fn below_ninety_no_warn() {
        let mut viz = ContextViz::new(100_000);
        viz.update(80_000, 5_000, 100_000);
        assert!(!viz.is_warn());
    }

    #[test]
    fn history_tracks_updates() {
        let mut viz = ContextViz::new(100_000);
        viz.update(10_000, 0, 100_000);
        viz.update(20_000, 0, 100_000);
        viz.update(30_000, 0, 100_000);
        assert_eq!(viz.history_len(), 3);
    }

    #[test]
    fn zero_max_gives_zero_percent() {
        let viz = ContextViz::new(0);
        assert_eq!(viz.fill_percent(), 0.0);
    }

    #[test]
    fn usage_string_contains_percent() {
        let mut viz = ContextViz::new(100_000);
        viz.update(50_000, 0, 100_000);
        let s = viz.usage_string();
        assert!(s.contains("50.0"));
    }

    #[test]
    fn history_trims_at_max() {
        let mut viz = ContextViz::new(100_000);
        for i in 0..100 {
            viz.update(i * 1000, 0, 100_000);
        }
        assert!(viz.history.len() <= HISTORY_MAX);
    }
}
