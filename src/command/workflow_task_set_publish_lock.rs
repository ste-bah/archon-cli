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
//! beyond one read. Readers hold it shared, so they never wait for each
//! other; writers hold it exclusive (Issue 336). The lock file is the one
//! `JournalPaths` always named; nothing on disk changes shape.
//!
//! A journal a read cannot settle — a committed publish whose cleanup keeps
//! failing, say — is [`UnsettledPublish`]: every read retries the settlement,
//! and a stage that meets it pauses the run with the evidence and the
//! operator's remedy instead of failing it.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use archon_workflow::task_set_publish_lock::{PublishLockFile, held_here};
use archon_workflow::{WorkflowError, WorkflowResult};

use super::journal::{JournalPaths, create_dir_all_durably};

/// The publish lock of one task set. Held exclusive for the whole life of a
/// publish transaction and by every recovery, and shared by every consistent
/// read, so a recovery never undoes a live publisher's work and a reader
/// never sees one half applied. The OS releases it when its holder dies;
/// dropping it unlocks at once. A thread that already holds it is refused,
/// never left waiting on itself.
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

    /// The shared lock of a read of the set pinned at `pin_path`, with any
    /// journal left there settled first by `settle` under the exclusive lock
    /// (the shared one let go before it is taken). The pin's directory
    /// exists, and this thread holds no publish lock of the set.
    /// A journal still left once the settlements run out is
    /// [`UnsettledPublish`] too: the run pauses on it, never fails.
    pub(super) fn acquire_shared_settled(
        pin_path: &Path,
        settle: impl FnMut() -> Result<()>,
    ) -> Result<Self> {
        let paths = JournalPaths::for_pin(pin_path);
        let file = PublishLockFile::acquire_shared_settled(
            pin_path,
            settle,
            |error| anyhow!(error),
            |exhausted| anyhow::Error::new(UnsettledPublish::new(&paths, &anyhow!(exhausted))),
        )?;
        Ok(Self { _file: file })
    }
}

/// Install the host's settlement of an interrupted publish for the check-
/// source repins and reads of `archon-workflow`, which hold the publish lock
/// themselves and settle before they write (Issue 336) or read (Issue 338).
/// Idempotent.
pub(crate) fn register_publish_settle() {
    archon_workflow::task_set_publish_lock::register_settle(|pin_path, tasks_root| {
        let paths = JournalPaths::for_pin(pin_path);
        super::recover::settle_all(&paths, pin_path, tasks_root)
            .map(drop)
            .map_err(|error| format!("settling the interrupted publish: {error:#}"))
    });
}

/// A publish journal a read found and could not settle (Issue 336). Every
/// read retries the settlement, so the set heals once the cause is gone; a
/// stage that meets this pauses the run rather than failing it.
#[derive(Debug)]
pub(crate) struct UnsettledPublish {
    journal: PathBuf,
    state: String,
    log: PathBuf,
    cause: String,
}

impl UnsettledPublish {
    pub(super) fn new(paths: &JournalPaths, cause: &anyhow::Error) -> Self {
        let state = std::fs::read(&paths.journal)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|journal| journal.get("state")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| {
                if paths.journal.exists() {
                    "unreadable".into()
                } else {
                    "a journal temp only".into()
                }
            });
        Self {
            journal: paths.journal.clone(),
            state,
            log: paths.log.clone(),
            cause: format!("{cause:#}"),
        }
    }

    /// Whether `error` is, or wraps, an unsettled publish.
    pub(crate) fn is(error: &anyhow::Error) -> bool {
        error.downcast_ref::<Self>().is_some()
    }
}

impl std::fmt::Display for UnsettledPublish {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "an interrupted publish of this task set left its journal {} (state: {}) and it could not be settled: {}. Nothing was read; every read retries the settlement. A committed journal means the new set is final and only its cleanup is left; any other state rolls back to the old set. Operator remedy: fix what the cause names (a file or directory beside the pin or in the task set that cannot be written or removed), then resume the run (`archon workflow resume --live --yes <RUN_ID>`); each settlement is recorded in {}",
            self.journal.display(),
            self.state,
            self.cause,
            self.log.display()
        )
    }
}

impl std::error::Error for UnsettledPublish {}

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
    /// version is an operational failure, never the author's artifact; one
    /// whose interrupted publish cannot be settled pauses the run.
    pub(crate) fn workflow(project_root: &Path, tasks_root: &Path) -> WorkflowResult<Self> {
        Self::of(project_root, tasks_root).map_err(stage_error)
    }

    /// [`Self::begin`] for a workflow stage.
    pub(crate) fn workflow_at(pin_path: &Path, tasks_root: &Path) -> WorkflowResult<Self> {
        Self::begin(pin_path, tasks_root).map_err(stage_error)
    }
}

/// A set left with a journal no read can settle pauses the run with the
/// evidence and the remedy (Issue 336): its cause is the host's environment,
/// fixed by an operator, never the author's artifact nor a reason to fail.
pub(super) fn stage_error(error: anyhow::Error) -> WorkflowError {
    pause_if_unsettled(&error).unwrap_or_else(|| WorkflowError::StageFailed(format!("{error:#}")))
}

/// The run's pause when `error` is, or wraps, an [`UnsettledPublish`]
/// (Issue 338): what a stage that meets one anywhere -- a read, a chain
/// lock's recovery, a child's exit -- ends with instead of its own failure.
pub(crate) fn pause_if_unsettled(error: &anyhow::Error) -> Option<WorkflowError> {
    UnsettledPublish::is(error).then(|| {
        tracing::warn!(error = %format!("{error:#}"), "pausing: the task set's interrupted publish could not be settled");
        WorkflowError::ControlPaused(format!("{error:#}"))
    })
}
