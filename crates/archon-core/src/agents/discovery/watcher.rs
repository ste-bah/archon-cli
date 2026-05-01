// TASK-AGS-303: FsWatcher — file system watcher with 100ms debounce.
//
// Uses notify crate to watch for file changes in the agent directory.
// Debounces bursts within 100ms into a single rescan.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, warn};

use super::local::LocalDiscoverySource;
use crate::agents::catalog::DiscoveryCatalog;

const DEBOUNCE_MS: u64 = 100;

/// File system watcher that triggers catalog reloads on file changes.
pub struct FsWatcher {
    _watcher: RecommendedWatcher,
    _worker: std::thread::JoinHandle<()>,
}

impl FsWatcher {
    /// Start watching `root` for changes. On any file event (after debounce),
    /// calls `source.load_all(catalog)` to refresh the catalog.
    pub fn start(
        root: &Path,
        source: Arc<LocalDiscoverySource>,
        catalog: Arc<DiscoveryCatalog>,
    ) -> Result<Self, notify::Error> {
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(e) => {
                    warn!("watcher error: {e}");
                }
            })?;

        watcher.watch(root, RecursiveMode::Recursive)?;

        let worker = std::thread::spawn(move || {
            while let Ok(_event) = rx.recv() {
                // Debounce: collect events for DEBOUNCE_MS, then rescan
                std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
                // Drain any accumulated events
                while rx.try_recv().is_ok() {}

                debug!("watcher debounce elapsed, rescanning");
                match source.load_all(&catalog) {
                    Ok(report) => {
                        debug!(
                            loaded = report.loaded,
                            invalid = report.invalid,
                            "watcher rescan complete"
                        );
                    }
                    Err(e) => {
                        warn!("watcher rescan failed: {e}");
                    }
                }
            }
        });

        Ok(Self {
            _watcher: watcher,
            _worker: worker,
        })
    }
}
