//! A stage's synchronous mutations belong to its dispatched executor.
use crate::control_pause::PauseOwner;
use crate::{RunStatus, WorkflowError, WorkflowStore};

#[derive(Clone)]
pub struct StageWriter {
    pub store: WorkflowStore,
    pub run_id: String,
    pub owner: PauseOwner,
}

tokio::task_local! { static WRITER: StageWriter; }

pub fn scope<T>(
    writer: StageWriter,
    work: impl std::future::Future<Output = T>,
) -> impl std::future::Future<Output = T> {
    // Ownership scopes must not duplicate the full stage's future on stack.
    WRITER.scope(writer, Box::pin(work))
}

thread_local! {
    static HELD: std::cell::RefCell<std::collections::BTreeSet<std::path::PathBuf>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}
struct Held(std::path::PathBuf);
impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|held| held.borrow_mut().remove(&self.0));
    }
}

/// Captured explicitly when work moves to a blocking thread.
pub fn current() -> Option<StageWriter> {
    WRITER.try_with(Clone::clone).ok()
}

/// Check and mutate under one run lock. Nested synchronous publications
/// retain the outer fence; the lock is never held across an await.
pub fn with_write<T, E: From<WorkflowError>>(act: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let Some(writer) = current() else {
        return act();
    };
    with_writer(&writer, act)
}
pub fn with_writer<T, E: From<WorkflowError>>(
    writer: &StageWriter,
    act: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let run_dir = writer.store.run_dir(&writer.run_id);
    if HELD.with(|held| held.borrow().contains(&run_dir)) {
        return act();
    }
    writer
        .store
        .with_run_lock(&writer.run_id, |store| {
            let run = store.load_state(&writer.run_id)?;
            writer.owner.require_writer(&run)?;
            match run.status {
                RunStatus::Paused => {
                    return Err(WorkflowError::ControlPaused(format!(
                        "run {} is paused; stage writes stopped",
                        run.id
                    )));
                }
                RunStatus::Cancelled => {
                    return Err(WorkflowError::ControlCancelled(format!(
                        "run {} is cancelled; stage writes stopped",
                        run.id
                    )));
                }
                _ => {}
            }
            HELD.with(|held| held.borrow_mut().insert(run_dir.clone()));
            let _held = Held(run_dir);
            Ok(act())
        })
        .map_err(E::from)?
}

/// Preserve a caller's error type while fencing the synchronous mutation.
pub fn mapped<T, E>(
    act: impl FnOnce() -> Result<T, E>,
    refused: impl FnOnce(WorkflowError) -> E,
) -> Result<T, E> {
    with_write(|| crate::WorkflowResult::Ok(act())).map_err(refused)?
}

/// A stage's durable bytes, atomically replaced under its ownership fence.
pub fn write_bytes(path: &std::path::Path, bytes: &[u8]) -> crate::WorkflowResult<()> {
    with_write(|| {
        crate::store::write_atomic(
            &path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4())),
            path,
            bytes,
        )
    })
}
pub fn best_effort_bytes(path: &std::path::Path, bytes: &[u8]) {
    if let Err(error) = write_bytes(path, bytes) {
        tracing::warn!(%error, path = %path.display(), "stage evidence not written");
    }
}

#[cfg(test)]
#[path = "stage_write_tests.rs"]
mod tests;
