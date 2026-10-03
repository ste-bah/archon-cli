//! The scratch Cargo home's content digest, hashed once per observation
//! and then re-verified per check from file metadata (Issue 255).
//!
//! Each check records the Cargo home's digest before and after it runs
//! ([`super::CheckEvidence`]), so a check that changes it is visible in the
//! evidence. Hashing every byte twice per check cost about 50 s on a 2.5 GB
//! seeded registry. A [`TreeSeal`] produces exactly the digest a full
//! [`super::inventory`] would, but re-reads only the files whose metadata
//! moved since it last hashed them.
//!
//! Why the metadata is enough. A file's digest is reused only when its
//! device, inode, mode, size, modification time AND status-change time
//! (`ctime`) are all exactly those seen when it was hashed. Every content
//! write, truncation, rename over it, `utimes` call or permission change
//! sets `ctime` to the current time, and no unprivileged process can set
//! `ctime` back; a replacement file is a new inode with a new `ctime`.
//! Added and removed paths change the walked path set itself, and links and
//! directories are compared by value on every walk.
//!
//! Two gaps remain, and both are covered or accepted:
//!
//! - Timestamp granularity: a write in the same timestamp tick as the stat
//!   that preceded the hash leaves every field unchanged on a coarse
//!   filesystem. A digest is therefore trusted only when the file's `ctime`
//!   was at least [`RACY_SECS`] older than the clock before that stat (the
//!   "racy clean" rule git's index uses); younger files are re-read.
//! - Writes the kernel has not yet stamped (a `MAP_SHARED` mapping whose
//!   timestamp update is deferred to write-back), and processes able to
//!   change the system clock or write the raw device. The Cargo home digest
//!   is evidence, not a gate: no decision reads it, and the check's own
//!   process group is reaped before the walk. Source and project trees,
//!   which do gate warm reuse, are still hashed in full.
use super::*;
use std::time::{Duration, SystemTime};

/// See the module docs: the oldest a file's last status change may be,
/// relative to the clock before its stat, for its digest to be reused.
pub(super) const RACY_SECS: u64 = 2;

/// The metadata a reused digest must match exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stat {
    dev: u64,
    ino: u64,
    mode: u32,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl Stat {
    #[cfg(unix)]
    fn of(meta: &std::fs::Metadata) -> Option<Self> {
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
            mode: meta.mode(),
            len: meta.size(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        })
    }

    /// Without a status-change time nothing is ever reused.
    #[cfg(not(unix))]
    fn of(_meta: &std::fs::Metadata) -> Option<Self> {
        None
    }

    fn changed_at(&self) -> Option<SystemTime> {
        let secs = u64::try_from(self.ctime.0).ok()?;
        let nanos = u32::try_from(self.ctime.1).ok()?;
        SystemTime::UNIX_EPOCH.checked_add(Duration::new(secs, nanos))
    }
}

struct Known {
    stat: Stat,
    digest: String,
    /// The clock read before the stat this digest was taken under.
    clock: SystemTime,
}

/// Content digests of one tree, verified from metadata between walks.
pub(super) struct TreeSeal {
    known: BTreeMap<PathBuf, Known>,
    racy: Duration,
    hashed: usize,
}

impl Default for TreeSeal {
    fn default() -> Self {
        Self::with_racy(Duration::from_secs(RACY_SECS))
    }
}

impl TreeSeal {
    pub(super) fn with_racy(racy: Duration) -> Self {
        Self {
            known: BTreeMap::new(),
            racy,
            hashed: 0,
        }
    }

    /// Files read and hashed so far.
    #[cfg(test)]
    pub(super) fn hashed(&self) -> usize {
        self.hashed
    }

    /// `content_digest` of `root`'s [`super::inventory`], reading only the
    /// files whose metadata changed (or are too young to trust) since this
    /// seal last hashed them.
    pub(super) fn digest(&mut self, root: &Path) -> WorkflowResult<String> {
        let mut visited = std::collections::BTreeSet::new();
        let inventory = io::inventory_with(root, &mut |path, meta| {
            visited.insert(path.to_path_buf());
            self.file_digest(path, meta)
        })?;
        self.known.retain(|path, _| visited.contains(path));
        Ok(crate::task_set_contract::content_digest(
            &serde_json::to_vec(&inventory)?,
        ))
    }

    fn file_digest(&mut self, path: &Path, meta: &std::fs::Metadata) -> WorkflowResult<String> {
        if let (Some(known), Some(stat)) = (self.known.get(path), Stat::of(meta))
            && known.stat == stat
            && stat
                .changed_at()
                .is_some_and(|changed| changed + self.racy <= known.clock)
        {
            return Ok(known.digest.clone());
        }
        // The clock first, then a fresh stat: the clock is never later than
        // the stat it vouches for.
        let clock = SystemTime::now();
        let fresh = std::fs::symlink_metadata(path).map_err(|e| WorkflowError::io(path, e))?;
        let digest = crate::task_set_contract::content_digest(&io::read(path)?);
        self.hashed += 1;
        match Stat::of(&fresh) {
            Some(stat) => {
                self.known.insert(
                    path.to_path_buf(),
                    Known {
                        stat,
                        digest: digest.clone(),
                        clock,
                    },
                );
            }
            None => {
                self.known.remove(path);
            }
        }
        Ok(digest)
    }
}

#[cfg(all(test, unix))]
#[path = "acceptance_scratch_seal_tests.rs"]
mod tests;
