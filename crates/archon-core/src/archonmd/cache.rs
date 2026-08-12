//! Issue #171 Part 5 — mtime-keyed cache for the hierarchical ARCHON.md render.
//!
//! `with_archon_md` used to call [`super::load_hierarchical_archon_md`] on every
//! subagent spawn, so a 10-agent fan-out walked and *read* the identical file
//! hierarchy ten times. The render depends on nothing but `working_dir`, so one
//! cache serves every subagent type.
//!
//! Revalidation is deliberately cheap but complete:
//!
//! - The hierarchy is **rediscovered** on every lookup ([`super::discover_archon_md_paths`],
//!   stat-only). A file added to or removed from the hierarchy therefore changes
//!   the discovered path list, which is part of the key — an mtime set alone
//!   would not notice either.
//! - Each discovered file contributes `(len, mtime)`. Any change to any file
//!   invalidates the entry.
//!
//! Only the file *reads* are skipped on a hit, which is the I/O amplification
//! the issue is about.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Identity of one discovered instructions file: its path plus the cheap
/// metadata that changes whenever its content does.
type FileStamp = (PathBuf, u64, Option<SystemTime>);

struct Entry {
    stamps: Vec<FileStamp>,
    rendered: Arc<str>,
}

/// Observed cache behaviour, for spawn fixtures and bench reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchonMdCacheStats {
    /// Lookups served from the cached render (no file reads).
    pub hits: usize,
    /// Lookups that had to read the hierarchy (cold or invalidated).
    pub misses: usize,
    /// Total instructions files read from disk across all misses.
    pub files_read: usize,
}

/// Session-scoped cache of the rendered ARCHON.md hierarchy, keyed by
/// `working_dir` and revalidated against the discovered files' `(len, mtime)`.
#[derive(Debug, Default)]
pub struct ArchonMdCache {
    entries: Mutex<HashMap<PathBuf, Entry>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    files_read: AtomicUsize,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("files", &self.stamps.len())
            .field("rendered_len", &self.rendered.len())
            .finish()
    }
}

impl ArchonMdCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the rendered ARCHON.md hierarchy for `working_dir`, reading the
    /// files only when the hierarchy has changed since the last call.
    pub fn load(&self, working_dir: &Path) -> Arc<str> {
        let paths = super::discover_archon_md_paths(working_dir);
        let stamps = stamp_all(&paths);

        // Fast path: a live entry whose discovered files are unchanged.
        match self.entries.lock() {
            Ok(entries) => {
                if let Some(entry) = entries.get(working_dir)
                    && entry.stamps == stamps
                {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(
                        working_dir = %working_dir.display(),
                        files = stamps.len(),
                        "archon-md cache hit (mtime set unchanged)"
                    );
                    return Arc::clone(&entry.rendered);
                }
            }
            Err(_) => {
                // A poisoned lock must not silently degrade into an empty
                // render — fall through to a full, uncached load.
                tracing::warn!("archon-md cache mutex poisoned; loading uncached");
                self.misses.fetch_add(1, Ordering::Relaxed);
                self.files_read.fetch_add(paths.len(), Ordering::Relaxed);
                return Arc::from(super::join_sections(super::render_sections(&paths), None));
            }
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        self.files_read.fetch_add(paths.len(), Ordering::Relaxed);
        tracing::debug!(
            working_dir = %working_dir.display(),
            files = paths.len(),
            "archon-md cache miss; re-reading hierarchy"
        );

        let rendered: Arc<str> =
            Arc::from(super::join_sections(super::render_sections(&paths), None));

        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                working_dir.to_path_buf(),
                Entry {
                    stamps,
                    rendered: Arc::clone(&rendered),
                },
            );
        }
        rendered
    }

    /// Snapshot the hit/miss/read counters.
    pub fn stats(&self) -> ArchonMdCacheStats {
        ArchonMdCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            files_read: self.files_read.load(Ordering::Relaxed),
        }
    }
}

/// Stat every discovered file. A file that vanished between discovery and this
/// call stamps as `(0, None)`, which differs from any successful stat and so
/// forces a re-read.
fn stamp_all(paths: &[PathBuf]) -> Vec<FileStamp> {
    paths
        .iter()
        .map(|path| match std::fs::metadata(path) {
            Ok(meta) => (path.clone(), meta.len(), meta.modified().ok()),
            Err(_) => (path.clone(), 0, None),
        })
        .collect()
}

#[cfg(test)]
mod tests;
