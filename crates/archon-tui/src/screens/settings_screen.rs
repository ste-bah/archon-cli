//! Settings screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// A setting field with type tag.
#[derive(Debug, Clone)]
pub enum SettingField {
    Toggle {
        key: String,
        value: bool,
    },
    Text {
        key: String,
        value: String,
    },
    Enum {
        key: String,
        value: String,
        options: Vec<String>,
    },
}

/// Trait for settings persistence.
/// Implemented by types that can store and retrieve settings key-value pairs.
pub trait SettingsStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: String);
}

/// Settings screen with virtualized list.
#[derive(Debug)]
pub struct SettingsScreen {
    fields: Vec<SettingField>,
    list: VirtualList<SettingField>,
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {
            fields: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&SettingField> {
        self.list.selected()
    }

    /// Set fields list.
    pub fn set_fields(&mut self, fields: Vec<SettingField>) {
        self.fields = fields;
        self.list.set_items(self.fields.clone());
    }

    /// Toggle selected field (if it's a Toggle).
    pub fn toggle_selected(&mut self) {
        let idx = self.list.selected_index();
        if idx < self.fields.len()
            && let SettingField::Toggle { ref mut value, .. } = self.fields[idx]
        {
            *value = !*value;
            self.list.set_items(self.fields.clone());
        }
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

    /// Render settings screen into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default().borders(Borders::ALL).title("Settings");

        let items: Vec<ListItem> = self
            .list
            .visible_items()
            .iter()
            .map(|field| {
                let label = match field {
                    SettingField::Toggle { key, value } => {
                        format!("{} [{}]", key, if *value { "on" } else { "off" })
                    }
                    SettingField::Text { key, value } => {
                        format!("{}: {}", key, value)
                    }
                    SettingField::Enum { key, value, .. } => {
                        format!("{}: {}", key, value)
                    }
                };
                ListItem::new(label)
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for SettingsScreen {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_screen_empty() {
        let screen = SettingsScreen::new();
        assert!(screen.is_empty());
    }

    #[test]
    fn set_fields_updates_list() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![
            SettingField::Toggle {
                key: "dark".into(),
                value: true,
            },
            SettingField::Text {
                key: "name".into(),
                value: "test".into(),
            },
        ]);
        assert_eq!(screen.len(), 2);
    }

    #[test]
    fn toggle_selected_changes_toggle() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![SettingField::Toggle {
            key: "debug".into(),
            value: false,
        }]);
        assert!(!matches!(
            screen.selected(),
            Some(SettingField::Toggle { value: true, .. })
        ));
        screen.toggle_selected();
        assert!(matches!(
            screen.selected(),
            Some(SettingField::Toggle { value: true, .. })
        ));
    }

    #[test]
    fn cursor_wraps() {
        let mut screen = SettingsScreen::new();
        screen.set_fields(vec![
            SettingField::Text {
                key: "a".into(),
                value: "1".into(),
            },
            SettingField::Text {
                key: "b".into(),
                value: "2".into(),
            },
        ]);
        screen.move_down();
        assert_eq!(screen.selected_index(), 1);
        screen.move_down();
        assert_eq!(screen.selected_index(), 0); // wrap
    }
}
