//! REQ-MOD-020: Centralized application state.
//!
//! AppState owns the dispatcher, session state, layout state, notifications,
//! metrics, theme, and keybindings. Direction invariant: this module must
//! NOT import from crate::screens, crate::app, crate::input, or crate::command.

use std::sync::Arc;

use archon_core::agent::TimestampedEvent;
use tokio::sync::mpsc::UnboundedSender;

use crate::layout::ReflowOutcome;
use crate::observability::ChannelMetrics;
use crate::task_dispatch::{AgentDispatcher, AgentRouter};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Skeleton module placeholders
// ---------------------------------------------------------------------------
//
// AppState is currently a construction skeleton — session UI state still
// lives on `app::App`. Future tasks (TUI-311+) will migrate fields
// incrementally using canonical types from `crate::app` / `crate::events`.
// ---------------------------------------------------------------------------

/// Notification queue for toast-style overlay messages (REQ-MOD-017).
pub use crate::notifications::NotificationQueue;

/// Placeholder for keybindings module. Populated in a later phase-3 task.
pub struct KeyBindings;

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// Session-scoped UI state.
///
/// Currently a skeleton placeholder — the real session UI state (session
/// picker, MCP manager overlay, etc.) still lives on `app::App`. Future
/// tasks (TUI-311+) will migrate those fields here using the canonical
/// types defined in `crate::app` / `crate::events`.
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    /// Session display name (shown on input line badge). Not yet wired.
    pub name: Option<String>,
    /// Whether vim keybinding mode is currently active. Not yet wired.
    pub vim_mode_active: bool,
}

// ---------------------------------------------------------------------------
// Layout state
// ---------------------------------------------------------------------------

/// Layout state derived from resize events.
#[derive(Debug, Clone)]
pub struct LayoutState {
    pub last_outcome: ReflowOutcome,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            last_outcome: ReflowOutcome {
                cols: 0,
                rows: 0,
                dirty: true,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// AppState
// ---------------------------------------------------------------------------

/// Centralized application state for the Archon TUI.
///
/// Owns:
/// - `dispatcher`: agent turn lifecycle (task_dispatch)
/// - `session`: session-scoped UI state
/// - `layout`: resize/reflow state
/// - `notifications`: notification overlay state (placeholder)
/// - `metrics`: channel observability instrumentation
/// - `theme`: active color theme
/// - `keybindings`: keybinding registry (placeholder)
///
/// # Direction invariant
/// AppState and this module must NOT import from:
/// - crate::screens
/// - crate::app
/// - crate::input
/// - crate::command
///
/// This ensures clean separation: state management does not depend on
/// rendering, input handling, or command dispatch.
pub struct AppState {
    /// Agent turn dispatcher.
    pub dispatcher: AgentDispatcher,
    /// Session-scoped UI state.
    pub session: SessionState,
    /// Layout and resize state.
    pub layout: LayoutState,
    /// Notification queue for toast-style overlay messages.
    pub notifications: NotificationQueue,
    /// Channel observability metrics.
    pub metrics: ChannelMetrics,
    /// Active color theme.
    pub theme: Theme,
    /// Keybinding registry (placeholder).
    pub keybindings: KeyBindings,
}

impl AppState {
    /// Construct a new AppState.
    ///
    /// # Arguments
    /// * `router` — agent router for `AgentDispatcher`
    /// * `agent_event_tx` — event channel sender
    /// * `theme` — initial color theme
    pub fn new(
        router: Arc<dyn AgentRouter>,
        agent_event_tx: UnboundedSender<TimestampedEvent>,
        theme: Theme,
    ) -> Self {
        Self {
            dispatcher: AgentDispatcher::new(router, agent_event_tx),
            session: SessionState {
                name: None,
                vim_mode_active: false,
            },
            layout: LayoutState::default(),
            notifications: NotificationQueue::new(),
            metrics: ChannelMetrics::new(),
            theme,
            keybindings: KeyBindings,
        }
    }
}
