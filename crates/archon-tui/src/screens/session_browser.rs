//! Session browser screen.
//!
//! REQ-TUI-MOD-006 / REQ-TUI-MOD-007
//! Layer 1 module — no imports from screens/ or app/.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use archon_session::storage::SessionStore;

use crate::virtual_list::VirtualList;
use crate::theme::Theme;

/// Branch point in session history.
/// Tracks a forked session with metadata about when the fork occurred.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchPoint {
    /// Unique identifier for this branch point.
    pub id: String,
    /// The parent session ID this branch was created from.
    pub parent_session: String,
    /// The message index at which branching occurred.
    pub branched_at_message: usize,
    /// Human-readable label for this branch.
    pub label: String,
    /// Timestamp when this branch point was created.
    pub created_at: DateTime<Utc>,
}

/// Session state tracking current session and branch history.
/// Used for session switching and history navigation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// The currently active session ID, if any.
    pub current_id: Option<String>,
    /// All known branch points in session history.
    pub branches: Vec<BranchPoint>,
    /// Current position in history navigation (0 = latest).
    pub history_cursor: usize,
}

impl SessionState {
    /// Create a new empty session state.
    pub fn new() -> Self {
        Self {
            current_id: None,
            branches: Vec::new(),
            history_cursor: 0,
        }
    }

    /// Create a session state with the given session ID.
    pub fn with_session(id: String) -> Self {
        Self {
            current_id: Some(id),
            branches: Vec::new(),
            history_cursor: 0,
        }
    }
}

/// Metadata for a session entry.
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub label: String,
    pub last_active: String,
}

/// Session summary for browser display.
/// Used by SessionBrowser to display session list items.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    /// Unique identifier for the session.
    pub id: String,
    /// Human-readable session name.
    pub name: String,
    /// Last time this session was updated.
    pub last_updated: DateTime<Utc>,
    /// Number of messages in the session.
    pub message_count: u64,
}

/// Session browser state.
pub struct SessionBrowser {
    /// Reference to the session store.
    store: Arc<SessionStore>,
    /// List of session summaries available for browsing.
    sessions: Vec<SessionSummary>,
    /// Current cursor position in the sessions list.
    cursor: usize,
    /// Current session state.
    state: SessionState,
}

impl std::fmt::Debug for SessionBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBrowser")
            .field("sessions", &self.sessions)
            .field("cursor", &self.cursor)
            .field("state", &self.state)
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
        }
    }

    /// Returns the current cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns a reference to the current session state.
    pub fn state(&self) -> &SessionState {
        &self.state
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

        let items: Vec<ListItem> = self.sessions.iter().map(|s| {
            ListItem::new(format!("{} [{} msgs]", s.name, s.message_count))
        }).collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

impl SessionBrowser {
    /// Create a SessionBrowser with a temporary test store.
    /// This is intended for testing scenarios where a real store is needed.
    pub fn new_for_tests() -> Self {
        use archon_session::storage::SessionStore;
        // Create an in-memory store for testing
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let store = SessionStore::open(&temp_dir.path().join("test.db"))
            .expect("test store");
        Self::new(Arc::new(store))
    }
}

impl Default for SessionBrowser {
    fn default() -> Self {
        Self::new_for_tests()
    }
}

/// Represents the outcome of attempting to resume a session.
/// EC-TUI-016: Context overflow handling when resuming sessions with large histories.
#[derive(Debug, Clone, PartialEq)]
pub enum ResumeOutcome {
    /// Session was successfully restored.
    Restored {
        /// The session ID that was restored.
        session_id: String,
        /// Number of messages loaded from the session history.
        messages_loaded: usize,
    },
    /// Session restore failed due to context overflow.
    /// EC-TUI-016: Context limit exceeded, action taken to handle overflow.
    ContextOverflow {
        /// Estimated token count at time of overflow.
        estimated_tokens: u64,
        /// The context limit that was exceeded.
        limit: u64,
        /// The action taken to handle the overflow.
        action: OverflowAction,
    },
    /// Session was not found in storage.
    NotFound,
}

/// Actions that can be taken when context overflow occurs.
/// TruncateOldest: Removes oldest messages until context is at 90% of limit.
/// SwitchModel: Switches to a model with larger context window.
/// Cancelled: User cancelled the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverflowAction {
    /// Truncate oldest messages to bring context within limit (90% of limit).
    TruncateOldest(usize),
    /// Switch to a different model with larger context window.
    SwitchModel(String),
    /// Operation was cancelled.
    Cancelled,
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
            SessionSummary { id: "1".into(), name: "A".into(), last_updated: Utc::now(), message_count: 5 },
            SessionSummary { id: "2".into(), name: "B".into(), last_updated: Utc::now(), message_count: 10 },
        ];
        browser.set_sessions(sessions);
        assert_eq!(browser.len(), 2);
        assert_eq!(browser.selected_index(), 0);
    }

    #[test]
    fn cursor_bounded_at_end() {
        let mut browser = SessionBrowser::new_for_tests();
        let sessions = vec![
            SessionSummary { id: "0".into(), name: "A".into(), last_updated: Utc::now(), message_count: 1 },
            SessionSummary { id: "1".into(), name: "B".into(), last_updated: Utc::now(), message_count: 2 },
            SessionSummary { id: "2".into(), name: "C".into(), last_updated: Utc::now(), message_count: 3 },
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
            SessionSummary { id: "0".into(), name: "A".into(), last_updated: Utc::now(), message_count: 1 },
            SessionSummary { id: "1".into(), name: "B".into(), last_updated: Utc::now(), message_count: 2 },
        ];
        browser.set_sessions(sessions);
        browser.move_up();
        // Should be bounded at 0
        assert_eq!(browser.selected_index(), 0);
    }
}