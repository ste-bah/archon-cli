use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
#[cfg(test)]
use std::sync::{Arc, Barrier, LazyLock, Mutex};

use cozo::DataValue;

use crate::plan_models::{PlanApproval, PlanApprovalDecision, PlanStatus, PlanStepStatus};

use super::{PersistedPlanTask, PlanApprovalAuthority, PlanDocument, PlanStore, PlanWrite, db_err};

#[cfg(test)]
struct LegacyAdoptionBarrier {
    validated: Arc<Barrier>,
    resume: Arc<Barrier>,
}

#[cfg(test)]
static LEGACY_ADOPTION_BARRIER: LazyLock<Mutex<Option<LegacyAdoptionBarrier>>> =
    LazyLock::new(|| Mutex::new(None));
#[cfg(test)]
static LEGACY_ADOPTION_TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(test)]
pub(crate) struct LegacyAdoptionBarrierReset {
    _serial: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for LegacyAdoptionBarrierReset {
    fn drop(&mut self) {
        *LEGACY_ADOPTION_BARRIER
            .lock()
            .expect("legacy adoption barrier lock") = None;
    }
}

#[cfg(test)]
impl PlanStore {
    pub(crate) fn set_legacy_adoption_barrier_for_test(
        validated: Arc<Barrier>,
        resume: Arc<Barrier>,
    ) -> LegacyAdoptionBarrierReset {
        let serial = LEGACY_ADOPTION_TEST_SERIAL
            .lock()
            .expect("legacy adoption test serial lock");
        *LEGACY_ADOPTION_BARRIER
            .lock()
            .expect("legacy adoption barrier lock") =
            Some(LegacyAdoptionBarrier { validated, resume });
        LegacyAdoptionBarrierReset { _serial: serial }
    }
}

#[cfg(test)]
fn wait_for_legacy_adoption_barrier_for_test() {
    let barriers = LEGACY_ADOPTION_BARRIER
        .lock()
        .expect("legacy adoption barrier lock")
        .as_ref()
        .map(|barrier| (Arc::clone(&barrier.validated), Arc::clone(&barrier.resume)));
    if let Some((validated, resume)) = barriers {
        validated.wait();
        resume.wait();
    }
}

#[cfg(not(test))]
fn wait_for_legacy_adoption_barrier_for_test() {}

impl PlanStore {
    /// Atomically claim a plan's task generation and persist canonical task rows.
    pub fn claim_plan_materialization_with_tasks(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        plan: &PlanDocument,
        tasks: &[PersistedPlanTask],
    ) -> Result<(), std::io::Error> {
        validate_canonical_task_generation(plan, tasks)?;
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            self.require_authority_in(&transaction, authority, session_id)?;
            self.ensure_durable_approval_in(&transaction, session_id, plan)?;
            if self.has_materialization_claim_in(&transaction, session_id, &plan.id)? {
                return self.ensure_matching_materialization_in(
                    &transaction,
                    session_id,
                    plan,
                    tasks,
                );
            }
            PlanWrite::materialization_claim(session_id, plan)?.run(&transaction)?;
            PlanWrite::plan(session_id, plan)?.run(&transaction)?;
            tasks
                .iter()
                .try_for_each(|task| PlanWrite::task(session_id, task)?.run(&transaction))
        })();
        self.finish_transaction(transaction, result)
    }

    /// Save a canonical plan/task generation. Existing coherent generations are
    /// adopted once, then idempotent; distinct generations are rejected.
    pub fn save_plan_with_tasks(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        plan: &PlanDocument,
        tasks: &[PersistedPlanTask],
    ) -> Result<(), std::io::Error> {
        validate_canonical_task_generation(plan, tasks)?;
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            self.require_authority_in(&transaction, authority, session_id)?;
            self.ensure_durable_approval_in(&transaction, session_id, plan)?;
            if self.has_materialization_claim_in(&transaction, session_id, &plan.id)? {
                return self.ensure_matching_materialization_in(
                    &transaction,
                    session_id,
                    plan,
                    tasks,
                );
            }
            if self.has_durable_tasks_for_plan_in(&transaction, session_id, &plan.id)? {
                self.ensure_matching_materialization_in(&transaction, session_id, plan, tasks)?;
                wait_for_legacy_adoption_barrier_for_test();
                return PlanWrite::materialization_claim(session_id, plan)?.run(&transaction);
            }
            PlanWrite::materialization_claim(session_id, plan)?.run(&transaction)?;
            PlanWrite::plan(session_id, plan)?.run(&transaction)?;
            tasks
                .iter()
                .try_for_each(|task| PlanWrite::task(session_id, task)?.run(&transaction))
        })();
        self.finish_transaction(transaction, result)
    }

    pub(super) fn save_unmaterialized_plan(
        &self,
        session_id: &str,
        plan: &PlanDocument,
    ) -> Result<(), std::io::Error> {
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            self.ensure_plan_unclaimed(&transaction, session_id, &plan.id)?;
            PlanWrite::plan(session_id, plan)?.run(&transaction)
        })();
        self.finish_transaction(transaction, result)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn save_unmaterialized_plan_task(
        &self,
        session_id: &str,
        task: &PersistedPlanTask,
    ) -> Result<(), std::io::Error> {
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            self.ensure_plan_unclaimed(&transaction, session_id, &task.plan_id)?;
            PlanWrite::task(session_id, task)?.run(&transaction)
        })();
        self.finish_transaction(transaction, result)
    }

    pub(super) fn finish_transaction(
        &self,
        transaction: cozo::MultiTransaction,
        result: Result<(), std::io::Error>,
    ) -> Result<(), std::io::Error> {
        if result.is_ok() {
            transaction.commit().map_err(db_err)?;
        } else {
            let _ = transaction.abort();
        }
        result
    }

    pub(super) fn ensure_plan_unclaimed(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<(), std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let rows = transaction
            .run_script(
                "?[plan_id] := *plan_materializations{session_id, plan_id}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?;
        if rows.rows.is_empty() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "materialized canonical plan may only be updated with its task status transaction",
            ))
        }
    }

    fn ensure_durable_approval_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan: &PlanDocument,
    ) -> Result<(), std::io::Error> {
        if !matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "materialization requires an approved plan lifecycle",
            ));
        }
        let canonical = self.load_durable_plan_in(transaction, session_id, &plan.id)?;
        if canonical.to_json() != plan.to_json() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "materialization requires the exact durable approved plan",
            ));
        }
        let approval = plan.approval.as_ref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "materialization requires a plan approval",
            )
        })?;
        ensure_approving_decision(approval)?;
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan.id.as_str()));
        let rows = transaction
            .run_script(
                "?[approval_json] := *plan_approval_events{session_id, plan_id, approval_json}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?;
        let is_durable = rows.rows.iter().any(|row| {
            row.first()
                .and_then(DataValue::get_str)
                .and_then(|json| serde_json::from_str::<PlanApproval>(json).ok())
                .is_some_and(|stored| {
                    stored == *approval && is_approving_decision(&stored.decision)
                })
        });
        if is_durable {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "materialization requires a matching durable approval record",
            ))
        }
    }

    fn load_durable_plan_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<PlanDocument, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let rows = transaction
            .run_script(
                "?[plan_json] := *plans{session_id, plan_id, plan_json}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?;
        let plan_json = rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "materialization requires a durable approved plan",
                )
            })?;
        PlanDocument::from_json(plan_json)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }

    fn has_materialization_claim_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<bool, std::io::Error> {
        self.relation_row_exists_in(transaction, "plan_materializations", session_id, plan_id)
    }

    fn has_durable_tasks_for_plan_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<bool, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let rows = transaction
            .run_script(
                "?[task_id] := *plan_tasks{session_id, task_id, plan_id}, session_id = $sid, plan_id = $pid :limit 1",
                params,
            )
            .map_err(db_err)?;
        Ok(!rows.rows.is_empty())
    }

    fn relation_row_exists_in(
        &self,
        transaction: &cozo::MultiTransaction,
        relation: &str,
        session_id: &str,
        plan_id: &str,
    ) -> Result<bool, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan_id));
        let rows = transaction
            .run_script(
                &format!(
                    "?[plan_id] := *{relation}{{session_id, plan_id}}, session_id = $sid, plan_id = $pid :limit 1"
                ),
                params,
            )
            .map_err(db_err)?;
        Ok(!rows.rows.is_empty())
    }

    fn ensure_matching_materialization_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan: &PlanDocument,
        tasks: &[PersistedPlanTask],
    ) -> Result<(), std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".to_string(), DataValue::from(session_id));
        params.insert("pid".to_string(), DataValue::from(plan.id.as_str()));
        let plan_rows = transaction
            .run_script(
                "?[plan_json] := *plans{session_id, plan_id, plan_json}, session_id = $sid, plan_id = $pid",
                params.clone(),
            )
            .map_err(db_err)?;
        let plan_json = plan_rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "materialization has no canonical plan",
                )
            })?;
        let existing = PlanDocument::from_json(plan_json)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let task_rows = transaction
            .run_script(
                "?[task_json] := *plan_tasks{session_id, task_id, plan_id, task_json}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?;
        let existing_tasks = task_rows
            .rows
            .iter()
            .map(|row| serde_json::from_str(row[0].get_str().unwrap_or("")).map_err(db_err))
            .collect::<Result<Vec<PersistedPlanTask>, _>>()?;
        if existing.to_json() == plan.to_json()
            && task_json_set(&existing_tasks)? == task_json_set(tasks)?
        {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "materialization generation already exists with different canonical plan or tasks",
            ))
        }
    }
}

pub(super) fn validate_canonical_task_generation(
    plan: &PlanDocument,
    tasks: &[PersistedPlanTask],
) -> Result<(), std::io::Error> {
    if !matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
        return Err(invalid_generation("plan must be approved or executing"));
    }
    if plan.steps.is_empty() || tasks.len() != plan.steps.len() {
        return Err(invalid_generation(
            "tasks must map one-to-one to every plan step",
        ));
    }
    let mut steps = HashMap::new();
    for step in &plan.steps {
        if steps.insert(step.number, step).is_some() {
            return Err(invalid_generation("plan has duplicate step numbers"));
        }
    }
    let mut task_ids = HashSet::new();
    let mut task_steps = HashSet::new();
    for task in tasks {
        if !task_ids.insert(task.task_id.as_str()) || !task_steps.insert(task.plan_step) {
            return Err(invalid_generation(
                "tasks have duplicate IDs or step mappings",
            ));
        }
        let step = steps
            .get(&task.plan_step)
            .ok_or_else(|| invalid_generation("task maps to an unknown plan step"))?;
        let expected_task_id = step
            .task_id
            .as_deref()
            .ok_or_else(|| invalid_generation("every plan step must have a task ID"))?;
        let expected_dependencies = step
            .blocked_by
            .iter()
            .map(|number| {
                steps
                    .get(number)
                    .and_then(|dependency| dependency.task_id.as_deref())
                    .ok_or_else(|| invalid_generation("step dependency has no canonical task ID"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if task.plan_id != plan.id
            || task.task_id != expected_task_id
            || task.description != step.description
            || task.status != canonical_task_status(step.status)
            || task.required_evidence != step.required_evidence
            || task
                .blocked_by
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != expected_dependencies
        {
            return Err(invalid_generation(
                "task does not exactly match its canonical plan step",
            ));
        }
    }
    Ok(())
}

pub(super) fn ensure_approving_decision(approval: &PlanApproval) -> Result<(), std::io::Error> {
    if is_approving_decision(&approval.decision) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "materialization requires an approving plan decision",
        ))
    }
}

fn is_approving_decision(decision: &PlanApprovalDecision) -> bool {
    matches!(
        decision,
        PlanApprovalDecision::Approve | PlanApprovalDecision::ApproveAcceptEdits
    )
}

fn canonical_task_status(status: PlanStepStatus) -> &'static str {
    match status {
        PlanStepStatus::Pending => "Pending",
        PlanStepStatus::InProgress => "Running",
        PlanStepStatus::Complete => "Completed",
        PlanStepStatus::Skipped => "Stopped",
        PlanStepStatus::Failed => "Failed",
    }
}

fn invalid_generation(message: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("invalid canonical task generation: {message}"),
    )
}
fn task_json_set(tasks: &[PersistedPlanTask]) -> Result<BTreeSet<String>, std::io::Error> {
    tasks
        .iter()
        .map(|task| serde_json::to_string(task).map_err(db_err))
        .collect()
}
