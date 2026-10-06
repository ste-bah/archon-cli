//! Finding and killing every process a supervised child started (Unix).
//!
//! A process group is not a container. A member can move to a group of its
//! own (a runner that gives each check its own group) or leave the session
//! as well (`setsid`), and its parent can exit, leaving no tie to the child
//! at all. A [`Tracker`] therefore scans while the child runs and pins every
//! member it adopts, by pid and start time; teardown kills every pinned
//! member still alive. The adoption rules, which never take a stranger that
//! reused a pid for ours, are in `tracker`; the identity and signalling
//! guarantees, and their documented residuals, are in `identity`.
//!
//! A process that left the session before any scan saw it is reachable only
//! through what it holds open, which [`holders`] finds.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "process_tree_bounded.rs"]
mod bounded;
pub use bounded::drain_probes;
#[path = "process_tree_holders.rs"]
mod holders_impl;
#[path = "process_tree_identity.rs"]
mod identity;
#[path = "process_tree_tracker.rs"]
mod tracker;
pub use holders_impl::{HOLDER_PROBE_DEADLINE, Holder, holders, holders_within, lsof_program};
pub use identity::{Pinned, Table, deliver, exited, identity_of, snapshot_until, start_of};
pub use tracker::{ReapToken, Tracker};
#[path = "process_tree_cleanup.rs"]
mod cleanup;
pub use cleanup::{drain_cleanup, lock_until, register_cleanup};
#[path = "process_tree_descriptors.rs"]
pub(crate) mod descriptors;
pub use descriptors::{descriptor_ceiling, inherit_only_stdio};

/// How long [`snapshot`] may take.
pub const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(5);

/// One process-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    /// `None` when the session could not be read (the process exited
    /// between the listing and the query, or is not ours to inspect).
    pub sid: Option<u32>,
    /// When the process started, in the kernel's own unit: with the pid, the
    /// process's identity, which a reused pid does not share.
    pub start: u64,
    /// Exited but not yet reaped: it runs no code and holds nothing open.
    pub zombie: bool,
}

impl Process {
    pub fn pinned(&self) -> Pinned {
        Pinned {
            pid: self.pid,
            start: self.start,
        }
    }
}

/// [`snapshot_until`] with [`SNAPSHOT_DEADLINE`].
pub fn snapshot() -> io::Result<Table> {
    snapshot_until(Instant::now() + SNAPSHOT_DEADLINE)
}

/// One `/proc/<pid>/stat` line. The command name is parenthesised and may
/// itself contain spaces and parentheses, so fields are read after the LAST
/// closing parenthesis: state (field 3), ppid, pgrp, session, and starttime
/// (field 22, clock ticks since boot).
pub fn parse_proc_stat(pid: u32, stat: &str) -> Option<Process> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let state = *fields.first()?;
    Some(Process {
        pid,
        ppid: fields.get(1)?.parse().ok()?,
        pgid: fields.get(2)?.parse().ok()?,
        sid: Some(fields.get(3)?.parse().ok()?),
        start: fields.get(19)?.parse().ok()?,
        zombie: matches!(state, "Z" | "X"),
    })
}

/// Every pid that must never be signalled from here: this process, its
/// ancestors, and the kernel's own (0 and 1).
fn protected(processes: &[Process]) -> BTreeSet<u32> {
    let parents: BTreeMap<u32, u32> = processes.iter().map(|p| (p.pid, p.ppid)).collect();
    let mut protected = BTreeSet::from([0, 1]);
    let mut current = std::process::id();
    while protected.insert(current) {
        match parents.get(&current) {
            Some(parent) => current = *parent,
            None => break,
        }
    }
    protected
}

/// Process-group and session selectors, for a one-off probe (is anything of
/// this group or session still running?). Never this process's own group or
/// session, nor this process or its ancestors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub groups: Vec<u32>,
    pub sessions: Vec<u32>,
}

impl Scope {
    /// The live members of the scope in `processes`.
    pub fn members_in(&self, processes: &[Process]) -> Vec<u32> {
        // SAFETY: both calls take plain integers and touch no memory.
        let (own_group, own_session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
        let own_group = u32::try_from(own_group).ok();
        let own_session = u32::try_from(own_session).ok();
        let protected = protected(processes);
        processes
            .iter()
            .filter(|p| !p.zombie && !protected.contains(&p.pid))
            .filter(|p| {
                (self.groups.contains(&p.pgid) && Some(p.pgid) != own_group && p.pgid > 1)
                    || p.sid.is_some_and(|sid| {
                        self.sessions.contains(&sid) && Some(sid) != own_session && sid > 1
                    })
            })
            .map(|p| p.pid)
            .collect()
    }

    /// The live members now.
    pub fn members(&self) -> io::Result<Vec<u32>> {
        Ok(self.members_in(&snapshot()?.processes))
    }
}

/// Make the calling process the reaper of its orphaned descendants (Linux
/// `PR_SET_CHILD_SUBREAPER`), so that they stay its descendants after their
/// own parent exits. Async-signal safe: meant for `pre_exec`, where it
/// survives the exec. A no-op where the kernel has no such attribute.
pub fn become_subreaper() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl with integer arguments touches no memory of ours.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Canonical spelling of each existing path, for prefix comparison with the
/// resolved paths the kernel reports.
fn canonical_roots(paths: &[&Path]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter_map(|path| path.canonicalize().ok())
        .collect()
}

#[cfg(test)]
#[path = "process_tree_tests.rs"]
mod tests;
