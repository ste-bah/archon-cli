use crate::task_manager::TaskManager;
use archon_session::plan::{
    PlanApprovalAuthority, PlanApprovalRecord, PlanDocument, PlanStatus, PlanStore,
};
use std::sync::Arc;

use super::{
    build_plan_task_infos, persisted_records, plan_has_materialized_steps,
    reject_plan_task_collisions, validate_canonical_plan_task_group,
    wait_for_materialization_barrier_for_test,
};

pub fn materialize_plan_tasks(
    manager: &TaskManager,
    store: &PlanStore,
    authority: &Arc<PlanApprovalAuthority>,
    session_id: &str,
    plan: &mut PlanDocument,
) -> Result<Vec<String>, String> {
    if !matches!(plan.status, PlanStatus::Approved | PlanStatus::Executing) {
        return Err(format!(
            "plan {} cannot materialize tasks from status {:?}",
            plan.id, plan.status
        ));
    }
    if let Some(canonical) = store
        .load_plan(session_id, &plan.id)
        .map_err(|error| error.to_string())?
        .filter(plan_has_materialized_steps)
    {
        let ids = canonical_materialization_ids(store, session_id, &canonical)?;
        *plan = canonical;
        return Ok(ids);
    }
    let mut candidate = plan.clone();
    let infos = build_plan_task_infos(session_id, &mut candidate)?;
    let records = persisted_records(&infos)?;
    let approval = candidate.approval.clone().ok_or_else(|| {
        format!(
            "plan {} requires an approving decision before task materialization",
            candidate.id
        )
    })?;
    let record = PlanApprovalRecord {
        plan_id: candidate.id.clone(),
        session_id: session_id.into(),
        approval,
    };
    reject_plan_task_collisions(manager, store, session_id, &infos)?;
    wait_for_materialization_barrier_for_test(session_id);
    let prepared = manager
        .prepare_plan_task_installation(authority, session_id, store.clone(), infos)
        .map_err(|error| error.to_string())?;
    store
        .save_terminal_plan_with_approval_and_tasks(
            authority, session_id, &candidate, &record, &records,
        )
        .or_else(|error| {
            let canonical = store.load_plan(session_id, &candidate.id)?;
            if canonical
                .as_ref()
                .is_some_and(|stored| stored.to_json() == candidate.to_json())
                && canonical_materialization_ids(store, session_id, &candidate).is_ok()
            {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|error| error.to_string())?;
    let ids = records.into_iter().map(|record| record.task_id).collect();
    *plan = candidate;
    prepared.install();
    Ok(ids)
}

fn canonical_materialization_ids(
    store: &PlanStore,
    session_id: &str,
    plan: &PlanDocument,
) -> Result<Vec<String>, String> {
    let tasks = store
        .load_plan_tasks(session_id)
        .map_err(|error| error.to_string())?;
    let plan_tasks = tasks
        .into_iter()
        .filter(|task| task.plan_id == plan.id)
        .collect::<Vec<_>>();
    validate_canonical_plan_task_group(plan, &plan_tasks)?;
    Ok(plan
        .steps
        .iter()
        .map(|step| step.task_id.clone().expect("validated materialized step"))
        .collect())
}
