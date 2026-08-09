use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub last_tick_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tick_started_at: Option<DateTime<Utc>>,
    pub ticks_run: u64,
    pub last_error: Option<String>,
    pub status: String,
    #[serde(default)]
    pub current_job: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub stale: bool,
    pub stop_requested: bool,
    pub state: Option<DaemonState>,
    pub state_path: PathBuf,
    pub lock_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DaemonPaths {
    pub root: PathBuf,
    pub state_path: PathBuf,
    pub lock_path: PathBuf,
    pub stop_path: PathBuf,
    /// Written by the launcher on session activity; read to detect abandonment.
    pub activity_path: PathBuf,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

impl DaemonState {
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            pid: std::process::id(),
            started_at: now,
            last_heartbeat_at: now,
            last_tick_at: None,
            tick_started_at: None,
            ticks_run: 0,
            last_error: None,
            status: "running".into(),
            current_job: None,
        }
    }

    pub fn heartbeat(&mut self) {
        self.last_heartbeat_at = Utc::now();
    }

    pub fn record_job_start(&mut self, job: impl Into<String>) {
        let now = Utc::now();
        if self.tick_started_at.is_none() {
            self.tick_started_at = Some(now);
        }
        self.current_job = Some(job.into());
        self.status = "running".into();
        self.last_heartbeat_at = now;
    }

    pub fn record_tick(&mut self, error: Option<String>) {
        let now = Utc::now();
        self.ticks_run = self.ticks_run.saturating_add(1);
        self.last_tick_at = Some(now);
        self.tick_started_at = None;
        self.last_heartbeat_at = now;
        self.last_error = error;
        self.current_job = None;
        self.status = "running".into();
    }
}

impl DaemonPaths {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            state_path: root.join("cognitive-daemon-state.json"),
            lock_path: root.join("cognitive-daemon.lock"),
            stop_path: root.join("cognitive-daemon.stop"),
            // Name matches ARCHON_ACTIVITY_FILE in the launcher, which writes it.
            activity_path: root.join("archon-activity.json"),
            root,
        }
    }

    pub fn ensure_root(&self) -> Result<(), CognitiveError> {
        std::fs::create_dir_all(&self.root)?;
        Ok(())
    }

    pub fn read_state(&self) -> Result<Option<DaemonState>, CognitiveError> {
        if !self.state_path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&self.state_path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub fn write_state(&self, state: &DaemonState) -> Result<(), CognitiveError> {
        self.ensure_root()?;
        let raw = serde_json::to_string_pretty(state)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.write_all(raw.as_bytes())?;
        persist_with_retry(temp, &self.state_path)?;
        Ok(())
    }

    pub fn request_stop(&self) -> Result<(), CognitiveError> {
        self.ensure_root()?;
        std::fs::write(&self.stop_path, "stop\n")?;
        Ok(())
    }

    pub fn clear_stop(&self) -> Result<(), CognitiveError> {
        if self.stop_path.exists() {
            std::fs::remove_file(&self.stop_path)?;
        }
        Ok(())
    }
}

/// Replace `destination` with `temp`, retrying briefly on a transient denial.
///
/// A replace-rename is atomic on Unix whatever else has the destination open.
/// Windows is not: `MoveFileEx` fails with `Access denied` (5) or
/// `Sharing violation` (32) for as long as another handle is open on the
/// target, and `CognitiveDaemon::status` opens exactly that file on every poll.
/// A single heartbeat write losing that race returned an error the daemon loop
/// treats as fatal, so a routine concurrent read could stop the heartbeat.
fn persist_with_retry(
    temp: tempfile::NamedTempFile,
    destination: &Path,
) -> Result<(), CognitiveError> {
    const ATTEMPTS: usize = 50;

    let mut temp = temp;
    for attempt in 0..ATTEMPTS {
        match temp.persist(destination) {
            Ok(_) => return Ok(()),
            Err(error) if attempt + 1 < ATTEMPTS && is_transient_replace_error(&error.error) => {
                temp = error.file;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    unreachable!("the final attempt returns rather than looping")
}

fn is_transient_replace_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
    ) || error.raw_os_error().is_some_and(|code| code == 32)
}

pub fn status_for(paths: &DaemonPaths, stale_ms: u64) -> Result<DaemonStatus, CognitiveError> {
    let state = paths.read_state()?;
    let stale = state
        .as_ref()
        .is_some_and(|state| heartbeat_is_stale(state, stale_ms) || !is_pid_alive(state.pid));
    Ok(DaemonStatus {
        running: paths.lock_path.exists() && !stale,
        stale,
        stop_requested: paths.stop_path.exists(),
        state,
        state_path: paths.state_path.clone(),
        lock_path: paths.lock_path.clone(),
    })
}

pub fn heartbeat_is_stale(state: &DaemonState, stale_ms: u64) -> bool {
    let elapsed = Utc::now()
        .signed_duration_since(state.last_heartbeat_at)
        .num_milliseconds();
    elapsed > stale_ms as i64
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: kill(pid, 0) sends no signal; it only checks process existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(target_os = "windows")]
pub(crate) fn is_pid_alive(_pid: u32) -> bool {
    false
}
