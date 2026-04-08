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
    Function,
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
    /// Static headers for HTTP hooks. Keys are header names, values are templates.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Env var names allowed for interpolation in header values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_env_vars: Vec<String>,
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
// HooksSettings — the `"hooks"` field in `.archon/settings.json`
// ---------------------------------------------------------------------------

pub type HooksSettings = HashMap<HookEvent, Vec<HookMatcher>>;

// ---------------------------------------------------------------------------
// HookOutcome — 4-variant outcome (maps to Claude Code's HookResult.outcome)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookOutcome {
    Success,
    Blocking,
    NonBlockingError,
    Cancelled,
}

// ---------------------------------------------------------------------------
// PermissionBehavior — 4-variant permission override
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PermissionBehavior {
    Allow,
    Deny,
    Ask,
    /// Let the normal permission flow decide (hook has no opinion).
    Passthrough,
}

// ---------------------------------------------------------------------------
// SourceAuthority — tracks where a hook was loaded from (merge precedence)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceAuthority {
    User,
    Project,
    Local,
    Policy,
}

// ---------------------------------------------------------------------------
// ElicitationAction — auto-respond action for Elicitation hooks (REQ-HOOK-019)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

// ---------------------------------------------------------------------------
// HookResult — full struct with modification support (REQ-HOOK-001)
//
// Replaces the former Allow/Block enum. Exit-code-only hooks produce
// HookResult::allow() or HookResult::block(reason) via convenience ctors.
// Hooks that emit JSON on stdout get the full field set.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResult {
    /// Outcome of hook execution.
    pub outcome: HookOutcome,
    /// Reason for block (displayed to user / logged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// System message shown as warning in TUI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,

    // --- PreToolUse fields ---
    /// Modified tool input JSON. Replaces original tool input before dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
    /// Permission behavior override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_behavior: Option<PermissionBehavior>,
    /// Reason for permission decision (for audit trail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,

    // --- PostToolUse fields ---
    /// Modified tool output. Replaces original tool output before LLM sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_mcp_tool_output: Option<serde_json::Value>,
    /// Additional context appended to tool output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,

    // --- Flow control ---
    /// If true, stop the conversation after this tool call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prevent_continuation: Option<bool>,
    /// Message shown when prevent_continuation is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// If true, re-execute the tool call (with possibly modified input).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<bool>,

    /// Status message for TUI display while hook runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,

    /// Source authority tag for merge precedence (REQ-HOOK-004a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_authority: Option<SourceAuthority>,

    /// Permission updates to apply (REQ-HOOK-016).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub updated_permissions: Vec<PermissionUpdate>,

    /// File paths to watch for changes (REQ-HOOK-017).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watch_paths: Vec<String>,

    /// Elicitation auto-respond action (REQ-HOOK-019).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elicitation_action: Option<ElicitationAction>,
    /// Elicitation content payload (REQ-HOOK-019).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elicitation_content: Option<serde_json::Value>,
}

impl Default for HookResult {
    fn default() -> Self {
        Self {
            outcome: HookOutcome::Success,
            reason: None,
            system_message: None,
            updated_input: None,
            permission_behavior: None,
            permission_decision_reason: None,
            updated_mcp_tool_output: None,
            additional_context: None,
            prevent_continuation: None,
            stop_reason: None,
            retry: None,
            status_message: None,
            source_authority: None,
            updated_permissions: Vec::new(),
            watch_paths: Vec::new(),
            elicitation_action: None,
            elicitation_content: None,
        }
    }
}

impl HookResult {
    /// Backward-compat constructor: equivalent to the old `HookResult::Allow`.
    pub fn allow() -> Self {
        Self::default()
    }

    /// Backward-compat constructor: equivalent to the old `HookResult::Block { reason }`.
    pub fn block(reason: String) -> Self {
        Self {
            outcome: HookOutcome::Blocking,
            reason: Some(reason),
            ..Default::default()
        }
    }

    /// Check if this result blocks execution.
    pub fn is_blocking(&self) -> bool {
        self.outcome == HookOutcome::Blocking
    }

    /// Check if this result allows execution.
    pub fn is_success(&self) -> bool {
        self.outcome == HookOutcome::Success
    }
}

// ---------------------------------------------------------------------------
// AggregatedHookResult — accumulated result from ALL matching hooks
// Reference: Claude Code types/hooks.ts:277-290
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct AggregatedHookResult {
    pub blocking_errors: Vec<String>,
    pub additional_contexts: Vec<String>,
    pub updated_input: Option<serde_json::Value>,
    pub updated_mcp_tool_output: Option<serde_json::Value>,
    pub permission_behavior: Option<PermissionBehavior>,
    pub permission_decision_reason: Option<String>,
    pub prevent_continuation: bool,
    pub stop_reason: Option<String>,
    pub retry: bool,
    pub system_messages: Vec<String>,
    pub status_messages: Vec<String>,
    pub updated_permissions: Vec<PermissionUpdate>,
    /// Collected watch paths from all hook results (REQ-HOOK-017).
    pub watch_paths: Vec<String>,
    /// Elicitation auto-respond action — last writer wins (REQ-HOOK-019).
    pub elicitation_action: Option<ElicitationAction>,
    /// Elicitation content payload — last writer wins (REQ-HOOK-019).
    pub elicitation_content: Option<serde_json::Value>,
    /// Number of hooks skipped due to aggregate timeout budget exhaustion.
    pub skipped_count: u32,
}

impl AggregatedHookResult {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge a single HookResult into this aggregate (REQ-HOOK-011).
    pub fn merge(&mut self, result: HookResult) {
        // outcome: any Blocking -> collect error
        if result.outcome == HookOutcome::Blocking {
            let reason = result
                .reason
                .clone()
                .unwrap_or_else(|| "hook blocked (no reason given)".to_owned());
            self.blocking_errors.push(reason);
        }

        // updated_input: last writer wins
        if result.updated_input.is_some() {
            self.updated_input = result.updated_input;
        }

        // updated_mcp_tool_output: last writer wins
        if result.updated_mcp_tool_output.is_some() {
            self.updated_mcp_tool_output = result.updated_mcp_tool_output;
        }

        // additional_context: collect all
        if let Some(ctx) = result.additional_context {
            self.additional_contexts.push(ctx);
        }

        // permission_behavior: policy wins; non-policy cannot Allow blocked tools (REQ-HOOK-004a)
        if let Some(ref pb) = result.permission_behavior {
            if *pb != PermissionBehavior::Passthrough {
                let is_policy = result.source_authority == Some(SourceAuthority::Policy);
                match pb {
                    PermissionBehavior::Allow if !is_policy => {
                        // Non-policy hook cannot grant Allow — silently dropped
                        tracing::warn!(
                            "non-policy hook attempted permission_behavior=allow; dropped"
                        );
                    }
                    _ => {
                        self.permission_behavior = Some(pb.clone());
                        if result.permission_decision_reason.is_some() {
                            self.permission_decision_reason =
                                result.permission_decision_reason.clone();
                        }
                    }
                }
            }
        }

        // prevent_continuation: any true wins
        if result.prevent_continuation == Some(true) {
            self.prevent_continuation = true;
            if result.stop_reason.is_some() {
                self.stop_reason = result.stop_reason;
            }
        }

        // retry: any true wins
        if result.retry == Some(true) {
            self.retry = true;
        }

        // system_message: collect all
        if let Some(msg) = result.system_message {
            self.system_messages.push(msg);
        }

        // status_message: collect all
        if let Some(msg) = result.status_message {
            self.status_messages.push(msg);
        }

        // updated_permissions: collect all (REQ-HOOK-016)
        if !result.updated_permissions.is_empty() {
            self.updated_permissions.extend(result.updated_permissions);
        }

        // watch_paths: collect all (REQ-HOOK-017)
        if !result.watch_paths.is_empty() {
            self.watch_paths.extend(result.watch_paths);
        }

        // elicitation_action: last writer wins (REQ-HOOK-019)
        if result.elicitation_action.is_some() {
            self.elicitation_action = result.elicitation_action;
        }
        // elicitation_content: last writer wins (REQ-HOOK-019)
        if result.elicitation_content.is_some() {
            self.elicitation_content = result.elicitation_content;
        }
    }

    /// Check if any hook blocked execution.
    pub fn is_blocked(&self) -> bool {
        !self.blocking_errors.is_empty()
    }

    /// Get combined block reason string.
    pub fn block_reason(&self) -> Option<String> {
        if self.blocking_errors.is_empty() {
            None
        } else {
            Some(self.blocking_errors.join("; "))
        }
    }
}

// ---------------------------------------------------------------------------
// PermissionUpdateDestination — where permission updates are persisted
// ---------------------------------------------------------------------------

/// Where permission updates are persisted (REQ-HOOK-016).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionUpdateDestination {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    Session,
}

// ---------------------------------------------------------------------------
// PermissionUpdate — 6 variants returned by PermissionRequest hooks
// ---------------------------------------------------------------------------

/// A single permission update returned by a PermissionRequest hook (REQ-HOOK-016).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PermissionUpdate {
    AddRules {
        destination: PermissionUpdateDestination,
        rules: Vec<String>,
    },
    ReplaceRules {
        destination: PermissionUpdateDestination,
        rules: Vec<String>,
    },
    RemoveRules {
        destination: PermissionUpdateDestination,
        rules: Vec<String>,
    },
    SetMode {
        destination: PermissionUpdateDestination,
        mode: String,
    },
    AddDirectories {
        destination: PermissionUpdateDestination,
        directories: Vec<String>,
    },
    RemoveDirectories {
        destination: PermissionUpdateDestination,
        directories: Vec<String>,
    },
}

impl PermissionUpdate {
    /// Get the destination for this update.
    pub fn destination(&self) -> &PermissionUpdateDestination {
        match self {
            Self::AddRules { destination, .. }
            | Self::ReplaceRules { destination, .. }
            | Self::RemoveRules { destination, .. }
            | Self::SetMode { destination, .. }
            | Self::AddDirectories { destination, .. }
            | Self::RemoveDirectories { destination, .. } => destination,
        }
    }
}

// ---------------------------------------------------------------------------
// HookError
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// HookExecutionConfig — aggregate timeout budget for hook execution
// ---------------------------------------------------------------------------

/// Configuration for hook execution behavior (e.g. aggregate timeout budget).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookExecutionConfig {
    /// Maximum total time in milliseconds for all hooks on a single event.
    /// Default: 30000 (30 seconds).
    pub aggregate_timeout_ms: u64,
}

impl Default for HookExecutionConfig {
    fn default() -> Self {
        Self {
            aggregate_timeout_ms: 30_000,
        }
    }
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
