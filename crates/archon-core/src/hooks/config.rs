use serde::Deserialize;

use super::types::{HookConfig, HookError, HookType};

// ---------------------------------------------------------------------------
// TOML entry config (per-hook entry in the hooks section)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct HookEntryConfig {
    pub command: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
}

fn default_hook_timeout() -> u64 {
    10000
}

// ---------------------------------------------------------------------------
// Hooks TOML section — one field per hook type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct HooksTomlConfig {
    pub setup: Vec<HookEntryConfig>,
    pub session_start: Vec<HookEntryConfig>,
    pub session_end: Vec<HookEntryConfig>,
    pub pre_tool_use: Vec<HookEntryConfig>,
    pub post_tool_use: Vec<HookEntryConfig>,
    pub post_tool_use_failure: Vec<HookEntryConfig>,
    pub pre_compact: Vec<HookEntryConfig>,
    pub post_compact: Vec<HookEntryConfig>,
    pub config_change: Vec<HookEntryConfig>,
    pub cwd_changed: Vec<HookEntryConfig>,
    pub file_changed: Vec<HookEntryConfig>,
    pub instructions_loaded: Vec<HookEntryConfig>,
    pub user_prompt_submit: Vec<HookEntryConfig>,
    pub stop: Vec<HookEntryConfig>,
    pub subagent_start: Vec<HookEntryConfig>,
    pub subagent_stop: Vec<HookEntryConfig>,
    pub task_created: Vec<HookEntryConfig>,
    pub task_completed: Vec<HookEntryConfig>,
    pub permission_denied: Vec<HookEntryConfig>,
    pub permission_request: Vec<HookEntryConfig>,
    pub notification: Vec<HookEntryConfig>,
}

impl HooksTomlConfig {
    /// Convert all entries into a flat list of `HookConfig`.
    fn into_hook_configs(self) -> Vec<HookConfig> {
        let mut out = Vec::new();

        let pairs: Vec<(HookType, Vec<HookEntryConfig>)> = vec![
            (HookType::Setup, self.setup),
            (HookType::SessionStart, self.session_start),
            (HookType::SessionEnd, self.session_end),
            (HookType::PreToolUse, self.pre_tool_use),
            (HookType::PostToolUse, self.post_tool_use),
            (HookType::PostToolUseFailure, self.post_tool_use_failure),
            (HookType::PreCompact, self.pre_compact),
            (HookType::PostCompact, self.post_compact),
            (HookType::ConfigChange, self.config_change),
            (HookType::CwdChanged, self.cwd_changed),
            (HookType::FileChanged, self.file_changed),
            (HookType::InstructionsLoaded, self.instructions_loaded),
            (HookType::UserPromptSubmit, self.user_prompt_submit),
            (HookType::Stop, self.stop),
            (HookType::SubagentStart, self.subagent_start),
            (HookType::SubagentStop, self.subagent_stop),
            (HookType::TaskCreated, self.task_created),
            (HookType::TaskCompleted, self.task_completed),
            (HookType::PermissionDenied, self.permission_denied),
            (HookType::PermissionRequest, self.permission_request),
            (HookType::Notification, self.notification),
        ];

        for (hook_type, entries) in pairs {
            for entry in entries {
                out.push(HookConfig {
                    hook_type: hook_type.clone(),
                    command: entry.command,
                    tool: entry.tool,
                    blocking: entry.blocking,
                    timeout_ms: entry.timeout,
                });
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Public parsing functions
// ---------------------------------------------------------------------------

/// Parse hook configs from a TOML string representing the `[hooks]` section.
///
/// The TOML should contain arrays of tables keyed by hook type name, e.g.:
/// ```toml
/// [[session_start]]
/// command = "echo hello"
/// blocking = true
/// ```
pub fn parse_hooks_from_toml(toml_str: &str) -> Result<Vec<HookConfig>, HookError> {
    if toml_str.trim().is_empty() {
        return Ok(Vec::new());
    }

    let config: HooksTomlConfig = toml::from_str(toml_str)
        .map_err(|e| HookError::ConfigError(format!("failed to parse hooks TOML: {e}")))?;

    Ok(config.into_hook_configs())
}
