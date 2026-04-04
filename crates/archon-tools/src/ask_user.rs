use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct AskUserTool;

#[async_trait::async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn description(&self) -> &str {
        "Ask the user a question. Returns their response as the tool result."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "choices": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional numbered choices for the user"
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let question = match input.get("question").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => return ToolResult::error("question is required and must be a string"),
        };

        let choices: Vec<String> = input
            .get("choices")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        // In the actual runtime, this will prompt the user via TUI.
        // For now, return a placeholder indicating the question was asked.
        // The agent loop will intercept AskUserQuestion tool calls and
        // route them to the TUI input handler.
        let mut prompt = question.to_string();
        if !choices.is_empty() {
            prompt.push('\n');
            for (i, choice) in choices.iter().enumerate() {
                prompt.push_str(&format!("  {}. {}\n", i + 1, choice));
            }
        }

        // This result will be replaced by the agent loop with actual user input.
        // The tool itself just validates the input and formats the question.
        ToolResult::success(format!("[PENDING_USER_INPUT]{prompt}"))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
