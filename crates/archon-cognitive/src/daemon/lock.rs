use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::CognitiveError;
use crate::daemon::state::{DaemonPaths, heartbeat_is_stale};

pub struct DaemonLock {
    path: PathBuf,
}

impl DaemonLock {
    pub fn acquire(paths: &DaemonPaths, stale_ms: u64) -> Result<Self, CognitiveError> {
        paths.ensure_root()?;
        clear_stale_lock(paths, stale_ms)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&paths.lock_path)
            .map_err(lock_error)?;
        writeln!(file, "pid={}", std::process::id())?;
        Ok(Self {
            path: paths.lock_path.clone(),
        })
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn clear_stale_lock(paths: &DaemonPaths, stale_ms: u64) -> Result<(), CognitiveError> {
    if !paths.lock_path.exists() {
        return Ok(());
    }
    let Some(state) = paths.read_state()? else {
        if lock_file_is_stale(paths, stale_ms)? {
            std::fs::remove_file(&paths.lock_path)?;
            return Ok(());
        }
        return Err(lock_error(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "daemon lock exists without readable state",
        )));
    };
    if heartbeat_is_stale(&state, stale_ms) {
        std::fs::remove_file(&paths.lock_path)?;
    }
    Ok(())
}

fn lock_file_is_stale(paths: &DaemonPaths, stale_ms: u64) -> Result<bool, CognitiveError> {
    let modified = std::fs::metadata(&paths.lock_path)?.modified()?;
    let Ok(elapsed) = modified.elapsed() else {
        return Ok(false);
    };
    Ok(elapsed.as_millis() > u128::from(stale_ms))
}

fn lock_error(error: std::io::Error) -> CognitiveError {
    CognitiveError::Store(format!(
        "cognitive daemon already running or locked: {error}"
    ))
}
