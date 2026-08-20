use std::collections::BTreeSet;
use std::path::PathBuf;

use thiserror::Error;

use super::write_mode_paths::{normalize_target_for_repository, normalize_targets_for_repository};
use super::{WorkflowV2Result, WorkflowV2Status, WorkflowV2WriteMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WriteItem {
    pub id: String,
    pub mode: WorkflowV2WriteMode,
    pub owned_targets: Vec<String>,
    pub owned_scopes: Vec<String>,
    pub artifact_only: bool,
}

impl WorkflowV2WriteItem {
    pub fn new(
        id: impl Into<String>,
        mode: WorkflowV2WriteMode,
        owned_targets: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            mode,
            owned_targets,
            owned_scopes: Vec::new(),
            artifact_only: false,
        }
    }

    pub fn with_owned_scopes(mut self, owned_scopes: Vec<String>) -> Self {
        self.owned_scopes = owned_scopes;
        self
    }

    pub fn artifact_only(id: impl Into<String>, mode: WorkflowV2WriteMode) -> Self {
        Self {
            id: id.into(),
            mode,
            owned_targets: Vec::new(),
            owned_scopes: Vec::new(),
            artifact_only: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WritePlan {
    pub mode: WorkflowV2WriteMode,
    pub waves: Vec<WorkflowV2WriteWave>,
    pub conflicts: Vec<WorkflowV2WriteConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WriteWave {
    pub assignments: Vec<WorkflowV2WriteAssignment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WriteAssignment {
    pub item_id: String,
    pub owned_targets: Vec<String>,
    pub owned_scopes: Vec<String>,
    pub worktree_path: Option<String>,
    pub artifact_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WriteConflict {
    pub left_item: String,
    pub right_item: String,
    pub target: String,
    pub isolated_by_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WritePlanner {
    worktree_root: PathBuf,
}

impl WorkflowV2WritePlanner {
    pub fn new(worktree_root: impl Into<PathBuf>) -> Self {
        Self {
            worktree_root: worktree_root.into(),
        }
    }

    pub fn plan(
        &self,
        items: &[WorkflowV2WriteItem],
    ) -> Result<WorkflowV2WritePlan, WorkflowV2WriteSafetyError> {
        let Some(first) = items.first() else {
            return Ok(WorkflowV2WritePlan {
                mode: WorkflowV2WriteMode::Serial,
                waves: Vec::new(),
                conflicts: Vec::new(),
            });
        };
        let mode = first.mode;
        if let Some(item) = items.iter().find(|item| item.mode != mode) {
            return Err(WorkflowV2WriteSafetyError::MixedWriteModes {
                first_mode: mode,
                item_id: item.id.clone(),
                item_mode: item.mode,
            });
        }
        let normalized = normalize_items(items)?;
        match mode {
            WorkflowV2WriteMode::Serial => Ok(serial_plan(mode, normalized)),
            WorkflowV2WriteMode::Coordinated => Ok(coordinated_plan(mode, normalized)),
            WorkflowV2WriteMode::Worktree => Ok(self.worktree_plan(mode, normalized)),
        }
    }

    fn worktree_plan(
        &self,
        mode: WorkflowV2WriteMode,
        items: Vec<NormalizedWriteItem>,
    ) -> WorkflowV2WritePlan {
        let conflicts = conflicts_for_items(&items, true);
        let mut waves: Vec<WorkflowV2WriteWave> = Vec::new();
        for item in items {
            let assignment = self.worktree_assignment(item);
            if let Some(wave) = waves
                .iter_mut()
                .find(|wave| !assignment_overlaps_wave(&assignment, wave))
            {
                wave.assignments.push(assignment);
            } else {
                waves.push(WorkflowV2WriteWave {
                    assignments: vec![assignment],
                });
            }
        }
        WorkflowV2WritePlan {
            mode,
            waves,
            conflicts,
        }
    }

    fn worktree_assignment(&self, item: NormalizedWriteItem) -> WorkflowV2WriteAssignment {
        WorkflowV2WriteAssignment {
            worktree_path: Some(
                self.worktree_root
                    .join(safe_path_segment(&item.id))
                    .display()
                    .to_string(),
            ),
            item_id: item.id,
            owned_targets: item.owned_targets,
            owned_scopes: item.owned_scopes,
            artifact_only: item.artifact_only,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2WriteSafetyError {
    #[error("write item '{0}' declares no target ownership")]
    MissingOwnership(String),
    #[error("write target '{target}' for item '{item_id}' is unsafe")]
    UnsafeTarget { item_id: String, target: String },
    #[error(
        "write mode mix is unsafe: first item uses {first_mode:?}, item '{item_id}' uses {item_mode:?}"
    )]
    MixedWriteModes {
        first_mode: WorkflowV2WriteMode,
        item_id: String,
        item_mode: WorkflowV2WriteMode,
    },
    #[error("write item '{item_id}' changed undeclared path '{path}'")]
    ChangedFileOutsideOwnership { item_id: String, path: String },
    #[error("accepted write item '{0}' did not report changed files")]
    AcceptedWriteWithoutChangedFiles(String),
}

pub fn validate_changed_files(
    item: &WorkflowV2WriteItem,
    result: &WorkflowV2Result,
) -> Result<(), WorkflowV2WriteSafetyError> {
    validate_changed_files_for_repository(item, result, None)
}

pub fn validate_changed_files_for_repository(
    item: &WorkflowV2WriteItem,
    result: &WorkflowV2Result,
    repository_root: Option<&str>,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if item.artifact_only {
        return validate_artifact_only_result(item, result);
    }
    if result.status == WorkflowV2Status::Accepted
        && result.files_changed.is_empty()
        && result.artifacts.is_empty()
    {
        return Err(WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles(item.id.clone()));
    }
    let owned = normalize_targets_for_repository(&item.id, &item.owned_targets, repository_root)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let scopes = normalize_scope_targets(&item.id, &item.owned_scopes, repository_root)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    for file in &result.files_changed {
        let normalized = normalize_target_for_repository(&item.id, &file.path, repository_root)?;
        if !path_is_owned(&normalized, &owned, &scopes) {
            return Err(WorkflowV2WriteSafetyError::ChangedFileOutsideOwnership {
                item_id: item.id.clone(),
                path: file.path.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct NormalizedWriteItem {
    id: String,
    owned_targets: Vec<String>,
    owned_scopes: Vec<String>,
    artifact_only: bool,
}

fn serial_plan(mode: WorkflowV2WriteMode, items: Vec<NormalizedWriteItem>) -> WorkflowV2WritePlan {
    let conflicts = conflicts_for_items(&items, false);
    let waves = items
        .into_iter()
        .map(|item| WorkflowV2WriteWave {
            assignments: vec![WorkflowV2WriteAssignment {
                item_id: item.id,
                owned_targets: item.owned_targets,
                owned_scopes: item.owned_scopes,
                worktree_path: None,
                artifact_only: item.artifact_only,
            }],
        })
        .collect();
    WorkflowV2WritePlan {
        mode,
        waves,
        conflicts,
    }
}

fn coordinated_plan(
    mode: WorkflowV2WriteMode,
    items: Vec<NormalizedWriteItem>,
) -> WorkflowV2WritePlan {
    let conflicts = conflicts_for_items(&items, false);
    let mut waves: Vec<WorkflowV2WriteWave> = Vec::new();
    for item in items {
        let assignment = WorkflowV2WriteAssignment {
            item_id: item.id,
            owned_targets: item.owned_targets,
            owned_scopes: item.owned_scopes,
            worktree_path: None,
            artifact_only: item.artifact_only,
        };
        if let Some(wave) = waves
            .iter_mut()
            .find(|wave| !assignment_overlaps_wave(&assignment, wave))
        {
            wave.assignments.push(assignment);
        } else {
            waves.push(WorkflowV2WriteWave {
                assignments: vec![assignment],
            });
        }
    }
    WorkflowV2WritePlan {
        mode,
        waves,
        conflicts,
    }
}

fn assignment_overlaps_wave(
    assignment: &WorkflowV2WriteAssignment,
    wave: &WorkflowV2WriteWave,
) -> bool {
    wave.assignments
        .iter()
        .any(|existing| targets_overlap(&assignment_paths(assignment), &assignment_paths(existing)))
}

fn conflicts_for_items(
    items: &[NormalizedWriteItem],
    isolated_by_worktree: bool,
) -> Vec<WorkflowV2WriteConflict> {
    let mut conflicts = Vec::new();
    for (left_index, left) in items.iter().enumerate() {
        for right in items.iter().skip(left_index + 1) {
            for target in overlapping_targets(&item_paths(left), &item_paths(right)) {
                conflicts.push(WorkflowV2WriteConflict {
                    left_item: left.id.clone(),
                    right_item: right.id.clone(),
                    target,
                    isolated_by_worktree,
                });
            }
        }
    }
    conflicts
}

fn targets_overlap(left: &[String], right: &[String]) -> bool {
    !overlapping_targets(left, right).is_empty()
}

fn path_is_owned(changed: &str, owned: &BTreeSet<String>, scopes: &BTreeSet<String>) -> bool {
    owned.iter().any(|target| path_overlaps(target, changed))
        || scopes.iter().any(|scope| path_overlaps(scope, changed))
}

fn item_paths(item: &NormalizedWriteItem) -> Vec<String> {
    let mut paths = item.owned_targets.clone();
    paths.extend(item.owned_scopes.clone());
    paths
}

fn assignment_paths(assignment: &WorkflowV2WriteAssignment) -> Vec<String> {
    let mut paths = assignment.owned_targets.clone();
    paths.extend(assignment.owned_scopes.clone());
    paths
}

fn overlapping_targets(left: &[String], right: &[String]) -> Vec<String> {
    let mut overlap = Vec::new();
    for left_target in left {
        for right_target in right {
            if path_overlaps(left_target, right_target) {
                overlap.push(left_target.clone());
            }
        }
    }
    overlap.sort();
    overlap.dedup();
    overlap
}

/// Whether two declared paths cover any of the same file.
///
/// Exposed for `write_scope_extension`, which must decide ownership with
/// exactly the same rule the wave planner uses — a scope extension granted
/// under looser matching than `plan()` applied would silently break the
/// disjoint-ownership invariant the planner just established.
///
/// Prefix matching is boundary-aware: `src/data_la` does not cover
/// `src/data_lake/identity.rs`, only a real `/` boundary counts.
pub(crate) fn paths_overlap(left: &str, right: &str) -> bool {
    path_overlaps(left, right)
}

fn path_overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalize_items(
    items: &[WorkflowV2WriteItem],
) -> Result<Vec<NormalizedWriteItem>, WorkflowV2WriteSafetyError> {
    items
        .iter()
        .map(|item| {
            Ok(NormalizedWriteItem {
                id: item.id.clone(),
                owned_targets: normalize_item_targets(item)?,
                owned_scopes: normalize_scope_targets(&item.id, &item.owned_scopes, None)?,
                artifact_only: item.artifact_only,
            })
        })
        .collect()
}

fn validate_artifact_only_result(
    item: &WorkflowV2WriteItem,
    result: &WorkflowV2Result,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if let Some(file) = result.files_changed.first() {
        return Err(WorkflowV2WriteSafetyError::ChangedFileOutsideOwnership {
            item_id: item.id.clone(),
            path: file.path.clone(),
        });
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && result.artifacts.is_empty()
    {
        return Err(WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles(item.id.clone()));
    }
    Ok(())
}

fn normalize_item_targets(
    item: &WorkflowV2WriteItem,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    if item.artifact_only && item.owned_targets.is_empty() {
        return Ok(Vec::new());
    }
    normalize_targets(&item.id, &item.owned_targets)
}

fn normalize_targets(
    item_id: &str,
    targets: &[String],
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    normalize_targets_for_repository(item_id, targets, None)
}

fn normalize_scope_targets(
    item_id: &str,
    scopes: &[String],
    repository_root: Option<&str>,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    if scopes.is_empty() {
        return Ok(Vec::new());
    }
    normalize_targets_for_repository(item_id, scopes, repository_root)
}

fn safe_path_segment(raw: &str) -> String {
    let sanitized = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let prefix = if sanitized.trim_matches('-').is_empty() {
        "item".to_string()
    } else {
        sanitized
    };
    let hash = blake3::hash(raw.as_bytes()).to_hex().to_string();
    format!("{prefix}-{}", &hash[..8])
}

#[cfg(test)]
#[path = "write_mode_tests.rs"]
mod tests;
