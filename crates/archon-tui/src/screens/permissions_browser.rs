//! Permissions browser screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// A tool permission entry.
#[derive(Debug, Clone)]
pub enum ToolPermission {
    Allow(String),
    Deny(String),
    Prompt(String),
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

    pub fn set_permissions(&mut self, perms: Vec<ToolPermission>) {
        self.tools = perms;
        self.list.set_items(self.tools.clone());
    }

    pub fn cycle_selected(&mut self) {
        let idx = self.list.selected_index();
        if idx < self.tools.len() {
            let current = &self.tools[idx];
            let next = match current {
                ToolPermission::Allow(name) => ToolPermission::Deny(name.clone()),
                ToolPermission::Deny(name) => ToolPermission::Prompt(name.clone()),
                ToolPermission::Prompt(name) => ToolPermission::Allow(name.clone()),
            };
            self.tools[idx] = next;
            self.list.set_items(self.tools.clone());
        }
    }

    pub fn move_up(&mut self) {
        self.list.move_up();
    }

    pub fn move_down(&mut self) {
        self.list.move_down();
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&ToolPermission> {
        self.list.selected()
    }

    /// Render permissions browser into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Permissions");

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|p| {
            let label = match p {
                ToolPermission::Allow(name) => format!("[ALLOW]  {}", name),
                ToolPermission::Deny(name) => format!("[DENY]   {}", name),
                ToolPermission::Prompt(name) => format!("[PROMPT] {}", name),
            };
            ListItem::new(label)
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
    fn new_empty() {
        let b = PermissionsBrowser::new();
        assert!(b.selected().is_none());
    }

    #[test]
    fn cycle_permissions() {
        let mut b = PermissionsBrowser::new();
        b.set_permissions(vec![
            ToolPermission::Allow("tool_a".into()),
        ]);
        b.cycle_selected();
        match b.selected() {
            Some(ToolPermission::Deny(_)) => {},
            _ => panic!("should be Deny after one cycle"),
        }
        b.cycle_selected();
        match b.selected() {
            Some(ToolPermission::Prompt(_)) => {},
            _ => panic!("should be Prompt after two cycles"),
        }
        b.cycle_selected();
        match b.selected() {
            Some(ToolPermission::Allow(_)) => {},
            _ => panic!("should wrap back to Allow"),
        }
    }

    #[test]
    fn move_down_wraps() {
        let mut b = PermissionsBrowser::new();
        b.set_permissions(vec![
            ToolPermission::Deny("x".into()),
        ]);
        b.move_down();
        assert_eq!(b.selected_index(), 0);
    }
}