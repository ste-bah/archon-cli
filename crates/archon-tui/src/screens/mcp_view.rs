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
    pub status: McpServerStatus,
    pub url: String,
}

/// MCP server status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerStatus {
    Connected,
    Disconnected,
    Connecting,
}

impl McpServerStatus {
    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            McpServerStatus::Connected => "connected",
            McpServerStatus::Disconnected => "disconnected",
            McpServerStatus::Connecting => "connecting",
        }
    }
}

/// Trait for MCP status store operations.
/// Implement this to connect McpView to actual MCP server state.
pub trait McpStatusStore {
    /// Get current list of MCP servers.
    fn get_servers(&self) -> Vec<McpServerRow>;

    /// Reconnect a server by name.
    fn reconnect_server(&mut self, name: &str);

    /// Disconnect a server by name.
    fn disconnect_server(&mut self, name: &str);

    /// Get list of tools for a server.
    fn server_tools(&self, name: &str) -> Vec<String>;
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

    /// Name of the selected server, if any.
    pub fn selected_server_name(&self) -> Option<String> {
        self.list.selected().map(|s| s.name.clone())
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

    /// Handle key 'r' — reconnect selected server.
    /// Returns server name if reconnect is possible.
    pub fn key_r_reconnect(&self) -> Option<String> {
        self.list.selected().and_then(|s| {
            if matches!(s.status, McpServerStatus::Disconnected | McpServerStatus::Connecting) {
                Some(s.name.clone())
            } else {
                None
            }
        })
    }

    /// Handle key 'd' — disconnect selected server.
    /// Returns server name if disconnect is possible.
    pub fn key_d_disconnect(&self) -> Option<String> {
        self.list.selected().and_then(|s| {
            if matches!(s.status, McpServerStatus::Connected) {
                Some(s.name.clone())
            } else {
                None
            }
        })
    }

    /// Handle key Enter — show tool list for selected server.
    /// Returns server name if tools are available.
    pub fn key_enter_tool_list(&self) -> Option<String> {
        self.list.selected().and_then(|s| {
            if matches!(s.status, McpServerStatus::Connected) {
                Some(s.name.clone())
            } else {
                None
            }
        })
    }

    /// Render MCP view into area.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("MCP Servers")
            .title_style(theme.header);

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|s| {
            ListItem::new(format!(
                "{} [{}] — {}",
                s.name,
                s.status.label(),
                s.url
            ))
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

    fn make_row(name: &str, status: McpServerStatus) -> McpServerRow {
        McpServerRow {
            name: name.to_string(),
            status,
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
        view.set_servers(vec![
            make_row("s1", McpServerStatus::Connected),
            make_row("s2", McpServerStatus::Disconnected),
        ]);
        assert_eq!(view.len(), 2);
    }

    #[test]
    fn cursor_wraps() {
        let mut view = McpView::new();
        view.set_servers(vec![
            make_row("a", McpServerStatus::Connected),
            make_row("b", McpServerStatus::Connected),
        ]);
        view.move_down();
        assert_eq!(view.selected_index(), 1);
        view.move_down();
        assert_eq!(view.selected_index(), 0);
    }

    #[test]
    fn key_r_reconnect_disconnected() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("test-reconnect", McpServerStatus::Disconnected)]);
        assert_eq!(view.key_r_reconnect(), Some("test-reconnect".to_string()));
    }

    #[test]
    fn key_r_reconnect_connected_no_op() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("already-connected", McpServerStatus::Connected)]);
        assert_eq!(view.key_r_reconnect(), None);
    }

    #[test]
    fn key_d_disconnect_connected() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("disconnect-me", McpServerStatus::Connected)]);
        assert_eq!(view.key_d_disconnect(), Some("disconnect-me".to_string()));
    }

    #[test]
    fn key_d_disconnect_disconnected_no_op() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("not-connected", McpServerStatus::Disconnected)]);
        assert_eq!(view.key_d_disconnect(), None);
    }

    #[test]
    fn key_enter_tool_list_connected() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("tool-server", McpServerStatus::Connected)]);
        assert_eq!(view.key_enter_tool_list(), Some("tool-server".to_string()));
    }

    #[test]
    fn key_enter_tool_list_disconnected_no_op() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("no-tools", McpServerStatus::Disconnected)]);
        assert_eq!(view.key_enter_tool_list(), None);
    }

    #[test]
    fn selected_server_name() {
        let mut view = McpView::new();
        view.set_servers(vec![make_row("my-server", McpServerStatus::Connecting)]);
        assert_eq!(view.selected_server_name(), Some("my-server".to_string()));
    }

    #[test]
    fn mcp_server_status_labels() {
        assert_eq!(McpServerStatus::Connected.label(), "connected");
        assert_eq!(McpServerStatus::Disconnected.label(), "disconnected");
        assert_eq!(McpServerStatus::Connecting.label(), "connecting");
    }
}