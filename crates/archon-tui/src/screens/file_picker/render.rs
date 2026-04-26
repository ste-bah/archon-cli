//! TASK-#207 SLASH-FILES — render layer for the file-picker overlay.
//!
//! Mirrors `screens/skills_menu.rs::render` and
//! `screens/message_selector.rs::render` — centered overlay ~9/10
//! wide, height clamped to the screen, scrolling visible-slice that
//! keeps the selected row on-screen, selected row in cyan+bold.
//!
//! Each row prefixed with `[D]` for directories and `[F]` for
//! files. The breadcrumb (relative to picker root) is in the title
//! bar so the user always knows where they are.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use super::FilePicker;
use crate::theme::Theme;

impl FilePicker {
    /// Render the file-picker overlay inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let overlay_width = (area.width * 9 / 10)
            .max(70)
            .min(area.width.saturating_sub(2));
        let overlay_height = (self.entries.len() as u16 + 3)
            .min(area.height.saturating_sub(4))
            .max(8);
        let x = (area.width.saturating_sub(overlay_width)) / 2;
        let y = (area.height.saturating_sub(overlay_height)) / 2;
        let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

        f.render_widget(Clear, overlay_area);

        // Visible slice — keep selected row on-screen when the list
        // is taller than the overlay. Two rows are reserved for
        // borders, so at most `overlay_height - 2` rows are
        // available for items.
        let body_rows = overlay_height.saturating_sub(2) as usize;
        let total = self.entries.len();
        let start = if total <= body_rows {
            0
        } else if self.selected_index >= body_rows {
            self.selected_index + 1 - body_rows
        } else {
            0
        };
        let end = (start + body_rows).min(total);

        let items: Vec<ListItem<'_>> = if self.entries.is_empty() {
            vec![ListItem::new(" (empty directory) ").style(Style::default().fg(theme.fg))]
        } else {
            self.entries[start..end]
                .iter()
                .enumerate()
                .map(|(offset, entry)| {
                    let idx = start + offset;
                    let style = if idx == self.selected_index {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.fg)
                    };
                    let badge = if entry.is_dir { "[D]" } else { "[F]" };
                    let name = truncate_chars(&entry.name, 64);
                    let line = format!(" {idx:>3}  {badge}  {name}");
                    ListItem::new(line).style(style)
                })
                .collect()
        };

        let crumb = self.breadcrumb();
        let title = format!(
            " /files — {crumb} (Up/Down, Enter dir-descend or file-pick, Backspace ascend, Esc cancel) "
        );
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(theme.accent)),
        );
        f.render_widget(list, overlay_area);
    }
}

/// Truncate `s` to at most `max` characters (char-boundary safe)
/// with a trailing ellipsis when clipped. Mirrors `skills_menu.rs::
/// truncate_preview`.
fn truncate_chars(s: &str, max: usize) -> String {
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

    #[test]
    fn truncate_chars_short_unchanged() {
        assert_eq!(truncate_chars("hello", 10), "hello");
    }

    #[test]
    fn truncate_chars_long_clipped() {
        let s: String = (0..100).map(|_| 'x').collect();
        let out = truncate_chars(&s, 64);
        assert_eq!(out.chars().count(), 64);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_chars_unicode_safe() {
        let s: String = (0..30).map(|_| 'α').collect();
        let out = truncate_chars(&s, 10);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('…'));
    }
}
