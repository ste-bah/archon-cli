use archon_core::dispatch::ToolRegistry;
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

struct TestTool {
    name: &'static str,
    succeeds: bool,
}

#[async_trait::async_trait]
impl Tool for TestTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "duplicate registration test tool"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        if self.succeeds {
            ToolResult::success("first")
        } else {
            ToolResult::error("second")
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[tokio::test]
async fn duplicate_tool_registration_keeps_first_tool() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(TestTool {
        name: "DuplicateTool",
        succeeds: true,
    }));
    registry.register(Box::new(TestTool {
        name: "DuplicateTool",
        succeeds: false,
    }));

    let result = registry
        .dispatch(
            "DuplicateTool",
            serde_json::json!({}),
            &ToolContext::default(),
        )
        .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "first");
}
