//! The fixed-run executor lease: a kernel advisory lock on
//! `decomposition/executor.lock`, plus a record of the process that holds it.
//!
//! The lock is the liveness test. The kernel releases it when its holder ends
//! for any reason (SIGKILL, OOM, a closed terminal group), so an acquire that
//! succeeds proves that no live process executes the run. The pid record is
//! evidence only, for the refusal message and the recovery event (Issue 251).
//! It never decides liveness, so a pid that the OS gave to a different process
//! cannot make a dead owner look live, or a live owner look dead.

use anyhow::{Result, anyhow};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(crate) const LEASE: &str = "decomposition/executor.lock";

/// What the lease holder writes into the lock file once it holds the lock.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LeaseHolder {
    pub(crate) schema_version: u32,
    pub(crate) pid: u32,
    pub(crate) acquired_at: String,
}

/// The held lease. Dropping it closes the file, which releases the lock.
#[derive(Debug)]
pub(crate) struct ExecutionLease {
    _file: File,
    previous_holder: Option<LeaseHolder>,
}

impl ExecutionLease {
    /// The record the previous holder left, read before this holder replaced
    /// it. `None` for a new lease, or for a lock file that an older build left
    /// empty.
    pub(crate) fn previous_holder(&self) -> Option<&LeaseHolder> {
        self.previous_holder.as_ref()
    }
}

/// Takes the lease of run `id` in `run_dir`, or refuses with the pid of the
/// live process that holds it.
pub(crate) fn acquire(run_dir: &Path, id: &str) -> Result<ExecutionLease> {
    let path = run_dir.join(LEASE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(anyhow!(live_owner_message(id, &path))),
        Err(TryLockError::Error(error)) => {
            return Err(anyhow!(
                "executor lease for {id} cannot be acquired ({}): {error}",
                path.display()
            ));
        }
    }
    let previous_holder = read_holder(&mut file);
    let record = LeaseHolder {
        schema_version: 1,
        pid: std::process::id(),
        acquired_at: chrono::Utc::now().to_rfc3339(),
    };
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&serde_json::to_vec(&record)?)?;
    file.flush()?;
    Ok(ExecutionLease {
        _file: file,
        previous_holder,
    })
}

fn read_holder(file: &mut File) -> Option<LeaseHolder> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

fn live_owner_message(id: &str, path: &Path) -> String {
    // Best effort: the holder writes its record just after it takes the lock,
    // and Windows refuses reads of a locked range, so the record can be absent.
    let holder = File::open(path)
        .ok()
        .and_then(|mut file| read_holder(&mut file));
    let who = match holder {
        Some(holder) => format!(
            "process {} holds {LEASE} (acquired {})",
            holder.pid, holder.acquired_at
        ),
        None => format!("another process holds {LEASE} (its pid record is not readable)"),
    };
    format!(
        "executor for {id} is live: {who}; a second launch or resume is refused while that process runs"
    )
}

/// Whether a process with `pid` exists now. Evidence only: the OS can give a
/// dead owner's pid to an unrelated process, so `true` never means "owner".
pub(crate) fn pid_running(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        let pid = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0)?;
        // SAFETY: signal 0 only probes whether the process exists; EPERM
        // means it exists and belongs to another user.
        let probed = unsafe { libc::kill(pid, 0) };
        Some(probed == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}
