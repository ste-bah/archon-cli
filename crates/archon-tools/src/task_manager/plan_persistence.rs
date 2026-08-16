use std::collections::HashMap;
use std::sync::Mutex;

use archon_completion::{RequiredEvidence, RequiredEvidenceKind, check_required_evidence};
use archon_session::plan::{PersistedPlanTask, PlanStore};
use chrono::Utc;

use crate::board::{DelegatedOutcome, close_delegated_task};

use super::{TaskInfo, TaskManager, TaskStatus};

pub(crate) fn is_valid_transition(from: &TaskStatus, to: &TaskStatus) -> bool {
    matches!(
        (from, to),
        (
            TaskStatus::Pending,
            TaskStatus::Running | TaskStatus::Failed | TaskStatus::Stopped
        ) | (
            TaskStatus::Running,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
        )
    )
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanTaskMetadata {
    pub session_id: String,
    pub plan_id: String,
    pub plan_step: u32,
    pub blocked_by: Vec<String>,
    pub required_evidence: Vec<RequiredEvidenceKind>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TaskTransitionError {
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("invalid transition for task {id}: {from} -> {to}")]
    InvalidTransition {
        id: String,
        from: TaskStatus,
        to: TaskStatus,
    },
    #[error("task {id} is blocked by incomplete dependencies: {blocked_by:?}")]
    BlockedDependency { id: String, blocked_by: Vec<String> },
    #[error("task {id} is missing required evidence: {missing:?}")]
    MissingEvidence {
        id: String,
        missing: Vec<RequiredEvidenceKind>,
    },
    #[error("task {id} has failed required evidence: {failed:?}")]
    FailedEvidence {
        id: String,
        failed: Vec<RequiredEvidenceKind>,
    },
    #[error("task manager lock poisoned: {0}")]
    Lock(String),
    #[error("failed to resolve trusted completion evidence: {0}")]
    EvidenceResolution(String),
    #[error("evidence {0} lacks trusted completion provenance")]
    UntrustedEvidence(String),
    #[error("plan-linked task descriptions are immutable: {0}")]
    PlanTaskDescriptionImmutable(String),
    #[error("failed to persist plan task: {0}")]
    Persistence(String),
}

pub(super) fn persist_plan_task(
    plan_persistence: &Mutex<HashMap<String, PlanStore>>,
    previous: &TaskInfo,
    info: &TaskInfo,
    evidence: &[RequiredEvidence],
) -> Result<(), TaskTransitionError> {
    let Some(metadata) = &info.metadata else {
        return Ok(());
    };
    let store = plan_persistence
        .lock()
        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?
        .get(&metadata.session_id)
        .cloned()
        .ok_or_else(|| {
            TaskTransitionError::Persistence(
                "plan-linked task session has no attached plan store".into(),
            )
        })?;
    let record = persisted_plan_task(info)?;
    let (evidence_run_id, evidence_ids) = durable_evidence_identity(evidence)?;
    store
        .transition_plan_task_checked(
            &metadata.session_id,
            &record.task_id,
            &previous.status.to_string(),
            &record.status,
            evidence_run_id.as_deref().unwrap_or_default(),
            &evidence_ids,
        )
        .map_err(|error| TaskTransitionError::Persistence(error.to_string()))
}

fn durable_evidence_identity(
    evidence: &[RequiredEvidence],
) -> Result<(Option<String>, Vec<String>), TaskTransitionError> {
    evidence.iter().try_fold(
        (None::<String>, Vec::with_capacity(evidence.len())),
        |(run_id, mut evidence_ids), supplied| {
            let supplied_run_id = supplied.run_id.as_deref().ok_or_else(|| {
                TaskTransitionError::UntrustedEvidence(
                    "completion evidence is missing its durable run ID".to_string(),
                )
            })?;
            let evidence_id = supplied.evidence_id.as_ref().ok_or_else(|| {
                TaskTransitionError::UntrustedEvidence(
                    "completion evidence is missing its durable ID".to_string(),
                )
            })?;
            if run_id
                .as_deref()
                .is_some_and(|existing| existing != supplied_run_id)
            {
                return Err(TaskTransitionError::UntrustedEvidence(
                    "completion evidence spans multiple durable runs".to_string(),
                ));
            }
            evidence_ids.push(evidence_id.clone());
            Ok((Some(supplied_run_id.to_string()), evidence_ids))
        },
    )
}

fn persisted_plan_task(info: &TaskInfo) -> Result<PersistedPlanTask, TaskTransitionError> {
    let metadata = info.metadata.as_ref().ok_or_else(|| {
        TaskTransitionError::Persistence("manual task cannot be persisted as a plan task".into())
    })?;
    Ok(PersistedPlanTask {
        task_id: info.id.clone(),
        plan_id: metadata.plan_id.clone(),
        plan_step: metadata.plan_step,
        description: info.description.clone(),
        status: info.status.to_string(),
        blocked_by: metadata.blocked_by.clone(),
        required_evidence: metadata.required_evidence.clone(),
        completion_evidence: Vec::new(),
        updated_at: Utc::now().to_rfc3339(),
    })
}

pub(super) fn set_status_checked(
    manager: &TaskManager,
    id: &str,
    status: TaskStatus,
    evidence: &[RequiredEvidence],
) -> Result<(), TaskTransitionError> {
    let mut mirror = None;
    let mut tasks = manager
        .tasks
        .lock()
        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
    let current_info = tasks
        .get(id)
        .cloned()
        .ok_or_else(|| TaskTransitionError::NotFound(id.into()))?;
    if !is_valid_transition(&current_info.status, &status) {
        return Err(TaskTransitionError::InvalidTransition {
            id: id.into(),
            from: current_info.status,
            to: status,
        });
    }
    if let Some(metadata) = &current_info.metadata {
        let blocked_by = metadata
            .blocked_by
            .iter()
            .filter(|dependency| {
                !matches!(
                    tasks.get(*dependency).map(|task| &task.status),
                    Some(TaskStatus::Completed)
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        if status == TaskStatus::Running && !blocked_by.is_empty() {
            return Err(TaskTransitionError::BlockedDependency {
                id: id.into(),
                blocked_by,
            });
        }
        if status == TaskStatus::Completed {
            let check = check_required_evidence(&metadata.required_evidence, evidence);
            if !check.missing.is_empty() {
                return Err(TaskTransitionError::MissingEvidence {
                    id: id.into(),
                    missing: check.missing,
                });
            }
            if !check.failed.is_empty() {
                return Err(TaskTransitionError::FailedEvidence {
                    id: id.into(),
                    failed: check.failed,
                });
            }
        }
    }
    let mut persisted_info = current_info.clone();
    persisted_info.status = status.clone();
    if matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
    ) {
        persisted_info.completed_at = Some(Utc::now());
        if let Some(item) = persisted_info.board_item_id.clone() {
            let outcome = match status {
                TaskStatus::Completed => DelegatedOutcome::Completed,
                TaskStatus::Stopped => DelegatedOutcome::Stopped,
                TaskStatus::Failed => DelegatedOutcome::Failed,
                _ => unreachable!("terminal status was checked"),
            };
            mirror = Some((item, outcome));
        }
    }
    persist_plan_task(
        &manager.plan_persistence,
        &current_info,
        &persisted_info,
        evidence,
    )?;
    tasks.insert(id.to_string(), persisted_info);
    drop(tasks);
    if let Some((item, outcome)) = mirror {
        close_delegated_task(&item, outcome);
    }
    Ok(())
}
