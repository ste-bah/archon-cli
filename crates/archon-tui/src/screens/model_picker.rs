//! Model picker screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// Provider identifier (placeholder).
pub type ProviderId = String;

/// Model identifier (placeholder).
pub type ModelId = String;

/// A single (provider, model) entry in the picker.
#[derive(Debug, Clone)]
pub struct ProviderEntry {
    pub provider_id: ProviderId,
    pub model_id: ModelId,
    pub label: String,
}

/// Model picker state with virtualized scrolling and fuzzy filter.
#[derive(Debug)]
pub struct ModelPicker {
    providers: Vec<ProviderEntry>,
    list: VirtualList<ProviderEntry>,
    query: String,
}

impl ModelPicker {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
            query: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&ProviderEntry> {
        self.list.selected()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Set the query string and filter the list.
    pub fn set_query(&mut self, q: &str) {
        self.query = q.to_string();
        self.rebuild_filtered();
    }

    /// Set the full provider list and reset filter.
    pub fn set_providers(&mut self, providers: Vec<ProviderEntry>) {
        self.providers = providers;
        self.query.clear();
        self.rebuild_filtered();
    }

    fn rebuild_filtered(&mut self) {
        let filtered: Vec<ProviderEntry> = if self.query.is_empty() {
            self.providers.clone()
        } else {
            let q = self.query.to_lowercase();
            self.providers.iter()
                .filter(|p| p.label.to_lowercase().contains(&q))
                .cloned()
                .collect()
        };
        self.list.set_items(filtered);
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

    /// Render model picker into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Model Picker{}{}",
                if self.query.is_empty() { "" } else { " — " },
                if self.query.is_empty() { "" } else { &self.query }
            ));

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|p| {
            ListItem::new(format!("{} / {}", p.provider_id, p.model_id))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for ModelPicker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider.to_string(),
            model_id: model.to_string(),
            label: format!("{}/{}", provider, model),
        }
    }

    #[test]
    fn new_picker_empty() {
        let picker = ModelPicker::new();
        assert!(picker.is_empty());
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn set_providers_updates_list() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            entry("anthropic", "claude-opus"),
            entry("openai", "gpt-5"),
        ]);
        assert_eq!(picker.len(), 2);
    }

    #[test]
    fn set_query_filters_list() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            entry("anthropic", "claude-opus"),
            entry("anthropic", "claude-sonnet"),
            entry("openai", "gpt-5"),
        ]);
        picker.set_query("sonnet");
        assert_eq!(picker.len(), 1); // only claude-sonnet matches
        picker.set_query("claude");
        assert_eq!(picker.len(), 2); // both anthropic entries match
    }

    #[test]
    fn cursor_wraps() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![
            entry("a", "b"),
            entry("c", "d"),
        ]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
    }

    #[test]
    fn empty_query_shows_all() {
        let mut picker = ModelPicker::new();
        picker.set_providers(vec![entry("x", "y")]);
        picker.set_query("");
        assert_eq!(picker.len(), 1);
    }
}