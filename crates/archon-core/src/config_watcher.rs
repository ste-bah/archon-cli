//! File-system watcher for config hot-reload.
//!
//! Watches config files for changes and supports debounced reloading so that
//! rapid successive edits (e.g. from a text editor doing write-rename) are
//! coalesced into a single reload.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{ArchonConfig, ConfigError};
use crate::config_diff::diff_configs;

// ---------------------------------------------------------------------------
// ConfigWatcher
// ---------------------------------------------------------------------------

/// Watches one or more config files for filesystem changes.
///
/// Internally uses [`notify::RecommendedWatcher`] which picks the best
/// backend for the current OS (inotify on Linux, FSEvents on macOS, etc.).
pub struct ConfigWatcher {
    /// Kept alive to maintain the watch; dropped when `ConfigWatcher` is dropped.
    _watcher: RecommendedWatcher,
    /// Receives filesystem events from the watcher.
    rx: mpsc::Receiver<notify::Result<Event>>,
}

impl ConfigWatcher {
    /// Start watching the given config file paths.
    ///
    /// Paths that do not exist on disk are silently skipped (the user may not
    /// have created every layer file).
    pub fn start(paths: &[PathBuf]) -> Result<Self, ConfigError> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(
            tx,
            notify::Config::default().with_poll_interval(Duration::from_secs(2)),
        )
        .map_err(|e| ConfigError::ValidationError(format!("failed to create file watcher: {e}")))?;

        for path in paths {
            if path.exists() {
                watcher
                    .watch(path, RecursiveMode::NonRecursive)
                    .map_err(|e| {
                        ConfigError::ValidationError(format!(
                            "failed to watch {}: {e}",
                            path.display()
                        ))
                    })?;
            }
        }

        Ok(Self {
            _watcher: watcher,
            rx,
        })
    }

    /// Non-blocking poll for pending filesystem change events.
    ///
    /// Returns the de-duplicated list of file paths that have been modified
    /// since the last call to `poll_changes`.
    pub fn poll_changes(&self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        while let Ok(Ok(event)) = self.rx.try_recv() {
            for path in event.paths {
                if !changed.contains(&path) {
                    changed.push(path);
                }
            }
        }
        changed
    }
}

// ---------------------------------------------------------------------------
// DebouncedReloader
// ---------------------------------------------------------------------------

/// Wraps a [`ConfigWatcher`] with debounce logic so that rapid successive
/// edits are coalesced into a single reload.
pub struct DebouncedReloader {
    watcher: ConfigWatcher,
    last_change: Option<Instant>,
    debounce_ms: u64,
    pending: bool,
    current_config: ArchonConfig,
}

impl DebouncedReloader {
    /// Create a new debounced reloader with the given debounce interval in
    /// milliseconds and the current config to diff against.
    pub fn new(watcher: ConfigWatcher, debounce_ms: u64, current_config: ArchonConfig) -> Self {
        Self {
            watcher,
            last_change: None,
            debounce_ms,
            pending: false,
            current_config,
        }
    }

    /// Check for config file changes and, if the debounce period has elapsed,
    /// reload and diff the config.
    ///
    /// Returns `Some(changed_keys)` when a reload occurred, or `None` if
    /// nothing has changed or the debounce period has not yet elapsed.
    ///
    /// On successful reload, the internal current config is updated to the
    /// new config so subsequent diffs are against the latest known state.
    pub fn check_and_reload(&mut self, config_paths: &[PathBuf]) -> Option<Vec<String>> {
        let changes = self.watcher.poll_changes();
        if !changes.is_empty() {
            self.last_change = Some(Instant::now());
            self.pending = true;
        }

        if !self.pending {
            return None;
        }

        if let Some(last) = self.last_change
            && last.elapsed() >= Duration::from_millis(self.debounce_ms)
        {
            self.pending = false;
            self.last_change = None;
            // Attempt to load a fresh config from the first path that exists
            if let Some(path) = config_paths.iter().find(|p| p.exists()) {
                match crate::config::load_config_from(path.clone()) {
                    Ok(new_cfg) => {
                        let keys = diff_configs(&self.current_config, &new_cfg);
                        self.current_config = new_cfg;
                        return Some(keys);
                    }
                    Err(e) => {
                        tracing::warn!("debounced reload failed: {e}");
                    }
                }
            }
        }

        None
    }

    /// Get a reference to the current config held by the reloader.
    pub fn current_config(&self) -> &ArchonConfig {
        &self.current_config
    }
}

// ---------------------------------------------------------------------------
// force_reload
// ---------------------------------------------------------------------------

/// Force an immediate reload of the configuration from the given file paths.
///
/// Reads the first existing path, parses it, diffs against the provided
/// `current` config, and returns the new config along with the list of
/// changed key paths.
pub fn force_reload(
    config_paths: &[PathBuf],
    current: &ArchonConfig,
) -> Result<(ArchonConfig, Vec<String>), ConfigError> {
    let path = config_paths
        .iter()
        .find(|p| p.exists())
        .ok_or_else(|| ConfigError::ValidationError("no config file found for reload".into()))?;

    let new_config = crate::config::load_config_from(path.clone())?;
    let changed = diff_configs(current, &new_config);
    Ok((new_config, changed))
}
