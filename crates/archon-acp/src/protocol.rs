//! The ACP wire types (#189 Phase 11).
//!
//! Field names and enum spellings are the protocol's, not ours: `camelCase`
//! members, `snake_case` discriminators, and `sessionUpdate` as the tag on
//! session updates. Every one is pinned by a test that asserts on the JSON
//! rather than on the Rust type, because a rename here is a silent
//! incompatibility with every editor on the other end — the compiler cannot
//! catch it and neither can a round-trip test that only talks to itself.

use serde::{Deserialize, Serialize};

/// The protocol revision this implementation speaks.
pub const PROTOCOL_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// initialize
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InitializeRequest {
    pub protocol_version: u32,
    pub client_capabilities: ClientCapabilities,
    pub client_info: Option<Implementation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientCapabilities {
    pub fs: FsCapabilities,
    pub terminal: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FsCapabilities {
    pub read_text_file: bool,
    pub write_text_file: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: u32,
    pub agent_capabilities: AgentCapabilities,
    pub agent_info: Implementation,
    /// Empty: this agent authenticates through the user's own machine, so
    /// there is nothing for a client to log into. An empty list is the
    /// protocol's way of saying that, and is not the same as omitting it.
    pub auth_methods: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentCapabilities {
    /// Resuming a stored session is not offered yet, and saying so is better
    /// than accepting `session/load` and returning something empty.
    pub load_session: bool,
    pub prompt_capabilities: PromptCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptCapabilities {
    pub image: bool,
    pub audio: bool,
    pub embedded_context: bool,
}

// ---------------------------------------------------------------------------
// session/new
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewSessionRequest {
    /// Absolute path. The protocol requires it, and a relative one would make
    /// every subsequent file operation mean something different to each side.
    pub cwd: String,
    pub mcp_servers: Vec<serde_json::Value>,
    pub additional_directories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResponse {
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// session/prompt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptRequest {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// One piece of a message. Only `text` is produced or consumed today; the
/// untagged catch-all keeps an image or audio block from failing the parse of
/// an otherwise-valid prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    #[serde(untagged)]
    Other(serde_json::Value),
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// The text this block contributes, or nothing.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: StopReason,
}

// ---------------------------------------------------------------------------
// session/cancel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CancelNotification {
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// session/update
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotification {
    pub session_id: String,
    pub update: SessionUpdate,
}

/// What the agent tells the client mid-turn.
///
/// Tagged on `sessionUpdate`, which is the protocol's own discriminator name —
/// not `type`, and not the Rust variant name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    #[serde(rename_all = "camelCase")]
    AgentMessageChunk { content: ContentBlock },
    #[serde(rename_all = "camelCase")]
    AgentThoughtChunk { content: ContentBlock },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        tool_call_id: String,
        title: String,
        kind: ToolKind,
        status: ToolStatus,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        content: Vec<ToolCallContent>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallContent {
    Content { content: ContentBlock },
}

impl ToolCallContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Content {
            content: ContentBlock::text(text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    Other,
}

impl ToolKind {
    /// Classify one of this agent's tools for the client's benefit.
    ///
    /// Editors use this to pick an icon and to decide how loudly to present a
    /// call, so a wrong answer is a misleading UI rather than a broken one —
    /// but `Execute` shown as `Read` would understate what is about to happen,
    /// which is why anything unrecognised falls to `Other` and not to `Read`.
    #[must_use]
    pub fn for_tool(name: &str) -> Self {
        match name {
            "Read" | "NotebookRead" => Self::Read,
            "Write" | "Edit" | "ApplyPatch" | "NotebookEdit" => Self::Edit,
            "Glob" | "Grep" | "SessionSearch" | "ToolSearch" => Self::Search,
            "Bash" | "PowerShell" | "Monitor" | "TerminalCreate" | "TerminalWrite" => Self::Execute,
            "WebFetch" | "WebSearch" => Self::Fetch,
            "Agent" | "TaskCreate" => Self::Think,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

// ---------------------------------------------------------------------------
// session/request_permission
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCallRef,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRef {
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionResponse {
    pub outcome: RequestPermissionOutcome,
}

/// Doubly tagged, and that is the protocol's shape rather than an accident:
/// the field is `outcome`, and the value inside it is another object whose own
/// `outcome` key is the discriminator.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RequestPermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Selected {
        option_id: String,
    },
    Cancelled,
}

impl RequestPermissionResponse {
    /// The option the user picked, or `None` if they dismissed the prompt.
    #[must_use]
    pub fn selected(&self) -> Option<&str> {
        match &self.outcome {
            RequestPermissionOutcome::Selected { option_id } => Some(option_id),
            RequestPermissionOutcome::Cancelled => None,
        }
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
