//! #284: delivering a fixed decomposition's pending lessons.
//!
//! A run's first fold can send hundreds of lessons. Two things follow. The
//! fold serializes on a lock of its own, never on the run's write lock, so a
//! resume or the next executor never waits for it. And delivery advances as
//! it goes: each lesson the learning stack accepts appends its key to
//! [`DELIVERED_KEYS_FILE`], so a fold killed half way resumes where it
//! stopped on the next fold. A lesson sent twice anyway (killed between the
//! store and the append) replaces its own episode, because episodes are keyed
//! by the lesson (`workflow_learning::episode_key`).
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use archon_workflow::{WorkflowLearningRecord, WorkflowStore};

use super::{digest, write_journal};

/// Keys of pending lessons the learning stack accepted, one per line.
const DELIVERED_KEYS_FILE: &str = "decomposition-delivered.keys";

/// Runs `operation` holding the run's fold lock (`learning/.fold.lock`):
/// folds of one run never overlap, and nothing else waits for them.
pub(in super::super) fn with_fold_lock<T>(
    store: &WorkflowStore,
    run_id: &str,
    operation: impl FnOnce(&WorkflowStore) -> archon_workflow::WorkflowResult<T>,
) -> archon_workflow::WorkflowResult<T> {
    let dir = store.run_dir(run_id).join("learning");
    std::fs::create_dir_all(&dir).map_err(|error| archon_workflow::WorkflowError::Io {
        path: dir.clone(),
        source: error,
    })?;
    let path = dir.join(".fold.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| archon_workflow::WorkflowError::Io {
            path: path.clone(),
            source: error,
        })?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock
        .write()
        .map_err(|error| archon_workflow::WorkflowError::Io {
            path,
            source: error,
        })?;
    operation(store)
}

fn read_keys(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.len() == 16 && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_string)
        .collect()
}

/// Send the lessons of `pending` (the spool at `pending_path`) not yet
/// delivered, recording each as it lands; empty the spool once every one has.
pub(super) fn deliver_pending(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    pending_path: &Path,
    pending: &[WorkflowLearningRecord],
) -> anyhow::Result<()> {
    let keys_path = pending_path.with_file_name(DELIVERED_KEYS_FILE);
    let mut delivered = read_keys(&keys_path);
    let wanted: BTreeSet<String> = pending
        .iter()
        .map(|record| digest(&record.stage_id))
        .collect();
    let unsent: Vec<WorkflowLearningRecord> = pending
        .iter()
        .filter(|record| !delivered.contains(&digest(&record.stage_id)))
        .cloned()
        .collect();
    let mut progress = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&keys_path)?;
    let mut progress_error = None;
    let outcome =
        crate::command::topology_fold::workflow_learning::bridge_workflow_learning_with_records(
            cwd,
            store,
            run_id,
            &unsent,
            &mut |call| {
                let key = digest(&call.stage_id);
                if !wanted.contains(&key) || progress_error.is_some() {
                    return;
                }
                match writeln!(progress, "{key}") {
                    Ok(()) => {
                        delivered.insert(key);
                    }
                    Err(error) => progress_error = Some(error),
                }
            },
        );
    drop(progress);
    if let Some(error) = progress_error {
        tracing::warn!(%error, %run_id, path = %keys_path.display(), "delivered lesson keys not recorded; the next fold sends the rest again, replacing their episodes");
    }
    let left = wanted.difference(&delivered).count();
    if left == 0 {
        write_journal(pending_path, &[])?;
        match std::fs::remove_file(&keys_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        tracing::warn!(
            %run_id,
            pending = wanted.len(),
            undelivered = left,
            dispatched = outcome.dispatched,
            integration_unavailable = outcome.integration_unavailable,
            "fixed decomposition lessons were not all dispatched to the learning stack; they stay pending for the next fold"
        );
    }
    tracing::debug!(%run_id, sent = unsent.len(), dispatched = outcome.dispatched, undelivered = left, "fixed decomposition lessons delivered");
    Ok(())
}
