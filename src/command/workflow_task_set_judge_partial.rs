//! The partial reply of a judge batch, saved for a retry (Issue 260).
//!
//! What is saved is exactly the work a retry can still use: the open JSON
//! document of the reply being continued, redacted, and the count of chunks
//! that built it. That count is the progress the host executor reads, so a
//! reply the judge spends (complete but unusable, or discarded) takes its
//! credit with it. Everything read back is validated (schema, key, size,
//! counter bounds); anything else is no saved reply. On Unix the file is
//! private (0600, its directory 0700).

use std::path::{Path, PathBuf};

use super::reply::{Document, document};
use crate::command::workflow_freeze_budget::FreezeProgress;

/// The most bytes a partial reply may hold. Far above any verdict document
/// (a few hundred bytes per check); a reply past it is no progress.
pub(crate) const MAX_PARTIAL_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// The saved file's layout; changing it, or the continuation protocol (the
/// prompt a continuation is asked with), invalidates every saved reply.
const SCHEMA: u32 = 2;

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Saved {
    schema: u32,
    key: String,
    reply: String,
    chunks: u64,
    /// Raw bytes added by credited chunks, independent of redaction and
    /// retained across resumes. None supports earlier schema-2 caches.
    #[serde(default)]
    credit_bytes: Option<u64>,
    /// The reply was judged and its verdicts saved: its chunks are part of
    /// that persisted work and keep counting.
    #[serde(default)]
    completed: bool,
}

impl Saved {
    /// Validate credit independently of the redacted reply's length. Each
    /// chunk grew the bounded raw document; redaction can shrink it.
    fn usable(&self, key: &str) -> bool {
        let credit_bound = self.credit_bytes.unwrap_or(MAX_PARTIAL_REPLY_BYTES as u64);
        let counted = if self.completed {
            self.reply.is_empty() && self.chunks <= credit_bound
        } else if self.reply.is_empty() {
            self.chunks == 0
        } else {
            self.chunks <= credit_bound && document(&self.reply) == Document::Open
        };
        self.schema == SCHEMA
            && self.key == key
            && self.reply.len() <= MAX_PARTIAL_REPLY_BYTES
            && counted
    }
}

/// A batch's partial reply under the freeze cache, and the progress it
/// counts.
pub(crate) struct PartialReply<'a> {
    path: PathBuf,
    key: String,
    progress: &'a FreezeProgress,
}

impl<'a> PartialReply<'a> {
    /// The partial reply of the batch whose judge input digests to `batch`.
    pub(crate) fn new(dir: &Path, batch: &str, progress: &'a FreezeProgress) -> Self {
        let key = archon_workflow::task_set_contract::content_digest(
            format!("{batch}|{}|{SCHEMA}", super::CONTINUE_PROMPT).as_bytes(),
        );
        Self {
            path: dir.join(format!("partial-{key}.json")),
            key,
            progress,
        }
    }

    fn load(&self) -> Saved {
        let readable = std::fs::metadata(&self.path)
            .is_ok_and(|meta| meta.len() <= (MAX_PARTIAL_REPLY_BYTES as u64) * 2 + 4096);
        readable
            .then(|| std::fs::read(&self.path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .filter(|saved| saved.usable(&self.key))
            .unwrap_or_default()
    }

    fn store(&self, reply: &str, chunks: u64, credit_bytes: u64, completed: bool) -> bool {
        let saved = Saved {
            schema: SCHEMA,
            key: self.key.clone(),
            // Never secrets on disk: the same redaction the logs get.
            reply: archon_observability::redaction::redact_text(reply),
            chunks,
            credit_bytes: Some(credit_bytes),
            completed,
        };
        // Check the bytes actually persisted, after redaction. A rejected
        // extension leaves the previous continuation and its credit intact.
        if !saved.usable(&self.key) {
            return false;
        }
        if let Err(error) = write_private(&self.path, &saved) {
            eprintln!(
                "the judge's partial reply could not be saved at {} ({error}); a retry continues from less",
                self.path.display()
            );
            return false;
        }
        true
    }

    /// Counts the chunks a saved, still usable reply holds, once per freeze
    /// attempt.
    pub(crate) fn count_saved(&self, verdict_saved: bool) {
        let saved = self.load();
        if saved.completed && !verdict_saved {
            // An orphan marker is neither progress nor a continuation. Clear
            // it before a fresh reply starts counting from its first chunk.
            self.store("", 0, 0, false);
            return;
        }
        self.progress.reused_judged(saved.chunks);
    }

    /// The saved reply to continue and its chunk count, when there is one.
    pub(super) fn open_reply(&self) -> Option<(String, u64)> {
        let saved = self.load();
        (!saved.completed && !saved.reply.is_empty()).then_some((saved.reply, saved.chunks))
    }

    /// Saves `reply`, now built of `chunks` chunks, and reports one unit.
    pub(super) fn extended(&self, reply: &str, chunks: u64, added_bytes: usize) -> bool {
        let previous = self.load();
        if added_bytes == 0 || chunks != previous.chunks.saturating_add(1) {
            return false;
        }
        let credit_bytes = previous
            .credit_bytes
            .unwrap_or(previous.chunks)
            .saturating_add(added_bytes as u64);
        if !self.store(reply, chunks, credit_bytes, false) {
            return false;
        }
        self.progress.saved(true);
        true
    }

    /// The reply is spent (judged, unusable or discarded): the next ask
    /// starts afresh, and its chunks no longer count as progress.
    pub(crate) fn spent(&self) {
        let saved = self.load();
        if saved.completed || (saved.reply.is_empty() && saved.chunks == 0) {
            return;
        }
        if self.store("", 0, 0, false) {
            self.progress.withdraw_judged(saved.chunks);
        }
    }

    /// The reply was judged and its verdicts are saved: the reply itself is
    /// no longer kept, and its chunks count as part of the saved verdicts.
    pub(crate) fn completed(&self) {
        let saved = self.load();
        if !saved.completed && !saved.reply.is_empty() {
            self.store(
                "",
                saved.chunks,
                saved.credit_bytes.unwrap_or(saved.chunks),
                true,
            );
        }
    }
}

/// Writes `saved` to `path` atomically, private to the user on Unix.
fn write_private(path: &Path, saved: &Saved) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let staging = path.with_extension(format!("{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(saved).map_err(std::io::Error::other)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| {
        use std::io::Write;
        let mut file = options.open(&staging)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&staging, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    written
}

#[cfg(test)]
#[path = "workflow_task_set_judge_partial_tests.rs"]
mod tests;
