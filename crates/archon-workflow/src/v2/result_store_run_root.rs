// Where the run this store belongs to lives, and the store of every run.

impl WorkflowV2ResultStore {
    /// The RUN's own directory — the parent of this store when the store is
    /// the run's `v2` subdirectory, and the store root itself otherwise.
    ///
    /// The store holds one engine's records inside a run directory that also
    /// holds the run's state, its event log and its artifact area. Callers
    /// that must talk about the whole run — the write boundary that keeps an
    /// agent out of the host's bookkeeping, for one — need that directory and
    /// not this one, and derive it here rather than each re-deriving the
    /// `v2` special case that [`Self::run_id`] already encodes.
    pub fn run_root(&self) -> &Path {
        if self.root.file_name().and_then(|name| name.to_str()) == Some("v2")
            && let Some(parent) = self.root.parent()
        {
            return parent;
        }
        &self.root
    }

    /// The directory every run of this project is kept in — the parent of
    /// [`Self::run_root`], since a run directory is the store root joined with
    /// the run id (`WorkflowStore::run_dir`).
    ///
    /// Stated here because this crate is the one that builds that layout. A
    /// consumer that needs "every run, not just this one" — the boundary that
    /// keeps an agent's walks out of accumulated history — must not guess it
    /// from a path it was handed.
    pub fn run_store_root(&self) -> Option<&Path> {
        self.run_root().parent()
    }
}
