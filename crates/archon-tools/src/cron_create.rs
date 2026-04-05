//! CronCreate tool for TASK-CLI-311.

use std::path::PathBuf;

use serde_json::json;
use uuid::Uuid;

use crate::cron_scheduler::validate_cron_expression;
use crate::cron_task::{CronStore, CronTask};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that creates a new scheduled cron task.
pub struct CronCreateTool {
    project_dir: PathBuf,
}

impl CronCreateTool {
    /// Create the tool for the given project directory.
    ///
    /// Tasks are written to `<project_dir>/.claude/scheduled_tasks.json`.
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn store(&self) -> CronStore {
        CronStore::new(
            self.project_dir
                .join(".claude")
                .join("scheduled_tasks.json"),
        )
    }
}

#[async_trait::async_trait]
impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "CronCreate"
    }

    fn description(&self) -> &str {
        "Create a scheduled cron task. The task will run the given prompt on the specified \
         cron schedule. Recurring tasks (default) persist and re-fire each interval. \
         One-shot tasks (recurring: false) fire once then are deleted."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "cron": {
                    "type": "string",
                    "description": "5-field cron expression (minute hour day month weekday). Example: '0 9 * * 1' = every Monday 9am."
                },
                "prompt": {
                    "type": "string",
                    "description": "Agent prompt to execute when this task fires."
                },
                "name": {
                    "type": "string",
                    "description": "Optional human-readable name for the task (Archon extension, stored in metadata)."
                },
                "recurring": {
                    "type": "boolean",
                    "description": "true (default) = keep and re-fire; false = one-shot, delete after first fire."
                }
            },
            "required": ["cron", "prompt"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // Parse cron
        let cron = match input.get("cron").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c.to_owned(),
            _ => return ToolResult::error("CronCreate: 'cron' is required"),
        };

        // Validate expression
        if let Err(e) = validate_cron_expression(&cron) {
            return ToolResult::error(format!("CronCreate: invalid cron expression: {e}"));
        }

        // Parse prompt
        let prompt = match input.get("prompt").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p.to_owned(),
            _ => return ToolResult::error("CronCreate: 'prompt' is required"),
        };

        // Parse optional fields
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned());
        let recurring = input.get("recurring").and_then(|v| v.as_bool());

        // Build task
        let task = CronTask {
            id: Uuid::new_v4().to_string(),
            cron: cron.clone(),
            prompt,
            created_at: chrono::Utc::now().timestamp_millis() as u64,
            recurring,
        };

        let id = task.id.clone();
        let store = self.store();

        if let Err(e) = store.add_with_name(task, name.as_deref()) {
            return ToolResult::error(format!("CronCreate: failed to persist task: {e}"));
        }

        // Compute next fire time
        let next = crate::cron_scheduler::next_fire_time(&cron, chrono::Utc::now())
            .map(|t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        ToolResult::success(
            json!({
                "id": id,
                "cron": cron,
                "name": name,
                "recurring": recurring.unwrap_or(true),
                "next_fire": next
            })
            .to_string(),
        )
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}
