//! TASK-P0-B.4 Skill tool — LLM-facing wrapper around [`SkillRegistry`].
//!
//! Exposes built-in skills (same set that /skills enumerates in the TUI)
//! to LLM tool_use blocks. Two actions:
//!   * `list` — returns JSON array of `{name, description}` for every
//!     registered skill.
//!   * `invoke` — looks up a skill by canonical name or alias, runs it
//!     with the supplied args, and returns the resulting text. Requires
//!     `name`; `args` defaults to empty.
//!
//! `SkillContext` is built from the `ToolContext`: session_id and
//! working_dir are threaded through verbatim; `model` is empty string
//! (skills that require it should handle that); `agent_registry` is
//! `None` — the Tool path does not have access. Skills that depend on
//! the agent registry (e.g. `/create-agent`) will gracefully fail with
//! an error string rather than panicking.

use serde_json::{json, Value};

use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

use super::{builtin, SkillContext, SkillOutput};

pub struct SkillTool;

#[async_trait::async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "Enumerate or invoke a built-in skill. Use action=\"list\" to \
         discover available skills. Use action=\"invoke\" with `name` \
         (canonical or alias) and optional `args` (array of strings) to \
         run one. Each skill returns text, markdown, or a prompt to \
         inject into the conversation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "invoke"],
                    "description": "`list` to enumerate skills; `invoke` to run one."
                },
                "name": {
                    "type": "string",
                    "description": "Skill name or alias (required when action=invoke)."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Positional arguments passed to the skill (action=invoke only)."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::error("missing required field: action"),
        };

        let registry = builtin::register_builtins();

        match action {
            "list" => {
                let skills: Vec<Value> = registry
                    .list_all()
                    .into_iter()
                    .map(|(name, desc)| json!({ "name": name, "description": desc }))
                    .collect();
                match serde_json::to_string_pretty(&skills) {
                    Ok(s) => ToolResult::success(s),
                    Err(e) => ToolResult::error(format!("failed to serialize skills: {e}")),
                }
            }
            "invoke" => {
                let name = match input.get("name").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => s,
                    _ => return ToolResult::error("action=invoke requires non-empty `name`"),
                };
                let args: Vec<String> = input
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();

                let Some(skill) = registry.resolve(name) else {
                    return ToolResult::error(format!("skill '{name}' not found"));
                };

                let skill_ctx = SkillContext {
                    session_id: ctx.session_id.clone(),
                    working_dir: ctx.working_dir.clone(),
                    model: String::new(),
                    agent_registry: None,
                };

                match skill.execute(&args, &skill_ctx) {
                    SkillOutput::Text(s) | SkillOutput::Markdown(s) | SkillOutput::Prompt(s) => {
                        ToolResult::success(s)
                    }
                    SkillOutput::Error(e) => ToolResult::error(e),
                }
            }
            other => ToolResult::error(format!(
                "action must be 'list' or 'invoke', got '{other}'"
            )),
        }
    }

    fn permission_level(&self, _input: &Value) -> PermissionLevel {
        // The Skill tool itself is Safe (read-only listing + dispatch). The
        // dispatched skill may have side effects but is classified
        // independently via the slash-command path where side-effecting
        // skills use their own approval flows.
        PermissionLevel::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: std::path::PathBuf::from("."),
            session_id: "test-session".to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn skill_tool_lists_builtins() {
        let tool = SkillTool;
        let result = tool
            .execute(json!({ "action": "list" }), &ctx())
            .await;
        assert!(!result.is_error);
        // The output is a JSON array of {name, description} objects.
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        let arr = parsed.as_array().expect("output must be array");
        assert!(!arr.is_empty(), "register_builtins must yield skills");
        for entry in arr {
            assert!(entry.get("name").is_some());
            assert!(entry.get("description").is_some());
        }
    }

    #[tokio::test]
    async fn skill_tool_invoke_requires_name() {
        let tool = SkillTool;
        let result = tool
            .execute(json!({ "action": "invoke" }), &ctx())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("name"));
    }

    #[tokio::test]
    async fn skill_tool_invoke_unknown_returns_error() {
        let tool = SkillTool;
        let result = tool
            .execute(
                json!({ "action": "invoke", "name": "definitely-not-a-skill-xyz" }),
                &ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn skill_tool_rejects_bad_action() {
        let tool = SkillTool;
        let result = tool
            .execute(json!({ "action": "delete" }), &ctx())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("action"));
    }

    #[tokio::test]
    async fn skill_tool_rejects_missing_action() {
        let tool = SkillTool;
        let result = tool.execute(json!({}), &ctx()).await;
        assert!(result.is_error);
    }

    #[test]
    fn skill_tool_schema_is_object() {
        let schema = SkillTool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["required"].is_array());
    }
}
