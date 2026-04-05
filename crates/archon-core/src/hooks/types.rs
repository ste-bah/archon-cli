use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// HookEvent — 27 variants, PascalCase serialization (serde default)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Notification,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    StopFailure,
    SubagentStart,
    SubagentStop,
    PreCompact,
    PostCompact,
    PermissionRequest,
    PermissionDenied,
    Setup,
    TeammateIdle,
    TaskCreated,
    TaskCompleted,
    Elicitation,
    ElicitationResult,
    ConfigChange,
    WorktreeCreate,
    WorktreeRemove,
    InstructionsLoaded,
    CwdChanged,
    FileChanged,
}

/// Backward-compat alias — `HookType` and `HookEvent` are the same type.
pub type HookType = HookEvent;

impl fmt::Display for HookEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Variant names are PascalCase — Debug gives exactly that.
        fmt::Debug::fmt(self, f)
    }
}

// ---------------------------------------------------------------------------
// HookCommandType — 4 variants, lowercase serialization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookCommandType {
    Command,
    Prompt,
    Agent,
    Http,
}

// ---------------------------------------------------------------------------
// HookConfig — new field structure: exit-code semantics, no blocking, no priority
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// The hook executor variant (command / prompt / agent / http).
    #[serde(rename = "type")]
    pub hook_type: HookCommandType,
    /// Shell command or URL depending on hook_type.
    pub command: String,
    /// Optional condition expression, e.g. `"Bash(git *)"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_condition: Option<String>,
    /// Timeout in seconds. Default 60.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u32>,
    /// If true, the hook fires at most once per session and is then removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// If true, hook is spawned in background and Allow is returned immediately.
    #[serde(rename = "async", default, skip_serializing_if = "Option::is_none")]
    pub r#async: Option<bool>,
    /// If true together with async, re-wake the agent after background hook completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub async_rewake: Option<bool>,
    /// Human-readable status message shown while hook runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
}

// ---------------------------------------------------------------------------
// HookMatcher — groups hooks with an optional tool-name filter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcher {
    /// Optional tool-name pattern to filter which tools trigger these hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// The hooks to run when the matcher applies.
    pub hooks: Vec<HookConfig>,
}

// ---------------------------------------------------------------------------
// HooksSettings — the `"hooks"` field in `.claude/settings.json`
// ---------------------------------------------------------------------------

pub type HooksSettings = HashMap<HookEvent, Vec<HookMatcher>>;

// ---------------------------------------------------------------------------
// HookResult — exit-code semantics: 0=Allow, 2=Block, other=Allow(logged)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum HookResult {
    /// Tool execution is allowed to proceed.
    Allow,
    /// Tool execution is blocked; `reason` is the hook's stderr output.
    Block { reason: String },
}

// ---------------------------------------------------------------------------
// HookError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("hook spawn failed: {0}")]
    SpawnError(String),

    #[error("hook timed out after {timeout_secs}s: {command}")]
    Timeout { command: String, timeout_secs: u32 },

    #[error("hook I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("hook config error: {0}")]
    ConfigError(String),

    #[error("hook JSON parse error: {0}")]
    JsonError(String),
}
