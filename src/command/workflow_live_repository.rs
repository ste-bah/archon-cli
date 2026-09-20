//! Where an implementation run's write repository comes from (Issue-55).
//!
//! A task set decomposed after Issue-55 carries `repository.lock`: the
//! repository its authors were grounded in and the base commit they read.
//! The implementation run that loads such a set uses that root as its
//! `target_repository_root` — the same tree the tasks describe — instead of
//! inferring one from marker files near the task directory. A task text that
//! names a different repository is refused: two answers to "which checkout"
//! is not a run to start. A `HEAD` that has moved since the base is not a
//! failure; it is recorded in the run's first event so the drift is visible
//! beside everything the run then does.
//!
//! Task sets without the record — every set decomposed before it existed —
//! keep the inference, unchanged.

use std::path::{Path, PathBuf};

use archon_workflow::repo_root::{explicit_target_repository_root, infer_target_repository_root};
use archon_workflow::repository_record::{
    REPOSITORY_LOCK_FILE, RepositoryRecordV1, git_head, read_repository_record,
};
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::{WorkflowError, WorkflowResult};

/// The record an implementation run is bound to, with the checkout's `HEAD`
/// read at planning time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryBinding {
    pub(crate) repository_root: String,
    pub(crate) recorded_base_commit: String,
    pub(crate) decomposition_run_id: String,
    pub(crate) head: String,
    pub(crate) record_path: PathBuf,
}

impl RepositoryBinding {
    pub(crate) fn drifted(&self) -> bool {
        self.recorded_base_commit != self.head
    }

    /// The detail of the run's first event, and the line the operator sees.
    pub(crate) fn event_detail(&self) -> serde_json::Value {
        serde_json::json!({
            "event": "repository_bound",
            "repository_root": self.repository_root,
            "recorded_base_commit": self.recorded_base_commit,
            "head": self.head,
            "drift": self.drifted(),
            "decomposition_run_id": self.decomposition_run_id,
            "record": self.record_path.display().to_string(),
        })
    }

    pub(crate) fn summary_line(&self) -> String {
        if self.drifted() {
            format!(
                "Repository {} bound from {}: decomposed at base commit {}, HEAD is now {} (drift recorded; the tasks describe the recorded base)\n",
                self.repository_root,
                self.record_path.display(),
                self.recorded_base_commit,
                self.head
            )
        } else {
            format!(
                "Repository {} bound from {} at base commit {}\n",
                self.repository_root,
                self.record_path.display(),
                self.recorded_base_commit
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResolution {
    pub(crate) target_repository_root: Option<String>,
    pub(crate) binding: Option<RepositoryBinding>,
}

/// The repository the run writes to: the task set's record when it has one,
/// else the inference every earlier set relied on.
pub(crate) fn resolve_target_repository(
    task: &str,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> WorkflowResult<RepositoryResolution> {
    let Some(universe) = universe else {
        return Ok(RepositoryResolution {
            target_repository_root: infer_target_repository_root(task, None),
            binding: None,
        });
    };
    let Some((record_path, record)) = recorded_repository(universe)? else {
        return Ok(RepositoryResolution {
            target_repository_root: infer_target_repository_root(task, Some(universe)),
            binding: None,
        });
    };
    let root = PathBuf::from(&record.repository_root);
    if !root.is_dir() {
        return Err(WorkflowError::SpecInvalid(format!(
            "{} records repository {} which is not a directory; the task set was decomposed against a checkout that is no longer there",
            record_path.display(),
            record.repository_root
        )));
    }
    if let Some(named) = explicit_target_repository_root(task) {
        let named_path = PathBuf::from(&named);
        let same = canonical(&named_path) == canonical(&root);
        if !same {
            return Err(WorkflowError::SpecInvalid(format!(
                "the task names repository {named} but {} records {}; a task set is implemented against the repository it was decomposed against — drop the repository from the task text or use a task set decomposed against {named}",
                record_path.display(),
                record.repository_root
            )));
        }
    }
    let head = git_head(&root)?;
    Ok(RepositoryResolution {
        target_repository_root: Some(record.repository_root.clone()),
        binding: Some(RepositoryBinding {
            repository_root: record.repository_root,
            recorded_base_commit: record.base_commit,
            decomposition_run_id: record.decomposition_run_id,
            head,
            record_path,
        }),
    })
}

/// The one record the universe's task directories carry. Two directories
/// recording different repositories is an error, not a choice.
fn recorded_repository(
    universe: &WorkflowV2TaskUniverse,
) -> WorkflowResult<Option<(PathBuf, RepositoryRecordV1)>> {
    let mut found: Option<(PathBuf, RepositoryRecordV1)> = None;
    for root in universe.source_roots.iter().map(Path::new).filter(|p| p.is_dir()) {
        let Some(record) = read_repository_record(root)? else {
            continue;
        };
        let path = root.join(REPOSITORY_LOCK_FILE);
        if let Some((first_path, first)) = &found
            && canonical(Path::new(&first.repository_root))
                != canonical(Path::new(&record.repository_root))
        {
            return Err(WorkflowError::SpecInvalid(format!(
                "{} records repository {} but {} records {}; one run implements one repository",
                first_path.display(),
                first.repository_root,
                path.display(),
                record.repository_root
            )));
        }
        if found.is_none() {
            found = Some((path, record));
        }
    }
    Ok(found)
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "workflow_live_repository_tests.rs"]
mod tests;
