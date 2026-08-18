use std::path::{Component, Path};

use archon_completion::check_required_evidence;

use crate::plan_models::{PlanReconciliationStatus, PlanStep, PlanStepReconciliation};

use super::{PersistedPlanTask, PlanDocument};

pub fn reconcile_durable_plan(
    plan: &PlanDocument,
    tasks: &[PersistedPlanTask],
) -> Vec<PlanStepReconciliation> {
    let approved_files = plan
        .steps
        .iter()
        .flat_map(|step| step.affected_files.iter())
        .collect::<Vec<_>>();
    let mut reconciliation = plan
        .steps
        .iter()
        .map(|step| durable_step_reconciliation(step, tasks))
        .collect::<Vec<_>>();
    if let Some(failure) = &plan.execution_evidence.observation_failure {
        reconciliation.push(PlanStepReconciliation {
            step: None,
            status: PlanReconciliationStatus::Deviated,
            detail: format!("filesystem observation incomplete: {failure}"),
        });
    }
    reconciliation.extend(
        plan.execution_evidence
            .touched_files
            .iter()
            .filter(|file| {
                !approved_files
                    .iter()
                    .any(|approved| same_file(approved, file))
            })
            .map(|file| PlanStepReconciliation {
                step: None,
                status: PlanReconciliationStatus::UnplannedExtra,
                detail: format!("touched unplanned file: {file}"),
            }),
    );
    reconciliation
}

pub fn reconciliation_summary(entries: &[PlanStepReconciliation]) -> Option<String> {
    let omitted = reconciliation_count(entries, PlanReconciliationStatus::Omitted);
    let deviated = reconciliation_count(entries, PlanReconciliationStatus::Deviated);
    let extras = reconciliation_count(entries, PlanReconciliationStatus::UnplannedExtra);
    (omitted + deviated + extras > 0).then(|| {
        format!("Plan reconciliation: {omitted} omitted, {deviated} deviated, {extras} unplanned extras.")
    })
}

fn durable_step_reconciliation(
    step: &PlanStep,
    tasks: &[PersistedPlanTask],
) -> PlanStepReconciliation {
    let status = step
        .task_id
        .as_deref()
        .and_then(|task_id| tasks.iter().find(|task| task.task_id == task_id))
        .map_or(PlanReconciliationStatus::Omitted, |task| {
            match task.status.as_str() {
                "Completed" if task_has_required_evidence(task) => {
                    PlanReconciliationStatus::Completed
                }
                "Completed" | "Failed" | "Stopped" => PlanReconciliationStatus::Deviated,
                _ => PlanReconciliationStatus::Omitted,
            }
        });
    PlanStepReconciliation {
        step: Some(step.number),
        detail: durable_reconciliation_detail(status),
        status,
    }
}

fn durable_reconciliation_detail(status: PlanReconciliationStatus) -> String {
    match status {
        PlanReconciliationStatus::Completed => {
            "canonical task completed with required durable evidence".into()
        }
        PlanReconciliationStatus::Deviated => {
            "canonical task reached terminal status without required durable evidence".into()
        }
        PlanReconciliationStatus::Omitted => "canonical plan task is not completed".into(),
        PlanReconciliationStatus::UnplannedExtra => unreachable!("not a task status"),
    }
}

fn task_has_required_evidence(task: &PersistedPlanTask) -> bool {
    let check = check_required_evidence(&task.required_evidence, &task.completion_evidence);
    check.missing.is_empty() && check.failed.is_empty()
}

fn reconciliation_count(
    entries: &[PlanStepReconciliation],
    status: PlanReconciliationStatus,
) -> usize {
    entries
        .iter()
        .filter(|entry| entry.status == status)
        .count()
}

fn same_file(approved: &str, actual: &str) -> bool {
    normalized_relative_path(approved)
        .zip(normalized_relative_path(actual))
        .is_some_and(|(approved, actual)| approved == actual)
}

fn normalized_relative_path(path: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return None;
    }
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}
