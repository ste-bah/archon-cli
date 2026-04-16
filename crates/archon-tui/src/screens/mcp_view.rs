//! MCP view screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// MCP server row for display.
#[derive(Debug, Clone)]
pub struct McpServerRow {
    pub name: String,
    pub status: String,
    pub url: String,
}

/// MCP server status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerStatus {
    Connected,
    Disconnected,
    Connecting,
}

/// MCP view with virtualized list of servers.
#[derive(Debug)]
pub struct McpView {
    servers: Vec<McpServerRow>,
    list: VirtualList<McpServerRow>,
}

impl McpView {
    pub fn new() -> Self {
        Self {
            servers: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&McpServerRow> {
        self.list.selected()
    }

    /// Set servers list.
    pub fn set_servers(&mut self, servers: Vec<McpServerRow>) {
        self.servers = servers;
        self.list.set_items(self.servers.clone());
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

    /// Reconnect selected server.
    pub fn reconnect_selected(&mut self) -> Option<String> {
        self.list.selected().map(|s| s.name.clone())
    }

    /// Disconnect selected server.
    pub fn disconnect_selected(&mut self) -> Option<String> {
        self.list.selected().map(|s| s.name.clone())
    }

    /// Render MCP view into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("MCP Servers");

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|s| {
            ListItem::new(format!("{} [{}] — {}", s.name, s.status, s.url))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for McpView {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, status: &str) -> McpServerRow {
        McpServerRow {
            name: name.to_string(),
            status: status.to_string(),
            url: format!("http://localhost/{}", name),
        }
    }

    #[test]
    fn new_view_empty() {
        let view = McpView::new();
        assert!(view.is_empty());
    }

    #[test]
    fn set_servers_updates_list() {
        let mut view = McpView::new();
        view.set_servers(vec![server("s1", "connected"), server("s2", "disconnected")]);
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn cursor_wraps() {
        let mut view = McpView::new();
        view.set_servers(vec![server("a", "connected"), server("b", "connected")]);
        view.move_down();
        assert_eq!(view.selected_index(), 1);
        view.move_down();
        assert_eq!(view.selected_index(), 0); // wrap
    }

    #[test]
    fn reconnect_selected_returns_name() {
        let mut view = McpView::new();
        view.set_servers(vec![server("test-server", "disconnected")]);
        assert_eq!(view.reconnect_selected(), Some("test-server".to_string()));
    }

    #[test]
    fn disconnect_selected_returns_name() {
        let mut view = McpView::new();
        view.set_servers(vec![server("my-server", "connected")]);
        assert_eq!(view.disconnect_selected(), Some("my-server".to_string()));
    }
}