//! Collect every independent skeleton shape defect before returning to its author.
use super::*;
use crate::defect::{ValidationDefect, defect_message};

pub fn skeleton_defects(skeleton: &TaskSkeleton, expected_digest: &str) -> Vec<ValidationDefect> {
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
    })
}
