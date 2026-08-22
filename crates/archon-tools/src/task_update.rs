use serde_json::json;

use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

/// Tool that updates a task's description or status.
pub struct TaskUpdateTool;

#[async_trait::async_trait]
impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::ControlPlane
    }

    fn description(&self) -> &str {
        "Update a task's description or status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The full task ID"
                },
                "description": {
                    "type": "string",
                    "description": "New description for the task"
                },
                "status": {
                    "type": "string",
                    "description": "New status: Pending, Running, Completed, Failed, or Stopped"
                },
                "evidence_run_id": {
                    "type": "string",
                    "description": "Durable completion-evidence run ID"
                },
                "evidence_ids": {
                    "type": "array",
                    "description": "Durable completion-evidence IDs for a plan-linked completion",
                    "items": { "type": "string" }
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let task_id = match input.get("task_id").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => s,
            _ => return ToolResult::error("missing required field: task_id"),
        };

        let mgr = &crate::task_manager::TASK_MANAGER;

        // Verify task exists
        if mgr.get_task(task_id).is_none() {
            return ToolResult::error(
                serde_json::json!({
                    "error": "task_not_found",
                    "message": format!("task not found: {task_id}"),
                })
                .to_string(),
            );
        }

        if (input.get("evidence_run_id").is_some() || input.get("evidence_ids").is_some())
            && input.get("status").is_none()
        {
            return ToolResult::error("evidence IDs require a status transition");
        }

        let new_desc = input.get("description").and_then(|value| value.as_str());
        if new_desc.is_some_and(|_| {
            mgr.get_task(task_id)
                .is_some_and(|task| task.metadata.is_some())
        }) {
            return ToolResult::error(
                json!({
                    "error": "plan_task_description_immutable",
                    "message": format!("plan-linked task descriptions are immutable: {task_id}"),
                })
                .to_string(),
            );
        }

        let status = match input.get("status").and_then(|value| value.as_str()) {
            Some("Pending") => Some(crate::task_manager::TaskStatus::Pending),
            Some("Running") => Some(crate::task_manager::TaskStatus::Running),
            Some("Completed") => Some(crate::task_manager::TaskStatus::Completed),
            Some("Failed") => Some(crate::task_manager::TaskStatus::Failed),
            Some("Stopped") => Some(crate::task_manager::TaskStatus::Stopped),
            Some(other) => {
                return ToolResult::error(format!(
                    "invalid status: '{other}'. Must be Pending, Running, Completed, Failed, or Stopped"
                ));
            }
            None => None,
        };
        let evidence = match input.get("evidence_ids") {
            Some(value) => match serde_json::from_value::<Vec<String>>(value.clone()) {
                Ok(ids) => ids
                    .into_iter()
                    .enumerate()
                    .map(
                        |(sequence, evidence_id)| archon_completion::RequiredEvidence {
                            kind: archon_completion::RequiredEvidenceKind::Tests,
                            status: archon_completion::RequiredEvidenceStatus::Unknown,
                            sequence: sequence as u64 + 1,
                            evidence_id: Some(evidence_id),
                            run_id: input
                                .get("evidence_run_id")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                        },
                    )
                    .collect::<Vec<_>>(),
                Err(error) => {
                    return ToolResult::error(format!("invalid evidence ID payload: {error}"));
                }
            },
            None => Vec::new(),
        };

        if let Some(description) = new_desc
            && let Err(error) = mgr.update_task(task_id, Some(description))
        {
            return ToolResult::error(
                json!({ "error": transition_error_code(&error), "message": error.to_string() })
                    .to_string(),
            );
        }
        if let Some(status) = status
            && let Err(error) = mgr.set_status_checked(task_id, status, &evidence)
        {
            return ToolResult::error(
                json!({ "error": transition_error_code(&error), "message": error.to_string() })
                    .to_string(),
            );
        }

        ToolResult::success(format!("task {task_id} updated"))
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::ExternalOnly
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

fn transition_error_code(error: &crate::task_manager::TaskTransitionError) -> &'static str {
    match error {
        crate::task_manager::TaskTransitionError::NotFound(_) => "task_not_found",
        crate::task_manager::TaskTransitionError::InvalidTransition { .. } => "invalid_transition",
        crate::task_manager::TaskTransitionError::BlockedDependency { .. } => "blocked_dependency",
        crate::task_manager::TaskTransitionError::MissingEvidence { .. } => "missing_evidence",
        crate::task_manager::TaskTransitionError::FailedEvidence { .. } => "failed_evidence",
        crate::task_manager::TaskTransitionError::Lock(_) => "task_manager_lock",
        crate::task_manager::TaskTransitionError::EvidenceResolution(_) => "evidence_resolution",
        crate::task_manager::TaskTransitionError::UntrustedEvidence(_) => "untrusted_evidence",
        crate::task_manager::TaskTransitionError::PlanTaskDescriptionImmutable(_) => {
            "plan_task_description_immutable"
        }
        crate::task_manager::TaskTransitionError::Persistence(_) => "plan_persistence",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_tasks::{materialize_plan_tasks, test_plan_approval_authority};
    use crate::task_manager::{TASK_MANAGER, TaskStatus};
    use crate::tool::Tool;
    use archon_completion::RequiredEvidenceKind;

    fn materialize_runtime_plan(
        session_id: &str,
    ) -> (
        Vec<String>,
        cozo::DbInstance,
        archon_session::plan::PlanStore,
        std::sync::Arc<archon_session::plan::PlanApprovalAuthority>,
    ) {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let store = archon_session::plan::PlanStore::new(&db).unwrap();
        let plan_id = uuid::Uuid::new_v4().to_string();
        let mut plan = archon_session::plan::PlanDocument::new(&plan_id, "Runtime transition plan");
        plan.status = archon_session::plan::PlanStatus::Approved;
        plan.approval = Some(archon_session::plan::PlanApproval {
            decision: archon_session::plan::PlanApprovalDecision::Approve,
            source: archon_session::plan::PlanApprovalSource::NonInteractive,
            decided_at: "2026-08-15T00:00:00Z".into(),
            user_edited: false,
        });
        plan.steps = vec![
            archon_session::plan::PlanStep {
                number: 1,
                description: "first".into(),
                affected_files: vec![],
                status: archon_session::plan::PlanStepStatus::Pending,
                blocked_by: vec![],
                required_evidence: vec![],
                task_id: None,
            },
            archon_session::plan::PlanStep {
                number: 2,
                description: "evidence step".into(),
                affected_files: vec![],
                status: archon_session::plan::PlanStepStatus::Pending,
                blocked_by: vec![1],
                required_evidence: vec![RequiredEvidenceKind::Tests],
                task_id: None,
            },
        ];
        let authority = test_plan_approval_authority(&store, session_id);
        let ids = materialize_plan_tasks(&TASK_MANAGER, &store, &authority, session_id, &mut plan)
            .unwrap();
        (ids, db, store, authority)
    }

    #[tokio::test]
    async fn nonexistent_task_returns_structured_error() {
        let result = TaskUpdateTool
            .execute(
                json!({"task_id": "missing", "status": "Running"}),
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("\"task_not_found\""));
    }

    #[tokio::test]
    async fn runtime_tool_reports_checked_transition_errors() {
        let (ids, _, _, _) =
            materialize_runtime_plan(&format!("runtime-update-{}", uuid::Uuid::new_v4()));
        let context = ToolContext::default();

        let invalid = TaskUpdateTool
            .execute(json!({"task_id": ids[0], "status": "Completed"}), &context)
            .await;
        assert!(invalid.is_error);
        assert!(invalid.content.contains("\"invalid_transition\""));

        let blocked = TaskUpdateTool
            .execute(json!({"task_id": ids[1], "status": "Running"}), &context)
            .await;
        assert!(blocked.is_error);
        assert!(blocked.content.contains("\"blocked_dependency\""));

        TaskUpdateTool
            .execute(json!({"task_id": ids[0], "status": "Running"}), &context)
            .await;
        TaskUpdateTool
            .execute(json!({"task_id": ids[0], "status": "Completed"}), &context)
            .await;
        let running = TaskUpdateTool
            .execute(json!({"task_id": ids[1], "status": "Running"}), &context)
            .await;
        assert!(!running.is_error, "{running:?}");

        let missing = TaskUpdateTool
            .execute(json!({"task_id": ids[1], "status": "Completed"}), &context)
            .await;
        assert!(missing.is_error);
        assert!(missing.content.contains("\"missing_evidence\""));

        let forged = TaskUpdateTool
            .execute(
                json!({
                    "task_id": ids[1],
                    "status": "Completed",
                    "evidence_run_id": "forged-run",
                    "evidence_ids": ["model-asserted-passed"]
                }),
                &context,
            )
            .await;
        assert!(forged.is_error);
        assert!(forged.content.contains("\"untrusted_evidence\""));
        assert_eq!(
            TASK_MANAGER.get_task(&ids[1]).unwrap().status,
            TaskStatus::Running
        );
    }

    #[tokio::test]
    async fn plan_description_update_is_rejected_without_partial_status_change() {
        let (ids, _, _, _) =
            materialize_runtime_plan(&format!("runtime-description-{}", uuid::Uuid::new_v4()));
        let context = ToolContext::default();
        let result = TaskUpdateTool
            .execute(
                json!({
                    "task_id": ids[0],
                    "description": "forbidden change",
                    "status": "Running"
                }),
                &context,
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("plan_task_description_immutable"));
        assert_eq!(TASK_MANAGER.get_task(&ids[0]).unwrap().description, "first");
        assert_eq!(
            TASK_MANAGER.get_task(&ids[0]).unwrap().status,
            TaskStatus::Pending
        );
    }

    #[tokio::test]
    async fn manual_description_update_remains_supported() {
        let id = TASK_MANAGER.create_task("old manual description");
        let result = TaskUpdateTool
            .execute(
                json!({"task_id": id, "description": "new manual description"}),
                &ToolContext::default(),
            )
            .await;

        assert!(!result.is_error, "{result:?}");
        assert_eq!(
            TASK_MANAGER.get_task(&id).unwrap().description,
            "new manual description"
        );
    }

    #[tokio::test]
    async fn trusted_durable_evidence_allows_plan_completion() {
        let session_id = format!("runtime-trusted-{}", uuid::Uuid::new_v4());
        let (ids, _db, store, authority) = materialize_runtime_plan(&session_id);
        let context = ToolContext::default();
        TaskUpdateTool
            .execute(json!({"task_id": ids[0], "status": "Running"}), &context)
            .await;
        TaskUpdateTool
            .execute(json!({"task_id": ids[0], "status": "Completed"}), &context)
            .await;
        TaskUpdateTool
            .execute(json!({"task_id": ids[1], "status": "Running"}), &context)
            .await;
        let evidence = store
            .record_authoritative_test_execution(
                &authority,
                &session_id,
                "trusted-tool",
                0,
                "cargo test -p archon-tools",
                "test result: ok. 1 passed; 0 failed",
                0,
            )
            .unwrap();
        store
            .verify_test_command_evidence(
                &authority,
                &session_id,
                &evidence.run_id,
                &evidence.evidence_id,
            )
            .unwrap();
        let completed = TaskUpdateTool
            .execute(
                json!({
                    "task_id": ids[1],
                    "status": "Completed",
                    "evidence_run_id": evidence.run_id,
                    "evidence_ids": [evidence.evidence_id]
                }),
                &context,
            )
            .await;
        assert!(!completed.is_error, "{completed:?}");
        assert_eq!(
            TASK_MANAGER.get_task(&ids[1]).unwrap().status,
            TaskStatus::Completed
        );
    }
}
