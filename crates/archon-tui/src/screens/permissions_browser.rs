//! Permissions browser screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// A tool permission entry.
#[derive(Debug, Clone)]
pub struct ToolPermission {
    pub name: String,
    pub description: String,
    pub allowed: bool,
    pub enabled: bool,
}

/// Permission state for a single tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermState {
    Allow,
    Deny,
    Ask,
}

impl ToolPermission {
    pub fn state(&self) -> PermState {
        if self.allowed { PermState::Allow }
        else if self.enabled { PermState::Ask }
        else { PermState::Deny }
    }
}

/// Permissions browser with virtualized list.
#[derive(Debug)]
pub struct PermissionsBrowser {
    tools: Vec<ToolPermission>,
    list: VirtualList<ToolPermission>,
}

impl PermissionsBrowser {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&ToolPermission> {
        self.list.selected()
    }

    /// Set tools list.
    pub fn set_tools(&mut self, tools: Vec<ToolPermission>) {
        self.tools = tools;
        self.list.set_items(self.tools.clone());
    }

    /// Cycle selected tool: Allow -> Deny -> Ask -> Allow
    pub fn cycle_selected(&mut self) {
        let idx = self.list.selected_index();
        if idx < self.tools.len() {
            let current = self.tools[idx].state();
            let next = match current {
                PermState::Allow => PermState::Deny,
                PermState::Deny => PermState::Ask,
                PermState::Ask => PermState::Allow,
            };
            match next {
                PermState::Allow => {
                    self.tools[idx].allowed = true;
                    self.tools[idx].enabled = false;
                }
                PermState::Deny => {
                    self.tools[idx].allowed = false;
                    self.tools[idx].enabled = false;
                }
                PermState::Ask => {
                    self.tools[idx].allowed = false;
                    self.tools[idx].enabled = true;
                }
            }
            self.list.set_items(self.tools.clone());
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

    /// Render permissions browser into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Permissions");

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|t| {
            let state = match t.state() {
                PermState::Allow => "[allow]",
                PermState::Deny => "[deny]",
                PermState::Ask => "[ask]",
            };
            ListItem::new(format!("{} {} — {}", state, t.name, t.description))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for PermissionsBrowser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_browser_empty() {
        let browser = PermissionsBrowser::new();
        assert!(browser.is_empty());
    }

    #[test]
    fn set_tools_updates_list() {
        let mut browser = PermissionsBrowser::new();
        browser.set_tools(vec![
            ToolPermission { name: "shell".into(), description: "run shell commands".into(), allowed: true, enabled: false },
            ToolPermission { name: "read".into(), description: "read files".into(), allowed: false, enabled: true },
        ]);
        assert_eq!(browser.len(), 2);
    }

    #[test]
    fn cycle_selected_changes_state() {
        let mut browser = PermissionsBrowser::new();
        browser.set_tools(vec![
            ToolPermission { name: "test".into(), description: "desc".into(), allowed: true, enabled: false },
        ]);
        // Start: Allow
        assert_eq!(browser.selected().unwrap().state(), PermState::Allow);
        browser.cycle_selected();
        // After cycle: Deny
        assert_eq!(browser.selected().unwrap().state(), PermState::Deny);
        browser.cycle_selected();
        // After cycle: Ask
        assert_eq!(browser.selected().unwrap().state(), PermState::Ask);
        browser.cycle_selected();
        // After cycle: Allow (wrap)
        assert_eq!(browser.selected().unwrap().state(), PermState::Allow);
    }

    #[test]
    fn cursor_wraps() {
        let mut browser = PermissionsBrowser::new();
        browser.set_tools(vec![
            ToolPermission { name: "a".into(), description: "1".into(), allowed: false, enabled: false },
            ToolPermission { name: "b".into(), description: "2".into(), allowed: false, enabled: false },
        ]);
        browser.move_down();
        assert_eq!(browser.selected_index(), 1);
        browser.move_down();
        assert_eq!(browser.selected_index(), 0); // wrap
    }
}