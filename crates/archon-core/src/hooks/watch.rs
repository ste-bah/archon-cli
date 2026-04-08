//! Dynamic file watch paths for the hooks system (TASK-HOOK-023 / REQ-HOOK-017).
//!
//! Hooks can return `watch_paths: Vec<String>` in their response. These paths
//! are registered with a file watcher (`notify` crate). When watched files
//! change, the caller fires a `FileChanged` hook event. Max paths per session
//! is capped. Cleanup happens on `SessionEnd`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// Manages dynamic file watch paths for the hooks system.
///
/// Hooks can return `watch_paths` in their response, which are registered here.
/// When watched files change, `FileChanged` events are fired by the caller.
pub struct FileWatchManager {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watched_paths: Mutex<HashSet<PathBuf>>,
    max_paths: usize,
}

impl FileWatchManager {
    /// Create a new `FileWatchManager` with the given maximum path count.
    pub fn new(max_paths: usize) -> Self {
        // Create a watcher that logs events. The actual FileChanged hook
        // firing is done by the caller when they receive watcher events.
        let watcher =
            notify::recommended_watcher(|res: Result<notify::Event, notify::Error>| match res {
                Ok(event) => {
                    tracing::debug!(paths = ?event.paths, kind = ?event.kind, "file watch event");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "file watch error");
                }
            })
            .ok();

        Self {
            watcher: Mutex::new(watcher),
            watched_paths: Mutex::new(HashSet::new()),
            max_paths,
        }
    }

    /// Add watch paths. Paths beyond `max_paths` are silently dropped with a warning.
    /// Duplicate paths are ignored. Non-existent paths may fail to register with
    /// the underlying watcher but will not cause a panic.
    pub fn add_watch_paths(&self, paths: Vec<String>) {
        let mut watched = self.watched_paths.lock().unwrap_or_else(|p| p.into_inner());
        let mut watcher_guard = self.watcher.lock().unwrap_or_else(|p| p.into_inner());

        for path_str in paths {
            if watched.len() >= self.max_paths {
                tracing::warn!(
                    limit = self.max_paths,
                    path = %path_str,
                    "watch path limit reached, ignoring"
                );
                break;
            }
            let path = PathBuf::from(&path_str);
            if watched.insert(path.clone()) {
                if let Some(ref mut w) = *watcher_guard {
                    if let Err(e) = w.watch(&path, RecursiveMode::NonRecursive) {
                        tracing::warn!(path = %path_str, error = %e, "failed to watch path");
                    }
                }
            }
        }
    }

    /// Remove all watches. Called on `SessionEnd` to clean up.
    pub fn clear(&self) {
        let mut watched = self.watched_paths.lock().unwrap_or_else(|p| p.into_inner());
        let mut watcher_guard = self.watcher.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(ref mut w) = *watcher_guard {
            for path in watched.drain() {
                let _ = w.unwatch(&path);
            }
        } else {
            watched.clear();
        }
    }

    /// Number of currently watched paths.
    pub fn watched_count(&self) -> usize {
        self.watched_paths
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}
