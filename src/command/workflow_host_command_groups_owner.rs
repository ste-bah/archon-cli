//! Who holds a host-command group record, and what a record's ids still
//! name (Issue 270 round 5).
//!
//! Commands of one run overlap (a script may await several at once), so a
//! record on disk is one of: a sibling this process still supervises, a
//! teardown that settled without clearing its record, or one left by an
//! owner that exited (a crash). Only the first is ordinary running work;
//! the last is judged by what it names, so it ends by itself once that has
//! ended. Every record is named by its leader's identity, so a reused group
//! id never collides with an old record.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::HostCommandGroupRecord;

/// The markers of the commands this process supervises right now: held from
/// before the record is written until its guard has settled it.
static LIVE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

fn live() -> std::sync::MutexGuard<'static, BTreeSet<PathBuf>> {
    LIVE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Picks the record path for a command whose leader is `pgid` started at
/// `leader_start`, and holds its marker as live. The name is free: no record,
/// marker or unknown record of that name exists, and no live guard holds it.
pub(super) fn claim(dir: &Path, pgid: u32, leader_start: Option<u64>) -> (PathBuf, PathBuf) {
    let mut live = live();
    let base = format!("{pgid}-{}", leader_start.unwrap_or_default());
    // Each taken name is an existing file or a live guard, so this ends.
    let mut n = 0u64;
    loop {
        let stem = if n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let path = dir.join(format!("{stem}.json"));
        let pending = path.with_extension("pending");
        let taken = live.contains(&pending)
            || [&path, &pending, &super::unknown_path(&path)]
                .iter()
                .any(|candidate| candidate.symlink_metadata().is_ok());
        if !taken {
            live.insert(pending.clone());
            return (path, pending);
        }
        n += 1;
    }
}

/// The guard of `pending` settled it (or never wrote it).
pub(super) fn release(pending: &Path) {
    live().remove(pending);
}

/// This process's start time, the second half of its identity.
pub(super) fn own_start() -> Option<u64> {
    #[cfg(unix)]
    {
        static START: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *START.get_or_init(|| archon_shell::process_tree::start_of(std::process::id()))
    }
    #[cfg(not(unix))]
    None
}

/// The start time of the leader `pid` (this process's unreaped child).
pub(super) fn leader_start(pid: u32) -> Option<u64> {
    #[cfg(unix)]
    {
        archon_shell::process_tree::start_of(pid)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Owner {
    /// A guard in this process holds it: a running command, not a stall.
    Supervising,
    /// The writer is proven gone (exited, or its pid now names another
    /// process): no teardown will ever settle the record.
    Exited,
    /// This process, settled without clearing it; another live process; or
    /// an owner whose identity cannot be read.
    Unknown,
}

/// Who holds `record`, whose marker is `pending`.
pub(super) fn owner(record: &HostCommandGroupRecord, pending: &Path) -> Owner {
    let ours = record.host_pid == std::process::id()
        && (record.host_start.is_none() || record.host_start == own_start());
    if ours {
        return if live().contains(pending) {
            Owner::Supervising
        } else {
            Owner::Unknown
        };
    }
    #[cfg(unix)]
    match archon_shell::process_tree::identity_of(record.host_pid) {
        Ok(None) => Owner::Exited,
        Ok(Some(start)) if record.host_start.is_some_and(|written| written != start) => {
            Owner::Exited
        }
        Ok(Some(_)) | Err(_) => Owner::Unknown,
    }
    #[cfg(not(unix))]
    Owner::Unknown
}

/// Whether the recorded leader's pid now names another process. A pid is
/// never reused while a process group or session of that id exists, so a
/// stranger holding it proves the recorded group and session ended. `None`
/// when that cannot be read.
#[cfg(unix)]
pub(super) fn leader_replaced(record: &HostCommandGroupRecord) -> Option<bool> {
    let Some(start) = record.leader_start else {
        return Some(false);
    };
    match archon_shell::process_tree::identity_of(record.pgid) {
        Ok(Some(now)) => Some(now != start),
        // Gone or a zombie: the group may still run without its leader.
        Ok(None) => Some(false),
        Err(_) => None,
    }
}
