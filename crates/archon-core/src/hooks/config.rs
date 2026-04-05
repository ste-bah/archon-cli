/// Legacy TOML hooks config loader.
///
/// Parses the old `hooks.toml` format and converts entries to the new
/// `HookRegistry` representation.  New code should load from
/// `.claude/settings.json` via `HookRegistry::load_from_settings_json`.
use serde::Deserialize;

use super::registry::HookRegistry;
use super::types::{HookCommandType, HookConfig, HookError, HookEvent, HookMatcher};

// ---------------------------------------------------------------------------
// TOML entry (per-hook)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct HookEntryConfig {
    pub command: String,
    /// Ignored — exit code 2 now determines blocking behaviour.
    #[allow(dead_code)]
    #[serde(default)]
    pub blocking: bool,
    /// Timeout in seconds (TOML used milliseconds historically; we reinterpret
    /// as seconds to match the new HookConfig.timeout field).
    #[serde(default = "default_timeout_secs")]
    pub timeout: u32,
}

fn default_timeout_secs() -> u32 {
    60
}

// ---------------------------------------------------------------------------
// TOML section — one field per event type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct HooksTomlConfig {
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

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn to_matchers(entries: Vec<HookEntryConfig>) -> Vec<HookMatcher> {
    entries
        .into_iter()
        .map(|e| HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: e.command,
                if_condition: None,
                timeout: Some(e.timeout),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
            }],
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse hook configs from a TOML string (old `hooks.toml` format) and
/// return a populated `HookRegistry`.
pub fn parse_hooks_from_toml(toml_str: &str) -> Result<HookRegistry, HookError> {
    if toml_str.trim().is_empty() {
        return Ok(HookRegistry::new());
    }

    let cfg: HooksTomlConfig = toml::from_str(toml_str)
        .map_err(|e| HookError::ConfigError(format!("failed to parse hooks TOML: {e}")))?;

    let mut registry = HookRegistry::new();

    let pairs: Vec<(HookEvent, Vec<HookEntryConfig>)> = vec![
        (HookEvent::Setup, cfg.setup),
        (HookEvent::SessionStart, cfg.session_start),
        (HookEvent::SessionEnd, cfg.session_end),
        (HookEvent::PreToolUse, cfg.pre_tool_use),
        (HookEvent::PostToolUse, cfg.post_tool_use),
        (HookEvent::PostToolUseFailure, cfg.post_tool_use_failure),
        (HookEvent::PreCompact, cfg.pre_compact),
        (HookEvent::PostCompact, cfg.post_compact),
        (HookEvent::ConfigChange, cfg.config_change),
        (HookEvent::CwdChanged, cfg.cwd_changed),
        (HookEvent::FileChanged, cfg.file_changed),
        (HookEvent::InstructionsLoaded, cfg.instructions_loaded),
        (HookEvent::UserPromptSubmit, cfg.user_prompt_submit),
        (HookEvent::Stop, cfg.stop),
        (HookEvent::SubagentStart, cfg.subagent_start),
        (HookEvent::SubagentStop, cfg.subagent_stop),
        (HookEvent::TaskCreated, cfg.task_created),
        (HookEvent::TaskCompleted, cfg.task_completed),
        (HookEvent::PermissionDenied, cfg.permission_denied),
        (HookEvent::PermissionRequest, cfg.permission_request),
        (HookEvent::Notification, cfg.notification),
    ];

    for (event, entries) in pairs {
        if !entries.is_empty() {
            registry.register_matchers(event, to_matchers(entries), None);
        }
    }

    Ok(registry)
}
