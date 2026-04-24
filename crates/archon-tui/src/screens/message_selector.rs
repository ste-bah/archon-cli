//! TASK-TUI-620 message-selector overlay (screen module).
//!
//! Scrollable list of prior conversation messages. User navigates via
//! Up/Down arrows; Enter selects and closes. Esc cancels.
//!
//! Gate 2 scope: struct + selection-navigation methods + unit tests.
//!
//! TUI-620-followup landed the ratatui render method below plus the
//! full render/input/truncate wiring in `render/body.rs`,
//! `event_loop/input.rs`, and `src/session.rs`.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

use crate::events::MessageSummary;
use crate::theme::Theme;

pub struct MessageSelector {
    pub messages: Vec<MessageSummary>,
    pub selected_index: usize,
}

impl MessageSelector {
    pub fn new(messages: Vec<MessageSummary>) -> Self {
        Self {
            messages,
            selected_index: 0,
        }
    }

    /// Move selection down by one (wraps to 0 at bottom).
    pub fn select_next(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % self.messages.len();
    }

    /// Move selection up by one (wraps to last at top).
    pub fn select_prev(&mut self) {
        if self.messages.is_empty() {
            return;
        }
        self.selected_index = if self.selected_index == 0 {
            self.messages.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&MessageSummary> {
        self.messages.get(self.selected_index)
    }

    /// Render the message-selector overlay inside `area`.
    ///
    /// Mirrors `render/body.rs::draw_session_picker` — a centered modal
    /// ~9/10 wide, height = items.len() + 3 clamped to fit. Long lists
    /// are scrolled so the currently selected row stays visible: the
    /// visible slice starts at `selected_index.saturating_sub(height-1)`
    /// and spans at most `height` rows.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let overlay_width = (area.width * 9 / 10)
            .max(70)
            .min(area.width.saturating_sub(2));
        let overlay_height = (self.messages.len() as u16 + 3)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = (area.width.saturating_sub(overlay_width)) / 2;
        let y = (area.height.saturating_sub(overlay_height)) / 2;
        let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

        f.render_widget(Clear, overlay_area);

        // Visible slice — keep selected row on-screen when the list is
        // taller than the overlay. Two rows are reserved for borders, so
        // at most `overlay_height - 2` rows are available for items.
        let body_rows = overlay_height.saturating_sub(2) as usize;
        let total = self.messages.len();
        let start = if total <= body_rows {
            0
        } else if self.selected_index >= body_rows {
            self.selected_index + 1 - body_rows
        } else {
            0
        };
        let end = (start + body_rows).min(total);

        let items: Vec<ListItem<'_>> = self.messages[start..end]
            .iter()
            .enumerate()
            .map(|(offset, msg)| {
                let idx = start + offset;
                let style = if idx == self.selected_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let preview = truncate_preview(&msg.preview, 60);
                let ts = msg.timestamp.format("%H:%M:%S");
                let line = format!(" {idx}: {preview} | {ts}");
                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(
                    " /rewind — pick message to rewind to (Up/Down, Enter select, Esc cancel) ",
                )
                .border_style(Style::default().fg(theme.accent)),
        );
        f.render_widget(list, overlay_area);
    }
}

/// Truncate `s` to at most `max` characters (char-boundary safe) with a
/// trailing ellipsis when clipped. Mirrors the preview-shortening used
/// by other overlays (e.g. `session_browser`).
fn truncate_preview(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn fixture(n: usize) -> Vec<MessageSummary> {
        (0..n)
            .map(|i| MessageSummary {
                id: format!("msg-{}", i),
                timestamp: Utc::now(),
                preview: format!("preview-{}", i),
            })
            .collect()
    }

    #[test]
    fn new_starts_at_zero() {
        let sel = MessageSelector::new(fixture(3));
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn select_next_advances() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.select_next();
        assert_eq!(sel.selected_index, 1);
        sel.select_next();
        assert_eq!(sel.selected_index, 2);
    }

    #[test]
    fn select_next_wraps_at_end() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.selected_index = 2;
        sel.select_next();
        assert_eq!(sel.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut sel = MessageSelector::new(fixture(3));
        sel.select_prev();
        assert_eq!(sel.selected_index, 2);
    }

    #[test]
    fn empty_list_noop() {
        let mut sel = MessageSelector::new(vec![]);
        sel.select_next();
        sel.select_prev();
        assert_eq!(sel.selected_index, 0);
        assert!(sel.selected().is_none());
    }

    #[test]
    fn truncate_preview_short_unchanged() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_preview_long_clipped_with_ellipsis() {
        let s = "abcdefghijklmnop";
        let out = truncate_preview(s, 6);
        assert_eq!(out.chars().count(), 6);
        assert!(out.ends_with('…'));
    }
}
