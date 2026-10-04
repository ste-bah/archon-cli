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

/// Settled evidence overrides the original record. Legacy markers cannot
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
                "host command pending marker {} cannot be read ({error}); survivors unknown: verify the tree before removing it",
                path.display()
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
        "host command pending marker {} is unreadable ({error}); survivors unknown: verify the tree before removing it", path.display()))
}

/// Removes its record when the supervisor is done with the group, unless
/// teardown stalled and the record is kept for a resume to see.
#[derive(Debug)]
pub(crate) struct GroupRecordGuard {
    pub(super) path: PathBuf,
    pub(super) kept: bool,
    pub(super) pending: PathBuf,
    pub(super) marker: Option<std::fs::File>,
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
        let rewritten = serde_json::to_vec(&settled)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                let staged = self.path.with_extension("json.tmp");
                std::fs::write(&staged, bytes).map_err(|error| error.to_string())?;
                std::fs::rename(&staged, &self.path).map_err(|error| error.to_string())
            });
        match rewritten {
            Ok(()) => {
                let _ = std::fs::remove_file(&self.pending);
                None
            }
            Err(error) => {
                // Directory permissions cannot revoke this already-open
                // marker handle. In-place fallback needs no new directory
                // entry and retains identities that escaped the scope.
                let marked = self.mark_settled(&settled);
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
                    let _ = std::fs::remove_file(&self.pending);
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

    fn mark_settled(&mut self, record: &HostCommandGroupRecord) -> Result<(), String> {
        use std::io::{Seek, SeekFrom, Write};
        let bytes = serde_json::to_vec(&SettledMarker {
            settled_record: record.clone(),
        })
        .map_err(|error| error.to_string())?;
        let marker = self.marker.as_mut().ok_or("marker handle missing")?;
        // Invalidate the old crash marker before writing. A partial write
        // is unreadable evidence, never permission to discard survivors.
        marker.set_len(0).map_err(|error| error.to_string())?;
        marker
            .seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        marker
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        marker.sync_all().map_err(|error| error.to_string())
    }
}

impl Drop for GroupRecordGuard {
    fn drop(&mut self) {
        if !self.kept {
            // The marker first: a record left without it is judged by what
            // it names, which is right for a confirmed teardown. A marker
            // that cannot go keeps both, read as unknown while this process
            // lives and judged by what they name once it has exited.
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
