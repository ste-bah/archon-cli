//! Hooks config menu screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// HookStore is the abstract interface for hook specification persistence.
/// Space bar toggles the enabled state of the selected hook.
pub trait HookStore: Send + Sync {
    /// Persist the updated enabled state for a hook by name.
    fn save_hook_enabled(&self, name: &str, enabled: bool) -> bool;

    /// Load all hook specifications from the store.
    fn load_hooks(&self) -> Vec<HookSpec>;
}

/// A hook specification.
#[derive(Debug, Clone)]
pub struct HookSpec {
    pub name: String,
    pub enabled: bool,
    pub script_path: String,
}

/// Hooks menu with virtualized list. Space toggles enabled.
#[derive(Debug)]
pub struct HooksMenu {
    hooks: Vec<HookSpec>,
    list: VirtualList<HookSpec>,
}

impl HooksMenu {
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&HookSpec> {
        self.list.selected()
    }

    /// Set hooks list.
    pub fn set_hooks(&mut self, hooks: Vec<HookSpec>) {
        self.hooks = hooks;
        self.list.set_items(self.hooks.clone());
    }

    /// Toggle enabled state of selected hook.
    pub fn toggle_selected(&mut self) {
        let idx = self.list.selected_index();
        if idx < self.hooks.len() {
            self.hooks[idx].enabled = !self.hooks[idx].enabled;
            self.list.set_items(self.hooks.clone());
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

    /// Render hooks menu into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default().borders(Borders::ALL).title("Hooks");

        let items: Vec<ListItem> = self
            .list
            .visible_items()
            .iter()
            .map(|h| {
                let flag = if h.enabled { "[x]" } else { "[ ]" };
                ListItem::new(format!("{} {} — {}", flag, h.name, h.script_path))
            })
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for HooksMenu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(name: &str, enabled: bool) -> HookSpec {
        HookSpec {
            name: name.to_string(),
            enabled,
            script_path: format!("/hooks/{}", name),
        }
    }

    #[test]
    fn new_menu_empty() {
        let menu = HooksMenu::new();
        assert!(menu.is_empty());
    }

    #[test]
    fn set_hooks_updates_list() {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![hook("pre-commit", true), hook("post-commit", false)]);
        assert_eq!(menu.len(), 2);
    }

    #[test]
    fn toggle_selected_changes_enabled() {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![hook("test-hook", true)]);
        assert!(menu.selected().unwrap().enabled);
        menu.toggle_selected();
        assert!(!menu.selected().unwrap().enabled);
    }

    #[test]
    fn cursor_wraps() {
        let mut menu = HooksMenu::new();
        menu.set_hooks(vec![hook("a", true), hook("b", false)]);
        menu.move_down();
        assert_eq!(menu.selected_index(), 1);
        menu.move_down();
        assert_eq!(menu.selected_index(), 0); // wrap
    }
}
