use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tool that lists all tracked tasks.
pub struct TaskListTool;

#[async_trait::async_trait]
impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }

    fn description(&self) -> &str {
        "List all tracked tasks with their current status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let summaries = crate::plan_tasks::task_list_json(&crate::task_manager::TASK_MANAGER);

        match serde_json::to_string_pretty(&summaries) {
            Ok(s) => ToolResult::success(s),
            Err(e) => ToolResult::error(format!("failed to serialize task list: {e}")),
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
    use crate::task_manager::TASK_MANAGER;
    use crate::tool::Tool;

    #[tokio::test]
    async fn runtime_tool_exposes_linkage_and_plan_progress() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let store = archon_session::plan::PlanStore::new(&db).unwrap();
        let plan_id = uuid::Uuid::new_v4().to_string();
        let mut plan = archon_session::plan::PlanDocument::new(&plan_id, "Task list plan");
        plan.status = archon_session::plan::PlanStatus::Approved;
        plan.approval = Some(archon_session::plan::PlanApproval {
            decision: archon_session::plan::PlanApprovalDecision::Approve,
            source: archon_session::plan::PlanApprovalSource::NonInteractive,
            decided_at: "2026-08-15T00:00:00Z".into(),
            user_edited: false,
        });
        plan.steps = (1..=2)
            .map(|number| archon_session::plan::PlanStep {
                number,
                description: format!("step {number}"),
                affected_files: vec![],
                status: archon_session::plan::PlanStepStatus::Pending,
                blocked_by: if number == 1 { vec![] } else { vec![1] },
                required_evidence: vec![],
                task_id: None,
            })
            .collect();
        let session_id = format!("list-runtime-{}", uuid::Uuid::new_v4());
        let ids = materialize_plan_tasks(
            &TASK_MANAGER,
            &store,
            &test_plan_approval_authority(&store, &session_id),
            &session_id,
            &mut plan,
        )
        .unwrap();

        let result = TaskListTool
            .execute(json!({}), &ToolContext::default())
            .await;
        assert!(!result.is_error, "{result:?}");
        let tasks: Vec<serde_json::Value> = serde_json::from_str(&result.content).unwrap();
        let linked = tasks
            .into_iter()
            .find(|task| task["id"] == ids[1])
            .expect("linked task in runtime task list");
        assert_eq!(linked["plan_id"], plan_id);
        assert_eq!(linked["plan_step"], 2);
        assert_eq!(linked["blocked_by"], json!([ids[0]]));
        assert_eq!(linked["plan_progress"], json!({"completed": 0, "total": 2}));
    }
}
