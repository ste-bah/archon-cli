// Where superseded records go (D79), and the per-call layout of the call
// archive (Issue-254).
//
// A call record a new execution displaces is archived, never destroyed:
// post-run adjudication and history reuse (Issue-250) are built on it. The
// archive used to be one flat directory for every call of the run,
// `results/superseded/`, so each history lookup (`call_history`,
// `next_attempt`, both on every dispatch) listed every archived attempt of
// every call to find its own. It is now one directory per call,
// `results/history/<slot stem>/`, and a lookup lists only that one.
//
// Retention: archived records are never pruned. A record that looks
// obsolete can still be the last accepted answer of its call, or the
// invalidation mark that stops an older answer from being reused, so the
// store keeps all of them. The per-call directory keeps the cost of a
// lookup proportional to the call's own attempts, not to the run's.
//
// A flat archive written by an older build is moved into the per-call
// directories the first time a lookup touches the store
// (`migrate_flat_archive`). The move is one rename per file, so a crash in
// the middle leaves every file in exactly one of the two places and the
// next lookup finishes the move.

/// The legacy flat call archive, read only to migrate it.
const FLAT_CALL_ARCHIVE: &str = "superseded";
/// The per-call call archive: `results/history/<slot stem>/`.
const CALL_ARCHIVE: &str = "history";
/// Where a legacy archived file that names no call goes.
const UNSORTED_ARCHIVE: &str = "_unsorted";

impl WorkflowV2ResultStore {
    /// The directory that holds every archived record of `call_id`.
    pub fn call_history_dir(&self, call_id: &str) -> PathBuf {
        let slot = self.result_path(call_id);
        let stem = slot
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("record");
        self.root.join("results").join(CALL_ARCHIVE).join(stem)
    }

    fn call_archive_root(&self) -> PathBuf {
        self.root.join("results").join(CALL_ARCHIVE)
    }

    /// Move every file of the legacy flat archive into the directory of the
    /// call it belongs to, then remove the emptied flat directory. A file
    /// another process moved first is skipped. Cost when there is nothing to
    /// migrate: one failed directory open.
    fn migrate_flat_archive(&self) -> WorkflowResult<()> {
        let flat = self.root.join("results").join(FLAT_CALL_ARCHIVE);
        let entries = match fs::read_dir(&flat) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(WorkflowError::io(&flat, err)),
        };
        for entry in entries {
            let entry = entry.map_err(|err| WorkflowError::io(&flat, err))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let dir = match self.legacy_archive_stem(&name, &path) {
                Some(stem) => self.call_archive_root().join(stem),
                None => self.call_archive_root().join(UNSORTED_ARCHIVE),
            };
            fs::create_dir_all(&dir).map_err(|err| WorkflowError::io(&dir, err))?;
            let target = dir.join(&name);
            match fs::rename(&path, &target) {
                Ok(()) if self.durable => {
                    // Persist the destination and all newly created directory
                    // links before making the source removal durable. Otherwise
                    // a rewind can outlive the invalidated destination and an
                    // old flat entry can return on the next migration.
                    crate::durable_io::sync_file(&target)?;
                    crate::durable_io::sync_dir(&dir)?;
                    crate::durable_io::sync_dir(&self.call_archive_root())?;
                    crate::durable_io::sync_dir(&self.root.join("results"))?;
                    crate::durable_io::sync_dir(&flat)?;
                }
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(WorkflowError::io(&target, err)),
            }
        }
        // Only an empty directory is removed; anything left stays readable
        // and is moved by the next lookup.
        match fs::remove_dir(&flat) {
            Ok(()) if self.durable => crate::durable_io::sync_dir(&self.root.join("results"))?,
            Ok(()) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            // Outside a restart a failed removal is retried by the next lookup;
            // only the restart path needs the flat directory gone durably.
            Err(_) if !self.durable => {}
            Err(err) => return Err(WorkflowError::io(&flat, err)),
        }
        Ok(())
    }

    /// The slot stem a legacy archived file belongs to: from its name
    /// (`<slot stem>-<time>-<pid>-<seq>.json`, as the older build wrote it),
    /// or else from the call id in its JSON.
    fn legacy_archive_stem(&self, name: &str, path: &Path) -> Option<String> {
        let from_name = name.strip_suffix(".json").and_then(|base| {
            let mut parts = base.rsplitn(4, '-');
            let numeric = (0..3).all(|_| {
                parts.next().is_some_and(|part| {
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
                })
            });
            parts
                .next()
                .filter(|stem| numeric && !stem.is_empty())
                .map(str::to_string)
        });
        from_name.or_else(|| {
            let raw = fs::read_to_string(path).ok()?;
            let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
            let call_id = value.get("call")?.get("id")?.as_str()?;
            self.call_history_dir(call_id)
                .file_name()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
    }
}

/// D79: a call id re-executed by a later cycle (e.g. a terminal-gate reroute)
/// must never silently destroy the prior record — post-run adjudication is
/// built on this history. When a NEW execution claims an occupied slot, the
/// existing file moves into a `superseded/` sibling directory first; an
/// unreadable existing file is archived rather than clobbered.
fn archive_superseded_json<T: DeserializeOwned>(
    path: &Path,
    durable: bool,
    same_execution: impl FnOnce(&T) -> bool,
) -> WorkflowResult<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    archive_superseded_json_into(path, &parent.join("superseded"), durable, same_execution)
}

/// [`archive_superseded_json`] into `dir`: a call record goes to its call's
/// own directory ([`WorkflowV2ResultStore::call_history_dir`]).
fn archive_superseded_json_into<T: DeserializeOwned>(
    path: &Path,
    dir: &Path,
    durable: bool,
    same_execution: impl FnOnce(&T) -> bool,
) -> WorkflowResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if let Ok(raw) = fs::read_to_string(path)
        && let Ok(existing) = serde_json::from_str::<T>(&raw)
        && same_execution(&existing)
    {
        return Ok(());
    }
    archive_file_into(path, dir, durable)?;
    Ok(())
}
