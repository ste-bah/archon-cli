use super::*;
pub fn compare_frozen_task(
    task: &WorkflowV2TaskUniverseTask,
    frozen: &FrozenTask,
) -> Vec<TaskSetFinding> {
    let mut findings = Vec::new();
    compare(
        &mut findings,
        "task_id",
        &task.canonical_task_id,
        &frozen.task_id,
    );
    let actual_file = Path::new(&task.source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    compare(
        &mut findings,
        "file_name",
        &actual_file.to_string(),
        &frozen.file_name,
    );
    compare_set(
        &mut findings,
        "depends_on",
        task.dependencies.clone(),
        frozen.depends_on.clone(),
    );
    compare_strings(
        &mut findings,
        "blocks",
        task.blocks_ids.clone(),
        frozen.blocks.clone(),
    );
    compare_strings(
        &mut findings,
        "implements",
        task.implements.clone(),
        frozen.implements.clone(),
    );
    compare_strings(
        &mut findings,
        "source_sections",
        task.source_sections.clone(),
        frozen.source_sections.clone(),
    );
    compare_set(
        &mut findings,
        "deliverable_contracts",
        task.deliverable_contracts.clone(),
        frozen.deliverable_contracts.clone(),
    );
    if frozen.status.is_some() || task.declared_status_is_complete() {
        compare(&mut findings, "status", &task.status, &frozen.status);
    }
    compare(
        &mut findings,
        "requirement_states",
        &task.requirement_states,
        &frozen
            .requirement_states
            .iter()
            .map(|(id, d)| (id.clone(), d.state.as_str().into()))
            .collect(),
    );
    for finding in &mut findings {
        finding.identity.subject.clone_from(&frozen.task_id);
    }
    findings
}

pub fn compare_task_set(
    tasks: &[WorkflowV2TaskUniverseTask],
    skeleton: &TaskSkeleton,
) -> Vec<TaskSetFinding> {
    let mut findings = Vec::new();
    let actual_ids: BTreeSet<_> = tasks
        .iter()
        .map(|task| task.canonical_task_id.as_str())
        .collect();
    let frozen_ids: BTreeSet<_> = skeleton
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect();
    for missing in frozen_ids.difference(&actual_ids) {
        findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("missing_frozen_task", *missing, "tasks"),
            field: "tasks".into(),
            message: format!(
                "frozen task '{}' has no TASK file; create its frozen file_name or re-run `workflow freeze-skeleton` before body writing",
                missing
            ),
        });
    }
    for (index, extra) in actual_ids.difference(&frozen_ids).enumerate() {
        findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("extra_task", "task set", format!("tasks/{index}")),
            field: "tasks".into(),
            message: format!(
                "TASK file '{}' is absent from the frozen skeleton; remove it or re-run `workflow freeze-skeleton` before body writing",
                extra
            ),
        });
    }
    for task in tasks {
        if let Some(frozen) = skeleton
            .tasks
            .iter()
            .find(|frozen| frozen.task_id == task.canonical_task_id)
        {
            findings.extend(compare_frozen_task(task, frozen));
        }
    }
    findings
}

fn compare<T: PartialEq + fmt::Debug>(
    findings: &mut Vec<TaskSetFinding>,
    field: &str,
    actual: &T,
    frozen: &T,
) {
    if actual != frozen {
        findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("frozen_field_changed", "frozen task", field),
            field: field.into(),
            message: format!(
                "frozen field '{field}' changed: expected {frozen:?}, actual {actual:?}; restore the frozen value or re-run `workflow freeze-skeleton` before body writing"
            ),
        });
    }
}

pub(super) fn compare_set<T: Ord + fmt::Debug>(
    findings: &mut Vec<TaskSetFinding>,
    field: &str,
    mut actual: Vec<T>,
    mut frozen: Vec<T>,
) {
    actual.sort();
    actual.dedup();
    frozen.sort();
    frozen.dedup();
    compare(findings, field, &actual, &frozen);
}

pub(super) fn compare_strings(
    findings: &mut Vec<TaskSetFinding>,
    field: &str,
    actual: Vec<String>,
    frozen: Vec<String>,
) {
    // Match the task parser's sorted_unique: trim, discard empty, de-duplicate.
    let normalize = |values: Vec<String>| {
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    compare_set(findings, field, normalize(actual), normalize(frozen));
}
