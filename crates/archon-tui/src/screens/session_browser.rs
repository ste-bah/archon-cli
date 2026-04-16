//! Session browser screen.
//! Layer 1 module — no imports from screens/ or app/.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// Metadata for a session entry.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub label: String,
    pub last_active: String,
}

/// Session browser state.
#[derive(Debug)]
pub struct SessionBrowser {
    list: VirtualList<SessionMeta>,
}

impl SessionBrowser {
    pub fn new() -> Self {
        Self { list: VirtualList::new(Vec::new(), 10) }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn selected_index(&self) -> usize {
        self.list.selected_index()
    }

    pub fn selected(&self) -> Option<&SessionMeta> {
        self.list.selected()
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionMeta>) {
        self.list.set_items(sessions);
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

    /// Render session list into area.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Session Browser");

        let items: Vec<ListItem> = self.list.visible_items().iter().map(|s| {
            ListItem::new(format!("{} [{}]", s.label, s.last_active))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl Default for SessionBrowser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_browser_empty() {
        let browser = SessionBrowser::new();
        assert!(browser.is_empty());
    }

    #[test]
    fn set_sessions_updates_list() {
        let mut browser = SessionBrowser::new();
        let sessions = vec![
            SessionMeta { id: "1".into(), label: "A".into(), last_active: "1m".into() },
            SessionMeta { id: "2".into(), label: "B".into(), last_active: "2m".into() },
        ];
        browser.set_sessions(sessions);
        assert_eq!(browser.len(), 2);
        assert_eq!(browser.selected_index(), 0);
    }

    #[test]
    fn cursor_wraps_at_boundaries() {
        let mut browser = SessionBrowser::new();
        let sessions = vec![
            SessionMeta { id: "0".into(), label: "A".into(), last_active: "1m".into() },
            SessionMeta { id: "1".into(), label: "B".into(), last_active: "2m".into() },
            SessionMeta { id: "2".into(), label: "C".into(), last_active: "3m".into() },
        ];
        browser.set_sessions(sessions);
        browser.move_down();
        assert_eq!(browser.selected_index(), 1);
        browser.move_down();
        assert_eq!(browser.selected_index(), 2);
        browser.move_down();
        assert_eq!(browser.selected_index(), 0); // wrap
    }

    #[test]
    fn move_up_wraps_to_last() {
        let mut browser = SessionBrowser::new();
        let sessions = vec![
            SessionMeta { id: "0".into(), label: "A".into(), last_active: "1m".into() },
            SessionMeta { id: "1".into(), label: "B".into(), last_active: "2m".into() },
        ];
        browser.set_sessions(sessions);
        browser.move_up();
        assert_eq!(browser.selected_index(), 1); // wrap to last
    }
}