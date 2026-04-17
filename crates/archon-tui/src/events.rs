//! TuiEvent — the canonical event enum for archon-tui.
//!
//! All variants consumed by the TUI event loop are defined here so that
//! downstream modules (`terminal.rs`, event loop handlers, etc.) do not
//! need to depend on `app.rs` directly.
//!
//! TUI-330: `SessionPickerEntry` and `McpServerEntry` were moved here from
//! `app.rs` to preserve the layer-0 direction invariant enforced by
//! `scripts/check-tui-module-cycles.sh` (events.rs must not import from
//! crate::app). `app.rs` re-exports these types with `pub use` so external
//! consumers (e.g. `src/session.rs`, `src/command/slash.rs`, and the
//! existing integration tests that reference `archon_tui::app::McpServerEntry`
//! / `archon_tui::app::SessionPickerEntry`) keep compiling unchanged.

/// A session entry for the /resume picker.
///
/// Defined here (layer 0) so that `TuiEvent::ShowSessionPicker` can reference
/// it without events.rs having to import from crate::app. Re-exported from
/// `crate::app` for external/public-API stability.
#[derive(Debug, Clone)]
pub struct SessionPickerEntry {
    pub id: String,
    pub name: String,
    pub turns: u64,
    pub cost: f64,
    pub last_active: String,
}

/// Identifies an overlay view the TUI can open in response to a
/// slash-command-originated `TuiEvent::OpenView`.
///
/// Defined at layer 0 (events.rs) so variants of `TuiEvent` can
/// reference it without the file taking on a `crate::app` dependency.
/// Re-exported from `crate::app` for external-API stability
/// (matches `SessionPickerEntry` / `McpServerEntry` pattern).
///
/// TASK-AGS-822: seeded with the 6 variants named by body-migrate
/// tickets AGS-806..819. Future view-opening slash commands extend
/// this enum per-ticket; the compile error on any missing arm is the
/// preferred contract (so `#[non_exhaustive]` is deliberately NOT
/// applied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    Tasks,
    Settings,
    Context,
    MemoryBrowser,
    ModelPicker,
    Status,
}

/// An MCP server entry shown in the MCP manager overlay.
///
/// Defined here (layer 0) so that `TuiEvent::ShowMcpManager` and
/// `TuiEvent::UpdateMcpManager` can reference it without events.rs having
/// to import from crate::app. Re-exported from `crate::app` for
/// external/public-API stability.
#[derive(Debug, Clone)]
pub struct McpServerEntry {
    pub name: String,
    /// One of: "ready", "crashed", "starting", "stopped", "disabled".
    pub state: String,
    pub tool_count: usize,
    pub disabled: bool,
    /// Fully-qualified tool names (mcp__server__tool) for View Tools.
    pub tools: Vec<String>,
}

/// Message type sent from the agent loop to the TUI.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolStart { name: String, id: String },
    ToolComplete {
        name: String,
        id: String,
        success: bool,
        output: String,
    },
    TurnComplete {
        input_tokens: u64,
        output_tokens: u64,
    },
    Error(String),
    /// Emitted by main.rs right before agent.process_message().
    GenerationStarted,
    /// Emitted by main.rs after a slash command completes.
    SlashCommandComplete,
    ThinkingToggle(bool),
    ModelChanged(String),
    BtwResponse(String),
    PermissionPrompt {
        tool: String,
        description: String,
    },
    SessionRenamed(String),
    PermissionModeChanged(String),
    ShowSessionPicker(Vec<SessionPickerEntry>),
    SetAccentColor(ratatui::style::Color),
    SetTheme(String),
    ShowMcpManager(Vec<McpServerEntry>),
    UpdateMcpManager(Vec<McpServerEntry>),
    /// TASK-AGS-822: open an overlay view identified by `ViewId`.
    /// Emitted by the slash-command dispatcher in response to
    /// view-opening commands (`/tasks`, `/settings`, `/context`,
    /// `/memory`, `/model`, `/status`). Clustered here with the other
    /// overlay-opening variants (`ShowSessionPicker`, `ShowMcpManager`)
    /// so future readers locate overlay events in one place.
    OpenView(ViewId),
    SetVimMode(bool),
    VimToggle,
    VoiceText(String),
    SetAgentInfo {
        name: String,
        color: Option<String>,
    },
    Resize { cols: u16, rows: u16 },
    UserInput(String),
    SlashCancel,
    SlashAgent(String),
    Done,
    /// Notification overlay with a duration in milliseconds (TUI-330).
    NotificationTimeout(u64),
}
