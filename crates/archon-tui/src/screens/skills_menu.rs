//! TASK-TUI-627 skills-menu overlay (screen module).
//!
//! Mirrors `screens/message_selector.rs` (TUI-620) exactly — scrollable
//! list with up/down navigation, reverse-video selected row, Enter
//! injects `/{skill-name} ` into the input buffer. Render + input
//! routing landed in TUI-627-followup.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

use crate::events::SkillEntry;
use crate::theme::Theme;

pub struct SkillsMenu {
    pub skills: Vec<SkillEntry>,
    pub selected_index: usize,
}

impl SkillsMenu {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self { skills, selected_index: 0 }
    }

    pub fn select_next(&mut self) {
        if self.skills.is_empty() { return; }
        self.selected_index = (self.selected_index + 1) % self.skills.len();
    }

    pub fn select_prev(&mut self) {
        if self.skills.is_empty() { return; }
        self.selected_index = if self.selected_index == 0 {
            self.skills.len() - 1
        } else {
            self.selected_index - 1
        };
    }

    pub fn selected(&self) -> Option<&SkillEntry> {
        self.skills.get(self.selected_index)
    }

    /// Render the skills-menu overlay inside `area`.
    ///
    /// Mirrors `screens/message_selector.rs::render` — a centered modal
    /// ~9/10 wide, height = items.len() + 3 clamped to fit. Long lists
    /// are scrolled so the currently selected row stays visible: the
    /// visible slice starts at `selected_index.saturating_sub(height-1)`
    /// and spans at most `height` rows. Each row shows
    /// ` {idx}: /{skill-name} — {description}` with the description
    /// truncated to 60 chars (char-boundary safe, trailing `…` on clip).
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let overlay_width = (area.width * 9 / 10)
            .max(70)
            .min(area.width.saturating_sub(2));
        let overlay_height = (self.skills.len() as u16 + 3)
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
        let total = self.skills.len();
        let start = if total <= body_rows {
            0
        } else if self.selected_index >= body_rows {
            self.selected_index + 1 - body_rows
        } else {
            0
        };
        let end = (start + body_rows).min(total);

        let items: Vec<ListItem<'_>> = self.skills[start..end]
            .iter()
            .enumerate()
            .map(|(offset, skill)| {
                let idx = start + offset;
                let style = if idx == self.selected_index {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg)
                };
                let desc = truncate_preview(&skill.description, 60);
                let line = format!(" {idx}: /{} — {desc}", skill.name);
                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(
                    " /skills — pick skill to inject (Up/Down, Enter select, Esc cancel) ",
                )
                .border_style(Style::default().fg(theme.accent)),
        );
        f.render_widget(list, overlay_area);
    }
}

/// Truncate `s` to at most `max` characters (char-boundary safe) with a
/// trailing ellipsis when clipped. Mirrors the preview-shortening used
/// by `screens/message_selector.rs`.
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

    fn fixture(n: usize) -> Vec<SkillEntry> {
        (0..n).map(|i| SkillEntry {
            name: format!("skill-{}", i),
            description: format!("desc-{}", i),
        }).collect()
    }

    #[test]
    fn new_starts_at_zero() {
        let m = SkillsMenu::new(fixture(3));
        assert_eq!(m.selected_index, 0);
    }

    #[test]
    fn select_next_advances() {
        let mut m = SkillsMenu::new(fixture(3));
        m.select_next();
        assert_eq!(m.selected_index, 1);
    }

    #[test]
    fn select_next_wraps_at_end() {
        let mut m = SkillsMenu::new(fixture(3));
        m.selected_index = 2;
        m.select_next();
        assert_eq!(m.selected_index, 0);
    }

    #[test]
    fn select_prev_wraps_at_start() {
        let mut m = SkillsMenu::new(fixture(3));
        m.select_prev();
        assert_eq!(m.selected_index, 2);
    }

    #[test]
    fn empty_list_noop() {
        let mut m = SkillsMenu::new(vec![]);
        m.select_next();
        m.select_prev();
        assert_eq!(m.selected_index, 0);
        assert!(m.selected().is_none());
    }

    #[test]
    fn truncate_preview_short_unchanged() {
        assert_eq!(truncate_preview("hello", 10), "hello");
    }

    #[test]
    fn truncate_preview_long_clipped() {
        let s: String = (0..70).map(|_| 'x').collect();
        let out = truncate_preview(&s, 60);
        assert_eq!(out.chars().count(), 60);
        assert!(out.ends_with('…'));
    }
}
