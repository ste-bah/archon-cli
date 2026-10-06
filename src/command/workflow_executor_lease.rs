//! The fixed-run executor lease: a kernel advisory lock on
//! `decomposition/executor.lock`, plus a record of the process that holds it.
//!
//! The lock is the liveness test. The kernel releases it when its holder ends
//! for any reason (SIGKILL, OOM, a closed terminal group), so an acquire that
//! succeeds proves that no live process executes the run. The pid record is
//! evidence only, for the refusal message and the recovery event (Issue 251).
//! It never decides liveness, so a pid that the OS gave to a different process
//! cannot make a dead owner look live, or a live owner look dead.
//!
//! Not every holder executes: a resume that fails its checks, or a reclaim,
//! holds the lease briefly too. So the record carries a role, and the last
//! process that actually executed the run is carried forward through such
//! holders: the recovery event names the executor, not the last checker.

use anyhow::{Result, anyhow};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(crate) const LEASE: &str = "decomposition/executor.lock";

/// A holder that took the lease to check or reclaim the run.
pub(crate) const ROLE_PREFLIGHT: &str = "preflight";
/// A holder that started executing the run.
pub(crate) const ROLE_EXECUTOR: &str = "executor";

/// One process that held the lease.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LeaseHolder {
    pub(crate) pid: u32,
    pub(crate) acquired_at: String,
}

/// What a holder writes into the lock file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LeaseRecord {
    pub(crate) schema_version: u32,
    pub(crate) role: String,
    pub(crate) holder: LeaseHolder,
    /// The last executor before this holder, kept while the holder is only
    /// a preflight.
    #[serde(default)]
    pub(crate) last_executor: Option<LeaseHolder>,
}

impl LeaseRecord {
    fn executor(&self) -> Option<LeaseHolder> {
        if self.role == ROLE_EXECUTOR {
            Some(self.holder.clone())
        } else {
            self.last_executor.clone()
        }
    }
}

/// The held lease. Dropping it unlocks the file, then closes it.
#[derive(Debug)]
pub(crate) struct ExecutionLease {
    file: File,
    holder: LeaseHolder,
    previous_executor: Option<LeaseHolder>,
}

impl Drop for ExecutionLease {
    fn drop(&mut self) {
        release_lock(&self.file);
    }
}

/// Releases a held lock file at once (Issue 330). The lock belongs to the
/// open file, and a child that any thread of this process forks shares that
/// file until its `exec` closes the CLOEXEC descriptor. A close alone
/// therefore leaves the lock held for as long as such a child waits to
/// `exec`, which under load is long enough for a free run to be refused as
/// live. An explicit unlock releases it for every copy of the open file.
/// Only a holder that is done calls this, so it never frees a lock another
/// live holder took; the kernel still releases a dead holder's lock.
pub(crate) fn release_lock(file: &File) {
    if let Err(error) = file.unlock() {
        tracing::warn!(%error, "lock file could not be unlocked; it is released when closed");
    }
}

impl ExecutionLease {
    /// The last process that executed the run before this lease was taken.
    /// `None` for a new run, or for a lock file that an older build left
    /// empty.
    pub(crate) fn previous_executor(&self) -> Option<&LeaseHolder> {
        self.previous_executor.as_ref()
    }

    /// Records this holder as the run's executor. Called when execution
    /// actually starts, after every check has passed.
    pub(crate) fn record_executor(&self) -> Result<()> {
        self.write(&LeaseRecord {
            schema_version: 1,
            role: ROLE_EXECUTOR.to_string(),
            holder: self.holder.clone(),
            last_executor: None,
        })
    }

    fn write(&self, record: &LeaseRecord) -> Result<()> {
        let mut file = &self.file;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&serde_json::to_vec(record)?)?;
        file.flush()?;
        Ok(())
    }
}

/// Takes the lease of run `id` in `run_dir` as a preflight holder, or refuses
/// with the pid of the live process that holds it.
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
    let previous_executor = read_record(&mut file).and_then(|record| record.executor());
    let lease = ExecutionLease {
        file,
        holder: LeaseHolder {
            pid: std::process::id(),
            acquired_at: chrono::Utc::now().to_rfc3339(),
        },
        previous_executor,
    };
    lease.write(&LeaseRecord {
        schema_version: 1,
        role: ROLE_PREFLIGHT.to_string(),
        holder: lease.holder.clone(),
        last_executor: lease.previous_executor.clone(),
    })?;
    Ok(lease)
}

pub(crate) fn read_record(file: &mut File) -> Option<LeaseRecord> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

fn live_owner_message(id: &str, path: &Path) -> String {
    // Best effort: the holder writes its record just after it takes the lock,
    // and Windows refuses reads of a locked range, so the record can be absent.
    let record = File::open(path)
        .ok()
        .and_then(|mut file| read_record(&mut file));
    let who = match record {
        Some(record) => format!(
            "process {} holds {LEASE} as {} (acquired {})",
            record.holder.pid, record.role, record.holder.acquired_at
        ),
        None => format!("another process holds {LEASE} (its pid record is not readable)"),
    };
    format!(
        "executor for {id} is live: {who}; a second launch or resume is refused while that process runs"
    )
}

#[cfg(test)]
#[path = "workflow_executor_lease_tests.rs"]
mod tests;
