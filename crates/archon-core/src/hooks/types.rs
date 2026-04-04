use std::fmt;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// HookType — 21 event variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookType {
    Setup,
    SessionStart,
    SessionEnd,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PreCompact,
    PostCompact,
    ConfigChange,
    CwdChanged,
    FileChanged,
    InstructionsLoaded,
    UserPromptSubmit,
    Stop,
    SubagentStart,
    SubagentStop,
    TaskCreated,
    TaskCompleted,
    PermissionDenied,
    PermissionRequest,
    Notification,
}

impl fmt::Display for HookType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setup => write!(f, "setup"),
            Self::SessionStart => write!(f, "session_start"),
            Self::SessionEnd => write!(f, "session_end"),
            Self::PreToolUse => write!(f, "pre_tool_use"),
            Self::PostToolUse => write!(f, "post_tool_use"),
            Self::PostToolUseFailure => write!(f, "post_tool_use_failure"),
            Self::PreCompact => write!(f, "pre_compact"),
            Self::PostCompact => write!(f, "post_compact"),
            Self::ConfigChange => write!(f, "config_change"),
            Self::CwdChanged => write!(f, "cwd_changed"),
            Self::FileChanged => write!(f, "file_changed"),
            Self::InstructionsLoaded => write!(f, "instructions_loaded"),
            Self::UserPromptSubmit => write!(f, "user_prompt_submit"),
            Self::Stop => write!(f, "stop"),
            Self::SubagentStart => write!(f, "subagent_start"),
            Self::SubagentStop => write!(f, "subagent_stop"),
            Self::TaskCreated => write!(f, "task_created"),
            Self::TaskCompleted => write!(f, "task_completed"),
            Self::PermissionDenied => write!(f, "permission_denied"),
            Self::PermissionRequest => write!(f, "permission_request"),
            Self::Notification => write!(f, "notification"),
        }
    }
}

// ---------------------------------------------------------------------------
// HookConfig — describes a single registered hook
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    pub hook_type: HookType,
    pub command: String,
    /// Optional tool filter for pre_tool_use / post_tool_use hooks.
    pub tool: Option<String>,
    /// If true, the dispatcher waits for this hook to complete before proceeding.
    #[serde(default)]
    pub blocking: bool,
    /// Timeout in milliseconds. Default 10000 for blocking, 0 for non-blocking.
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    10000
}

// ---------------------------------------------------------------------------
// HookResult — JSON response from a blocking hook's stdout
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HookResult {
    /// For pre_tool_use: if false, block the tool execution.
    #[serde(default)]
    pub allow: Option<bool>,
    /// For user_prompt_submit: rewrite the prompt.
    #[serde(default)]
    pub modified_prompt: Option<String>,
    /// Human-readable reason (e.g., denial explanation).
    #[serde(default)]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// HookError
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("hook spawn failed: {0}")]
    SpawnError(String),

    #[error("hook timed out after {timeout_ms}ms: {command}")]
    Timeout { command: String, timeout_ms: u64 },

    #[error("hook exited with status {code}: {stderr}")]
    NonZeroExit { code: i32, stderr: String },

    #[error("hook I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("hook output parse error: {0}")]
    ParseError(String),

    #[error("hook config error: {0}")]
    ConfigError(String),
}
