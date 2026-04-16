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
// Skeleton module placeholders (replace when modules are populated)
// ---------------------------------------------------------------------------

/// Placeholder for notifications module. Populated in a later phase-3 task.
pub struct NotificationManager;

/// Placeholder for keybindings module. Populated in a later phase-3 task.
pub struct KeyBindings;

// ---------------------------------------------------------------------------
// Session state (local structs — avoids crate::app import)
// ---------------------------------------------------------------------------

/// Active session picker state.
///
/// We define a local struct rather than importing from crate::app to preserve
/// the direction invariant (state.rs must not import from crate::app).
#[derive(Debug, Clone)]
pub struct SessionPicker {
    pub entries: Vec<SessionPickerEntry>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct SessionPickerEntry {
    pub id: String,
    pub name: String,
    pub turns: u64,
    pub cost: f64,
    pub last_active: String,
}

/// MCP manager view state.
///
/// We define a local enum rather than importing from crate::app to preserve
/// the direction invariant.
#[derive(Debug, Clone)]
pub enum McpManagerView {
    ServerList { selected: usize },
    ServerMenu { server_idx: usize, action_idx: usize },
    ToolList { scroll: usize },
}

/// Active MCP manager state.
#[derive(Debug, Clone)]
pub struct McpManager {
    pub server_count: usize,
    pub view: McpManagerView,
}

/// Session-scoped UI state extracted from app.rs App struct.
///
/// These fields represent transient UI state tied to the current session,
/// not the session itself (which lives in archon_core).
#[derive(Debug, Clone)]
pub struct SessionState {
    /// Whether the session picker overlay is currently active.
    pub session_picker: Option<SessionPicker>,
    /// Session display name (shown on input line badge).
    pub name: Option<String>,
    /// Whether vim keybinding mode is currently active.
    pub vim_mode_active: bool,
    /// Active MCP manager overlay, if any.
    pub mcp_manager: Option<McpManager>,
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
    /// Notification overlay state (placeholder).
    pub notifications: NotificationManager,
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
                session_picker: None,
                name: None,
                vim_mode_active: false,
                mcp_manager: None,
            },
            layout: LayoutState::default(),
            notifications: NotificationManager,
            metrics: ChannelMetrics::new(),
            theme,
            keybindings: KeyBindings,
        }
    }
}
