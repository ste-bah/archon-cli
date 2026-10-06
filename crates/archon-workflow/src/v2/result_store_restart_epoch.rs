// The restart epoch (Issue-256, round 2).
//
// Every restart moves `v2/restart-epoch.json` on, under the run lock. A
// store remembers the epoch it opened with; a live session's restore and
// persistence check it under the same lock and refuse once a restart moved
// it. This covers every run kind: a fixed decomposition also checks its
// generation, but a run that owns no generation has only this.

const RESTART_EPOCH_FILE: &str = "restart-epoch.json";

/// The restart epoch recorded under the v2 store `root`: 0 before the first
/// restart.
fn read_restart_epoch(root: &Path) -> WorkflowResult<u64> {
    let path = root.join(RESTART_EPOCH_FILE);
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(WorkflowError::io(&path, err)),
    };
    serde_json::from_slice::<serde_json::Value>(&raw)?
        .get("epoch")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            WorkflowError::StateCorrupt(format!("{} names no restart epoch", path.display()))
        })
}

impl WorkflowV2ResultStore {
    /// The restart epoch on disk now.
    pub fn restart_epoch(&self) -> WorkflowResult<u64> {
        read_restart_epoch(&self.root)
    }

    /// Move the restart epoch on, durably. Called by a restart, under the
    /// run lock.
    pub fn bump_restart_epoch(&self) -> WorkflowResult<u64> {
        self.with_session_write_lock(|| {
            let next = self.restart_epoch()?.saturating_add(1);
            write_json_synced(
                &self.root.join(RESTART_EPOCH_FILE),
                &serde_json::json!({ "epoch": next }),
            )?;
            Ok(next)
        })
    }

    /// Refuse a write of this store's session once a restart moved the
    /// epoch on after the store opened. The caller holds the run lock, the
    /// lock every restart holds.
    pub fn require_session_restart_epoch(&self) -> WorkflowResult<()> {
        let now = self.restart_epoch()?;
        match self.opened_restart_epoch {
            Some(opened) if opened == now => Ok(()),
            opened => Err(WorkflowError::ControlCancelled(format!(
                "a restart of run {} moved the restart epoch from {} to {now} after this session opened; the session writes nothing more",
                self.run_id(),
                opened.map_or_else(|| "unreadable".to_string(), |epoch| epoch.to_string()),
            ))),
        }
    }
}

impl WorkflowV2ResultStore {
    /// A session write under the run's control lock. This is the same lock
    /// `WorkflowStore::with_run_lock` and every restart hold; use the run
    /// directory directly so standalone result stores have the same boundary.
    pub fn with_session_write_lock<T>(
        &self,
        write: impl FnOnce() -> WorkflowResult<T>,
    ) -> WorkflowResult<T> {
        self.session_workflow_store()?
            .with_run_lock(&self.run_id(), |_| {
                self.require_session_owner()?;
                write()
            })
    }

    /// The same captured-owner store used by host, audit and admission fences.
    pub fn session_workflow_store(&self) -> WorkflowResult<crate::WorkflowStore> {
        let root = self.run_root().parent().ok_or_else(|| {
            WorkflowError::StateCorrupt("result store has no runs directory".into())
        })?;
        let store = crate::WorkflowStore::new(root);
        Ok(match self.session_executor() {
            Some(generation) => store.for_executor(&self.run_id(), generation),
            None => store,
        })
    }
}
