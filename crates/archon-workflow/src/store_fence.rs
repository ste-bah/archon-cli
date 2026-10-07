//! One captured-owner boundary for run writes and asynchronous admission.
//!
//! Every durable write of an executor-bound store takes the run lock for the
//! write alone and re-checks ownership inside it. Asynchronous work (a host
//! call, a provider stream, a tool or a spawned agent) is checked before each
//! poll by a short locked read and then polled with NO lock held: a
//! synchronous verifier or git snapshot inside the work never makes `pause`,
//! `cancel` or `resume` wait (Issue 291). The poll rule itself is
//! `archon_tools::workflow_read_guard::drive_fenced`, shared with the fences
//! carried into spawned agent and tool contexts.
use super::*;
use archon_tools::workflow_read_guard::{
    AdmissionFence, AdmissionStop, FenceKind, FenceStop, StopKind, drive_fenced,
};
use std::{cell::RefCell, future::Future, sync::Arc};

thread_local! {
    // Only synchronous nesting on the same thread is reentrant. No entry
    // outlives a synchronous operation; other threads/processes still take
    // the OS lock. Canonical paths unify /var and /private/var.
    static HELD: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}
struct Held;
impl Drop for Held {
    fn drop(&mut self) {
        HELD.with(|held| {
            held.borrow_mut().pop();
        });
    }
}

/// A fence refusal: the typed workflow error and its run-control meaning.
pub(crate) struct OwnerRefusal {
    kind: StopKind,
    error: WorkflowError,
}
impl FenceStop for OwnerRefusal {
    fn stop_kind(&self) -> StopKind {
        self.kind
    }
}

impl WorkflowStore {
    /// A private clone for this executor. Clones keep their first binding;
    /// lifecycle/operator stores stay unbound and can transfer ownership.
    pub fn for_executor(&self, run_id: &str, generation: u64) -> Self {
        let mut store = self.clone();
        if store.executor.is_none() {
            store.executor = Some((run_id.to_string(), generation));
        }
        store
    }
    fn require_writer(&self, run_id: &str) -> WorkflowResult<()> {
        if let Some((_, generation)) = &self.executor {
            crate::control_pause::require_executor(&self.load_state(run_id)?, *generation)?;
        }
        Ok(())
    }
    /// Bound writes always enter the same lock and check at the write itself.
    /// Unbound stores retain their caller's locking contract (creation/control).
    pub(crate) fn with_writer<T>(
        &self,
        run_id: &str,
        work: impl FnOnce() -> WorkflowResult<T>,
    ) -> WorkflowResult<T> {
        if self.executor.is_some() {
            self.with_run_lock(run_id, |_| work())
        } else {
            work()
        }
    }
    /// Provider/tool admission: stops for a pause, cancel or new executor.
    /// Work admitted before a pause may finish; its result is returned.
    pub async fn execute_owned<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
    ) -> WorkflowResult<T> {
        // Boxed at once (#246): inner fence frames hold a pointer, never a
        // second copy of the caller's call tree.
        let work = Box::pin(work);
        self.execute_fenced(run_id, work, FenceKind::Admission)
            .await
    }
    /// Ownership only: a paused executor may preserve partial work while it
    /// unwinds; a new executor stops it.
    pub async fn execute_writer<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
    ) -> WorkflowResult<T> {
        // Boxed at once (#246): inner fence frames hold a pointer, never a
        // second copy of the caller's call tree.
        let work = Box::pin(work);
        self.execute_fenced(run_id, work, FenceKind::Ownership)
            .await
    }
    /// Carry this exact owner boundary into independently spawned execution.
    pub fn admission_fence(&self, run_id: &str) -> AdmissionFence {
        let (store, id) = (self.clone(), run_id.to_string());
        AdmissionFence::new(self.fence_owner(run_id), move || {
            store
                .owner_check(&id, FenceKind::Admission)
                .map_err(|refused| AdmissionStop {
                    kind: refused.kind,
                    reason: refused.error.to_string(),
                })
        })
    }
    /// Fails unless this store's executor may still admit work for `run_id`:
    /// the same check every fenced poll makes.
    pub fn require_admission(&self, run_id: &str) -> WorkflowResult<()> {
        self.owner_check(run_id, FenceKind::Admission)
            .map_err(|refused| refused.error)
    }

    fn fence_owner(&self, run_id: &str) -> Arc<str> {
        let generation = self.executor.as_ref().map(|(_, generation)| *generation);
        // Canonical, so every store spelling of one run names one owner.
        let dir = self.run_dir(run_id);
        let dir = dir.canonicalize().unwrap_or(dir);
        Arc::from(format!(
            "{}#{}",
            dir.display(),
            generation.map_or_else(|| "unbound".to_string(), |g| g.to_string())
        ))
    }

    /// One locked state read, then the owner rule for `kind`. A bound store
    /// never creates the run directory: a run its successor removed is gone.
    fn owner_check(&self, run_id: &str, kind: FenceKind) -> Result<(), OwnerRefusal> {
        let Some((_, generation)) = &self.executor else {
            return Ok(());
        };
        let generation = *generation;
        let refused = |kind, error| Err(OwnerRefusal { kind, error });
        let run = match lock_run(self, run_id, || self.load_state(run_id)) {
            Ok(run) => run,
            // The run directory itself is gone: its successor removed it.
            Err(WorkflowError::Io { .. }) if !self.run_dir(run_id).exists() => {
                return refused(
                    StopKind::Superseded,
                    WorkflowError::ControlCancelled(format!(
                        "run {run_id} no longer exists; executor generation {generation} stops"
                    )),
                );
            }
            Err(error) => return refused(StopKind::Refused, error),
        };
        if let Err(error) = crate::control_pause::require_executor(&run, generation) {
            return refused(StopKind::Superseded, error);
        }
        if kind == FenceKind::Admission {
            let stop = match run.status {
                RunStatus::Paused => StopKind::Paused,
                RunStatus::Cancelled => StopKind::Cancelled,
                _ => return Ok(()),
            };
            if let Err(error) =
                crate::control_pause::PauseOwner::Executor(generation).require_pauser(&run)
            {
                return refused(stop, error);
            }
        }
        Ok(())
    }

    async fn execute_fenced<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
        kind: FenceKind,
    ) -> WorkflowResult<T> {
        if self.executor.is_none() {
            // An unbound store has no captured owner to fence.
            return work.await;
        }
        drive_fenced(
            self.fence_owner(run_id),
            kind,
            || self.owner_check(run_id, kind),
            work,
        )
        .await
        .map_err(|refused| refused.error)?
    }
}

/// The run lock for one synchronous operation, with no ownership check and,
/// for a bound store, no directory creation.
fn lock_run<T>(
    store: &WorkflowStore,
    run_id: &str,
    operation: impl FnOnce() -> WorkflowResult<T>,
) -> WorkflowResult<T> {
    let run_dir = store.run_dir(run_id);
    if let Some((id, _)) = &store.executor {
        if id != run_id {
            return Err(WorkflowError::SpecInvalid(
                "executor store cannot write another run".into(),
            ));
        }
        // An executor needs an existing authority. Lock acquisition must not
        // recreate a run removed by its successor before checking that authority.
    } else {
        fs::create_dir_all(&run_dir).map_err(|e| WorkflowError::io(&run_dir, e))?;
    }
    let path = run_dir
        .canonicalize()
        .map_err(|e| WorkflowError::io(&run_dir, e))?
        .join(".control.lock");
    if HELD.with(|held| held.borrow().contains(&path)) {
        return operation();
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| WorkflowError::io(&path, e))?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.write().map_err(|e| WorkflowError::io(&path, e))?;
    HELD.with(|held| held.borrow_mut().push(path));
    let _held = Held;
    operation()
}

pub(super) fn with_run_lock<T>(
    store: &WorkflowStore,
    run_id: &str,
    operation: impl FnOnce(&WorkflowStore) -> WorkflowResult<T>,
) -> WorkflowResult<T> {
    lock_run(store, run_id, || {
        store.require_writer(run_id)?;
        operation(store)
    })
}

#[cfg(test)]
#[path = "store_fence_tests.rs"]
mod tests;
