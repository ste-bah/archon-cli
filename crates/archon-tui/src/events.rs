//! Canonical TUI event enum and layer-0 payload types.

/// A session entry for the /resume picker.
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
    GameTheory,
    Docs,
    Learning,
}

/// Source-of-truth row payload for Evidence Engine inspection overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRowPayload {
    pub id: String,
    pub title: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivityRole {
    Parent,
    Subagent,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivityStatus {
    Queued,
    Running,
    Waiting,
    WaitingForTool,
    Backgrounded,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentActivityUpdate {
    pub id: String,
    pub name: String,
    pub role: AgentActivityRole,
    pub status: AgentActivityStatus,
    pub current_tool: Option<String>,
    pub detail: Option<String>,
    pub run_id: Option<String>,
    pub parent_id: Option<String>,
    pub artifact_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
}

pub const ACTIVITY_STREAM_PREFIX: &str = "archon_activity_stream:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityStreamLineKind {
    Status,
    Thinking,
    Text,
    ToolCall,
    ToolResult,
    FinalOutput,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivityStreamUpdate {
    pub id: String,
    pub name: String,
    pub role: AgentActivityRole,
    pub status: AgentActivityStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub kind: ActivityStreamLineKind,
    pub text: String,
    pub tool: Option<String>,
    pub is_error: bool,
}

impl ActivityStreamUpdate {
    pub fn from_activity_event(event: archon_observability::AgentActivityEvent) -> Self {
        let base = AgentActivityUpdate::from(event.clone());
        if let Some(payload) = activity_stream_payload(&event.message) {
            return Self {
                id: base.id,
                name: base.name,
                role: base.role,
                status: base.status,
                provider: base.provider,
                model: base.model,
                kind: payload.kind,
                text: payload.text,
                tool: payload.tool,
                is_error: payload.is_error,
            };
        }
        Self {
            id: base.id,
            name: base.name,
            role: base.role,
            status: base.status,
            provider: base.provider,
            model: base.model,
            kind: ActivityStreamLineKind::Status,
            text: base
                .detail
                .unwrap_or_else(|| format!("{:?}", base.status).to_lowercase()),
            tool: base.current_tool,
            is_error: matches!(
                base.status,
                AgentActivityStatus::Failed | AgentActivityStatus::Cancelled
            ),
        }
    }
}

pub fn is_activity_stream_payload(message: &str) -> bool {
    message.starts_with(ACTIVITY_STREAM_PREFIX)
}

struct ActivityPayload {
    kind: ActivityStreamLineKind,
    text: String,
    tool: Option<String>,
    is_error: bool,
}

fn activity_stream_payload(message: &str) -> Option<ActivityPayload> {
    let raw = message.strip_prefix(ACTIVITY_STREAM_PREFIX)?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let kind = match value.get("kind")?.as_str()? {
        "thinking" => ActivityStreamLineKind::Thinking,
        "text" => ActivityStreamLineKind::Text,
        "tool_call" => ActivityStreamLineKind::ToolCall,
        "tool_result" => ActivityStreamLineKind::ToolResult,
        "final" => ActivityStreamLineKind::FinalOutput,
        "error" => ActivityStreamLineKind::Error,
        _ => ActivityStreamLineKind::Status,
    };
    Some(ActivityPayload {
        kind,
        text: value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        tool: value
            .get("tool")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        is_error: value
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

impl From<archon_observability::AgentActivityEvent> for AgentActivityUpdate {
    fn from(event: archon_observability::AgentActivityEvent) -> Self {
        let role = activity_role(&event);
        let status = activity_status(&event);
        let id = activity_id(&event);
        let name = activity_name(&event, role);
        let current_tool = match event.kind {
            archon_observability::AgentActivityKind::ToolStarted
            | archon_observability::AgentActivityKind::ToolCompleted
            | archon_observability::AgentActivityKind::ToolFailed => Some(event.message.clone()),
            _ => None,
        };

        Self {
            id,
            name,
            role,
            status,
            current_tool,
            detail: Some(event.message),
            run_id: event.run_id,
            parent_id: event.parent_id,
            artifact_id: event.artifact_id,
            provider: event.provider,
            model: event.model,
            cost_usd: event.cost_usd,
        }
    }
}

fn activity_role(event: &archon_observability::AgentActivityEvent) -> AgentActivityRole {
    match event.kind {
        archon_observability::AgentActivityKind::ParentTurnStarted
        | archon_observability::AgentActivityKind::ParentTurnCompleted
        | archon_observability::AgentActivityKind::ToolStarted
        | archon_observability::AgentActivityKind::ToolCompleted
        | archon_observability::AgentActivityKind::ToolFailed => AgentActivityRole::Parent,
        archon_observability::AgentActivityKind::AgentBackgrounded
        | archon_observability::AgentActivityKind::AgentResumed => AgentActivityRole::Background,
        _ => AgentActivityRole::Subagent,
    }
}

fn activity_status(event: &archon_observability::AgentActivityEvent) -> AgentActivityStatus {
    match event.kind {
        archon_observability::AgentActivityKind::ToolStarted => {
            return AgentActivityStatus::WaitingForTool;
        }
        archon_observability::AgentActivityKind::ToolCompleted => {
            return AgentActivityStatus::Complete;
        }
        archon_observability::AgentActivityKind::ToolFailed => {
            return AgentActivityStatus::Failed;
        }
        _ => {}
    }

    match event.status {
        archon_observability::AgentActivityStatus::Queued => AgentActivityStatus::Queued,
        archon_observability::AgentActivityStatus::Running => AgentActivityStatus::Running,
        archon_observability::AgentActivityStatus::Waiting => AgentActivityStatus::Waiting,
        archon_observability::AgentActivityStatus::Backgrounded => {
            AgentActivityStatus::Backgrounded
        }
        archon_observability::AgentActivityStatus::Completed => AgentActivityStatus::Complete,
        archon_observability::AgentActivityStatus::Failed => AgentActivityStatus::Failed,
        archon_observability::AgentActivityStatus::Cancelled => AgentActivityStatus::Cancelled,
    }
}

fn activity_id(event: &archon_observability::AgentActivityEvent) -> String {
    event
        .subagent_id
        .clone()
        .or_else(|| event.agent_id.clone())
        .or_else(|| event.run_id.clone())
        .unwrap_or_else(|| "parent".to_string())
}

fn activity_name(
    event: &archon_observability::AgentActivityEvent,
    role: AgentActivityRole,
) -> String {
    event
        .agent_key
        .clone()
        .or_else(|| event.subagent_type.clone())
        .or_else(|| event.model.clone())
        .unwrap_or_else(|| match role {
            AgentActivityRole::Parent => "Parent".to_string(),
            AgentActivityRole::Subagent => "Subagent".to_string(),
            AgentActivityRole::Background => "Background".to_string(),
        })
}

/// Summary of a conversation message for the /rewind overlay list (TASK-TUI-620).
///
/// Defined at layer 0 (events.rs) so that `TuiEvent::ShowMessageSelector` can
/// reference it without events.rs having to import from crate::app. Re-exported
/// from `crate::app` for external/public-API stability (matches
/// `SessionPickerEntry` / `McpServerEntry` pattern).
#[derive(Debug, Clone)]
pub struct MessageSummary {
    /// Stable message identifier from the session store.
    pub id: String,
    /// Wall-clock timestamp of when the message was recorded.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// First N characters of the message body (N=60 per spec).
    pub preview: String,
}

/// TASK-TUI-627: summary of a registered skill for the /skills overlay list.
#[derive(Debug, Clone)]
pub struct SkillEntry {
    /// Canonical skill name (no leading `/`).
    pub name: String,
    /// One-line human description.
    pub description: String,
}

/// TASK-#207 SLASH-FILES: a single entry in the /files file-picker
/// overlay. Defined at layer 0 (events.rs) so `TuiEvent::ShowFilePicker`
/// can reference it without events.rs importing from `crate::app`.
/// Re-exported from `crate::app` for external/public-API stability.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Display name — the file's basename, no parent path.
    pub name: String,
    /// Absolute path. Used for `@<path>` injection on file-Enter,
    /// and as the new `current_dir` when the picker descends into a
    /// directory.
    pub path: std::path::PathBuf,
    /// `true` for directories, `false` for regular files.
    pub is_dir: bool,
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
    ToolStart {
        name: String,
        id: String,
    },
    ToolComplete {
        name: String,
        id: String,
        success: bool,
        output: String,
    },
    TurnComplete {
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
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
    /// TASK-TUI-620: open the message-selector overlay with a pre-computed
    /// list of MessageSummary entries. Follow-up ticket wires input/render;
    /// for now the event only sets `app.message_selector`.
    ShowMessageSelector(Vec<MessageSummary>),
    /// TASK-TUI-627: open the skills-menu overlay with a pre-computed
    /// list of SkillEntry. Input/render routing deferred to TUI-627-followup.
    ShowSkillsMenu(Vec<SkillEntry>),
    /// TASK-#207 SLASH-FILES: open the file-picker overlay with a
    /// pre-walked initial listing of the working directory. The
    /// event-loop handler constructs `FilePicker::new(root, entries)`
    /// where `root` is the picker's clamp-anchor (passed via the
    /// first entry's parent — see `event_loop/tui_events.rs` for the
    /// resolution path).
    ShowFilePicker {
        /// Original working directory (the picker's ascent-clamp root).
        root: std::path::PathBuf,
        /// Pre-walked initial listing of `root`. Empty Vec is
        /// acceptable (rendered as `(empty directory)`).
        entries: Vec<FileEntry>,
    },
    /// TASK-#208 SLASH-SEARCH: open the search-results overlay with
    /// the original query string and a list of matched paths. The
    /// event-loop handler constructs `SearchResults::new(query,
    /// entries)` and assigns to `app.search_results`.
    ShowSearchResults {
        /// The original query the user supplied to `/search <query>`.
        /// Used for the case-insensitive highlight match in the
        /// rendered rows.
        query: String,
        /// The matched file paths (cap'd at the slash handler's
        /// `max_results` ceiling).
        entries: Vec<FileEntry>,
    },
    /// TASK-AGS-822: open an overlay view identified by `ViewId`.
    /// Emitted by the slash-command dispatcher in response to
    /// view-opening commands (`/tasks`, `/settings`, `/context`,
    /// `/memory`, `/model`, `/status`). Clustered here with the other
    /// overlay-opening variants (`ShowSessionPicker`, `ShowMcpManager`)
    /// so future readers locate overlay events in one place.
    OpenView(ViewId),
    /// Open an Evidence Engine overlay with rows loaded by the slash handler
    /// from the authoritative store.
    OpenViewRows {
        view_id: ViewId,
        rows: Vec<EvidenceRowPayload>,
    },
    /// Update a visible parent/subagent/background activity row.
    AgentActivity(AgentActivityUpdate),
    /// Append/update the foreground activity stream buffer.
    ActivityStream(ActivityStreamUpdate),
    ContextPressureUpdated {
        tokens_used: u64,
        context_window: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        context_name: Option<String>,
        resolution_source: Option<String>,
    },
    SetVimMode(bool),
    VimToggle,
    VoiceText(String),
    SetAgentInfo {
        name: String,
        color: Option<String>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    UserInput(String),
    SlashCancel,
    SlashAgent(String),
    Done,
    /// Notification overlay with a duration in milliseconds (TUI-330).
    NotificationTimeout(u64),
}

impl TuiEvent {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::TextDelta(_) => "TextDelta",
            Self::ThinkingDelta(_) => "ThinkingDelta",
            Self::ToolStart { .. } => "ToolStart",
            Self::ToolComplete { .. } => "ToolComplete",
            Self::TurnComplete { .. } => "TurnComplete",
            Self::Error(_) => "Error",
            Self::GenerationStarted => "GenerationStarted",
            Self::SlashCommandComplete => "SlashCommandComplete",
            Self::ThinkingToggle(_) => "ThinkingToggle",
            Self::ModelChanged(_) => "ModelChanged",
            Self::BtwResponse(_) => "BtwResponse",
            Self::PermissionPrompt { .. } => "PermissionPrompt",
            Self::SessionRenamed(_) => "SessionRenamed",
            Self::PermissionModeChanged(_) => "PermissionModeChanged",
            Self::ShowSessionPicker(_) => "ShowSessionPicker",
            Self::SetAccentColor(_) => "SetAccentColor",
            Self::SetTheme(_) => "SetTheme",
            Self::ShowMcpManager(_) => "ShowMcpManager",
            Self::UpdateMcpManager(_) => "UpdateMcpManager",
            Self::ShowMessageSelector(_) => "ShowMessageSelector",
            Self::ShowSkillsMenu(_) => "ShowSkillsMenu",
            Self::ShowFilePicker { .. } => "ShowFilePicker",
            Self::ShowSearchResults { .. } => "ShowSearchResults",
            Self::OpenView(_) => "OpenView",
            Self::OpenViewRows { .. } => "OpenViewRows",
            Self::AgentActivity(_) => "AgentActivity",
            Self::ActivityStream(_) => "ActivityStream",
            Self::ContextPressureUpdated { .. } => "ContextPressureUpdated",
            Self::SetVimMode(_) => "SetVimMode",
            Self::VimToggle => "VimToggle",
            Self::VoiceText(_) => "VoiceText",
            Self::SetAgentInfo { .. } => "SetAgentInfo",
            Self::Resize { .. } => "Resize",
            Self::UserInput(_) => "UserInput",
            Self::SlashCancel => "SlashCancel",
            Self::SlashAgent(_) => "SlashAgent",
            Self::Done => "Done",
            Self::NotificationTimeout(_) => "NotificationTimeout",
        }
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
