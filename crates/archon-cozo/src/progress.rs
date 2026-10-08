//! Writer progress evidence for no-progress windows.
//!
//! A window never measures total time. It measures time since the last
//! observed progress: a guarded writer's `.progress` marker, or a change to
//! the database, its `-wal` or its `-journal` file. A store with no lock path
//! has no files to observe, so only this process's guarded writes count.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Successful guarded writes to stores that have no lock path.
static UNPATHED_PROGRESS: AtomicU64 = AtomicU64::new(0);

fn progress_path(lock: &Path) -> PathBuf {
    let mut name = lock.as_os_str().to_owned();
    name.push(".progress");
    PathBuf::from(name)
}

/// A completed guarded statement is progress even when its SQL changes no bytes.
pub(crate) fn record(config: &crate::CozoGuardConfig) {
    let Some(lock) = &config.write_lock_path else {
        UNPATHED_PROGRESS.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let stamp = format!("{} {:?}", std::process::id(), SystemTime::now());
    if let Err(error) = std::fs::write(progress_path(lock), stamp) {
        tracing::warn!(path = %lock.display(), %error, "failed to record Cozo writer progress");
    }
}

pub(crate) struct Window {
    paths: Vec<PathBuf>,
    snapshot: Vec<Option<(SystemTime, u64)>>,
    generation: Option<u64>,
    last_progress: Instant,
    wait: Duration,
}

impl Window {
    pub(crate) fn new(lock: &Path, wait: Duration) -> Self {
        let mut paths = vec![progress_path(lock)];
        if let Some(name) = lock
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".archon-cozo-write.lock"))
        {
            let database = lock.with_file_name(name);
            paths.push(database.clone());
            for suffix in ["-wal", "-journal"] {
                let mut path = database.as_os_str().to_owned();
                path.push(suffix);
                paths.push(PathBuf::from(path));
            }
        }
        let snapshot = snapshot(&paths);
        Self {
            paths,
            snapshot,
            generation: None,
            last_progress: Instant::now(),
            wait,
        }
    }

    /// The window for a guard config: its lock's files, or, with no lock
    /// path, this process's guarded writes to unpathed stores.
    pub(crate) fn for_config(config: &crate::CozoGuardConfig, wait: Duration) -> Self {
        match &config.write_lock_path {
            Some(lock) => Self::new(lock, wait),
            None => Self {
                paths: Vec::new(),
                snapshot: Vec::new(),
                generation: Some(UNPATHED_PROGRESS.load(Ordering::Relaxed)),
                last_progress: Instant::now(),
                wait,
            },
        }
    }

    /// Time left before the wait must pause; `None` once a full window has
    /// passed with no observed progress.
    pub(crate) fn remaining(&mut self) -> Option<Duration> {
        let current = snapshot(&self.paths);
        let generation = self
            .generation
            .map(|_| UNPATHED_PROGRESS.load(Ordering::Relaxed));
        if current != self.snapshot || generation != self.generation {
            self.snapshot = current;
            self.generation = generation;
            self.last_progress = Instant::now();
        }
        self.wait
            .checked_sub(self.last_progress.elapsed())
            .filter(|d| !d.is_zero())
    }
}

fn snapshot(paths: &[PathBuf]) -> Vec<Option<(SystemTime, u64)>> {
    paths
        .iter()
        .map(|path| {
            std::fs::metadata(path)
                .ok()
                .and_then(|m| Some((m.modified().ok()?, m.len())))
        })
        .collect()
}
