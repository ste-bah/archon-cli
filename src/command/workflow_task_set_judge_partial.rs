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
    /// The reply was judged and its verdicts saved: its chunks are part of
    /// that persisted work and keep counting.
    #[serde(default)]
    completed: bool,
}

impl Saved {
    /// Whether this is a reply a retry may continue, with sane counters:
    /// each saved chunk added at least one byte.
    fn usable(&self, key: &str) -> bool {
        let counted = if self.completed {
            self.reply.is_empty() && self.chunks <= MAX_PARTIAL_REPLY_BYTES as u64
        } else if self.reply.is_empty() {
            self.chunks == 0
        } else {
            self.chunks <= self.reply.len() as u64 && document(&self.reply) == Document::Open
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

    fn store(&self, reply: &str, chunks: u64, completed: bool) {
        let saved = Saved {
            schema: SCHEMA,
            key: self.key.clone(),
            // Never secrets on disk: the same redaction the logs get.
            reply: archon_observability::redaction::redact_text(reply),
            chunks,
            completed,
        };
        if let Err(error) = write_private(&self.path, &saved) {
            eprintln!(
                "the judge's partial reply could not be saved at {} ({error}); a retry continues from less",
                self.path.display()
            );
        }
    }

    /// Counts the chunks a saved, still usable reply holds, once per freeze
    /// attempt.
    pub(crate) fn count_saved(&self) {
        self.progress.reused_judged(self.load().chunks);
    }

    /// The saved reply to continue and its chunk count, when there is one.
    pub(super) fn open_reply(&self) -> Option<(String, u64)> {
        let saved = self.load();
        (!saved.completed && !saved.reply.is_empty()).then_some((saved.reply, saved.chunks))
    }

    /// Saves `reply`, now built of `chunks` chunks, and reports one unit.
    pub(super) fn extended(&self, reply: &str, chunks: u64) {
        self.store(reply, chunks, false);
        self.progress.saved(true);
    }

    /// The reply is spent (judged, unusable or discarded): the next ask
    /// starts afresh, and its chunks no longer count as progress.
    pub(crate) fn spent(&self) {
        let saved = self.load();
        if saved.completed || (saved.reply.is_empty() && saved.chunks == 0) {
            return;
        }
        self.progress.withdraw_judged(saved.chunks);
        self.store("", 0, false);
    }

    /// The reply was judged and its verdicts are saved: the reply itself is
    /// no longer kept, and its chunks count as part of the saved verdicts.
    pub(crate) fn completed(&self) {
        let saved = self.load();
        if !saved.completed && !saved.reply.is_empty() {
            self.store("", saved.chunks, true);
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
