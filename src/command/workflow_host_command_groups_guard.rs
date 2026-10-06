//! Settling durable command records, including a fallback that works when
//! directory entries cannot be created, renamed, or removed.
use super::{HostCommandGroupRecord, owner, unknown_path};
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct SettledMarker {
    settled_record: HostCommandGroupRecord,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SupervisedMarker {
    supervised_record: HostCommandGroupRecord,
}

pub(super) fn supervised_marker(record: &HostCommandGroupRecord) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&SupervisedMarker {
        supervised_record: record.clone(),
    })
}

/// Checkpointed and settled evidence overrides the original record. Legacy markers cannot
/// distinguish a crash from an unwritten stall, so they stay unknown.
pub(super) struct PendingRecord {
    pub(super) record: HostCommandGroupRecord,
    pub(super) settled: bool,
}

pub(super) fn pending_record(path: &Path) -> anyhow::Result<Option<PendingRecord>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "host command pending marker {} cannot be read ({error}); survivors unknown: verify the tree, then remove {} and resume again",
                path.display(),
                super::refusal_files(path)
            ));
        }
    };
    let decode = || -> Result<_, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value.get("settled_record").is_some() {
            Ok(Some(PendingRecord {
                record: serde_json::from_value::<SettledMarker>(value)?.settled_record,
                settled: true,
            }))
        } else if value.get("supervised_record").is_some() {
            // Only an explicit supervision marker permits crash healing.
            // Incomplete fallback writes cannot masquerade as this envelope.
            Ok(Some(PendingRecord {
                record: serde_json::from_value::<SupervisedMarker>(value)?.supervised_record,
                settled: false,
            }))
        } else {
            let mut legacy: HostCommandGroupRecord = serde_json::from_value(value)?;
            legacy.stalled = true;
            legacy.survivors_unknown = true;
            Ok(Some(PendingRecord {
                record: legacy,
                settled: true,
            }))
        }
    };
    decode().map_err(|error| anyhow::anyhow!(
        "host command pending marker {} is unreadable ({error}); survivors unknown: verify the tree, then remove {} and resume again", path.display(), super::refusal_files(path)))
}

/// Removes its record when the supervisor is done with the group, unless
/// teardown stalled and the record is kept for a resume to see.
#[derive(Debug)]
pub(crate) struct GroupRecordGuard {
    pub(super) path: PathBuf,
    pub(super) kept: bool,
    pub(super) pending: PathBuf,
    pub(super) marker: Option<GroupEvidence>,
    pub(super) record: HostCommandGroupRecord,
}

impl GroupRecordGuard {
    /// Where the record is written.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Keep the record: teardown stalled. `survivors` (pid, start time) are
    /// written into it so a resume refuses while any of them still runs;
    /// `None` means they are not known, and the record then runs until the
    /// tree is verified. If replacing the record fails, the already-open
    /// pending marker retains the complete settled record. A reader uses
    /// that evidence even after the owner exits, and heals once its members
    /// and scope end. Renaming to an unknown record is the last fallback.
    /// Any persistence failure is also returned as stall evidence.
    pub(crate) fn keep(mut self, survivors: Option<&[(u32, u64)]>) -> Option<String> {
        self.kept = true;
        // The registration snapshot survives unreadable or damaged files;
        // fallback evidence must not depend on reading the old empty record.
        let mut settled = self.record.clone();
        settled.stalled = true;
        settled.survivors_unknown = survivors.is_none();
        settled.survivors = survivors.map(<[_]>::to_vec).unwrap_or_default();
        // The marker is settled first, through the handle already open: if
        // the record rewrite then fails, the marker can no longer still say
        // "supervised", which a reader would judge as a crash once this
        // process exits.
        let marked = self.mark_settled(&settled);
        let rewritten = serde_json::to_vec(&settled)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let staged = self.path.with_extension("json.tmp");
                std::fs::write(&staged, bytes).map_err(|error| error.to_string())?;
                std::fs::rename(&staged, &self.path).map_err(|error| error.to_string())
            });
        match rewritten {
            Ok(()) => {
                self.remove_pending();
                None
            }
            Err(error) => {
                // Directory permissions cannot revoke this already-open
                // marker handle. In-place fallback needs no new directory
                // entry and retains identities that escaped the scope.
                if marked.is_ok() {
                    let evidence = format!(
                        "record rewrite failed ({error}); survivor evidence retained in pending marker"
                    );
                    tracing::error!(%evidence, record = %self.path.display(), "host command teardown used marker fallback");
                    return Some(evidence);
                }
                let unknown = unknown_path(&self.path);
                let renamed = std::fs::rename(&self.path, &unknown);
                let removed = if renamed.is_ok() {
                    self.remove_pending();
                    None
                } else {
                    // The marker alone then says "unknown survivors", even
                    // after this process exits; the old empty record, which
                    // a reader would judge after a crash, goes.
                    Some(std::fs::remove_file(&self.path))
                };
                let evidence = format!(
                    "survivors unknown: record rewrite failed ({error}); marker fallback: {marked:?}; fallback rename: {renamed:?}; record removal: {removed:?}"
                );
                tracing::error!(%evidence, record = %self.path.display(), "host command teardown record could not name survivors");
                Some(evidence)
            }
        }
    }

    /// The marker handle closes before its file is removed: on Windows an
    /// open handle keeps the name visible, and a sibling's stall check that
    /// reads it would be refused and pause the run.
    fn remove_pending(&mut self) {
        if let Some(marker) = &self.marker
            && let Err(error) = marker.close()
        {
            tracing::warn!(%error, marker = %self.pending.display(), "survivor marker handle could not close");
        }
        self.marker = None;
        let _ = std::fs::remove_file(&self.pending);
    }

    fn mark_settled(&mut self, record: &HostCommandGroupRecord) -> Result<(), String> {
        let bytes = serde_json::to_vec(&SettledMarker {
            settled_record: record.clone(),
        })
        .map_err(|error| error.to_string())?;
        self.marker
            .as_ref()
            .ok_or("marker handle missing")?
            .write(&bytes)
    }
}

impl Drop for GroupRecordGuard {
    fn drop(&mut self) {
        if !self.kept {
            // The marker first: a record left without it is judged by what
            // it names, which is right for a confirmed teardown. A marker
            // that cannot go keeps both, read as unknown while this process
            // lives and judged by what they name once it has exited.
            if let Some(marker) = &self.marker
                && let Err(error) = marker.close()
            {
                tracing::warn!(%error, marker = %self.pending.display(), "survivor marker handle could not close");
            }
            self.marker = None;
            match std::fs::remove_file(&self.pending) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&self.path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let _ = std::fs::remove_file(&self.path);
                }
                Err(error) => {
                    tracing::warn!(%error, "confirmed teardown marker could not be removed; record retained")
                }
            }
        }
        owner::release(&self.pending);
    }
}

/// Shared with tree scans; every adopted identity is durably retained before
/// a signal can erase its ancestry. The opened handle needs no directory write.
#[derive(Clone, Debug)]
pub(crate) struct GroupEvidence(std::sync::Arc<std::sync::Mutex<Evidence>>);
#[derive(Debug)]
struct Evidence {
    file: Option<std::fs::File>,
    record: HostCommandGroupRecord,
    failed: bool,
}
impl GroupEvidence {
    pub(super) fn new(file: std::fs::File, record: HostCommandGroupRecord) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Evidence {
            file: Some(file),
            record,
            failed: false,
        })))
    }
    fn close(&self) -> std::io::Result<()> {
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| std::io::Error::other("survivor marker lock unavailable"))?;
        state.file = None;
        Ok(())
    }
    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        let mut state = self
            .0
            .try_lock()
            .map_err(|_| "survivor marker lock unavailable")?;
        state.write(bytes).map_err(|error| error.to_string())
    }
    pub(crate) fn begin(&self) -> std::io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| std::io::Error::other("survivor marker lock unavailable"))?;
        state.record.survivors_unknown = true;
        state.checkpoint()
    }
    pub(crate) fn complete(&self, survivors: &[(u32, u64)]) -> std::io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| std::io::Error::other("survivor marker lock unavailable"))?;
        state.record.survivors = survivors.to_vec();
        state.record.survivors_unknown = state.failed;
        state.checkpoint()?;
        if state.failed {
            return Err(std::io::Error::other(
                "an earlier survivor marker write failed; survivors unknown",
            ));
        }
        Ok(())
    }
    pub(crate) fn remember(&self, identities: &[(u32, u64)]) -> std::io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| std::io::Error::other("survivor marker lock unavailable"))?;
        // The tracker's complete pin set only forgets proven exits/reuse.
        // Replace it instead of accumulating an unbounded history of dead pids.
        state.record.survivors = identities.to_vec();
        state.checkpoint()
    }
}
impl Evidence {
    fn checkpoint(&mut self) -> std::io::Result<()> {
        let bytes = supervised_marker(&self.record)?;
        self.write(&bytes)
    }
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let result = (|| {
            // Invalidate and sync first: a crash partway through the replacement
            // leaves unreadable evidence, never an old empty survivor list.
            let file = self
                .file
                .as_mut()
                .ok_or_else(|| std::io::Error::other("survivor marker handle closed"))?;
            file.set_len(0)?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(bytes)?;
            file.sync_all()
        })();
        if result.is_err() {
            self.failed = true;
            self.record.survivors_unknown = true;
        }
        result
    }
}
#[cfg(unix)]
impl archon_shell::process_tree::IdentityRecorder for GroupEvidence {
    fn checkpoint(&mut self, pinned: &[archon_shell::process_tree::Pinned]) -> std::io::Result<()> {
        let identities: Vec<_> = pinned.iter().map(|pin| (pin.pid, pin.start)).collect();
        GroupEvidence::remember(self, &identities)
    }
}
impl GroupRecordGuard {
    pub(crate) fn evidence(&self) -> Option<GroupEvidence> {
        self.marker.clone()
    }
}
