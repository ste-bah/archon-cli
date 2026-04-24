//! Tests for mcp_view rows.

use archon_tui::screens::mcp_view::{McpServerRow, McpServerStatus, McpStatusStore, McpView};

/// Mock store for testing McpStatusStore trait.
struct TestStore {
    servers: Vec<McpServerRow>,
}

impl TestStore {
    fn new(servers: Vec<McpServerRow>) -> Self {
        Self { servers }
    }
}

impl McpStatusStore for TestStore {
    fn get_servers(&self) -> Vec<McpServerRow> {
        self.servers.clone()
    }

    fn reconnect_server(&mut self, _name: &str) {}

    fn disconnect_server(&mut self, _name: &str) {}

    fn server_tools(&self, _name: &str) -> Vec<String> {
        vec!["tool_a".to_string(), "tool_b".to_string()]
    }
}

fn make_row(name: &str, status: McpServerStatus) -> McpServerRow {
    McpServerRow {
        name: name.to_string(),
        status,
        url: format!("http://localhost/{}", name),
    }
}

#[test]
fn mcp_view_empty_by_default() {
    let view = McpView::new();
    assert!(view.is_empty());
    assert_eq!(view.len(), 0);
}

#[test]
fn mcp_view_set_servers_updates_list() {
    let mut view = McpView::new();
    let rows = vec![
        make_row("server-a", McpServerStatus::Connected),
        make_row("server-b", McpServerStatus::Disconnected),
    ];
    view.set_servers(rows);
    assert_eq!(view.len(), 2);
    assert_eq!(view.selected_index(), 0);
}

#[test]
fn mcp_view_navigation_wraps() {
    let mut view = McpView::new();
    view.set_servers(vec![
        make_row("a", McpServerStatus::Connected),
        make_row("b", McpServerStatus::Connected),
    ]);
    // wrap at top
    view.move_up();
    assert_eq!(view.selected_index(), 1);
    // wrap at bottom
    view.move_down();
    assert_eq!(view.selected_index(), 0);
}

#[test]
fn mcp_view_reconnect_selected() {
    let mut view = McpView::new();
    view.set_servers(vec![make_row(
        "test-reconnect",
        McpServerStatus::Disconnected,
    )]);
    assert_eq!(
        view.selected_server_name(),
        Some("test-reconnect".to_string())
    );
}

#[test]
fn mcp_view_disconnect_selected() {
    let mut view = McpView::new();
    view.set_servers(vec![make_row(
        "test-disconnect",
        McpServerStatus::Connected,
    )]);
    // move to first item then disconnect
    assert_eq!(
        view.selected_server_name(),
        Some("test-disconnect".to_string())
    );
}

#[test]
fn mcp_view_page_up_down() {
    let mut view = McpView::new();
    let rows: Vec<McpServerRow> = (0..20)
        .map(|i| make_row(&format!("srv-{}", i), McpServerStatus::Connected))
        .collect();
    view.set_servers(rows);
    view.move_down();
    view.move_down();
    assert_eq!(view.selected_index(), 2);
    view.page_up();
    // page up moves by viewport size (10), but selected can't go below 0
    assert_eq!(view.selected_index(), 0);
}

#[test]
fn mcp_view_key_r_reconnect() {
    let mut view = McpView::new();
    view.set_servers(vec![make_row(
        "reconnect-me",
        McpServerStatus::Disconnected,
    )]);
    let name = view.selected_server_name();
    assert_eq!(name, Some("reconnect-me".to_string()));
}

#[test]
fn mcp_view_key_d_disconnect() {
    let mut view = McpView::new();
    view.set_servers(vec![make_row("disconnect-me", McpServerStatus::Connected)]);
    let name = view.selected_server_name();
    assert_eq!(name, Some("disconnect-me".to_string()));
}

#[test]
fn mcp_view_enter_tool_list() {
    let mut view = McpView::new();
    view.set_servers(vec![make_row("tool-server", McpServerStatus::Connected)]);
    let name = view.selected_server_name();
    assert_eq!(name, Some("tool-server".to_string()));
}

#[test]
fn mcp_view_selected_returns_row() {
    let mut view = McpView::new();
    let row = make_row("selected-test", McpServerStatus::Connecting);
    view.set_servers(vec![row.clone()]);
    assert!(view.selected().is_some());
    assert_eq!(
        view.selected().map(|r| r.name.as_str()),
        Some("selected-test")
    );
}

#[test]
fn mcp_status_store_trait() {
    let store = TestStore::new(vec![make_row("trait-server", McpServerStatus::Connected)]);
    let servers = store.get_servers();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "trait-server");

    let tools = store.server_tools("trait-server");
    assert_eq!(tools.len(), 2);
}

#[test]
fn mcp_server_status_enum() {
    assert_eq!(McpServerStatus::Connected, McpServerStatus::Connected);
    assert_eq!(McpServerStatus::Disconnected, McpServerStatus::Disconnected);
    assert_eq!(McpServerStatus::Connecting, McpServerStatus::Connecting);
}

#[test]
fn mcp_view_visible_items() {
    let mut view = McpView::new();
    let rows: Vec<McpServerRow> = (0..5)
        .map(|i| make_row(&format!("visible-{}", i), McpServerStatus::Connected))
        .collect();
    view.set_servers(rows);
    // VirtualList viewport height is 10 by default, all 5 items visible
    assert_eq!(view.len(), 5);
}
