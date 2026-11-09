//! Collect every independent skeleton shape defect before returning to its author.
use super::*;
use crate::defect::{ValidationDefect, defect_message};

pub fn skeleton_defects(skeleton: &TaskSkeleton, expected_digest: &str) -> Vec<ValidationDefect> {
    skeleton_validation_defects(skeleton, expected_digest, true)
}

pub(crate) fn skeleton_shape_defects(
    skeleton: &TaskSkeleton,
    expected_digest: &str,
) -> Vec<ValidationDefect> {
    skeleton_validation_defects(skeleton, expected_digest, false)
}

fn skeleton_validation_defects(
    skeleton: &TaskSkeleton,
    expected_digest: &str,
    audit_contracts: bool,
) -> Vec<ValidationDefect> {
    let mut defects = Vec::new();
    if skeleton.schema_version != 1 {
        defects.push(ValidationDefect::new("invalid_schema_version", "skeleton", "schema_version", format!(
            "task skeleton schema_version must be 1, found {}; set it to 1 and re-run `workflow freeze-skeleton`", skeleton.schema_version)));
    }
    if skeleton.acceptance_digest != expected_digest {
        defects.push(ValidationDefect::new("acceptance_digest_mismatch", "skeleton", "acceptance_digest", format!(
            "task skeleton acceptance_digest is {}, expected {}; restore the acceptance-bound skeleton or re-run `workflow freeze-skeleton`", skeleton.acceptance_digest, expected_digest)));
    }
    if skeleton.tasks.is_empty() {
        defects.push(ValidationDefect::new("empty_task_set", "skeleton", "tasks",
            "task skeleton examined zero tasks; add every future TASK entry before running `workflow freeze-skeleton`"));
    }
    defects.extend(super::states::defects(skeleton));
    let mut ids = BTreeSet::new();
    let mut files = BTreeSet::new();
    for (index, task) in skeleton.tasks.iter().enumerate() {
        // A rejected id never enters a stable identity; use the entry's structural slot.
        let subject = skeleton_subject(&task.task_id, index);
        if !strict_task_id(&task.task_id) {
            defects.push(ValidationDefect::new(
                "invalid_task_id",
                &subject,
                "task_id",
                format!(
                    "task skeleton task_id '{}' is invalid; use canonical TASK-<DOMAIN>-<NNN>",
                    task.task_id
                ),
            ));
        }
        if !ids.insert(task.task_id.clone()) {
            defects.push(ValidationDefect::new(
                "duplicate_task_id",
                &subject,
                &format!("tasks/{index}/task_id"),
                format!(
                    "task skeleton task_id '{}' is duplicated; keep exactly one entry",
                    task.task_id
                ),
            ));
        }
        if !files.insert(task.file_name.clone()) {
            defects.push(ValidationDefect::new("duplicate_filename", &subject, "file_name", format!(
                "task skeleton file_name '{}' is duplicated; give each task one distinct TASK-*.md filename", task.file_name)));
        }
        if audit_contracts {
            for (index, contract) in task.deliverable_contracts.iter().enumerate() {
                defects.extend(
                    crate::task_universe_contract_audit::contract_defects(
                        &subject, index, contract,
                    )
                    .into_iter()
                    .map(|mut defect| {
                        defect.message =
                            format!("task-skeleton.json task {subject}: {}", defect.message);
                        defect
                    }),
                );
            }
        }
        let file = Path::new(&task.file_name);
        if file
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
            || task.file_name.contains('\\')
            || !task.file_name.starts_with(&task.task_id)
            || !task.file_name.ends_with(".md")
        {
            defects.push(ValidationDefect::new("invalid_filename", &subject, "file_name", format!(
                "task '{}' file_name '{}' must be a direct TASK-*.md filename beginning with the canonical task id; remove directory components or rename it before freezing", task.task_id, task.file_name)));
        }
    }
    defects
}

pub fn validate_skeleton(skeleton: &TaskSkeleton, expected_digest: &str) -> SkeletonResult<()> {
    let defects = skeleton_defects(skeleton, expected_digest);
    if defects.is_empty() {
        return Ok(());
    }
    Err(TaskSkeletonError {
        message: defect_message(&defects),
        defects,
        io_cause: None,
        io_kind: None,
    })
}

/// New candidates must be canonical; frozen records remain readable for repair.
pub(crate) fn entry_findings(skeleton: &TaskSkeleton) -> Vec<TaskSetFinding> {
    let mut findings = Vec::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for (field, values) in [
            (
                "implements",
                task.implements
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            ),
            (
                "source_sections",
                task.source_sections.iter().map(String::as_str).collect(),
            ),
            ("blocks", task.blocks.iter().map(String::as_str).collect()),
            (
                "depends_on",
                task.depends_on
                    .iter()
                    .map(|edge| edge.task_id.as_str())
                    .collect(),
            ),
        ] {
            let mut seen = BTreeSet::new();
            for value in values {
                let code = if value.is_empty() || value.trim() != value {
                    Some("untrimmed_entry")
                } else if !seen.insert(value) {
                    Some("duplicate_entry")
                } else {
                    None
                };
                if let Some(code) = code {
                    let message = if code == "duplicate_entry" {
                        format!(
                            "task {} lists '{value}' twice in {field}; list each value once",
                            task.task_id
                        )
                    } else {
                        format!(
                            "task {} lists an empty or untrimmed value '{value}' in {field}; use a non-empty value without leading or trailing whitespace",
                            task.task_id
                        )
                    };
                    findings.push(TaskSetFinding {
                        identity: crate::defect::DeterministicDefect::new(
                            code,
                            skeleton_subject(&task.task_id, slot),
                            format!("{field}/{value}"),
                        ),
                        field: field.into(),
                        message,
                    });
                }
            }
        }
    }
    findings
}
