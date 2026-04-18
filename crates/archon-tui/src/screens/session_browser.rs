//! Session browser screen.
//!
//! REQ-TUI-MOD-006 / REQ-TUI-MOD-007
//! Layer 1 module — no imports from screens/ or app/.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};
use serde::{Deserialize, Serialize};

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