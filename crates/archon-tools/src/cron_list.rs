//! CronList tool for TASK-CLI-311.

use std::path::PathBuf;

use serde_json::json;

use crate::cron_task::CronStore;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that lists all scheduled cron tasks.
pub struct CronListTool {
    project_dir: PathBuf,
}

impl CronListTool {
    pub fn new(project_dir: PathBuf) -> Self {
        Self { project_dir }
    }

    fn store(&self) -> CronStore {
        let new_path = self.project_dir.join(".archon").join("scheduled_tasks.json");
        if new_path.exists() {
            return CronStore::new(new_path);
        }
        let old_path = self.project_dir.join(".claude").join("scheduled_tasks.json");
        if old_path.exists() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_path.display(),
                new_path.display()
            );
            return CronStore::new(old_path);
        }
        CronStore::new(new_path)
    }
}

#[async_trait::async_trait]
impl Tool for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }

    fn description(&self) -> &str {
        "List all scheduled cron tasks for this project. Shows id, cron expression, \
         next fire time, and recurring flag."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let store = self.store();
        let tasks: Vec<crate::cron_task::CronTask> = match store.load() {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("CronList: failed to load tasks: {e}")),
        };

        if tasks.is_empty() {
            return ToolResult::success("No scheduled tasks.");
        }

        let now = chrono::Utc::now();
        let mut lines = vec![format!("{} scheduled task(s):\n", tasks.len())];

        for t in &tasks {
            let next = crate::cron_scheduler::next_fire_time(&t.cron, now)
                .map(|dt: chrono::DateTime<chrono::Utc>| {
                    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                })
                .unwrap_or_else(|| "unknown".to_string());

            let recurring_label = match t.recurring {
                Some(false) => "one-shot",
                _ => "recurring",
            };

            let name = store
                .get_name(&t.id)
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();

            lines.push(format!(
                "  id:         {}\n  cron:       {}\n  prompt:     {}\n  next fire:  {}\n  type:       {}{}\n",
                t.id, t.cron, t.prompt, next, recurring_label, name
            ));
        }

        ToolResult::success(lines.join("\n"))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}
