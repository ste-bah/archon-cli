//! Thread-safe dynamic tool registry.
//!
//! Wraps tools in `Arc<RwLock<..>>` so they can be shared across async tasks
//! and modified at runtime (e.g., when MCP servers connect/disconnect).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::tool::Tool;

/// A thread-safe, dynamically mutable tool registry.
///
/// Tools can be registered and unregistered at runtime while concurrent
/// readers enumerate or invoke tools through shared references.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a tool. Replaces any existing tool with the same name.
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        let mut map = self.tools.write().await;
        map.insert(name, tool);
    }

    /// Remove a tool by name. Returns `true` if the tool existed.
    pub async fn unregister(&self, name: &str) -> bool {
        let mut map = self.tools.write().await;
        map.remove(name).is_some()
    }

    /// Retrieve a tool by name.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let map = self.tools.read().await;
        map.get(name).cloned()
    }

    /// List all registered tool names (unordered).
    pub async fn list(&self) -> Vec<String> {
        let map = self.tools.read().await;
        map.keys().cloned().collect()
    }

    /// Return JSON tool definitions suitable for inclusion in an API request.
    ///
    /// Each entry contains `name`, `description`, and `input_schema`.
    pub async fn tool_definitions(&self) -> Vec<serde_json::Value> {
        let map = self.tools.read().await;
        map.values()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "input_schema": tool.input_schema(),
                })
            })
            .collect()
    }

    /// Return the number of registered tools.
    pub async fn len(&self) -> usize {
        let map = self.tools.read().await;
        map.len()
    }

    /// Return `true` if no tools are registered.
    pub async fn is_empty(&self) -> bool {
        let map = self.tools.read().await;
        map.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{PermissionLevel, ToolContext, ToolResult};

    /// Minimal tool for testing.
    struct FakeTool {
        tool_name: String,
    }

    impl FakeTool {
        fn new(name: &str) -> Self {
            Self {
                tool_name: name.to_string(),
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            &self.tool_name
        }

        fn description(&self) -> &str {
            "A fake tool for testing"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::success("ok")
        }

        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Safe
        }
    }

    #[tokio::test]
    async fn register_and_get() {
        let registry = ToolRegistry::new();
        let tool = Arc::new(FakeTool::new("alpha"));
        registry.register(tool).await;

        let fetched = registry.get("alpha").await;
        assert!(fetched.is_some());
        assert_eq!(fetched.as_ref().map(|t| t.name()), Some("alpha"));
    }

    #[tokio::test]
    async fn unregister_removes_tool() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(FakeTool::new("beta"))).await;

        assert!(registry.unregister("beta").await);
        assert!(registry.get("beta").await.is_none());
        // Unregistering non-existent tool returns false.
        assert!(!registry.unregister("beta").await);
    }

    #[tokio::test]
    async fn list_returns_all_names() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(FakeTool::new("a"))).await;
        registry.register(Arc::new(FakeTool::new("b"))).await;
        registry.register(Arc::new(FakeTool::new("c"))).await;

        let mut names = registry.list().await;
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn tool_definitions_returns_json() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(FakeTool::new("x"))).await;

        let defs = registry.tool_definitions().await;
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0]["name"], "x");
        assert_eq!(defs[0]["description"], "A fake tool for testing");
        assert!(defs[0]["input_schema"].is_object());
    }

    #[tokio::test]
    async fn len_and_is_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty().await);
        assert_eq!(registry.len().await, 0);

        registry.register(Arc::new(FakeTool::new("z"))).await;
        assert!(!registry.is_empty().await);
        assert_eq!(registry.len().await, 1);
    }

    #[tokio::test]
    async fn register_replaces_existing() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(FakeTool::new("dup"))).await;
        registry.register(Arc::new(FakeTool::new("dup"))).await;
        assert_eq!(registry.len().await, 1);
    }
}
