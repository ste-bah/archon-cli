//! Session browser screen.
//!
//! REQ-TUI-MOD-006 / REQ-TUI-MOD-007
//! Layer 1 module — no imports from screens/ or app/.
//!
//! Split into sub-modules to stay under the 500-line per-file ceiling
//! (see scripts/check-tui-file-sizes.sh):
//!   - `browser`: `SessionBrowser` struct + navigation + render + test constructors.
//!   - `restore`: restore / context-overflow logic.
//!
//! This `mod.rs` owns the shared data types and the public re-export surface,
//! so external callers continue to import from `archon_tui::screens::session_browser`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

mod browser;
mod restore;

pub use browser::SessionBrowser;

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
