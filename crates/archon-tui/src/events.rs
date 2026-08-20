//! Canonical TUI event enum and layer-0 payload types.

pub use crate::video_events::VideoIngestProgressEvent;

#[derive(Debug, Clone)]
pub struct SessionPickerEntry {
    pub id: String,
    pub name: String,
    pub turns: u64,
    pub cost: f64,
    pub last_active: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewId {
    Tasks,
    Settings,
    Context,
    MemoryBrowser,
    ModelPicker,
    Status,
    Cognitive,
    GameTheory,
    Docs,
    Learning,
    Video,
    Workflow,
    World,
}

/// Source-of-truth row payload for Evidence Engine inspection overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRowPayload {
    pub id: String,
    pub title: String,
    pub status: String,
    pub detail: String,
}

/// Agent-activity payload types live in `events_activity.rs` (500-line gate).
pub use crate::events_activity::{
    ACTIVITY_STREAM_PREFIX, ActivityStreamLineKind, ActivityStreamUpdate, AgentActivityRole,
    AgentActivityStatus, AgentActivityUpdate, is_activity_stream_payload,
};

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
    TransientThinkingDelta(String),
    CommitThinkingPreview,
    DiscardThinkingPreview,
    ToolStart {
        name: String,
        id: String,
    },
    ToolOutputChunk {
        id: String,
        chunk: String,
    },
    ToolComplete {
        name: String,
        id: String,
        success: bool,
        output: String,
        transcript_summary: Option<String>,
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
    /// Open the completed thinking archive overlay.
    OpenThinkingArchive,
    ModelChanged(String),
    BtwResponse(String),
    PermissionPrompt {
        tool: String,
        description: String,
    },
    AskUserPrompt {
        question: String,
        kind: archon_core::agent::AskUserPromptKind,
    },
    SessionRenamed(String),
    PermissionModeChanged(String),
    ShowSessionPicker(Vec<SessionPickerEntry>),
    SetAccentColor(ratatui::style::Color),
    SetTheme(String),
    ShowMcpManager(Vec<McpServerEntry>),
    UpdateMcpManager(Vec<McpServerEntry>),
    /// Open the message-selector overlay with pre-computed rows.
    ShowMessageSelector(Vec<MessageSummary>),
    /// Open the skills-menu overlay with pre-computed rows.
    ShowSkillsMenu(Vec<SkillEntry>),
    /// Open the model-picker overlay (#192).
    ///
    /// Each entry is `(provider_id, model_id, label)`. Resolved at the dispatch
    /// site rather than in the TUI: the handler is sync and the model config
    /// lives behind an async lock, which is the same reason `ModelSnapshot`
    /// exists.
    ShowModelPicker(Vec<(String, String, String)>),
    /// Open the theme picker (#192). Each entry is `(name, is_active)`.
    ShowThemePicker(Vec<(String, bool)>),
    /// Open the settings overlay (#192, `/config` with no arguments).
    ///
    /// Each entry is `(key, value, is_bool, read_only)`, resolved at the
    /// dispatch site: the key registry lives in `archon-tools`, which the TUI
    /// depends on only as a dev-dependency.
    ShowSettings(Vec<(String, String, bool, bool)>),
    /// Open the hooks overlay (#192, `/hooks` with no subcommand).
    ///
    /// Each entry is `(id, event, command, source, enabled)`, taken from the
    /// registry summaries the text listing already renders.
    ShowHooks(Vec<(String, String, String, String, bool)>),
    /// Open the permission-rules overlay (#192, `/permissions` with no mode).
    ///
    /// `mode` is what the mode line already says; `rules` are the
    /// `[permissions]` entries evaluated ahead of it, as
    /// `(effect, tool, pattern)` where effect is `deny`, `allow` or `ask`.
    ShowPermissions {
        mode: String,
        rules: Vec<(String, String, String)>,
    },
    /// Open the memory-files overlay (#192, `/memory files`).
    ///
    /// Each entry is `(scope, path, size_bytes)` in the order the files layer
    /// into the system prompt.
    ShowMemoryFiles(Vec<(String, String, u64)>),
    /// Open the branch picker (#192, `/fork-at` with no arguments).
    ///
    /// Each entry is `(index, role, summary)`. The index is what `/fork-at`
    /// takes and what the fork keeps through, inclusive.
    ShowBranchPicker(Vec<(usize, String, String)>),
    /// Open the voice capture overlay (#192, `/voice` with no arguments).
    ///
    /// Carries the configured VAD threshold so the overlay marks the level a
    /// recording actually has to beat, rather than a hard-coded guess.
    ShowVoiceCapture {
        vad_threshold: f32,
    },
    /// Open the token attribution overlay (#192 scope B, `/context`).
    ///
    /// Carries message previews and nothing else: the ranking is already on the
    /// `App`, put there by `ContextPressureUpdated`, because only the agent has
    /// the calibrated surface. `/context` supplies the text for those indices
    /// because only the session log has it. Each side sends what it knows.
    ///
    /// Entries are `(message_index, role, summary)`.
    ShowTokenAttribution(Vec<(usize, String, String)>),
    /// A recording started (`true`) or ended (`false`).
    ///
    /// Emitted by the voice pipeline, not by a key handler: the hotkey only
    /// asks for a recording, and whether one begins depends on the microphone.
    VoiceRecording(bool),
    /// One RMS level reading from the capture thread, for the overlay meter.
    VoiceLevel(f32),
    /// Open the file-picker overlay with a pre-walked listing.
    ShowFilePicker {
        /// Original working directory (the picker's ascent-clamp root).
        root: std::path::PathBuf,
        /// Pre-walked initial listing of `root`.
        entries: Vec<FileEntry>,
    },
    /// Open the search-results overlay with matched paths.
    ShowSearchResults {
        /// The original query the user supplied to `/search <query>`.
        query: String,
        /// The matched file paths.
        entries: Vec<FileEntry>,
    },
    /// Open an overlay view identified by `ViewId`.
    OpenView(ViewId),
    /// Open an Evidence Engine overlay with rows loaded by the slash handler
    /// from the authoritative store.
    OpenViewRows {
        view_id: ViewId,
        rows: Vec<EvidenceRowPayload>,
    },
    /// Incremental video ingest progress for the video overlay.
    VideoIngestProgress(VideoIngestProgressEvent),
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
        /// Tokens attributed to the largest single message (#189 Phase 3).
        heaviest_message_tokens: u64,
        /// The heaviest messages, biggest first, as `(message_index, tokens)`
        /// (#192 scope B). What `/context` lists when it opens the attribution
        /// overlay.
        top_contributors: Vec<(usize, u64)>,
        /// Attributed tokens across every message, so a share is computable
        /// from a truncated ranking.
        attributed_total: u64,
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
    Done,
    /// Notification overlay with a duration in milliseconds (TUI-330).
    NotificationTimeout(u64),
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
