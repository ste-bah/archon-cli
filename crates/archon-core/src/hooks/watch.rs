//! Dynamic file watch paths for the hooks system (TASK-HOOK-023 / REQ-HOOK-017).
//!
//! Hooks can return `watch_paths: Vec<String>` in their response. These paths
//! are registered with a file watcher (`notify` crate). When watched files
//! change, the caller fires a `FileChanged` hook event. Max paths per session
//! is capped. Cleanup happens on `SessionEnd`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use notify::RecursiveMode;

use crate::bounded_watcher::BoundedWatcher;

/// Manages dynamic file watch paths for the hooks system.
///
/// Hooks can return `watch_paths` in their response, which are registered here.
/// When watched files change, `FileChanged` events are fired by the caller.
pub struct FileWatchManager {
    watcher: Mutex<Result<BoundedWatcher, String>>,
    watched_paths: Mutex<HashSet<PathBuf>>,
    max_paths: usize,
}

impl FileWatchManager {
    /// Create a new `FileWatchManager` with the given maximum path count.
    pub fn new(max_paths: usize) -> Self {
        // Create a watcher that logs events. The actual FileChanged hook
        // firing is done by the caller when they receive watcher events.
        let watcher = BoundedWatcher::new(
            |res: Result<notify::Event, notify::Error>| match res {
                Ok(event) => {
                    tracing::debug!(paths = ?event.paths, kind = ?event.kind, "file watch event");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "file watch error");
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| e.to_string());

        Self {
            watcher: Mutex::new(watcher),
            watched_paths: Mutex::new(HashSet::new()),
            max_paths,
        }
    }

    /// Add watch paths. Paths beyond `max_paths` are silently dropped with a warning.
    /// Duplicate paths are ignored. Non-existent paths may fail to register with
    /// the underlying watcher but will not cause a panic.
    /// Returns an error if native watcher startup failed or a native operation
    /// timed out. A path the backend refuses is logged, not counted, and skipped.
    pub fn add_watch_paths(&self, paths: Vec<String>) -> notify::Result<()> {
        let mut watched = self.watched_paths.lock().unwrap_or_else(|p| p.into_inner());
        let mut watcher_guard = self.watcher.lock().unwrap_or_else(|p| p.into_inner());

        let watcher = watcher_guard
            .as_mut()
            .map_err(|e| notify::Error::generic(e))?;

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
            if !watched.contains(&path) {
                match watcher.watch(&path, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        watched.insert(path);
                    }
                    // A timed-out worker refuses every later path; report it once.
                    Err(e) if !watcher.is_available() => return Err(e),
                    Err(e) => {
                        tracing::warn!(path = %path_str, error = %e, "failed to watch path");
                    }
                }
            }
        }
        Ok(())
    }

    /// Remove all watches. Called on `SessionEnd` to clean up.
    pub fn clear(&self) {
        let mut watched = self.watched_paths.lock().unwrap_or_else(|p| p.into_inner());
        let mut watcher_guard = self.watcher.lock().unwrap_or_else(|p| p.into_inner());

        if let Ok(ref mut w) = *watcher_guard {
            for path in watched.drain() {
                if let Err(e) = w.unwatch(&path) {
                    tracing::warn!(path = %path.display(), error = %e, "failed to unwatch path");
                }
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
