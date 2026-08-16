use std::collections::BTreeMap;

use crate::plan_models::PlanStep;
use archon_completion::check_required_evidence;
use chrono::Utc;
use cozo::DataValue;

use super::{
    PersistedPlanTask, PlanDocument, PlanStepStatus, PlanStore, db_err,
    plan_store_materialization::validate_canonical_task_generation,
};

impl PlanStore {
    /// Atomically apply a validated plan-task transition and mirror its step.
    ///
    /// This is the only cross-crate status writer. It independently reloads and
    /// validates canonical durable state, dependency completion, and evidence.
    pub fn transition_plan_task_checked(
        &self,
        session_id: &str,
        task_id: &str,
        expected_status: &str,
        next_status: &str,
        evidence_run_id: &str,
        evidence_ids: &[String],
    ) -> Result<(), std::io::Error> {
        let completion_evidence = if next_status == "Completed" {
            let task = self
                .load_plan_tasks(session_id)?
                .into_iter()
                .find(|task| task.task_id == task_id)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "plan task not found")
                })?;
            let evidence = self.resolve_required_evidence(
                evidence_run_id,
                evidence_ids,
                &task.required_evidence,
            )?;
            let check = check_required_evidence(&task.required_evidence, &evidence);
            if !check.missing.is_empty() || !check.failed.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "plan task completion lacks valid required evidence",
                ));
            }
            Some((task.required_evidence, evidence))
        } else {
            None
        };
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            let mut task_params = BTreeMap::new();
            task_params.insert("sid".into(), DataValue::from(session_id));
            task_params.insert("tid".into(), DataValue::from(task_id));
            let task_rows = transaction
                .run_script(
                    "?[task_json] := *plan_tasks{session_id, task_id, task_json}, session_id = $sid, task_id = $tid",
                    task_params,
                )
                .map_err(db_err)?;
            let mut task: PersistedPlanTask = serde_json::from_str(
                task_rows
                    .rows
                    .first()
                    .and_then(|row| row.first())
                    .and_then(DataValue::get_str)
                    .ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "plan task not found")
                    })?,
            )
            .map_err(db_err)?;
            if task.status != expected_status || !valid_task_transition(&task.status, next_status) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "plan task transition does not match durable state",
                ));
            }
            let plan = self.load_plan_in(&transaction, session_id, &task.plan_id)?;
            let all_tasks = self.load_plan_tasks_in(&transaction, session_id, &plan.id)?;
            validate_canonical_task_generation(&plan, &all_tasks)?;
            let step = plan
                .steps
                .iter()
                .find(|step| step.number == task.plan_step)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "plan step not found")
                })?;
            validate_task_matches_step(&task, &plan, step)?;
            if next_status == "Running" {
                self.ensure_dependencies_completed_in(&transaction, session_id, &task.blocked_by)?;
            }
            if next_status == "Completed" {
                let (required, evidence) = completion_evidence.as_ref().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "plan task completion lacks resolved evidence",
                    )
                })?;
                if &task.required_evidence != required {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "plan task evidence requirements changed during transition",
                    ));
                }
                let check = check_required_evidence(&task.required_evidence, evidence);
                if !check.missing.is_empty() || !check.failed.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "plan task completion lacks valid required evidence",
                    ));
                }
            }
            task.status = next_status.to_string();
            task.updated_at = Utc::now().to_rfc3339();
            self.write_task_and_step_in(&transaction, session_id, &plan, &task)
        })();
        self.finish_transaction(transaction, result)
    }

    fn load_plan_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<PlanDocument, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        params.insert("pid".into(), DataValue::from(plan_id));
        let rows = transaction
            .run_script(
                "?[plan_json] := *plans{session_id, plan_id, plan_json}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?;
        PlanDocument::from_json(
            rows.rows
                .first()
                .and_then(|row| row.first())
                .and_then(DataValue::get_str)
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "plan not found")
                })?,
        )
        .map_err(db_err)
    }

    fn load_plan_tasks_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan_id: &str,
    ) -> Result<Vec<PersistedPlanTask>, std::io::Error> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        params.insert("pid".into(), DataValue::from(plan_id));
        transaction
            .run_script(
                "?[task_json] := *plan_tasks{session_id, task_id, plan_id, task_json}, session_id = $sid, plan_id = $pid",
                params,
            )
            .map_err(db_err)?
            .rows
            .iter()
            .map(|row| {
                serde_json::from_str(
                    row.first()
                        .and_then(DataValue::get_str)
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "plan task is missing JSON",
                            )
                        })?,
                )
                .map_err(db_err)
            })
            .collect()
    }

    fn ensure_dependencies_completed_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        dependencies: &[String],
    ) -> Result<(), std::io::Error> {
        for dependency in dependencies {
            let mut params = BTreeMap::new();
            params.insert("sid".into(), DataValue::from(session_id));
            params.insert("tid".into(), DataValue::from(dependency.as_str()));
            let rows = transaction
                .run_script(
                    "?[task_json] := *plan_tasks{session_id, task_id, task_json}, session_id = $sid, task_id = $tid",
                    params,
                )
                .map_err(db_err)?;
            let task: PersistedPlanTask = serde_json::from_str(
                rows.rows
                    .first()
                    .and_then(|row| row.first())
                    .and_then(DataValue::get_str)
                    .ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "dependency task missing",
                        )
                    })?,
            )
            .map_err(db_err)?;
            if task.status != "Completed" {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "plan task dependency is incomplete",
                ));
            }
        }
        Ok(())
    }

    fn write_task_and_step_in(
        &self,
        transaction: &cozo::MultiTransaction,
        session_id: &str,
        plan: &PlanDocument,
        task: &PersistedPlanTask,
    ) -> Result<(), std::io::Error> {
        let mut updated_plan = plan.clone();
        let step = updated_plan
            .steps
            .iter_mut()
            .find(|step| step.number == task.plan_step)
            .expect("validated plan step exists");
        step.status = plan_step_status(&task.status)?;
        let task_json = serde_json::to_string(task).map_err(db_err)?;
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        params.insert("pid".into(), DataValue::from(task.plan_id.as_str()));
        params.insert("tid".into(), DataValue::from(task.task_id.as_str()));
        params.insert("step".into(), DataValue::from(i64::from(task.plan_step)));
        params.insert(
            "plan".into(),
            DataValue::from(updated_plan.to_json().as_str()),
        );
        params.insert("task".into(), DataValue::from(task_json.as_str()));
        params.insert("updated".into(), DataValue::from(task.updated_at.as_str()));
        transaction
            .run_script(
                "?[session_id, plan_id, plan_json, updated_at] <- [[$sid, $pid, $plan, $updated]]
             :put plans {session_id, plan_id => plan_json, updated_at}",
                params.clone(),
            )
            .map_err(db_err)?;
        transaction.run_script(
            "?[session_id, task_id, plan_id, plan_step, task_json, updated_at] <- [[$sid, $tid, $pid, $step, $task, $updated]]
             :put plan_tasks {session_id, task_id => plan_id, plan_step, task_json, updated_at}",
            params,
        ).map_err(db_err)?;
        Ok(())
    }
}

fn valid_task_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("Pending", "Running" | "Failed" | "Stopped")
            | ("Running", "Completed" | "Failed" | "Stopped")
    )
}

fn validate_task_matches_step(
    task: &PersistedPlanTask,
    plan: &PlanDocument,
    step: &PlanStep,
) -> Result<(), std::io::Error> {
    let dependencies = step
        .blocked_by
        .iter()
        .map(|number| {
            plan.steps
                .iter()
                .find(|candidate| candidate.number == *number)
                .and_then(|dependency| dependency.task_id.as_deref())
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid step dependency")
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if task.plan_id != plan.id
        || step.task_id.as_deref() != Some(task.task_id.as_str())
        || task.description != step.description
        || task.required_evidence != step.required_evidence
        || task
            .blocked_by
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != dependencies
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "durable plan task does not match canonical plan step",
        ));
    }
    Ok(())
}

fn plan_step_status(status: &str) -> Result<PlanStepStatus, std::io::Error> {
    match status {
        "Pending" => Ok(PlanStepStatus::Pending),
        "Failed" => Ok(PlanStepStatus::Failed),
        "Stopped" => Ok(PlanStepStatus::Skipped),
        "Running" => Ok(PlanStepStatus::InProgress),
        "Completed" => Ok(PlanStepStatus::Complete),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unknown task status",
        )),
    }
}
