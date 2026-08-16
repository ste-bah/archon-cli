use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that retrieves detailed information about a task.
pub struct TaskGetTool;

#[async_trait::async_trait]
impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }

    fn description(&self) -> &str {
        "Get detailed information about a task by its ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID, the subagent ID it dispatched, or an unambiguous prefix of either"
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        match crate::task_manager::TASK_MANAGER.resolve_task(task_id) {
            Some(info) => {
                match serde_json::to_string_pretty(&crate::plan_tasks::task_info_json(&info, None))
                {
                    Ok(s) => ToolResult::success(s),
                    Err(e) => ToolResult::error(format!("failed to serialize task info: {e}")),
                }
            }
            None => ToolResult::error(format!("task not found: {task_id}")),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_tasks::{materialize_plan_tasks, test_plan_approval_authority};
    use crate::task_manager::{TASK_MANAGER, TaskManager};
    use crate::tool::Tool;

    fn linked_plan() -> archon_session::plan::PlanDocument {
        let plan_id = uuid::Uuid::new_v4().to_string();
        let mut plan = archon_session::plan::PlanDocument::new(&plan_id, "Task get plan");
        plan.status = archon_session::plan::PlanStatus::Approved;
        plan.approval = Some(archon_session::plan::PlanApproval {
            decision: archon_session::plan::PlanApprovalDecision::Approve,
            source: archon_session::plan::PlanApprovalSource::NonInteractive,
            decided_at: "2026-08-15T00:00:00Z".into(),
            user_edited: false,
        });
        plan.steps.push(archon_session::plan::PlanStep {
            number: 1,
            description: "linked".into(),
            affected_files: vec![],
            status: archon_session::plan::PlanStepStatus::Pending,
            blocked_by: vec![],
            required_evidence: vec![],
            task_id: None,
        });
        plan
    }

    #[test]
    fn plan_task_json_exposes_linkage_fields() {
        let manager = TaskManager::new();
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let store = archon_session::plan::PlanStore::new(&db).unwrap();
        let mut plan = linked_plan();
        let task_id = materialize_plan_tasks(
            &manager,
            &store,
            &test_plan_approval_authority(&store, "get-session"),
            "get-session",
            &mut plan,
        )
        .unwrap()
        .pop()
        .unwrap();
        let value = crate::plan_tasks::task_info_json(&manager.get_task(&task_id).unwrap(), None);
        assert_eq!(value["plan_id"], plan.id);
        assert_eq!(value["plan_step"], 1);
        assert!(value.get("blocked_by").is_some());
        assert!(value.get("required_evidence").is_some());
    }

    #[tokio::test]
    async fn runtime_tool_returns_plan_linkage_fields() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let store = archon_session::plan::PlanStore::new(&db).unwrap();
        let mut plan = linked_plan();
        let session_id = format!("get-runtime-{}", uuid::Uuid::new_v4());
        let task_id = materialize_plan_tasks(
            &TASK_MANAGER,
            &store,
            &test_plan_approval_authority(&store, &session_id),
            &session_id,
            &mut plan,
        )
        .unwrap()
        .pop()
        .unwrap();

        let result = TaskGetTool
            .execute(json!({"task_id": task_id}), &ToolContext::default())
            .await;
        assert!(!result.is_error, "{result:?}");
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["plan_id"], plan.id);
        assert_eq!(value["plan_step"], 1);
        assert_eq!(value["id"], task_id);
    }
}
