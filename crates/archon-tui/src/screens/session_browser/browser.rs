//! `SessionBrowser` struct, navigation, rendering, and test constructors.
//!
//! The restore / context-overflow logic lives in `super::restore`.

use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};
use std::sync::Arc;

use archon_session::storage::SessionStore;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

use super::{SessionState, SessionSummary};

/// Session browser state.
pub struct SessionBrowser {
    /// Reference to the session store.
    pub(super) store: Arc<SessionStore>,
    /// List of session summaries available for browsing.
    pub(super) sessions: Vec<SessionSummary>,
    /// Current cursor position in the sessions list.
    pub(super) cursor: usize,
    /// Current session state.
    pub(super) state: SessionState,
    /// Name of the last successfully restored session (for badge display).
    pub(super) last_restored_name: Option<String>,
    /// Keep temp directory alive for test stores (None for normal use).
    #[allow(dead_code)]
    temp_dir: Option<tempfile::TempDir>,
}

impl std::fmt::Debug for SessionBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBrowser")
            .field("sessions", &self.sessions)
            .field("cursor", &self.cursor)
            .field("state", &self.state)
            .field("last_restored_name", &self.last_restored_name)
            .finish()
    }
}

impl SessionBrowser {
    /// Create a new SessionBrowser with the given store.
    pub fn new(store: Arc<SessionStore>) -> Self {
        Self {
            store,
            sessions: Vec::new(),
            cursor: 0,
            state: SessionState::new(),
            last_restored_name: None,
            temp_dir: None,
        }
    }

    /// Returns a reference to the session store.
    pub fn store(&self) -> Arc<SessionStore> {
        Arc::clone(&self.store)
    }

    /// Returns the current cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns a reference to the current session state.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Returns the name of the last successfully restored session, if any.
    pub fn last_restored_name(&self) -> Option<&str> {
        self.last_restored_name.as_deref()
    }

    /// Refresh sessions from the store.
    pub async fn refresh(&mut self) -> Result<(), archon_session::storage::SessionError> {
        let metadata = self.store.list_sessions(100)?;
        self.sessions = metadata
            .into_iter()
            .map(|m| SessionSummary {
                id: m.id,
                name: m.name.unwrap_or_else(|| "Unnamed Session".to_string()),
                last_updated: m.last_active.parse().unwrap_or_else(|_| Utc::now()),
                message_count: m.message_count,
            })
            .collect();
        // Reset cursor to valid position after refresh
        if !self.sessions.is_empty() && self.cursor >= self.sessions.len() {
            self.cursor = self.sessions.len() - 1;
        }
        Ok(())
    }

    /// Move cursor down (toward end of list), bounded.
    pub fn move_cursor_down(&mut self) {
        if self.sessions.is_empty() {
            return;
        }
        if self.cursor < self.sessions.len().saturating_sub(1) {
            self.cursor += 1;
        }
        // else stay at max position
    }

    /// Move cursor up (toward start of list), bounded.
    pub fn move_cursor_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        // else stay at 0
    }

    /// Returns the selected session summary, if any.
    pub fn selected(&self) -> Option<&SessionSummary> {
        self.sessions.get(self.cursor)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn selected_index(&self) -> usize {
        self.cursor
    }

    /// Set sessions directly (bypassing store refresh).
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        self.sessions = sessions;
        // Reset cursor to valid position
        if self.cursor > 0 && self.cursor >= self.sessions.len() {
            self.cursor = self.sessions.len().saturating_sub(1);
        }
    }

    pub fn move_up(&mut self) {
        self.move_cursor_up();
    }

    pub fn move_down(&mut self) {
        self.move_cursor_down();
    }

    pub fn page_up(&mut self) {
        // Move up by 10 or to start
        self.cursor = self.cursor.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        // Move down by 10 or to end
        let max_idx = self.sessions.len().saturating_sub(1);
        self.cursor = (self.cursor + 10).min(max_idx);
    }

    /// Render session list into area.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Session Browser");

        let items: Vec<ListItem> = self
            .sessions
            .iter()
            .map(|s| ListItem::new(format!("{} [{} msgs]", s.name, s.message_count)))
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl SessionBrowser {
    /// Create a SessionBrowser with a temporary test store.
    /// This is intended for testing scenarios where a real store is needed.
    pub fn new_for_tests() -> Self {
        // Create an in-memory store for testing
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = SessionStore::open(&temp_dir.path().join("test.db")).expect("test store");
        Self {
            store: Arc::new(store),
            sessions: Vec::new(),
            cursor: 0,
            state: SessionState::new(),
            last_restored_name: None,
            temp_dir: Some(temp_dir),
        }
    }
}

impl Default for SessionBrowser {
    fn default() -> Self {
        Self::new_for_tests()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_browser_empty() {
        let browser = SessionBrowser::new_for_tests();
        assert!(browser.is_empty());
    }

    #[test]
    fn set_sessions_updates_list() {
        let mut browser = SessionBrowser::new_for_tests();
        let sessions = vec![
            SessionSummary {
                id: "1".into(),
                name: "A".into(),
                last_updated: Utc::now(),
                message_count: 5,
            },
            SessionSummary {
                id: "2".into(),
                name: "B".into(),
                last_updated: Utc::now(),
                message_count: 10,
            },
        ];
        browser.set_sessions(sessions);
        assert_eq!(browser.len(), 2);
        assert_eq!(browser.selected_index(), 0);
    }

    #[test]
    fn cursor_bounded_at_end() {
        let mut browser = SessionBrowser::new_for_tests();
        let sessions = vec![
            SessionSummary {
                id: "0".into(),
                name: "A".into(),
                last_updated: Utc::now(),
                message_count: 1,
            },
            SessionSummary {
                id: "1".into(),
                name: "B".into(),
                last_updated: Utc::now(),
                message_count: 2,
            },
            SessionSummary {
                id: "2".into(),
                name: "C".into(),
                last_updated: Utc::now(),
                message_count: 3,
            },
        ];
        browser.set_sessions(sessions);
        browser.move_down();
        assert_eq!(browser.selected_index(), 1);
        browser.move_down();
        assert_eq!(browser.selected_index(), 2);
        browser.move_down();
        // Should be bounded at last index
        assert_eq!(browser.selected_index(), 2);
    }

    #[test]
    fn move_up_bounded_at_zero() {
        let mut browser = SessionBrowser::new_for_tests();
        let sessions = vec![
            SessionSummary {
                id: "0".into(),
                name: "A".into(),
                last_updated: Utc::now(),
                message_count: 1,
            },
            SessionSummary {
                id: "1".into(),
                name: "B".into(),
                last_updated: Utc::now(),
                message_count: 2,
            },
        ];
        browser.set_sessions(sessions);
        browser.move_up();
        // Should be bounded at 0
        assert_eq!(browser.selected_index(), 0);
    }
}
