use crate::task_manager::{TaskInfo, TaskManager, TaskStatus};
#[cfg(test)]
use archon_session::plan::PlanStatus;
use archon_session::plan::{PersistedPlanTask, PlanDocument, PlanStepStatus, PlanStore};
use chrono::{DateTime, Utc};
use std::collections::HashSet;
#[cfg(any(test, feature = "test-support"))]
use {
    archon_session::plan::PlanApprovalAuthority,
    std::{
        cell::RefCell,
        collections::VecDeque,
        sync::{Arc, Barrier, LazyLock, Mutex},
    },
};

pub use crate::task_manager::PlanTaskMetadata;

#[path = "plan_task_materialization.rs"]
mod materialization;
pub use materialization::materialize_plan_tasks;

#[cfg(any(test, feature = "test-support"))]
pub fn test_plan_approval_authority(store: &PlanStore, session_id: &str) -> PlanApprovalAuthority {
    store
        .bootstrap_approval_authority_for_test(session_id)
        .expect("test plan approval authority")
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static NEXT_PLAN_TASK_IDS_FOR_TEST: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
}

#[cfg(any(test, feature = "test-support"))]
type MaterializationBarrier = Option<(String, Arc<Barrier>)>;

#[cfg(any(test, feature = "test-support"))]
static MATERIALIZATION_BARRIER_FOR_TEST: LazyLock<Mutex<MaterializationBarrier>> =
    LazyLock::new(|| Mutex::new(None));
#[cfg(any(test, feature = "test-support"))]
static MATERIALIZATION_BARRIER_TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct MaterializationBarrierReset {
    _serial: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for MaterializationBarrierReset {
    fn drop(&mut self) {
        *MATERIALIZATION_BARRIER_FOR_TEST
            .lock()
            .expect("test barrier lock") = None;
    }
}

#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub fn set_materialization_barrier_for_test(
    target_session: String,
    barrier: Option<Arc<Barrier>>,
) -> MaterializationBarrierReset {
    let serial = MATERIALIZATION_BARRIER_TEST_SERIAL
        .lock()
        .expect("test barrier serial lock");
    *MATERIALIZATION_BARRIER_FOR_TEST
        .lock()
        .expect("test barrier lock") = barrier.map(|barrier| (target_session, barrier));
    MaterializationBarrierReset {
        _serial: Some(serial),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn wait_for_materialization_barrier_for_test(session_id: &str) {
    let barrier = MATERIALIZATION_BARRIER_FOR_TEST
        .lock()
        .expect("test barrier lock")
        .as_ref()
        .filter(|(target, _)| target == session_id)
        .map(|(_, barrier)| Arc::clone(barrier));
    if let Some(barrier) = barrier {
        barrier.wait();
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn wait_for_materialization_barrier_for_test(_session_id: &str) {}

#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub fn set_next_plan_task_ids_for_test(ids: impl IntoIterator<Item = String>) {
    NEXT_PLAN_TASK_IDS_FOR_TEST.with(|next_ids| {
        *next_ids.borrow_mut() = ids.into_iter().collect();
    });
}

#[cfg(any(test, feature = "test-support"))]
fn next_plan_task_id() -> String {
    NEXT_PLAN_TASK_IDS_FOR_TEST
        .with(|next_ids| next_ids.borrow_mut().pop_front())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string())
}

#[cfg(not(any(test, feature = "test-support")))]
fn next_plan_task_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn build_plan_task_infos(
    session_id: &str,
    plan: &mut PlanDocument,
) -> Result<Vec<TaskInfo>, String> {
    for step in &mut plan.steps {
        if step.task_id.is_none() {
            step.task_id = Some(next_plan_task_id());
        }
    }
    let task_id_by_step = plan
        .steps
        .iter()
        .map(|step| (step.number, step.task_id.clone().expect("task id assigned")))
        .collect::<std::collections::HashMap<_, _>>();
    plan.steps
        .iter()
        .map(|step| {
            let blocked_by = step
                .blocked_by
                .iter()
                .map(|number| {
                    task_id_by_step.get(number).cloned().ok_or_else(|| {
                        format!(
                            "plan step {} has unresolved dependency {number}",
                            step.number
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TaskInfo {
                id: step.task_id.clone().expect("task id assigned"),
                description: step.description.clone(),
                status: TaskStatus::Pending,
                created_at: Utc::now(),
                completed_at: None,
                output: String::new(),
                cost: 0.0,
                agent_id: None,
                board_item_id: None,
                metadata: Some(PlanTaskMetadata {
                    session_id: session_id.to_string(),
                    plan_id: plan.id.clone(),
                    plan_step: step.number,
                    blocked_by,
                    required_evidence: step.required_evidence.clone(),
                }),
            })
        })
        .collect()
}

pub fn reject_plan_task_collisions(
    manager: &TaskManager,
    store: &PlanStore,
    session_id: &str,
    infos: &[TaskInfo],
) -> Result<(), String> {
    let mut batch_ids = HashSet::with_capacity(infos.len());
    for info in infos {
        if !batch_ids.insert(info.id.as_str()) {
            return Err(format!(
                "plan task ID collision in pending batch: {}",
                info.id
            ));
        }
        if manager.get_task(&info.id).is_some() {
            return Err(format!(
                "task ID collision with existing manager task: {}",
                info.id
            ));
        }
    }
    let durable = store
        .load_plan_tasks(session_id)
        .map_err(|error| format!("failed to inspect durable plan task IDs: {error}"))?;
    for task in durable {
        if batch_ids.contains(task.task_id.as_str()) {
            return Err(format!(
                "task ID collision with durable plan task: {}",
                task.task_id
            ));
        }
    }
    Ok(())
}

pub fn persisted_records(infos: &[TaskInfo]) -> Result<Vec<PersistedPlanTask>, String> {
    infos
        .iter()
        .map(|info| {
            let metadata = info
                .metadata
                .as_ref()
                .ok_or_else(|| "manual task cannot be materialized as a plan task".to_string())?;
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
        })
        .collect()
}

pub fn rehydrate_plan_tasks(
    manager: &TaskManager,
    store: &PlanStore,
    authority: &archon_session::plan::PlanApprovalAuthority,
    session_id: &str,
) -> Result<usize, String> {
    let tasks = store
        .load_plan_tasks(session_id)
        .map_err(|error| error.to_string())?;
    let plans = load_canonical_materialized_plans(store, session_id)?;
    if plans.is_empty() && tasks.is_empty() {
        return Ok(0);
    }
    validate_canonical_plan_task_rows(&plans, &tasks)?;
    let infos = tasks
        .iter()
        .map(|task| task_info_from_persisted(session_id, task))
        .collect::<Result<Vec<_>, String>>()?;
    let prepared = manager
        .prepare_plan_task_rehydration(authority, session_id, store.clone(), infos)
        .map_err(|error| error.to_string())?;
    Ok(prepared.install())
}

#[path = "plan_tasks_validation.rs"]
mod validation;
use validation::{
    load_canonical_materialized_plans, plan_has_materialized_steps,
    validate_canonical_plan_task_group, validate_canonical_plan_task_rows,
};

fn task_info_from_persisted(
    session_id: &str,
    task: &PersistedPlanTask,
) -> Result<TaskInfo, String> {
    let status = parse_status(&task.status)?;
    let completed_at = if matches!(
        status,
        TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Stopped
    ) {
        DateTime::parse_from_rfc3339(&task.updated_at)
            .ok()
            .map(|time| time.with_timezone(&Utc))
    } else {
        None
    };
    Ok(TaskInfo {
        id: task.task_id.clone(),
        description: task.description.clone(),
        status,
        created_at: Utc::now(),
        completed_at,
        output: String::new(),
        cost: 0.0,
        agent_id: None,
        board_item_id: None,
        metadata: Some(PlanTaskMetadata {
            session_id: session_id.to_string(),
            plan_id: task.plan_id.clone(),
            plan_step: task.plan_step,
            blocked_by: task.blocked_by.clone(),
            required_evidence: task.required_evidence.clone(),
        }),
    })
}

pub fn plan_step_status(status: &TaskStatus) -> PlanStepStatus {
    match status {
        TaskStatus::Pending => PlanStepStatus::Pending,
        TaskStatus::Running => PlanStepStatus::InProgress,
        TaskStatus::Completed => PlanStepStatus::Complete,
        TaskStatus::Failed => PlanStepStatus::Failed,
        TaskStatus::Stopped => PlanStepStatus::Skipped,
    }
}

fn parse_status(value: &str) -> Result<TaskStatus, String> {
    match value {
        "Pending" => Ok(TaskStatus::Pending),
        "Running" => Ok(TaskStatus::Running),
        "Completed" => Ok(TaskStatus::Completed),
        "Failed" => Ok(TaskStatus::Failed),
        "Stopped" => Ok(TaskStatus::Stopped),
        _ => Err(format!("unknown persisted task status: {value}")),
    }
}

#[path = "plan_task_json.rs"]
mod plan_task_json;
pub use plan_task_json::{task_info_json, task_list_json};

#[cfg(test)]
#[path = "plan_tasks_concurrency_tests.rs"]
mod concurrency_tests;
#[cfg(test)]
#[path = "plan_tasks_identity_tests.rs"]
mod identity_tests;
#[cfg(test)]
#[path = "plan_tasks_integrity_tests.rs"]
mod integrity_tests;
#[cfg(test)]
#[path = "plan_tasks_live_fixture_tests.rs"]
mod live_fixture_tests;
#[cfg(test)]
#[path = "plan_tasks_rehydration_tests.rs"]
mod rehydration_tests;
#[cfg(test)]
#[path = "plan_tasks_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "plan_tasks_tests.rs"]
mod tests;
