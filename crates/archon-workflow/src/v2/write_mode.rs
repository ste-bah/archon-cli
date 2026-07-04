use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use super::{WorkflowV2Result, WorkflowV2Status, WorkflowV2WriteMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2WriteItem {
    pub id: String,
    pub mode: WorkflowV2WriteMode,
    pub owned_targets: Vec<String>,
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
    pub worktree_path: Option<String>,
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
        let assignments = items
            .into_iter()
            .map(|item| WorkflowV2WriteAssignment {
                worktree_path: Some(
                    self.worktree_root
                        .join(safe_path_segment(&item.id))
                        .display()
                        .to_string(),
                ),
                item_id: item.id,
                owned_targets: item.owned_targets,
            })
            .collect();
        WorkflowV2WritePlan {
            mode,
            waves: vec![WorkflowV2WriteWave { assignments }],
            conflicts,
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
    if result.status == WorkflowV2Status::Accepted
        && result.files_changed.is_empty()
        && result.artifacts.is_empty()
    {
        return Err(WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles(item.id.clone()));
    }
    let owned = normalize_targets_for_repository(&item.id, &item.owned_targets, repository_root)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    for file in &result.files_changed {
        let normalized = normalize_target_for_repository(&item.id, &file.path, repository_root)?;
        if !owned
            .iter()
            .any(|owned_target| path_overlaps(owned_target, &normalized))
        {
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
}

fn serial_plan(mode: WorkflowV2WriteMode, items: Vec<NormalizedWriteItem>) -> WorkflowV2WritePlan {
    let conflicts = conflicts_for_items(&items, false);
    let waves = items
        .into_iter()
        .map(|item| WorkflowV2WriteWave {
            assignments: vec![WorkflowV2WriteAssignment {
                item_id: item.id,
                owned_targets: item.owned_targets,
                worktree_path: None,
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
            worktree_path: None,
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
        .any(|existing| targets_overlap(&assignment.owned_targets, &existing.owned_targets))
}

fn conflicts_for_items(
    items: &[NormalizedWriteItem],
    isolated_by_worktree: bool,
) -> Vec<WorkflowV2WriteConflict> {
    let mut conflicts = Vec::new();
    for (left_index, left) in items.iter().enumerate() {
        for right in items.iter().skip(left_index + 1) {
            for target in overlapping_targets(&left.owned_targets, &right.owned_targets) {
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
                owned_targets: normalize_targets(&item.id, &item.owned_targets)?,
            })
        })
        .collect()
}

fn normalize_targets(
    item_id: &str,
    targets: &[String],
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    normalize_targets_for_repository(item_id, targets, None)
}

pub fn normalize_targets_for_repository(
    item_id: &str,
    targets: &[String],
    repository_root: Option<&str>,
) -> Result<Vec<String>, WorkflowV2WriteSafetyError> {
    if targets.is_empty() {
        return Err(WorkflowV2WriteSafetyError::MissingOwnership(
            item_id.to_string(),
        ));
    }
    let mut normalized = BTreeSet::new();
    for target in targets {
        normalized.insert(normalize_target_for_repository(
            item_id,
            target,
            repository_root,
        )?);
    }
    Ok(normalized.into_iter().collect())
}

pub fn normalize_target_for_repository(
    item_id: &str,
    target: &str,
    repository_root: Option<&str>,
) -> Result<String, WorkflowV2WriteSafetyError> {
    let trimmed = target.trim();
    let path = Path::new(trimmed);
    if !path.is_absolute() {
        return normalize_target(item_id, trimmed);
    }
    let Some(root) = repository_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    else {
        return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        });
    };
    let root_path = Path::new(root);
    if !root_path.is_absolute() {
        return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        });
    }
    let clean_root = clean_absolute_path(item_id, root, root_path)?;
    let clean_target = clean_absolute_path(item_id, target, path)?;
    let relative = clean_target.strip_prefix(&clean_root).map_err(|_| {
        WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        }
    })?;
    if relative.as_os_str().is_empty() {
        return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        });
    }
    normalize_target(item_id, &relative.to_string_lossy().replace('\\', "/"))
}

fn normalize_target(item_id: &str, target: &str) -> Result<String, WorkflowV2WriteSafetyError> {
    let trimmed = target.trim();
    let path = Path::new(trimmed);
    if trimmed.is_empty() || path.is_absolute() {
        return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        });
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
                    item_id: item_id.to_string(),
                    target: target.to_string(),
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: item_id.to_string(),
            target: target.to_string(),
        });
    }
    Ok(parts.join("/"))
}

fn clean_absolute_path(
    item_id: &str,
    original: &str,
    path: &Path,
) -> Result<PathBuf, WorkflowV2WriteSafetyError> {
    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => cleaned.push(prefix.as_os_str()),
            Component::RootDir => cleaned.push(Path::new("/")),
            Component::CurDir => {}
            Component::Normal(part) => cleaned.push(part),
            Component::ParentDir => {
                return Err(WorkflowV2WriteSafetyError::UnsafeTarget {
                    item_id: item_id.to_string(),
                    target: original.to_string(),
                });
            }
        }
    }
    Ok(cleaned)
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
