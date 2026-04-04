use std::fmt;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Maximum number of todo items allowed.
const MAX_ITEMS: usize = 100;

/// Valid status values for a todo item.
const VALID_STATUSES: &[&str] = &["pending", "in_progress", "done"];

/// Global todo storage, thread-safe.
static TODO_STORE: LazyLock<Mutex<Vec<TodoItem>>> = LazyLock::new(|| Mutex::new(Vec::new()));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: String,
}

impl fmt::Display for TodoItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let icon = match self.status.as_str() {
            "pending" => "[ ]",
            "in_progress" => "[~]",
            "done" => "[x]",
            _ => "[?]",
        };
        write!(f, "{} {} - {}", icon, self.id, self.content)
    }
}

pub struct TodoWriteTool;

#[async_trait::async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Writes/overwrites the full todo list. Replaces all existing items with the provided list. \
         Send an empty array to clear. Maximum 100 items."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete todo list (replaces all existing items)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier for the todo item"
                            },
                            "content": {
                                "type": "string",
                                "description": "Description of the todo item"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "done"],
                                "description": "Current status of the todo item"
                            }
                        },
                        "required": ["id", "content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let todos_val = match input.get("todos") {
            Some(v) => v,
            None => return ToolResult::error("'todos' field is required"),
        };

        let raw_items: Vec<serde_json::Value> = match todos_val.as_array() {
            Some(arr) => arr.clone(),
            None => return ToolResult::error("'todos' must be an array"),
        };

        if raw_items.len() > MAX_ITEMS {
            return ToolResult::error(format!(
                "Too many items: got {}, maximum is {MAX_ITEMS}",
                raw_items.len()
            ));
        }

        let mut items = Vec::with_capacity(raw_items.len());
        for (i, raw) in raw_items.iter().enumerate() {
            let id = match raw.get("id").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    return ToolResult::error(format!(
                        "Item at index {i}: 'id' is required and must be a string"
                    ));
                }
            };

            let content = match raw.get("content").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    return ToolResult::error(format!(
                        "Item at index {i}: 'content' is required and must be a string"
                    ));
                }
            };

            let status = match raw.get("status").and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    return ToolResult::error(format!(
                        "Item at index {i}: 'status' is required and must be a string"
                    ));
                }
            };

            if !VALID_STATUSES.contains(&status.as_str()) {
                return ToolResult::error(format!(
                    "Item at index {i}: invalid status '{status}'. Must be one of: {}",
                    VALID_STATUSES.join(", ")
                ));
            }

            items.push(TodoItem {
                id,
                content,
                status,
            });
        }

        // Replace the entire list
        let store = match TODO_STORE.lock() {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::error(format!("Failed to acquire lock: {e}"));
            }
        };
        let mut store = store;
        *store = items.clone();

        if store.is_empty() {
            return ToolResult::success("Todo list cleared (0 items)");
        }

        let mut output = format!("Todo list updated ({} item{}):\n", store.len(), if store.len() == 1 { "" } else { "s" });
        for item in store.iter() {
            output.push_str(&format!("  {item}\n"));
        }

        ToolResult::success(output.trim_end().to_string())
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Read the current todo list (for use by other tools or tests).
pub fn read_todos() -> Vec<TodoItem> {
    TODO_STORE
        .lock()
        .map(|store| store.clone())
        .unwrap_or_default()
}

/// Clear the global store (useful for test isolation).
#[cfg(test)]
fn clear_store() {
    if let Ok(mut store) = TODO_STORE.lock() {
        store.clear();
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
        let tool = TodoWriteTool;
        assert_eq!(tool.name(), "TodoWrite");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "todos");
        assert_eq!(schema["properties"]["todos"]["type"], "array");
    }

    #[test]
    fn permission_is_safe() {
        let tool = TodoWriteTool;
        assert_eq!(
            tool.permission_level(&json!({})),
            PermissionLevel::Safe
        );
    }

    #[tokio::test]
    async fn write_and_read_todos() {
        clear_store();
        let tool = TodoWriteTool;
        let input = json!({
            "todos": [
                { "id": "1", "content": "First task", "status": "pending" },
                { "id": "2", "content": "Second task", "status": "done" }
            ]
        });

        let result = tool.execute(input, &test_ctx()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(result.content.contains("2 items"));
        assert!(result.content.contains("First task"));
        assert!(result.content.contains("[x] 2"));

        let items = read_todos();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "1");
        assert_eq!(items[1].status, "done");
    }

    #[tokio::test]
    async fn empty_todos_clears_list() {
        clear_store();
        let tool = TodoWriteTool;

        // Write something first
        let write_result = tool.execute(json!({
            "todos": [{ "id": "1", "content": "task", "status": "pending" }]
        }), &test_ctx()).await;
        assert!(!write_result.is_error);

        // Now clear
        let result = tool.execute(json!({ "todos": [] }), &test_ctx()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("cleared"));
        assert!(result.content.contains("0 items"));
    }

    #[tokio::test]
    async fn rejects_invalid_status() {
        clear_store();
        let tool = TodoWriteTool;
        let result = tool.execute(json!({
            "todos": [{ "id": "1", "content": "task", "status": "invalid" }]
        }), &test_ctx()).await;

        assert!(result.is_error);
        assert!(result.content.contains("invalid status"));
        assert!(result.content.contains("'invalid'"));
    }

    #[tokio::test]
    async fn rejects_over_max_items() {
        clear_store();
        let tool = TodoWriteTool;
        let items: Vec<serde_json::Value> = (0..101)
            .map(|i| json!({ "id": format!("{i}"), "content": "task", "status": "pending" }))
            .collect();

        let result = tool.execute(json!({ "todos": items }), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("Too many items"));
        assert!(result.content.contains("101"));
    }

    #[tokio::test]
    async fn missing_todos_field_is_error() {
        let tool = TodoWriteTool;
        let result = tool.execute(json!({}), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("'todos' field is required"));
    }

    #[tokio::test]
    async fn missing_item_fields_are_errors() {
        clear_store();
        let tool = TodoWriteTool;

        // Missing id
        let result = tool.execute(json!({
            "todos": [{ "content": "task", "status": "pending" }]
        }), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("'id' is required"));

        // Missing content
        let result = tool.execute(json!({
            "todos": [{ "id": "1", "status": "pending" }]
        }), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("'content' is required"));

        // Missing status
        let result = tool.execute(json!({
            "todos": [{ "id": "1", "content": "task" }]
        }), &test_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("'status' is required"));
    }

    #[tokio::test]
    async fn overwrites_previous_list() {
        clear_store();
        let tool = TodoWriteTool;

        // Write initial list
        let _ = tool.execute(json!({
            "todos": [
                { "id": "a", "content": "old task", "status": "pending" }
            ]
        }), &test_ctx()).await;

        // Overwrite with new list
        let result = tool.execute(json!({
            "todos": [
                { "id": "b", "content": "new task", "status": "in_progress" }
            ]
        }), &test_ctx()).await;

        assert!(!result.is_error);
        let items = read_todos();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "b");
        assert_eq!(items[0].status, "in_progress");
    }

    #[tokio::test]
    async fn in_progress_status_accepted() {
        clear_store();
        let tool = TodoWriteTool;
        let result = tool.execute(json!({
            "todos": [{ "id": "1", "content": "wip", "status": "in_progress" }]
        }), &test_ctx()).await;

        assert!(!result.is_error);
        assert!(result.content.contains("[~]"));
    }
}
