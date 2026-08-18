use std::collections::HashMap;

use archon_session::plan::{PersistedPlanTask, PlanDocument, PlanStatus, PlanStep};

use super::{PlanStore, parse_status, plan_step_status};

pub(super) fn load_canonical_materialized_plans(
    store: &PlanStore,
    session_id: &str,
) -> Result<Vec<PlanDocument>, String> {
    store
        .load_plans(session_id)
        .map_err(|error| error.to_string())
        .map(|plans| {
            plans
                .into_iter()
                .filter(plan_has_materialized_steps)
                .collect()
        })
}

pub(super) fn plan_has_materialized_steps(plan: &PlanDocument) -> bool {
    matches!(
        plan.status,
        PlanStatus::Approved | PlanStatus::Executing | PlanStatus::Completed
    ) && plan.steps.iter().any(|step| step.task_id.is_some())
}

pub(super) fn validate_canonical_plan_task_rows(
    plans: &[PlanDocument],
    tasks: &[PersistedPlanTask],
) -> Result<(), String> {
    let plan_ids = plans
        .iter()
        .map(|plan| plan.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if plan_ids.len() != plans.len() {
        return Err("canonical materialized plans contain duplicate IDs".to_string());
    }
    for task in tasks {
        if !plan_ids.contains(task.plan_id.as_str()) {
            return Err(format!(
                "durable task {} does not belong to an active materialized plan",
                task.task_id
            ));
        }
    }
    for plan in plans {
        let plan_tasks = tasks
            .iter()
            .filter(|task| task.plan_id == plan.id)
            .cloned()
            .collect::<Vec<_>>();
        validate_canonical_plan_task_group(plan, &plan_tasks)?;
    }
    Ok(())
}

pub(super) fn validate_canonical_plan_task_group(
    plan: &PlanDocument,
    tasks: &[PersistedPlanTask],
) -> Result<(), String> {
    let task_by_id = indexed_durable_tasks(tasks)?;
    let step_by_number = indexed_plan_steps(plan)?;
    validate_materialized_step_count(&step_by_number, tasks, plan)?;
    for task in tasks {
        validate_durable_task_row(plan, &step_by_number, task)?;
    }
    validate_durable_rows_present(plan, &task_by_id)
}

fn indexed_durable_tasks(
    tasks: &[PersistedPlanTask],
) -> Result<HashMap<&str, &PersistedPlanTask>, String> {
    let indexed = tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect::<HashMap<_, _>>();
    if indexed.len() == tasks.len() {
        Ok(indexed)
    } else {
        Err("canonical plan task rows contain duplicate task IDs".to_string())
    }
}

fn indexed_plan_steps(plan: &PlanDocument) -> Result<HashMap<u32, &PlanStep>, String> {
    let indexed = plan
        .steps
        .iter()
        .map(|step| (step.number, step))
        .collect::<HashMap<_, _>>();
    if indexed.len() == plan.steps.len() {
        Ok(indexed)
    } else {
        Err("canonical plan contains duplicate step numbers".to_string())
    }
}

fn validate_materialized_step_count(
    steps: &HashMap<u32, &PlanStep>,
    tasks: &[PersistedPlanTask],
    plan: &PlanDocument,
) -> Result<(), String> {
    if steps.len() != tasks.len() {
        return Err("canonical plan task rows do not match materialized step count".to_string());
    }
    for step in &plan.steps {
        if step.task_id.is_none() {
            return Err(format!(
                "canonical plan {} step {} has no materialized task ID",
                plan.id, step.number
            ));
        }
    }
    Ok(())
}

fn validate_durable_task_row(
    plan: &PlanDocument,
    steps: &HashMap<u32, &PlanStep>,
    task: &PersistedPlanTask,
) -> Result<(), String> {
    let step = steps.get(&task.plan_step).ok_or_else(|| {
        format!(
            "canonical plan {} is missing durable task step {}",
            plan.id, task.plan_step
        )
    })?;
    let expected_id = step
        .task_id
        .as_deref()
        .expect("validated materialized step");
    let dependencies = expected_dependency_task_ids(plan, step, steps)?;
    let status = parse_status(&task.status)?;
    if task.plan_id == plan.id
        && task.task_id == expected_id
        && task.description == step.description
        && task.blocked_by == dependencies
        && task.required_evidence == step.required_evidence
        && plan_step_status(&status) == step.status
    {
        Ok(())
    } else {
        Err(format!(
            "canonical plan {} disagrees with durable task {}",
            plan.id, task.task_id
        ))
    }
}

fn expected_dependency_task_ids(
    plan: &PlanDocument,
    step: &PlanStep,
    steps: &HashMap<u32, &PlanStep>,
) -> Result<Vec<String>, String> {
    step.blocked_by
        .iter()
        .map(|number| {
            steps
                .get(number)
                .and_then(|dependency| dependency.task_id.clone())
                .ok_or_else(|| {
                    format!(
                        "canonical plan {} step {} has unresolved dependency {number}",
                        plan.id, step.number
                    )
                })
        })
        .collect()
}

fn validate_durable_rows_present(
    plan: &PlanDocument,
    tasks: &HashMap<&str, &PersistedPlanTask>,
) -> Result<(), String> {
    for step in &plan.steps {
        let task_id = step
            .task_id
            .as_deref()
            .expect("validated materialized step");
        if !tasks.contains_key(task_id) {
            return Err(format!(
                "canonical plan {} step {} has no durable task row",
                plan.id, step.number
            ));
        }
    }
    Ok(())
}
