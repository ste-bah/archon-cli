use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Global runtime overlay
// ---------------------------------------------------------------------------

static OVERLAY: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// ---------------------------------------------------------------------------
// Key metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyType {
    Str,
    U32,
    U64,
    U8,
    F32,
    F64,
}

struct KeyMeta {
    ty: KeyType,
    default: &'static str,
    read_only: bool,
}

fn key_registry() -> &'static HashMap<&'static str, KeyMeta> {
    static REGISTRY: LazyLock<HashMap<&'static str, KeyMeta>> = LazyLock::new(|| {
        let mut m = HashMap::new();
        m.insert(
            "api.default_model",
            KeyMeta {
                ty: KeyType::Str,
                default: "claude-sonnet-4-6",
                read_only: false,
            },
        );
        m.insert(
            "api.thinking_budget",
            KeyMeta {
                ty: KeyType::U32,
                default: "16384",
                read_only: false,
            },
        );
        m.insert(
            "api.default_effort",
            KeyMeta {
                ty: KeyType::Str,
                default: "high",
                read_only: false,
            },
        );
        m.insert(
            "context.compact_threshold",
            KeyMeta {
                ty: KeyType::F32,
                default: "0.80",
                read_only: false,
            },
        );
        m.insert(
            "context.preserve_recent_turns",
            KeyMeta {
                ty: KeyType::U32,
                default: "3",
                read_only: false,
            },
        );
        m.insert(
            "tools.bash_timeout",
            KeyMeta {
                ty: KeyType::U64,
                default: "120",
                read_only: false,
            },
        );
        m.insert(
            "tools.max_concurrency",
            KeyMeta {
                ty: KeyType::U8,
                default: "4",
                read_only: false,
            },
        );
        m.insert(
            "cost.warn_threshold",
            KeyMeta {
                ty: KeyType::F64,
                default: "5.0",
                read_only: false,
            },
        );
        m.insert(
            "cost.hard_limit",
            KeyMeta {
                ty: KeyType::F64,
                default: "0.0",
                read_only: false,
            },
        );
        m.insert(
            "checkpoint.max_checkpoints",
            KeyMeta {
                ty: KeyType::U32,
                default: "10",
                read_only: false,
            },
        );
        m
    });
    &REGISTRY
}

/// All known key names, for suggestion purposes.
pub fn all_keys() -> Vec<&'static str> {
    key_registry().keys().copied().collect()
}

/// Find the closest matching keys using segment-aware substring matching.
fn suggest_keys(unknown: &str) -> Vec<&'static str> {
    let parts: Vec<&str> = unknown.split('.').collect();
    let mut matches: Vec<&str> = all_keys()
        .into_iter()
        .filter(|k| {
            // Match if any part of the unknown key is a substring of the candidate
            // or vice versa.
            let kl = k.to_lowercase();
            parts.iter().any(|p| {
                let pl = p.to_lowercase();
                kl.contains(&pl) || pl.contains(&kl)
            })
        })
        .collect();
    matches.sort();
    matches.truncate(3);
    matches
}

/// Validate that `value` is parseable as `ty`.
fn validate_type(ty: KeyType, value: &str) -> Result<(), String> {
    match ty {
        KeyType::Str => Ok(()),
        KeyType::U32 => value
            .parse::<u32>()
            .map(|_| ())
            .map_err(|_| format!("expected u32, got \"{value}\"")),
        KeyType::U64 => value
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| format!("expected u64, got \"{value}\"")),
        KeyType::U8 => value
            .parse::<u8>()
            .map(|_| ())
            .map_err(|_| format!("expected u8 (0-255), got \"{value}\"")),
        KeyType::F32 => value
            .parse::<f32>()
            .map(|_| ())
            .map_err(|_| format!("expected f32, got \"{value}\"")),
        KeyType::F64 => value
            .parse::<f64>()
            .map(|_| ())
            .map_err(|_| format!("expected f64, got \"{value}\"")),
    }
}

// ---------------------------------------------------------------------------
// Public helpers for other crates to read overlay values
// ---------------------------------------------------------------------------

/// Read a runtime config value from the overlay, falling back to the default.
pub fn get_config_value(key: &str) -> Option<String> {
    let registry = key_registry();
    let meta = registry.get(key)?;
    let guard = match OVERLAY.lock() {
        Ok(g) => g,
        Err(_) => return None,
    };
    Some(
        guard
            .get(key)
            .cloned()
            .unwrap_or_else(|| meta.default.to_string()),
    )
}

// ---------------------------------------------------------------------------
// Tool implementation
// ---------------------------------------------------------------------------

pub struct ConfigTool;

#[async_trait::async_trait]
impl Tool for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> &str {
        "Get or set runtime configuration values. Changes are ephemeral \
         (session-scoped) and do not modify config files on disk."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["get", "set"],
                    "description": "Whether to read or write a config key"
                },
                "key": {
                    "type": "string",
                    "description": "Dotted config key, e.g. \"api.default_model\""
                },
                "value": {
                    "type": "string",
                    "description": "New value (required for set, ignored for get)"
                }
            },
            "required": ["action", "key"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::error("\"action\" is required and must be \"get\" or \"set\"");
            }
        };

        let key = match input.get("key").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => return ToolResult::error("\"key\" is required and must be a string"),
        };

        match action {
            "get" => self.handle_get(key),
            "set" => {
                let value = match input.get("value").and_then(|v| v.as_str()) {
                    Some(v) => v,
                    None => return ToolResult::error("\"value\" is required for set action"),
                };
                self.handle_set(key, value)
            }
            other => ToolResult::error(format!(
                "Unknown action \"{other}\". Must be \"get\" or \"set\"."
            )),
        }
    }

    fn permission_level(&self, input: &serde_json::Value) -> PermissionLevel {
        match input.get("action").and_then(|v| v.as_str()) {
            Some("get") => PermissionLevel::Safe,
            _ => PermissionLevel::Risky,
        }
    }
}

impl ConfigTool {
    fn handle_get(&self, key: &str) -> ToolResult {
        // personality.* namespace — read-only, no overlay
        if key.starts_with("personality.") {
            return ToolResult::success("personality.* keys are read-only and not available through the runtime overlay. \
                 Edit the config file or personality profile directly.".to_string());
        }

        let registry = key_registry();
        let Some(meta) = registry.get(key) else {
            let suggestions = suggest_keys(key);
            let hint = if suggestions.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}?", suggestions.join(", "))
            };
            return ToolResult::error(format!("Unknown config key \"{key}\".{hint}"));
        };

        let guard = match OVERLAY.lock() {
            Ok(g) => g,
            Err(_) => return ToolResult::error("Config overlay lock poisoned"),
        };
        let value = guard
            .get(key)
            .cloned()
            .unwrap_or_else(|| meta.default.to_string());
        let source = if guard.contains_key(key) {
            "overlay"
        } else {
            "default"
        };
        drop(guard);

        ToolResult::success(format!("{key} = \"{value}\" (source: {source})"))
    }

    fn handle_set(&self, key: &str, value: &str) -> ToolResult {
        // personality.* is read-only
        if key.starts_with("personality.") {
            return ToolResult::error(
                "personality.* keys are read-only. Edit the personality profile directly.",
            );
        }

        let registry = key_registry();
        let Some(meta) = registry.get(key) else {
            let suggestions = suggest_keys(key);
            let hint = if suggestions.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}?", suggestions.join(", "))
            };
            return ToolResult::error(format!("Unknown config key \"{key}\".{hint}"));
        };

        if meta.read_only {
            return ToolResult::error(format!("Config key \"{key}\" is read-only."));
        }

        if let Err(e) = validate_type(meta.ty, value) {
            return ToolResult::error(format!("Invalid value for \"{key}\": {e}"));
        }

        let mut guard = match OVERLAY.lock() {
            Ok(g) => g,
            Err(_) => return ToolResult::error("Config overlay lock poisoned"),
        };
        guard.insert(key.to_string(), value.to_string());
        drop(guard);

        ToolResult::success(format!("{key} set to \"{value}\""))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{AgentMode, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test-config".into(),
            mode: AgentMode::Normal,
            extra_dirs: vec![],
            ..Default::default()
        }
    }

    /// Clear the overlay between tests to avoid cross-contamination.
    fn clear_overlay() {
        let mut guard = OVERLAY.lock().expect("lock poisoned");
        guard.clear();
    }

    #[test]
    fn metadata() {
        let tool = ConfigTool;
        assert_eq!(tool.name(), "Config");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|v| v == "action"));
        assert!(required.iter().any(|v| v == "key"));
    }

    #[test]
    fn permission_safe_for_get_risky_for_set() {
        let tool = ConfigTool;
        assert_eq!(
            tool.permission_level(&json!({"action": "get", "key": "api.default_model"})),
            PermissionLevel::Safe,
        );
        assert_eq!(
            tool.permission_level(
                &json!({"action": "set", "key": "api.default_model", "value": "x"})
            ),
            PermissionLevel::Risky,
        );
    }

    #[tokio::test]
    async fn get_returns_default_when_no_overlay() {
        clear_overlay();
        let tool = ConfigTool;
        let result = tool
            .execute(
                json!({"action": "get", "key": "api.default_model"}),
                &test_ctx(),
            )
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("claude-sonnet-4-6"));
        assert!(result.content.contains("default"));
    }

    #[tokio::test]
    async fn set_then_get_returns_overlay_value() {
        clear_overlay();
        let tool = ConfigTool;
        let ctx = test_ctx();

        let set_res = tool
            .execute(
                json!({"action": "set", "key": "api.default_model", "value": "claude-opus-4-6"}),
                &ctx,
            )
            .await;
        assert!(!set_res.is_error, "set failed: {}", set_res.content);

        let get_res = tool
            .execute(json!({"action": "get", "key": "api.default_model"}), &ctx)
            .await;
        assert!(!get_res.is_error);
        assert!(get_res.content.contains("claude-opus-4-6"));
        assert!(get_res.content.contains("overlay"));
    }

    #[tokio::test]
    async fn set_numeric_rejects_non_numeric() {
        clear_overlay();
        let tool = ConfigTool;
        let result = tool
            .execute(
                json!({"action": "set", "key": "api.thinking_budget", "value": "not-a-number"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("expected u32"));
    }

    #[tokio::test]
    async fn personality_key_is_read_only() {
        let tool = ConfigTool;
        let result = tool
            .execute(
                json!({"action": "set", "key": "personality.name", "value": "evil"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("read-only"));
    }

    #[tokio::test]
    async fn unknown_key_suggests_alternatives() {
        let tool = ConfigTool;
        let result = tool
            .execute(json!({"action": "get", "key": "api.model"}), &test_ctx())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown config key"));
        // Should suggest api.default_model since "model" is a substring
        assert!(result.content.contains("api.default_model"));
    }

    #[tokio::test]
    async fn missing_action_returns_error() {
        let tool = ConfigTool;
        let result = tool
            .execute(json!({"key": "api.default_model"}), &test_ctx())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn set_without_value_returns_error() {
        let tool = ConfigTool;
        let result = tool
            .execute(
                json!({"action": "set", "key": "api.default_model"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("value"));
    }

    #[test]
    fn get_config_value_helper() {
        clear_overlay();
        assert_eq!(
            get_config_value("tools.bash_timeout"),
            Some("120".to_string()),
        );
        assert!(get_config_value("nonexistent.key").is_none());
    }
}
