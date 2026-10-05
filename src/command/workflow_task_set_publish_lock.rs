//! The publish lock of one task set, and the consistent read it gives every
//! reader of the frozen chain (Issue 294).
//!
//! Every publisher holds this lock for one whole transaction, and every
//! recovery runs under it. A reader of the frozen chain inside a run — the
//! host-command binding and postconditions, the acceptance stage, the
//! run-end observer, lint — takes it through [`ChainRead`] for one complete
//! read, so it sees the whole old set or the whole new one, never a mix, and
//! a journaled publish a crash interrupted is settled before it reads. A
//! reader holds it only while it reads files: never across a model call, a
//! check run, an `.await`, or a wait for the chain lock or a run lock
//! (publishers take those first), so a reader never keeps a writer waiting
//! beyond one read. The lock file is the one `JournalPaths` always named;
//! nothing on disk changes shape.

use std::path::Path;

use anyhow::{Result, anyhow};
use archon_workflow::task_set_publish_lock::{PublishLockFile, held_here};
use archon_workflow::{WorkflowError, WorkflowResult};

use super::journal::{JournalPaths, create_dir_all_durably};

/// The exclusive publish lock of one task set. Held for the whole life of a
/// publish transaction, by every recovery and by every consistent read, so
/// a recovery never undoes a live publisher's work and a reader never sees
/// one half applied. The OS releases it when its holder dies; dropping it
/// unlocks at once. A thread that already holds it is refused, never left
/// waiting on itself.
pub(crate) struct PublishLock {
    _file: PublishLockFile,
}

impl PublishLock {
    /// Blocks until the lock is free: a holder keeps it only while it writes
    /// and verifies one transaction, or reads the chain once.
    pub(super) fn acquire(paths: &JournalPaths) -> Result<Self> {
        if let Some(parent) = paths.lock.parent() {
            create_dir_all_durably(parent)?;
        }
        let file = PublishLockFile::acquire(&paths.lock).map_err(|error| anyhow!(error))?;
        Ok(Self { _file: file })
    }
}

/// One complete, consistent read of a task set's frozen chain: its publish
/// lock is held from before the first file is read until the guard drops.
#[must_use = "the read is consistent only while the guard is held"]
pub(crate) struct ChainRead {
    _lock: Option<PublishLock>,
}

impl ChainRead {
    /// Begin a read of the set pinned at `pin_path`. Nothing is locked when
    /// this thread already holds the set's publish lock (the read runs under
    /// its holder), when the task root does not exist, or when no set of the
    /// project was ever frozen (the pin's directory does not exist): a reader
    /// never creates the project's pin store.
    pub(crate) fn begin(pin_path: &Path, tasks_root: &Path) -> Result<Self> {
        let paths = JournalPaths::for_pin(pin_path);
        let frozen_here = paths.lock.parent().is_some_and(Path::is_dir);
        if held_here(&paths.lock) || !tasks_root.is_dir() || !frozen_here {
            return Ok(Self { _lock: None });
        }
        let lock = super::recover::lock_for_read(pin_path, tasks_root).map_err(|error| {
            error.context(format!(
                "reading the frozen task set under {} as one version",
                tasks_root.display()
            ))
        })?;
        Ok(Self { _lock: Some(lock) })
    }

    /// [`Self::begin`] for the set under `tasks_root` in `project_root`.
    pub(crate) fn of(project_root: &Path, tasks_root: &Path) -> Result<Self> {
        Self::begin(
            &super::super::acceptance_pin_path(project_root, tasks_root),
            tasks_root,
        )
    }

    /// [`Self::of`] for a workflow stage: a set that cannot be read as one
    /// version is an operational failure, never the author's artifact.
    pub(crate) fn workflow(project_root: &Path, tasks_root: &Path) -> WorkflowResult<Self> {
        Self::of(project_root, tasks_root).map_err(stage_error)
    }

    /// [`Self::begin`] for a workflow stage.
    pub(crate) fn workflow_at(pin_path: &Path, tasks_root: &Path) -> WorkflowResult<Self> {
        Self::begin(pin_path, tasks_root).map_err(stage_error)
    }
}

fn stage_error(error: anyhow::Error) -> WorkflowError {
    WorkflowError::StageFailed(format!("{error:#}"))
}
