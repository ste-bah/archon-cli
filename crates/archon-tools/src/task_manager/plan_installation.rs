use std::collections::{HashMap, HashSet};
use std::sync::{Arc, MutexGuard};

use archon_session::plan::{PlanApprovalAuthority, PlanStore};
use tokio_util::sync::CancellationToken;

use super::{PlanTaskMetadata, TaskInfo, TaskManager, TaskTransitionError};

/// Holds all fallible manager lock acquisition before a durable plan commit.
/// `install` cannot report an error after that commit succeeds.
pub struct PreparedPlanTaskInstallation<'a> {
    tasks: MutexGuard<'a, HashMap<String, TaskInfo>>,
    cancellation_tokens: MutexGuard<'a, HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    execution_tokens: MutexGuard<'a, HashMap<String, CancellationToken>>,
    persistence: MutexGuard<'a, HashMap<String, PlanStore>>,
    session_id: String,
    store: PlanStore,
    infos: Vec<TaskInfo>,
}

/// Holds every manager lock while validated durable tasks are restored.
/// `install` publishes the complete batch and its store attachment together.
pub struct PreparedPlanTaskRehydration<'a> {
    tasks: MutexGuard<'a, HashMap<String, TaskInfo>>,
    cancellation_tokens: MutexGuard<'a, HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
    execution_tokens: MutexGuard<'a, HashMap<String, CancellationToken>>,
    persistence: MutexGuard<'a, HashMap<String, PlanStore>>,
    session_id: String,
    store: PlanStore,
    absent_infos: Vec<TaskInfo>,
}

impl TaskManager {
    pub fn prepare_plan_task_installation(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        store: PlanStore,
        infos: Vec<TaskInfo>,
    ) -> Result<PreparedPlanTaskInstallation<'_>, TaskTransitionError> {
        validate_installation_authority(&store, authority, session_id)?;
        validate_plan_task_batch(session_id, &infos)?;
        let tasks = self.lock_tasks()?;
        reject_existing_task_ids(&tasks, &infos)?;
        self.fail_plan_installation_for_test(session_id)?;
        let cancellation_tokens = self.lock_cancellation_tokens()?;
        let execution_tokens = self.lock_execution_tokens()?;
        let persistence = self.lock_plan_persistence()?;
        validate_store_attachment(&persistence, session_id, &store)?;
        Ok(PreparedPlanTaskInstallation {
            tasks,
            cancellation_tokens,
            execution_tokens,
            persistence,
            session_id: session_id.to_string(),
            store,
            infos,
        })
    }

    /// Validate durable tasks against current manager state and retain every
    /// manager lock until the complete restore batch and attachment are visible.
    pub fn prepare_plan_task_rehydration(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        store: PlanStore,
        infos: Vec<TaskInfo>,
    ) -> Result<PreparedPlanTaskRehydration<'_>, TaskTransitionError> {
        validate_installation_authority(&store, authority, session_id)?;
        validate_plan_task_batch(session_id, &infos)?;
        let tasks = self.lock_tasks()?;
        let cancellation_tokens = self.lock_cancellation_tokens()?;
        let execution_tokens = self.lock_execution_tokens()?;
        let persistence = self.lock_plan_persistence()?;
        validate_store_attachment(&persistence, session_id, &store)?;
        let absent_infos =
            validate_rehydration_batch(&tasks, &cancellation_tokens, &execution_tokens, &infos)?;
        self.fail_plan_installation_for_test(session_id)?;
        Ok(PreparedPlanTaskRehydration {
            tasks,
            cancellation_tokens,
            execution_tokens,
            persistence,
            session_id: session_id.to_string(),
            store,
            absent_infos,
        })
    }

    fn lock_tasks(&self) -> Result<MutexGuard<'_, HashMap<String, TaskInfo>>, TaskTransitionError> {
        self.tasks
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))
    }

    fn lock_cancellation_tokens(
        &self,
    ) -> Result<
        MutexGuard<'_, HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
        TaskTransitionError,
    > {
        self.cancellation_tokens
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))
    }

    fn lock_execution_tokens(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, CancellationToken>>, TaskTransitionError> {
        self.execution_tokens
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))
    }

    fn lock_plan_persistence(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, PlanStore>>, TaskTransitionError> {
        self.plan_persistence
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn fail_plan_installation_for_test(&self, session_id: &str) -> Result<(), TaskTransitionError> {
        let mut target = self
            .fail_next_plan_installation
            .lock()
            .map_err(|error| TaskTransitionError::Lock(error.to_string()))?;
        if target.as_deref() == Some(session_id) {
            *target = None;
            return Err(TaskTransitionError::Persistence(
                "injected plan task installation preparation failure".to_string(),
            ));
        }
        Ok(())
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn fail_plan_installation_for_test(
        &self,
        _session_id: &str,
    ) -> Result<(), TaskTransitionError> {
        Ok(())
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_plan_task_installation_for_test(&self, session_id: impl Into<String>) {
        *self
            .fail_next_plan_installation
            .lock()
            .expect("plan installation fault lock") = Some(session_id.into());
    }

    #[doc(hidden)]
    #[cfg(any(test, feature = "test-support"))]
    pub fn installed_plan_task_count_for_session_for_test(&self, session_id: &str) -> usize {
        self.tasks
            .lock()
            .map(|tasks| {
                tasks
                    .values()
                    .filter(|task| {
                        task.metadata
                            .as_ref()
                            .is_some_and(|metadata| metadata.session_id == session_id)
                    })
                    .count()
            })
            .unwrap_or_default()
    }
}

impl PreparedPlanTaskInstallation<'_> {
    pub fn install(mut self) {
        self.persistence
            .entry(self.session_id)
            .or_insert(self.store);
        for info in self.infos.drain(..) {
            insert_task_with_tokens(
                &mut self.tasks,
                &mut self.cancellation_tokens,
                &mut self.execution_tokens,
                info,
            );
        }
    }
}

impl PreparedPlanTaskRehydration<'_> {
    pub fn install(mut self) -> usize {
        self.persistence
            .entry(self.session_id)
            .or_insert(self.store);
        let inserted = self.absent_infos.len();
        for info in self.absent_infos.drain(..) {
            insert_task_with_tokens(
                &mut self.tasks,
                &mut self.cancellation_tokens,
                &mut self.execution_tokens,
                info,
            );
        }
        inserted
    }
}

fn validate_installation_authority(
    store: &PlanStore,
    authority: &PlanApprovalAuthority,
    session_id: &str,
) -> Result<(), TaskTransitionError> {
    store
        .validate_approval_authority(authority, session_id)
        .map_err(|error| TaskTransitionError::Persistence(error.to_string()))
}

fn validate_plan_task_batch(
    session_id: &str,
    infos: &[TaskInfo],
) -> Result<(), TaskTransitionError> {
    let mut ids = HashSet::with_capacity(infos.len());
    for info in infos {
        let Some(PlanTaskMetadata {
            session_id: owner, ..
        }) = &info.metadata
        else {
            return Err(TaskTransitionError::Persistence(
                "plan task installation requires plan metadata".to_string(),
            ));
        };
        if owner != session_id {
            return Err(TaskTransitionError::Persistence(format!(
                "plan task {} belongs to session {owner}, not {session_id}",
                info.id
            )));
        }
        if !ids.insert(info.id.as_str()) {
            return Err(TaskTransitionError::Persistence(format!(
                "task ID collision in pending plan-task batch: {}",
                info.id
            )));
        }
    }
    Ok(())
}

fn reject_existing_task_ids(
    tasks: &HashMap<String, TaskInfo>,
    infos: &[TaskInfo],
) -> Result<(), TaskTransitionError> {
    for info in infos {
        if tasks.contains_key(&info.id) {
            return Err(TaskTransitionError::Persistence(format!(
                "task ID collision with existing manager task: {}",
                info.id
            )));
        }
    }
    Ok(())
}

fn validate_store_attachment(
    persistence: &HashMap<String, PlanStore>,
    session_id: &str,
    store: &PlanStore,
) -> Result<(), TaskTransitionError> {
    if persistence
        .get(session_id)
        .is_some_and(|existing| !existing.is_same_store(store))
    {
        return Err(TaskTransitionError::Persistence(format!(
            "session {session_id} is already attached to a different plan store"
        )));
    }
    Ok(())
}

fn validate_rehydration_batch(
    tasks: &HashMap<String, TaskInfo>,
    cancellation_tokens: &HashMap<String, Arc<std::sync::atomic::AtomicBool>>,
    execution_tokens: &HashMap<String, CancellationToken>,
    infos: &[TaskInfo],
) -> Result<Vec<TaskInfo>, TaskTransitionError> {
    let mut absent = Vec::new();
    for info in infos {
        if let Some(existing) = tasks.get(&info.id) {
            if !same_plan_task(existing, info) {
                return Err(TaskTransitionError::Persistence(format!(
                    "canonical mismatch while rehydrating durable plan task: {}",
                    info.id
                )));
            }
            continue;
        }
        if cancellation_tokens.contains_key(&info.id) || execution_tokens.contains_key(&info.id) {
            return Err(TaskTransitionError::Persistence(format!(
                "task ID collision while restoring durable plan task: {}",
                info.id
            )));
        }
        absent.push(info.clone());
    }
    Ok(absent)
}

fn same_plan_task(existing: &TaskInfo, durable: &TaskInfo) -> bool {
    existing.id == durable.id
        && existing.description == durable.description
        && existing.status == durable.status
        && existing.metadata == durable.metadata
}

fn insert_task_with_tokens(
    tasks: &mut HashMap<String, TaskInfo>,
    cancellation_tokens: &mut HashMap<String, Arc<std::sync::atomic::AtomicBool>>,
    execution_tokens: &mut HashMap<String, CancellationToken>,
    info: TaskInfo,
) {
    let id = info.id.clone();
    tasks.insert(id.clone(), info);
    cancellation_tokens.insert(
        id.clone(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    execution_tokens.insert(id, CancellationToken::new());
}
