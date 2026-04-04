use std::time::Duration;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Maximum allowed sleep duration in seconds.
const MAX_SLEEP_SECS: u64 = 300;

pub struct SleepTool;

#[async_trait::async_trait]
impl Tool for SleepTool {
    fn name(&self) -> &str {
        "Sleep"
    }

    fn description(&self) -> &str {
        "Pauses execution for the specified number of seconds (max 300)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "integer",
                    "description": "Number of seconds to sleep (0-300)"
                }
            },
            "required": ["seconds"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let seconds = match input.get("seconds").and_then(|v| v.as_u64()) {
            Some(s) => s,
            None => return ToolResult::error("seconds is required and must be a non-negative integer"),
        };

        if seconds > MAX_SLEEP_SECS {
            return ToolResult::error(format!(
                "seconds must be at most {MAX_SLEEP_SECS}, got {seconds}"
            ));
        }

        if seconds == 0 {
            return ToolResult::success("Slept for 0 seconds");
        }

        tokio::time::sleep(Duration::from_secs(seconds)).await;
        ToolResult::success(format!("Slept for {seconds} seconds"))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{AgentMode, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            mode: AgentMode::Normal,
        }
    }

    #[test]
    fn metadata() {
        let tool = SleepTool;
        assert_eq!(tool.name(), "Sleep");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "seconds");
    }

    #[test]
    fn permission_is_safe() {
        let tool = SleepTool;
        assert_eq!(
            tool.permission_level(&json!({"seconds": 10})),
            PermissionLevel::Safe
        );
    }

    #[tokio::test]
    async fn zero_is_noop() {
        let tool = SleepTool;
        let result = tool.execute(json!({"seconds": 0}), &test_ctx()).await;
        assert!(!result.is_error);
        assert_eq!(result.content, "Slept for 0 seconds");
    }

    #[tokio::test]
    async fn rejects_over_max() {
        let tool = SleepTool;
        let result = tool.execute(json!({"seconds": 301}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("at most 300"));
    }

    #[tokio::test]
    async fn missing_seconds_is_error() {
        let tool = SleepTool;
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("seconds is required"));
    }

    #[tokio::test]
    async fn small_sleep_succeeds() {
        let tool = SleepTool;
        let start = std::time::Instant::now();
        let result = tool.execute(json!({"seconds": 1}), &test_ctx()).await;
        let elapsed = start.elapsed();
        assert!(!result.is_error);
        assert_eq!(result.content, "Slept for 1 seconds");
        assert!(elapsed >= Duration::from_millis(900));
    }
}
