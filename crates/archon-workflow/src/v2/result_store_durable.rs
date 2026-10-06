// Durable restart writes (Issue-267, round 2).
//
// A restart commits its v2 cache invalidation and branch revocation, then
// the rewound run state (`store::save_state`, a synced temporary file). A
// plain write or rename is not on disk until the file and its directory are
// synced, so a machine crash could keep the rewound state and lose the
// invalidation: the old answers would then be replayed. A store made
// `with_durable_writes` syncs each written file before its rename and each
// directory a name was created, renamed or removed in, so everything the
// restart wrote is on disk before it saves the state.

impl WorkflowV2ResultStore {
    /// This store with every write and rename synced to disk.
    pub fn with_durable_writes(mut self) -> Self {
        self.durable = true;
        self
    }

    /// Write `value` to `path`, synced when this store is durable.
    fn write_record<T: Serialize>(&self, path: &Path, value: &T) -> WorkflowResult<()> {
        self.with_session_write_lock(|| {
            if self.durable {
                write_json_synced(path, value)
            } else {
                write_json(path, value)
            }
        })
    }
}

/// [`write_json`], durably: the temporary file is synced before the rename
/// and the directory after it.
fn write_json_synced<T: Serialize>(path: &Path, value: &T) -> WorkflowResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
    crate::durable_io::sync_file(&tmp)?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::io(path, err))?;
    crate::durable_io::sync_dir(parent)
}
