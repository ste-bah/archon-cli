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
#[cfg(test)]
thread_local! {
    /// Test fault: the next owner-check state reads on this thread fail.
    static OWNER_READ_FAULTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
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
        match self.owner_state(run_id) {
            Ok(run) => owner_rule(&run, generation, kind),
            // The run directory itself is gone: its successor removed it.
            Err(WorkflowError::Io { .. }) if !self.run_dir(run_id).exists() => Err(OwnerRefusal {
                kind: StopKind::Superseded,
                error: WorkflowError::ControlCancelled(format!(
                    "run {run_id} no longer exists; executor generation {generation} stops"
                )),
            }),
            Err(error @ (WorkflowError::Io { .. } | WorkflowError::StateCorrupt(_))) => {
                Err(self.pause_on_unreadable_state(run_id, generation, kind, error))
            }
            Err(error) => Err(OwnerRefusal {
                kind: StopKind::Refused,
                error,
            }),
        }
    }

    /// The fence's one locked state read.
    fn owner_state(&self, run_id: &str) -> WorkflowResult<WorkflowRun> {
        #[cfg(test)]
        if OWNER_READ_FAULTS.with(|faults| {
            let left = faults.get();
            faults.set(left.saturating_sub(1));
            left > 0
        }) {
            return Err(WorkflowError::io(
                self.state_path(run_id),
                std::io::Error::other("injected transient read fault"),
            ));
        }
        lock_run(self, run_id, || self.load_state(run_id))
    }

    /// Review minor 3 (round 5): a state read that fails (a descriptor limit,
    /// a disk fault, a corrupt file) says nothing about run control, and the
    /// fence cannot let work go on unverified. Nor may it end the work as a
    /// plain error, which fails the run: a stall pauses, never fails. The
    /// fence pauses the run with the read error as evidence (when the state
    /// reads again for the pause) and stops the work as paused. When the
    /// pause cannot be recorded, the stop is still a typed pause that says
    /// so: the run end records it, or the operator resumes the run.
    fn pause_on_unreadable_state(
        &self,
        run_id: &str,
        generation: u64,
        kind: FenceKind,
        error: WorkflowError,
    ) -> OwnerRefusal {
        let resume = format!("archon workflow resume --live --yes {run_id}");
        let detail = serde_json::json!({
            "event": "state_read_pause",
            "cause": "state_unreadable",
            "error": error.to_string(),
            "executor_generation": generation,
            "resume": resume,
        });
        let owner = crate::control_pause::PauseOwner::Executor(generation);
        let recorded = match crate::control_pause::pause_owned(self, run_id, owner, detail) {
            Ok(_) => "the pause is recorded".to_string(),
            // The state reads again and holds another control decision (a
            // pause, a cancel or a newer executor): that decision answers.
            Err(
                refused @ (WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)),
            ) => {
                let decided = self
                    .owner_state(run_id)
                    .ok()
                    .and_then(|run| owner_rule(&run, generation, kind).err());
                return decided.unwrap_or(OwnerRefusal {
                    kind: StopKind::Superseded,
                    error: refused,
                });
            }
            Err(unrecorded) => format!("the pause is not recorded ({unrecorded})"),
        };
        OwnerRefusal {
            kind: StopKind::Paused,
            error: WorkflowError::ControlPaused(format!(
                "the state of run {run_id} could not be read by executor generation {generation} \
                 ({error}); a stall pauses, never fails, so the fenced work stopped and {recorded}. \
                 When the state file reads again: {resume}"
            )),
        }
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

/// The owner rule for `kind` on a state the fence read.
fn owner_rule(run: &WorkflowRun, generation: u64, kind: FenceKind) -> Result<(), OwnerRefusal> {
    let refused = |kind, error| Err(OwnerRefusal { kind, error });
    if let Err(error) = crate::control_pause::require_executor(run, generation) {
        return refused(StopKind::Superseded, error);
    }
    if kind == FenceKind::Admission {
        let stop = match run.status {
            RunStatus::Paused => StopKind::Paused,
            RunStatus::Cancelled => StopKind::Cancelled,
            _ => return Ok(()),
        };
        if let Err(error) =
            crate::control_pause::PauseOwner::Executor(generation).require_pauser(run)
        {
            return refused(stop, error);
        }
    }
    Ok(())
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
