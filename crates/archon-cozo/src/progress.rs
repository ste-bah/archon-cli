//! Acquisition is limited by inactivity, never total writer lifetime.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

fn progress_path(lock: &Path) -> PathBuf {
    let mut name = lock.as_os_str().to_owned();
    name.push(".progress");
    PathBuf::from(name)
}

/// A completed guarded statement is progress even when its SQL changes no bytes.
pub(crate) fn record(config: &crate::CozoGuardConfig) {
    if let Some(lock) = &config.write_lock_path {
        let stamp = format!("{} {:?}", std::process::id(), SystemTime::now());
        if let Err(error) = std::fs::write(progress_path(lock), stamp) {
            tracing::warn!(path = %lock.display(), %error, "failed to record Cozo writer progress");
        }
    }
}

pub(crate) struct Window {
    paths: Vec<PathBuf>,
    snapshot: Vec<Option<(SystemTime, u64)>>,
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
            last_progress: Instant::now(),
            wait,
        }
    }
    pub(crate) fn remaining(&mut self) -> Option<Duration> {
        let current = snapshot(&self.paths);
        if current != self.snapshot {
            self.snapshot = current;
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
