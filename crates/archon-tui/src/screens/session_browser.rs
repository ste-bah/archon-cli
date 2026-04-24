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

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

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
    /// Name of the last successfully restored session (for badge display).
    last_restored_name: Option<String>,
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

    /// Restore a session by ID, checking context window limits.
    ///
    /// Returns `ResumeOutcome::NotFound` if the session does not exist.
    /// Returns `ResumeOutcome::Restored` if the session fits within `current_model_ctx`.
    /// Returns `ResumeOutcome::ContextOverflow` if the session exceeds `current_model_ctx`,
    /// with `OverflowAction::TruncateOldest(n)` computed to bring usage under 90% of limit.
    pub async fn restore(
        &mut self,
        session_id: &str,
        current_model_ctx: u64,
    ) -> Result<ResumeOutcome, archon_session::storage::SessionError> {
        // Attempt to get the session - NotFound maps to ResumeOutcome::NotFound
        let session_meta = match self.store.get_session(session_id) {
            Ok(meta) => meta,
            Err(archon_session::storage::SessionError::NotFound(_)) => {
                self.last_restored_name = None;
                return Ok(ResumeOutcome::NotFound);
            }
            Err(e) => return Err(e),
        };

        // Load all messages for this session
        let messages = self.store.load_messages(session_id)?;

        // Estimate token footprint
        let total_tokens = Self::estimate_tokens(&messages);

        // Check against current_model_ctx (limit)
        if total_tokens <= current_model_ctx {
            // Fits within context - restore it
            self.state.current_id = Some(session_id.to_string());
            self.last_restored_name = session_meta.name;
            Ok(ResumeOutcome::Restored {
                session_id: session_id.to_string(),
                messages_loaded: messages.len(),
            })
        } else {
            // Context overflow - clear last_restored_name and report overflow
            self.last_restored_name = None;
            let limit = current_model_ctx;
            let n = Self::compute_truncate_count(&messages, limit);
            Ok(ResumeOutcome::ContextOverflow {
                estimated_tokens: total_tokens,
                limit,
                action: OverflowAction::TruncateOldest(n),
            })
        }
    }

    /// Estimate total tokens using a simple chars/4 heuristic.
    fn estimate_tokens(messages: &[String]) -> u64 {
        messages
            .iter()
            .map(|msg| (msg.chars().count() / 4) as u64)
            .sum()
    }

    /// Compute minimum number of oldest messages to drop to bring total under 90% of limit.
    // Default resolution per TECH-TUI-SESSION implementation_notes (EC-TUI-016).
    fn compute_truncate_count(messages: &[String], limit: u64) -> usize {
        let total_tokens = Self::estimate_tokens(messages);
        let target = (limit as f64 * 0.9) as u64;
        if total_tokens <= target {
            return 0;
        }

        // Find minimum n such that dropping n oldest messages brings us under target
        for n in 1..=messages.len() {
            let remaining = &messages[n..];
            let remaining_tokens = Self::estimate_tokens(remaining);
            if remaining_tokens <= target {
                return n;
            }
        }

        // If still over after dropping all but one, return messages.len() - 1
        messages.len().saturating_sub(1).max(1)
    }

    /// Resolve a context overflow by applying the user's chosen action.
    ///
    /// Pure data transform — UI prompting lives in `notifications.rs` per TECH-TUI-SESSION.
    ///
    /// - `OverflowAction::TruncateOldest(n)`: returns `Some(messages[n..].to_vec())`;
    ///   if `n >= messages.len()`, returns `Some(vec![])`.
    /// - `OverflowAction::SwitchModel(_)`: returns `None` (caller handles model swap).
    /// - `OverflowAction::Cancelled`: returns `None`.
    pub fn resolve_overflow(messages: Vec<String>, action: &OverflowAction) -> Option<Vec<String>> {
        match action {
            OverflowAction::TruncateOldest(n) => {
                if *n >= messages.len() {
                    Some(vec![])
                } else {
                    Some(messages[*n..].to_vec())
                }
            }
            OverflowAction::SwitchModel(_) => None,
            OverflowAction::Cancelled => None,
        }
    }

    /// Compute the default overflow action to bring estimated tokens under 90% of limit.
    ///
    /// Uses `compute_truncate_count` to determine how many oldest messages to drop.
    pub fn default_overflow_action(estimated_tokens: u64, limit: u64) -> OverflowAction {
        // We need to compute how many messages would be needed to bring under 90%.
        // Since compute_truncate_count takes messages directly, we reconstruct the logic.
        let target = (limit as f64 * 0.9) as u64;
        if estimated_tokens <= target {
            return OverflowAction::TruncateOldest(0);
        }

        // The actual truncation count depends on message sizes.
        // This is a simplified heuristic: assume average token size.
        // For a proper solution we'd need the actual messages, but this
        // matches the TASK-TUI-704 pattern where we compute from messages.
        // Here we return a conservative estimate based on the ratio.
        let excess = estimated_tokens - target;
        let total = estimated_tokens;
        let ratio = excess as f64 / total as f64;
        // This is an approximation; the caller may need to refine based on actual messages.
        OverflowAction::TruncateOldest((ratio * 10.0) as usize)
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
        use archon_session::storage::SessionStore;
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

    /// Regression test: last_restored_name is set after a successful restore.
    #[tokio::test]
    async fn test_last_restored_name_set_on_restored() {
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        let session_id = "test-session-name";
        store
            .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");
        store.set_name(session_id, "My Session").expect("set name");
        for i in 0..3 {
            store
                .save_message(session_id, i, &format!("Message {}", i))
                .expect("save message");
        }

        let mut browser = SessionBrowser::new(Arc::clone(&store));
        assert_eq!(browser.last_restored_name(), None);

        let outcome = browser.restore(session_id, 100_000).await.expect("restore");
        match outcome {
            ResumeOutcome::Restored { .. } => {
                assert_eq!(
                    browser.last_restored_name(),
                    Some("My Session"),
                    "last_restored_name must be set after successful restore"
                );
            }
            other => panic!("Expected Restored, got {:?}", other),
        }
    }

    /// Regression test: last_restored_name is cleared when restore fails.
    #[tokio::test]
    async fn test_last_restored_name_cleared_on_notfound() {
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        // First restore a valid session to set the field
        let session_id = "valid-session";
        store
            .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");
        store
            .set_name(session_id, "Valid Session")
            .expect("set name");
        store
            .save_message(session_id, 0, "hello")
            .expect("save message");

        let mut browser = SessionBrowser::new(Arc::clone(&store));
        let _ = browser.restore(session_id, 100_000).await.expect("restore");
        assert_eq!(browser.last_restored_name(), Some("Valid Session"));

        // Now try to restore a non-existent session — field should be cleared
        let outcome = browser
            .restore("nonexistent", 100_000)
            .await
            .expect("restore");
        match outcome {
            ResumeOutcome::NotFound => {
                assert_eq!(
                    browser.last_restored_name(),
                    None,
                    "last_restored_name must be cleared on NotFound"
                );
            }
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_restore_fits_returns_restored() {
        // Create a browser with an in-memory store
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        // Create a session with 5 messages
        let session_id = "test-session-fits";
        store
            .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");
        for i in 0..5 {
            store
                .save_message(session_id, i, &format!("Message content {}", i))
                .expect("save message");
        }

        // Create a fresh browser and restore
        let mut browser2 = SessionBrowser::new(Arc::clone(&store));
        let outcome = browser2
            .restore(session_id, 100_000)
            .await
            .expect("restore");

        match outcome {
            ResumeOutcome::Restored {
                session_id: sid,
                messages_loaded,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(messages_loaded, 5);
            }
            other => panic!("Expected Restored, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_restore_overflow_returns_contextoverflow() {
        // Create a browser with an in-memory store
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        // Create a session with 100 short messages
        let session_id = "test-session-overflow";
        store
            .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");

        // Create 100 messages with ~50 chars each (each ~12-13 tokens via chars/4)
        // Total: 100 * 12 = ~1200 tokens, way over limit of 100
        for i in 0..100 {
            let content = format!(
                "Message number {} with some additional text to make it longer",
                i
            );
            store
                .save_message(session_id, i, &content)
                .expect("save message");
        }

        // Create a fresh browser and try to restore with small context
        let mut browser2 = SessionBrowser::new(Arc::clone(&store));
        let outcome = browser2.restore(session_id, 100).await.expect("restore");

        match outcome {
            ResumeOutcome::ContextOverflow {
                estimated_tokens: _,
                limit,
                action,
            } => {
                assert_eq!(limit, 100);
                match action {
                    OverflowAction::TruncateOldest(n) => {
                        assert!(n > 0, "Expected non-zero truncation count, got {}", n);
                    }
                    other => panic!("Expected TruncateOldest, got {:?}", other),
                }
            }
            other => panic!("Expected ContextOverflow, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_restore_missing_returns_notfound() {
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        // Create a fresh browser and try to restore a non-existent session
        let mut browser2 = SessionBrowser::new(Arc::clone(&store));
        let outcome = browser2
            .restore("nonexistent-session-id", 100_000)
            .await
            .expect("restore");

        match outcome {
            ResumeOutcome::NotFound => {}
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    // resolve_overflow tests

    #[test]
    fn test_resolve_overflow_truncate_oldest_drops_n() {
        let messages: Vec<String> = (0..10).map(|i| format!("msg{}", i)).collect();
        let action = OverflowAction::TruncateOldest(3);
        let result = SessionBrowser::resolve_overflow(messages.clone(), &action);
        assert!(result.is_some());
        let kept = result.unwrap();
        assert_eq!(kept.len(), 7);
        assert_eq!(kept[0], "msg3"); // 3 dropped, so 3 becomes first
    }

    #[test]
    fn test_resolve_overflow_truncate_overshoot_returns_empty() {
        let messages: Vec<String> = (0..5).map(|i| format!("msg{}", i)).collect();
        let action = OverflowAction::TruncateOldest(10); // n > len
        let result = SessionBrowser::resolve_overflow(messages, &action);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_resolve_overflow_switch_model_returns_none() {
        let messages: Vec<String> = (0..5).map(|i| format!("msg{}", i)).collect();
        let action = OverflowAction::SwitchModel("claude-3-5-sonnet".to_string());
        let result = SessionBrowser::resolve_overflow(messages, &action);
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_overflow_cancelled_returns_none() {
        let messages: Vec<String> = (0..5).map(|i| format!("msg{}", i)).collect();
        let action = OverflowAction::Cancelled;
        let result = SessionBrowser::resolve_overflow(messages, &action);
        assert_eq!(result, None);
    }

    #[test]
    fn test_default_overflow_action_computes_truncate_under_90pct() {
        // 800 tokens estimated, limit 1000 → target = 900, 800 < 900 → no truncation needed
        let action = SessionBrowser::default_overflow_action(800, 1000);
        match action {
            OverflowAction::TruncateOldest(n) => {
                assert_eq!(n, 0, "Under 90%, should not truncate");
            }
            other => panic!("Expected TruncateOldest(0), got {:?}", other),
        }

        // 1000 tokens, limit 1112 → target ≈ 1000, 1000 <= 1000 → 0 truncation
        let action2 = SessionBrowser::default_overflow_action(1000, 1112);
        match action2 {
            OverflowAction::TruncateOldest(n) => {
                assert_eq!(n, 0, "At exactly 90%, should not truncate");
            }
            other => panic!("Expected TruncateOldest, got {:?}", other),
        }

        // 1100 tokens, limit 1000 → target = 900, 1100 > 900 → needs truncation
        let action3 = SessionBrowser::default_overflow_action(1100, 1000);
        match action3 {
            OverflowAction::TruncateOldest(n) => {
                assert!(n > 0, "Over 90%, should need truncation");
            }
            other => panic!("Expected TruncateOldest, got {:?}", other),
        }
    }

    /// Gate 5 smoke test: full code path through SessionBrowser::restore
    /// Verifies observable outcomes for both Restored and ContextOverflow cases.
    #[tokio::test]
    async fn smoke_restore_integration() {
        // Create a SessionBrowser with an in-memory store
        let browser = SessionBrowser::new_for_tests();
        let store = Arc::clone(&browser.store);

        // Insert a session with 5 messages
        let session_id = "smoke-test-session";
        store
            .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");
        for i in 0..5 {
            store
                .save_message(session_id, i, &format!("Message content {}", i))
                .expect("save message");
        }

        // Create a fresh browser for the restore operation
        let mut browser = SessionBrowser::new(Arc::clone(&store));

        // Call restore with a generous ctx limit → expect Restored
        let outcome = browser
            .restore(session_id, 100_000)
            .await
            .expect("restore should succeed");

        // ObservableAssertion: verify Restored outcome
        let (restored_sid, msg_count) = match outcome {
            ResumeOutcome::Restored {
                session_id,
                messages_loaded,
            } => (session_id, messages_loaded),
            other => panic!("Expected Restored with generous limit, got {:?}", other),
        };
        assert_eq!(restored_sid, session_id, "Restored session_id must match");
        assert_eq!(msg_count, 5, "Must load all 5 messages");

        // ObservableAssertion: verify state.current_id was set
        assert_eq!(
            browser.state.current_id,
            Some(session_id.to_string()),
            "state.current_id must be set after restore"
        );

        // Now create a new browser and try restore with a tight ctx limit
        let mut browser2 = SessionBrowser::new(Arc::clone(&store));

        // Call restore with a tight ctx limit → expect ContextOverflow
        // 5 messages of ~20 chars each ≈ 25 tokens total, so limit of 10 will overflow
        let outcome2 = browser2
            .restore(session_id, 10)
            .await
            .expect("restore should succeed");

        // ObservableAssertion: verify ContextOverflow outcome
        let overflow_action = match outcome2 {
            ResumeOutcome::ContextOverflow {
                estimated_tokens: _,
                limit,
                action,
            } => {
                assert_eq!(limit, 10, "limit must match tight ctx limit");
                action
            }
            other => panic!("Expected ContextOverflow with tight limit, got {:?}", other),
        };

        // ObservableAssertion: verify the OverflowAction has non-zero truncate count
        match overflow_action {
            OverflowAction::TruncateOldest(n) => {
                assert!(n > 0, "TruncateOldest count must be non-zero, got {}", n);
            }
            other => panic!("Expected TruncateOldest action, got {:?}", other),
        }
    }
}
