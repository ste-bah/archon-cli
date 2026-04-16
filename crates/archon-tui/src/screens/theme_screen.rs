//! Theme screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// A theme entry for the picker.
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub name: String,
    pub is_active: bool,
}

/// Theme screen with virtualized list of themes.
#[derive(Debug)]
pub struct ThemeScreen {
    themes: Vec<ThemeEntry>,
    list: VirtualList<ThemeEntry>,
}

impl ThemeScreen {
    pub fn new() -> Self {
        Self {
            themes: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.themes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&ThemeEntry> {
        self.list.selected()
    }

    /// Set themes list.
    pub fn set_themes(&mut self, themes: Vec<ThemeEntry>) {
        self.themes = themes;
        self.list.set_items(self.themes.clone());
    }

    /// Select theme (marks active, unmarks others).
    pub fn select_theme(&mut self, name: &str) {
        for t in &mut self.themes {
            t.is_active = t.name == name;
        }
        self.list.set_items(self.themes.clone());
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn page_up(&mut self) {
        self.list.page_up();
    }

    pub fn page_down(&mut self) {
        self.list.page_down();
    }

    /// Render theme screen into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Theme");

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|t| {
            let flag = if t.is_active { "*" } else { " " };
            ListItem::new(format!("[{}] {}", flag, t.name))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for ThemeScreen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_screen_empty() {
        let screen = ThemeScreen::new();
        assert!(screen.is_empty());
    }

    #[test]
    fn set_themes_updates_list() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry { name: "dark".into(), is_active: true },
            ThemeEntry { name: "light".into(), is_active: false },
        ]);
        assert_eq!(screen.len(), 2);
    }

    #[test]
    fn select_theme_marks_active() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry { name: "dark".into(), is_active: true },
            ThemeEntry { name: "light".into(), is_active: false },
            ThemeEntry { name: "nord".into(), is_active: false },
        ]);
        screen.select_theme("light");
        let light = screen.themes.iter().find(|t| t.name == "light").unwrap();
        assert!(light.is_active);
        let dark = screen.themes.iter().find(|t| t.name == "dark").unwrap();
        assert!(!dark.is_active);
    }

    #[test]
    fn cursor_wraps() {
        let mut screen = ThemeScreen::new();
        screen.set_themes(vec![
            ThemeEntry { name: "a".into(), is_active: false },
            ThemeEntry { name: "b".into(), is_active: false },
        ]);
        screen.move_down();
        assert_eq!(screen.selected_index(), 1);
        screen.move_down();
        assert_eq!(screen.selected_index(), 0); // wrap
    }
}