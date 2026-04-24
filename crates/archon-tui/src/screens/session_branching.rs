//! Session branching / branch picker screen.
//! Layer 1 module — no imports from screens/ or app/.

use std::sync::Arc;
use thiserror::Error;

use anyhow::Result;
use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, List, ListItem};

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

use super::session_browser::{BranchPoint, SessionState};
use archon_session::fork;
use archon_session::storage::SessionStore;

/// Reference to a message / branch point.
#[derive(Debug, Clone)]
pub struct MessageRef {
    pub id: String,
    pub summary: String,
    pub timestamp: String,
}

/// Branch picker for session branching UI.
#[derive(Debug)]
pub struct BranchPicker {
    parent_id: String,
    list: VirtualList<MessageRef>,
}

impl BranchPicker {
    pub fn new(parent_id: String) -> Self {
        Self {
            parent_id,
            list: VirtualList::new(Vec::new(), 10),
        }
    }

    pub fn parent_id(&self) -> &str {
        &self.parent_id
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

    pub fn selected(&self) -> Option<&MessageRef> {
        self.list.selected()
    }

    pub fn set_candidates(&mut self, candidates: Vec<MessageRef>) {
        self.list.set_items(candidates);
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

    /// Render branch picker into area.
    pub fn render(&self, f: &mut Frame, area: Rect, _theme: &Theme) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("Branch from {}", self.parent_id));

        let items: Vec<ListItem> = self
            .list
            .visible_items()
            .iter()
            .map(|m| ListItem::new(format!("{} — {}", m.summary, m.timestamp)))
            .collect();

        let list = List::new(items).block(block);
        f.render_widget(list, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_picker_empty() {
        let picker = BranchPicker::new("parent-1".to_string());
        assert!(picker.is_empty());
        assert_eq!(picker.parent_id(), "parent-1");
    }

    #[test]
    fn set_candidates_updates_list() {
        let mut picker = BranchPicker::new("p".to_string());
        let refs = vec![
            MessageRef {
                id: "1".into(),
                summary: "A".into(),
                timestamp: "10:00".into(),
            },
            MessageRef {
                id: "2".into(),
                summary: "B".into(),
                timestamp: "10:05".into(),
            },
        ];
        picker.set_candidates(refs);
        assert_eq!(picker.len(), 2);
    }

    #[test]
    fn cursor_wraps() {
        let mut picker = BranchPicker::new("p".to_string());
        picker.set_candidates(vec![
            MessageRef {
                id: "0".into(),
                summary: "A".into(),
                timestamp: "10:00".into(),
            },
            MessageRef {
                id: "1".into(),
                summary: "B".into(),
                timestamp: "10:05".into(),
            },
            MessageRef {
                id: "2".into(),
                summary: "C".into(),
                timestamp: "10:10".into(),
            },
        ]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 1);
        picker.move_down();
        assert_eq!(picker.selected_index(), 2);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
    }

    #[test]
    fn single_candidate_wraps() {
        let mut picker = BranchPicker::new("p".to_string());
        picker.set_candidates(vec![MessageRef {
            id: "0".into(),
            summary: "A".into(),
            timestamp: "10:00".into(),
        }]);
        picker.move_down();
        assert_eq!(picker.selected_index(), 0); // wrap
        picker.move_up();
        assert_eq!(picker.selected_index(), 0); // wrap
    }
}

// ============================================================================
// SessionBranching — TUI-707
// ============================================================================

/// Error returned when a branch operation fails.
#[derive(Debug, Error)]
pub enum BranchError {
    /// The requested branch ID was not found in visible_branches.
    #[error("unknown branch: {0}")]
    UnknownBranch(String),
}

/// SessionBranching manages session branch switching and visibility.
///
/// Stores a reference to the session store, current session state,
/// and tracks which branch points are visible for switching.
pub struct SessionBranching {
    store: Arc<SessionStore>,
    state: SessionState,
    visible_branches: Vec<BranchPoint>,
}

impl std::fmt::Debug for SessionBranching {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBranching")
            .field("state", &self.state)
            .field("visible_branches", &self.visible_branches)
            .finish()
    }
}

impl SessionBranching {
    /// Create a new SessionBranching with the given store and initial state.
    pub fn new(store: Arc<SessionStore>, state: SessionState) -> Self {
        let visible_branches = state.branches.clone();
        Self {
            store,
            state,
            visible_branches,
        }
    }

    /// Returns a slice of the currently visible branch points.
    pub fn branches(&self) -> &[BranchPoint] {
        &self.visible_branches
    }

    /// Switch to the branch with the given branch_id.
    ///
    /// Returns Ok(()) if the branch was found and state.current_id is updated.
    /// Returns Err(BranchError::UnknownBranch) if the branch_id is not in visible_branches.
    pub fn switch(&mut self, branch_id: &str) -> Result<(), BranchError> {
        let found = self.visible_branches.iter().any(|b| b.id == branch_id);
        if found {
            self.state.current_id = Some(branch_id.to_string());
            Ok(())
        } else {
            Err(BranchError::UnknownBranch(branch_id.to_string()))
        }
    }

    /// Fork the current session at the given message index with the given label.
    ///
    /// Merge is FUTURE per PRD out-of-scope.
    ///
    /// Adapter: archon_session::fork::fork_at does not exist in the current API.
    /// Uses fork_session from archon-session/src/fork.rs:16-47 which copies all
    /// messages (no at_message truncation). The at_message parameter is stored in
    /// BranchPoint.branched_at_message for UI display only.
    pub fn fork(&mut self, at_message: usize, label: &str) -> Result<BranchPoint> {
        // 1. Require an active session
        let parent_session_id = self
            .state
            .current_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot fork: no active session"))?;

        // 2. Persist via archon_session fork primitive
        let new_session_id =
            fork::fork_session(self.store.as_ref(), parent_session_id, Some(label))
                .map_err(|e| anyhow::anyhow!("fork failed: {}", e))?;

        // 3. Build BranchPoint with correct fields
        let branch_point = BranchPoint {
            id: new_session_id,
            parent_session: parent_session_id.clone(),
            branched_at_message: at_message,
            label: label.to_string(),
            created_at: Utc::now(),
        };

        // 4. Push onto visible_branches only on success
        self.visible_branches.push(branch_point.clone());

        Ok(branch_point)
    }

    /// Returns a reference to the current session state.
    pub fn state(&self) -> &SessionState {
        &self.state
    }
}

// ---------------------------------------------------------------------------
// SessionBranching fork tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod session_branching_tests {
    use super::*;

    /// Wrapper that keeps temp_dir alive alongside the store.
    struct TestEnv {
        _temp_dir: tempfile::TempDir,
        store: Arc<SessionStore>,
    }

    impl TestEnv {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().expect("temp dir");
            let store = Arc::new(
                archon_session::storage::SessionStore::open(&temp_dir.path().join("test.db"))
                    .expect("test store"),
            );
            Self {
                _temp_dir: temp_dir,
                store,
            }
        }
    }

    fn make_state(current_id: Option<String>) -> SessionState {
        SessionState {
            current_id,
            branches: Vec::new(),
            history_cursor: 0,
        }
    }

    #[test]
    fn test_fork_without_active_session_errs() {
        let env = TestEnv::new();
        let state = make_state(None);
        let mut branching = SessionBranching::new(env.store.clone(), state);

        let result = branching.fork(5, "test-branch");
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("no active session"));
    }

    #[test]
    fn test_fork_appends_branch_point_on_success() {
        let env = TestEnv::new();

        // Create a parent session first so fork has something to fork from
        let parent_meta = env
            .store
            .create_session("/tmp", None, "gpt-4")
            .expect("parent session");
        let state = make_state(Some(parent_meta.id.clone()));
        let mut branching = SessionBranching::new(env.store.clone(), state);

        let initial_len = branching.branches().len();

        let result = branching.fork(3, "my-branch");
        assert!(result.is_ok());

        assert_eq!(branching.branches().len(), initial_len + 1);
    }

    #[test]
    fn test_fork_returned_branchpoint_has_correct_parent_and_offset() {
        let env = TestEnv::new();

        // Create a parent session
        let parent_meta = env
            .store
            .create_session("/tmp", None, "gpt-4")
            .expect("parent session");
        let parent_id = parent_meta.id.clone();
        let state = make_state(Some(parent_id.clone()));
        let mut branching = SessionBranching::new(env.store.clone(), state);

        let result = branching.fork(7, "offset-test");
        assert!(result.is_ok());

        let bp = result.unwrap();
        assert_eq!(bp.parent_session, parent_id);
        assert_eq!(bp.branched_at_message, 7);
        assert_eq!(bp.label, "offset-test");
    }
}
