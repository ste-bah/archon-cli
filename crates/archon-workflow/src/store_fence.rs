//! One captured-owner boundary for run writes and asynchronous admission.
//! A lock lives only for a synchronous operation/poll, never across Pending.
use super::*;
use std::{cell::RefCell, future::Future, task::Poll, time::Duration};

thread_local! {
    // Only synchronous nesting on the same thread is reentrant. Async tasks
    // release this entry before returning Pending; other threads/processes
    // still take the OS lock. Canonical paths unify /var and /private/var.
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
    /// Each poll includes actual provider/tool admission, continuations and
    /// retries after any await. The watchdog only wakes pending work; it is
    /// never the ownership fence. Errors remain control errors.
    pub async fn execute_owned<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
    ) -> WorkflowResult<T> {
        self.execute_fenced(run_id, work, true).await
    }
    /// Ownership-only polling permits the current executor to preserve partial
    /// work while unwinding a pause. Resume still fences every subsequent poll.
    pub async fn execute_writer<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
    ) -> WorkflowResult<T> {
        self.execute_fenced(run_id, work, false).await
    }
    /// Carry this exact owner boundary into independently spawned execution.
    pub fn admission_fence(
        &self,
        run_id: &str,
    ) -> archon_tools::workflow_read_guard::AdmissionFence {
        let (store, id) = (self.clone(), run_id.to_string());
        archon_tools::workflow_read_guard::AdmissionFence::new(move |work| {
            store
                .with_admission(&id, || {
                    work();
                    Ok(())
                })
                .map_err(|error| error.to_string())
        })
    }
    fn with_admission<T>(
        &self,
        run_id: &str,
        work: impl FnOnce() -> WorkflowResult<T>,
    ) -> WorkflowResult<T> {
        self.with_run_lock(run_id, |locked| {
            if let Some((_, generation)) = self.executor {
                crate::control_pause::PauseOwner::Executor(generation)
                    .require_pauser(&locked.load_state(run_id)?)?;
            }
            work()
        })
    }

    async fn execute_fenced<T>(
        &self,
        run_id: &str,
        work: impl Future<Output = WorkflowResult<T>>,
        admission: bool,
    ) -> WorkflowResult<T> {
        let mut work = Box::pin(work);
        let mut watch = tokio::time::interval(Duration::from_secs(2));
        watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        std::future::poll_fn(|cx| {
            while watch.poll_tick(cx).is_ready() {}
            let mut poll = || Ok(work.as_mut().poll(cx));
            let polled = if admission {
                self.with_admission(run_id, poll)
            } else {
                self.with_run_lock(run_id, |_| poll())
            };
            match polled {
                Ok(Poll::Ready(out)) => Poll::Ready(out),
                Ok(Poll::Pending) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            }
        })
        .await
    }
}

pub(super) fn with_run_lock<T>(
    store: &WorkflowStore,
    run_id: &str,
    operation: impl FnOnce(&WorkflowStore) -> WorkflowResult<T>,
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
        store.require_writer(run_id)?;
        return operation(store);
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
    store.require_writer(run_id)?;
    operation(store)
}
