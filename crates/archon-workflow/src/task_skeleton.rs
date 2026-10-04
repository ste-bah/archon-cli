//! Frozen set-level fields for a decomposed task directory.
//!
//! TASK files remain the runtime source. This module records what the runtime
//! parser understood before body writers fan out and compares those parsed
//! values after writing; it never creates a runtime task universe from JSON.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, AcceptancePin, FreezeGateStamp,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest, validate_gate_stamp,
};
use crate::task_universe::{WorkflowV2DeliverableContract, WorkflowV2TaskUniverseTask};

#[path = "task_skeleton_graph.rs"]
mod graph;

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConsumedArtifact {
    pub artifact_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FrozenDependency {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub consumes: Vec<ConsumedArtifact>,
    #[serde(default)]
    pub ordering_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSkeleton {
    pub schema_version: u32,
    pub acceptance_digest: String,
    pub tasks: Vec<FrozenTask>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenTask {
    pub task_id: String,
    pub file_name: String,
    #[serde(default)]
    pub depends_on: Vec<FrozenDependency>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub implements: Vec<String>,
    #[serde(default)]
    pub deliverable_contracts: Vec<WorkflowV2DeliverableContract>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSkeletonLock {
    pub algorithm: String,
    pub digest: String,
    pub acceptance_digest: String,
    pub gate: FreezeGateStamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSetFinding {
    pub identity: crate::defect::DeterministicDefect,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSkeletonError {
    message: String,
    pub defects: Vec<crate::defect::ValidationDefect>,
}

impl fmt::Display for TaskSkeletonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TaskSkeletonError {}

type SkeletonResult<T> = Result<T, TaskSkeletonError>;

#[path = "task_skeleton_validate.rs"]
mod validate;
pub use validate::{skeleton_defects, validate_skeleton};

pub fn validate_skeleton_set(
    skeleton: &TaskSkeleton,
    expected_obligations: &BTreeSet<String>,
) -> Vec<TaskSetFinding> {
    let ids: BTreeSet<_> = skeleton
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect();
    let mut findings = Vec::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for (index, dependency) in task.depends_on.iter().enumerate() {
            if !ids.contains(dependency.task_id.as_str()) {
                findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("missing_dependency", skeleton_subject(&task.task_id, slot), format!("depends_on/{index}")),
                    field: "depends_on".into(),
                    message: format!(
                        "task '{}' depends_on missing task '{}'; add the missing task to the skeleton or remove the edge",
                        task.task_id, dependency.task_id
                    ),
                });
            }
        }
        for (index, blocked) in task.blocks.iter().enumerate() {
            if !ids.contains(blocked.as_str()) {
                findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("missing_blocked_task", skeleton_subject(&task.task_id, slot), format!("blocks/{index}")),
                    field: "blocks".into(),
                    message: format!(
                        "task '{}' blocks missing task '{}'; add the missing task to the skeleton or remove the edge",
                        task.task_id, blocked
                    ),
                });
            }
        }
    }

    findings.extend(graph::graph_shape_findings(skeleton));

    let mut owners: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for (index, obligation) in task.implements.iter().enumerate() {
            owners
                .entry(obligation.as_str())
                .or_default()
                .push(task.task_id.as_str());
            if !expected_obligations.contains(obligation) {
                findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("unknown_obligation", skeleton_subject(&task.task_id, slot), format!("implements/{index}")),
                    field: "implements".into(),
                    message: format!(
                        "task '{}' claims unknown obligation '{}'; remove it or correct it to an id defined by the PRD",
                        task.task_id, obligation
                    ),
                });
            }
        }
    }
    for obligation in expected_obligations {
        if !owners.contains_key(obligation.as_str()) {
            findings.push(TaskSetFinding {
            identity: crate::defect::DeterministicDefect::new("unowned_obligation", obligation, "implements"),
                field: "implements".into(),
                message: format!(
                    "PRD obligation '{}' has no skeleton owner; add it to at least one task's implements list",
                    obligation
                ),
            });
        }
    }
    findings
}

pub fn validate_full_chain(tasks_root: &Path, pin: &AcceptancePin) -> SkeletonResult<TaskSkeleton> {
    let canonical_root = tasks_root.canonicalize().map(archon_shell::paths::plain).map_err(|source| error(format!(
        "task_root {} could not be canonicalized: {source}; restore it or re-run `workflow freeze-skeleton`",
        tasks_root.display()
    )))?;
    let pinned = PathBuf::from(&pin.task_root);
    let pinned = pinned
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or(pinned);
    if canonical_root != pinned {
        return invalid(format!(
            "freeze event '{}' binds task_root {}, actual {}; restore the frozen directory or re-run `workflow freeze-skeleton`",
            pin.freeze_event_id,
            pinned.display(),
            canonical_root.display()
        ));
    }

    let acceptance_bytes = read_required(&tasks_root.join(ACCEPTANCE_CONTRACT_FILE))?;
    let acceptance_lock: AcceptanceLock = read_json(&tasks_root.join(ACCEPTANCE_LOCK_FILE))?;
    require_blake3(&acceptance_lock.algorithm, ACCEPTANCE_LOCK_FILE)?;
    let acceptance_actual = content_digest(&acceptance_bytes);
    validate_gate_stamp(&acceptance_lock.gate, ACCEPTANCE_LOCK_FILE)
        .map_err(|source| error(source.to_string()))?;
    validate_gate_stamp(&pin.acceptance_gate, "acceptance pin")
        .map_err(|source| error(source.to_string()))?;
    if acceptance_lock.gate != pin.acceptance_gate {
        return invalid(
            "acceptance lock/pin gate provenance mismatch; re-run `workflow freeze-acceptance` with the current binary",
        );
    }
    if acceptance_lock.digest != acceptance_actual || pin.acceptance_digest != acceptance_actual {
        return invalid(format!(
            "acceptance chain differs from freeze event '{}': lock expected {}, pin expected {}, actual {}; restore the frozen version or re-run `workflow freeze-acceptance`",
            pin.freeze_event_id, acceptance_lock.digest, pin.acceptance_digest, acceptance_actual
        ));
    }

    let skeleton_bytes = read_required(&tasks_root.join(TASK_SKELETON_FILE))?;
    let lock: TaskSkeletonLock = read_json(&tasks_root.join(TASK_SKELETON_LOCK_FILE))?;
    require_blake3(&lock.algorithm, TASK_SKELETON_LOCK_FILE)?;
    validate_gate_stamp(&lock.gate, TASK_SKELETON_LOCK_FILE)
        .map_err(|source| error(source.to_string()))?;
    let pin_skeleton_gate = pin.skeleton_gate.as_ref().ok_or_else(|| {
        error(format!(
            "freeze event '{}' has no skeleton gate provenance; re-run `workflow freeze-skeleton` with the current binary",
            pin.freeze_event_id
        ))
    })?;
    validate_gate_stamp(pin_skeleton_gate, "skeleton pin")
        .map_err(|source| error(source.to_string()))?;
    if &lock.gate != pin_skeleton_gate {
        return invalid(
            "task skeleton lock/pin gate provenance mismatch; re-run `workflow freeze-skeleton` with the current binary",
        );
    }
    let actual = content_digest(&skeleton_bytes);
    let pinned_skeleton = pin.skeleton_digest.as_deref().ok_or_else(|| error(format!(
        "freeze event '{}' has no skeleton_digest; run `workflow freeze-skeleton` before writing task bodies",
        pin.freeze_event_id
    )))?;
    if lock.digest != actual || pinned_skeleton != actual {
        return invalid(format!(
            "task skeleton differs from freeze event '{}': lock expected {}, pin expected {}, actual {}; restore the frozen version or re-run `workflow freeze-skeleton`",
            pin.freeze_event_id, lock.digest, pinned_skeleton, actual
        ));
    }
    if lock.acceptance_digest != acceptance_actual {
        return invalid(format!(
            "{} acceptance_digest is {}, actual frozen acceptance digest {}; restore the frozen version or re-run `workflow freeze-skeleton`",
            TASK_SKELETON_LOCK_FILE, lock.acceptance_digest, acceptance_actual
        ));
    }
    let skeleton: TaskSkeleton = serde_json::from_slice(&skeleton_bytes).map_err(|parse| {
        error(format!(
            "{} is malformed JSON: {parse}; repair it or re-run `workflow freeze-skeleton`",
            tasks_root.join(TASK_SKELETON_FILE).display()
        ))
    })?;
    validate_skeleton(&skeleton, &acceptance_actual)?;
    Ok(skeleton)
}

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
    compare_set(
        &mut findings,
        "blocks",
        task.blocks_ids.clone(),
        frozen.blocks.clone(),
    );
    compare_set(
        &mut findings,
        "implements",
        task.implements.clone(),
        frozen.implements.clone(),
    );
    compare_set(
        &mut findings,
        "deliverable_contracts",
        task.deliverable_contracts.clone(),
        frozen.deliverable_contracts.clone(),
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

fn compare_set<T: Ord + fmt::Debug>(
    findings: &mut Vec<TaskSetFinding>,
    field: &str,
    mut actual: Vec<T>,
    mut frozen: Vec<T>,
) {
    actual.sort();
    frozen.sort();
    compare(findings, field, &actual, &frozen);
}

pub(crate) fn skeleton_subject(id: &str, slot: usize) -> String {
    if strict_task_id(id) {
        id.to_string()
    } else {
        format!("tasks/{slot}")
    }
}

fn strict_task_id(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    parts.len() == 3
        && parts[0] == "TASK"
        && !parts[1].is_empty()
        && parts[1]
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        && parts[2].len() == 3
        && parts[2].chars().all(|ch| ch.is_ascii_digit())
}

fn require_blake3(algorithm: &str, file: &str) -> SkeletonResult<()> {
    if algorithm == "blake3" {
        Ok(())
    } else {
        invalid(format!(
            "{file} algorithm must be 'blake3'; re-run `workflow freeze-skeleton`"
        ))
    }
}

fn read_required(path: &Path) -> SkeletonResult<Vec<u8>> {
    std::fs::read(path).map_err(|read| {
        error(format!(
            "required task-set artifact {} could not be read: {read}; restore it or run the named freeze command",
            path.display()
        ))
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> SkeletonResult<T> {
    let bytes = read_required(path)?;
    serde_json::from_slice(&bytes).map_err(|parse| {
        error(format!(
            "{} is malformed JSON: {parse}; repair it or re-run the named freeze command",
            path.display()
        ))
    })
}

fn invalid<T>(message: impl Into<String>) -> SkeletonResult<T> {
    Err(error(message))
}

fn error(message: impl Into<String>) -> TaskSkeletonError {
    TaskSkeletonError {
        message: message.into(),
        defects: Vec::new(),
    }
}

#[cfg(test)]
#[path = "task_skeleton_defect_tests.rs"]
mod defect_tests;
