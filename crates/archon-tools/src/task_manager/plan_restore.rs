use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::{TaskInfo, TaskManager, TaskTransitionError};

impl TaskManager {
    pub(super) fn attach_plan_store_for_test(
        &self,
        store: archon_session::plan::PlanStore,
        session_id: impl Into<String>,
    ) -> Result<(), TaskTransitionError> {
        let mut persistence = self
            .plan_persistence
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        let session_id = session_id.into();
        if persistence
            .get(&session_id)
            .is_some_and(|existing| !existing.is_same_store(&store))
        {
            return Err(TaskTransitionError::Persistence(format!(
                "session {session_id} is already attached to a different plan store"
            )));
        }
        persistence.entry(session_id).or_insert(store);
        Ok(())
    }

    /// Restore an already durably persisted plan-linked task.
    ///
    /// This compatibility entry point refuses to publish caller-supplied plan
    /// metadata unless the exact task row is present in the session's attached
    /// store. Transaction-backed materialization and batch rehydration use
    /// their dedicated prepared-installation paths instead.
    #[allow(dead_code)]
    pub(crate) fn insert_plan_task(&self, info: TaskInfo) -> Result<(), TaskTransitionError> {
        let metadata = info.metadata.as_ref().ok_or_else(|| {
            TaskTransitionError::Persistence("insert_plan_task requires plan metadata".to_string())
        })?;
        let store = self
            .plan_persistence
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?
            .get(&metadata.session_id)
            .cloned()
            .ok_or_else(|| {
                TaskTransitionError::Persistence(format!(
                    "plan-linked task session {} has no attached plan store",
                    metadata.session_id
                ))
            })?;
        let durable = store
            .load_plan_tasks(&metadata.session_id)
            .map_err(|error| TaskTransitionError::Persistence(error.to_string()))?;
        let exists = durable.iter().any(|task| {
            task.task_id == info.id
                && task.plan_id == metadata.plan_id
                && task.plan_step == metadata.plan_step
                && task.description == info.description
                && task.status == info.status.to_string()
                && task.blocked_by == metadata.blocked_by
                && task.required_evidence == metadata.required_evidence
        });
        if !exists {
            return Err(TaskTransitionError::Persistence(format!(
                "plan-linked task {} is not durably persisted",
                info.id
            )));
        }
        self.restore_plan_task(info)
    }

    /// Insert a task that was already durably persisted.
    ///
    /// Used only during session rehydration and transaction-backed plan
    /// materialization.
    #[allow(dead_code)]
    pub(crate) fn restore_plan_task(&self, info: TaskInfo) -> Result<(), TaskTransitionError> {
        if info.metadata.is_none() {
            return Err(TaskTransitionError::Persistence(
                "restore_plan_task requires plan metadata".to_string(),
            ));
        }
        let id = info.id.clone();
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        let mut cancellation_tokens = self
            .cancellation_tokens
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        let mut execution_tokens = self
            .execution_tokens
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        if tasks.contains_key(&id)
            || cancellation_tokens.contains_key(&id)
            || execution_tokens.contains_key(&id)
        {
            return Err(TaskTransitionError::Persistence(format!(
                "task ID collision while restoring durable plan task: {id}"
            )));
        }
        tasks.insert(id.clone(), info);
        cancellation_tokens.insert(
            id.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        execution_tokens.insert(id, CancellationToken::new());
        Ok(())
    }
}
