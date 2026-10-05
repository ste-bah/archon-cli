// The executor a session writes for (Issue 291).
//
// A resume hands a run to a new executor (`WorkflowRun::executor_generation`
// moves past the generation the old one started under). An old executor still
// running -- its script caught the control error and kept calling -- is a
// stale session: it may dispatch nothing and write nothing more. The store
// remembers the generation its executor started under, bound once when the
// executor starts, and every session write checks it beside the restart epoch
// (`require_session_owner`), under the run lock. The ownership rule itself is
// `control_pause::require_executor`; this only applies it.

impl WorkflowV2ResultStore {
    /// Bind this session (the store and its clones) to the executor that
    /// started under run `generation`. The first binding stands: a later
    /// script of the same executor keeps the generation it started under.
    pub fn bind_session_executor(&self, generation: u64) {
        let _ = self.session.executor.set(generation);
    }

    /// The generation this session's executor started under, if bound.
    pub fn session_executor(&self) -> Option<u64> {
        self.session.executor.get().copied()
    }

    /// Fails unless this session's executor still owns `run`. An unbound
    /// session (no executor, e.g. an operator command) is not fenced. Every
    /// refusal is logged: a stale write is never dropped silently.
    pub fn require_session_executor(&self, run: &crate::WorkflowRun) -> WorkflowResult<()> {
        let Some(generation) = self.session_executor() else {
            return Ok(());
        };
        crate::control_pause::require_executor(run, generation).inspect_err(|refused| {
            tracing::warn!(
                run_id = %run.id,
                executor_generation = generation,
                %refused,
                "stale executor session refused"
            );
        })
    }

    /// Refuse a write of this store's session once a restart moved the
    /// epoch on, or once a newer executor owns the run. The caller holds the
    /// run lock for a write.
    pub fn require_session_owner(&self) -> WorkflowResult<()> {
        self.require_session_restart_epoch()?;
        if self.session_executor().is_none() {
            return Ok(());
        }
        let run_root = self.run_root();
        let workflows = run_root.parent().ok_or_else(|| {
            WorkflowError::StateCorrupt(format!("{} has no runs directory", run_root.display()))
        })?;
        let run = crate::WorkflowStore::new(workflows).load_state(&self.run_id())?;
        self.require_session_executor(&run)
    }
}

#[cfg(test)]
#[path = "result_store_executor_tests.rs"]
mod executor_tests;
