use tracing::warn;

use crate::agents::definition::{AgentMemoryScope, AgentMeta, AgentQuality, PermissionMode};
use crate::hooks::{HookConfig, HookEvent};

pub(super) fn parse_memory_keys(
    json_str: &str,
    agent_name: &str,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Option<AgentMemoryScope>,
) {
    if json_str.trim().is_empty() {
        return (Vec::new(), Vec::new(), Vec::new(), None);
    }

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "malformed memory-keys.json, using defaults"
            );
            return (Vec::new(), Vec::new(), Vec::new(), None);
        }
    };

    let recall = extract_string_array(&parsed, "recall_queries");
    let leann = extract_string_array(&parsed, "leann_queries");
    let tags = extract_string_array(&parsed, "tags");

    // Parse optional memory_scope field (values: "user", "project", "local")
    let memory_scope = parsed
        .get("memory_scope")
        .and_then(|v| v.as_str())
        .and_then(|s| {
            serde_json::from_value::<AgentMemoryScope>(serde_json::Value::String(s.to_string()))
                .ok()
        });

    (recall, leann, tags, memory_scope)
}

fn extract_string_array(val: &serde_json::Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Meta.json parsing — handles both PRD schema and legacy on-disk schema
// ---------------------------------------------------------------------------

/// Execution config fields extracted from meta.json (not part of AgentMeta).
#[derive(Debug, Default)]
pub(super) struct ExecConfig {
    pub(super) model: Option<String>,
    pub(super) effort: Option<String>,
    pub(super) max_turns: Option<u32>,
    pub(super) permission_mode: Option<PermissionMode>,
    pub(super) background: bool,
    pub(super) initial_prompt: Option<String>,
    pub(super) color: Option<String>,
    pub(super) disallowed_tools: Option<Vec<String>>,
    pub(super) isolation: Option<String>,
    pub(super) mcp_servers: Option<Vec<String>>,
    pub(super) required_mcp_servers: Option<Vec<String>>,
    pub(super) hooks: Option<serde_json::Value>,
    pub(super) omit_claude_md: bool,
    pub(super) skills: Option<Vec<String>>,
    pub(super) critical_system_reminder: Option<String>,
}

pub(super) fn parse_meta_json(json_str: &str, agent_name: &str) -> (AgentMeta, ExecConfig) {
    if json_str.trim().is_empty() {
        return (AgentMeta::default(), ExecConfig::default());
    }

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "malformed meta.json, using defaults"
            );
            return (AgentMeta::default(), ExecConfig::default());
        }
    };

    // Try PRD schema first (has "version" as string, "created_at", "updated_at")
    // Fallback to legacy schema (has "version" as int, "created", "last_used")
    let meta = match serde_json::from_value::<AgentMeta>(parsed.clone()) {
        Ok(m) => m,
        Err(_) => {
            // Legacy format — extract what we can
            let mut m = AgentMeta::default();
            if let Some(v) = parsed.get("version") {
                if let Some(n) = v.as_u64() {
                    m.version = format!("{n}.0");
                } else if let Some(s) = v.as_str() {
                    m.version = s.to_string();
                }
            }
            if let Some(n) = parsed.get("invocation_count").and_then(|v| v.as_u64()) {
                m.invocation_count = n;
            }
            if let Some(q) = parsed.get("quality") {
                m.quality = AgentQuality {
                    applied_rate: q
                        .get("applied_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    completion_rate: q
                        .get("completion_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                };
            }
            if let Some(archived) = parsed.get("archived").and_then(|v| v.as_bool()) {
                m.archived = archived;
            }
            m
        }
    };

    // Extract execution config fields (same key names in both schemas)
    let exec = ExecConfig {
        model: parsed
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from),
        effort: parsed
            .get("effort")
            .and_then(|v| v.as_str())
            .map(String::from),
        max_turns: parsed
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        permission_mode: parsed
            .get("permission_mode")
            .and_then(|v| v.as_str())
            .and_then(PermissionMode::from_str_opt),
        background: parsed
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        initial_prompt: parsed
            .get("initial_prompt")
            .and_then(|v| v.as_str())
            .map(String::from),
        color: parsed
            .get("color")
            .and_then(|v| v.as_str())
            .map(String::from),
        disallowed_tools: parsed.get("disallowed_tools").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        isolation: parsed
            .get("isolation")
            .and_then(|v| v.as_str())
            .map(String::from),
        mcp_servers: parsed.get("mcp_servers").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        required_mcp_servers: parsed.get("required_mcp_servers").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        hooks: parsed.get("hooks").cloned(),
        omit_claude_md: parsed
            .get("omit_claude_md")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        skills: parsed.get("skills").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        critical_system_reminder: parsed
            .get("critical_system_reminder")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
    };

    (meta, exec)
}

// ---------------------------------------------------------------------------
// Agent hooks parsing (AGT-019)
// ---------------------------------------------------------------------------

/// Parse agent hooks JSON into `(HookEvent, HookConfig)` pairs.
///
/// Expected format (same as Claude Code frontmatter hooks):
/// ```json
/// {
///   "PreToolUse": [{ "type": "command", "command": "check.sh" }],
///   "SessionStart": [{ "type": "command", "command": "setup.sh" }]
/// }
/// ```
///
/// Returns an error if the JSON is not an object, contains unknown event names,
/// or has malformed hook configs.
pub fn parse_agent_hooks(json: &serde_json::Value) -> Result<Vec<(HookEvent, HookConfig)>, String> {
    let map = json
        .as_object()
        .ok_or_else(|| "hooks must be a JSON object".to_string())?;

    let mut result = Vec::new();
    for (event_name, configs_val) in map {
        let event: HookEvent =
            serde_json::from_value(serde_json::Value::String(event_name.clone()))
                .map_err(|e| format!("unknown hook event '{event_name}': {e}"))?;

        let configs: Vec<HookConfig> = serde_json::from_value(configs_val.clone())
            .map_err(|e| format!("malformed hook configs for '{event_name}': {e}"))?;

        for config in configs {
            result.push((event.clone(), config));
        }
    }
    Ok(result)
}
