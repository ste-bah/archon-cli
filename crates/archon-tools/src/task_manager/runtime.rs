use std::sync::atomic::Ordering;

use chrono::Utc;

use crate::board::{DelegatedOutcome, close_delegated_task};

use super::plan_persistence::is_valid_transition;
use super::{TaskManager, TaskStatus, TaskTransitionError};

pub(super) fn set_status(
    manager: &TaskManager,
    id: &str,
    status: TaskStatus,
) -> Result<(), TaskTransitionError> {
    let mut mirror = None;
    let mut tasks = manager
        .tasks
        .lock()
        .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
    let info = tasks
        .get_mut(id)
        .ok_or_else(|| TaskTransitionError::NotFound(id.to_string()))?;
    if !is_valid_transition(&info.status, &status) {
        return Err(TaskTransitionError::InvalidTransition {
            id: id.to_string(),
            from: info.status.clone(),
            to: status,
        });
    }
    info.status = status.clone();
    if matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
    ) {
        info.completed_at = Some(Utc::now());
        mirror = info.board_item_id.clone().map(|item| {
            let outcome = match status {
                TaskStatus::Completed => DelegatedOutcome::Completed,
                TaskStatus::Stopped => DelegatedOutcome::Stopped,
                _ => DelegatedOutcome::Failed,
            };
            (item, outcome)
        });
    }
    drop(tasks);
    if let Some((item, outcome)) = mirror {
        close_delegated_task(&item, outcome);
    }
    Ok(())
}

pub(super) fn stop_task(manager: &TaskManager, id: &str) -> Result<(), String> {
    let task = manager
        .get_task(id)
        .ok_or_else(|| format!("task not found: {id}"))?;
    if is_terminal(&task.status) {
        return Ok(());
    }
    let plan_linked = task.metadata.is_some();
    if plan_linked {
        match manager.set_status_checked_with_evidence_ids(id, TaskStatus::Stopped, "", &[]) {
            Ok(()) => {}
            Err(TaskTransitionError::InvalidTransition { from, .. }) if is_terminal(&from) => {
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    manager
        .cancellation_tokens
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?
        .get(id)
        .ok_or_else(|| format!("task not found: {id}"))?
        .store(true, Ordering::SeqCst);
    manager
        .execution_tokens
        .lock()
        .map_err(|error| format!("lock poisoned: {error}"))?
        .get(id)
        .ok_or_else(|| format!("task not found: {id}"))?
        .cancel();
    if !plan_linked {
        match set_status(manager, id, TaskStatus::Stopped) {
            Ok(()) => {}
            Err(TaskTransitionError::InvalidTransition { from, .. }) if is_terminal(&from) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
    )
}
