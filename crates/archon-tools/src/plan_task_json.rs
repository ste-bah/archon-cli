use serde_json::json;

use crate::task_manager::{TaskInfo, TaskManager, TaskStatus};

pub fn task_info_json(task: &TaskInfo, plan_progress: Option<(usize, usize)>) -> serde_json::Value {
    let mut value = json!({
        "id": task.id,
        "description": task.description,
        "status": task.status,
        "created_at": task.created_at.to_rfc3339(),
        "completed_at": task.completed_at.map(|time| time.to_rfc3339()),
        "output": task.output,
        "cost": task.cost,
        "agent_id": task.agent_id,
        "board_item_id": task.board_item_id,
    });
    if let Some(metadata) = &task.metadata {
        value["plan_id"] = json!(metadata.plan_id);
        value["plan_step"] = json!(metadata.plan_step);
        value["blocked_by"] = json!(metadata.blocked_by);
        value["required_evidence"] = json!(metadata.required_evidence);
        if let Some((completed, total)) = plan_progress {
            value["plan_progress"] = json!({ "completed": completed, "total": total });
        }
    }
    value
}

pub fn task_list_json(manager: &TaskManager) -> serde_json::Value {
    let tasks = manager.list_tasks();
    let plan_totals = tasks.iter().fold(
        std::collections::HashMap::<String, (usize, usize)>::new(),
        |mut totals, task| {
            if let Some(metadata) = &task.metadata {
                let entry = totals.entry(metadata.plan_id.clone()).or_default();
                entry.1 += 1;
                if task.status == TaskStatus::Completed {
                    entry.0 += 1;
                }
            }
            totals
        },
    );
    serde_json::Value::Array(
        tasks
            .into_iter()
            .map(|task| {
                let progress = task
                    .metadata
                    .as_ref()
                    .and_then(|metadata| plan_totals.get(&metadata.plan_id).copied());
                task_info_json(&task, progress)
            })
            .collect(),
    )
}
