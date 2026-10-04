//! Finding and killing every process a supervised child started (Unix).
//!
//! A process group is not a container. A member can move to a group of its
//! own (a runner that gives each check its own group) or leave the session
//! as well (`setsid`). A group kill never reaches such a process, and a group
//! probe then reports the group empty while the process still runs. A
//! [`Scope`] names a supervised tree by every handle that still reaches it:
//! its process groups, its session, and ancestry - a live process whose
//! parent is in scope is in scope.
//!
//! Ancestry ends where a parent exits: its children are reparented at once.
//! A [`Tracker`] therefore scans while the child runs and remembers every
//! member it ever saw, so a descendant whose parent exited later is still
//! torn down. Every remembered or scanned process is pinned by its start
//! time, and its identity is checked again immediately before any signal
//! (`identity`): a pid that was reused by another process is never signalled.
//! A process that left the session before any scan saw it is reachable only
//! through what it holds open, which [`holders`] finds.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

#[path = "process_tree_bounded.rs"]
mod bounded;
#[path = "process_tree_holders.rs"]
mod holders_impl;
#[path = "process_tree_identity.rs"]
mod identity;
#[path = "process_tree_tracker.rs"]
mod tracker;
pub use holders_impl::{HOLDER_PROBE_DEADLINE, Holder, holders, holders_within};
pub use identity::{Pinned, deliver, start_of};
pub use tracker::Tracker;

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

/// The live process table: syscalls and `/proc` only, no subprocess, so a
/// scan cannot hang on another program.
pub fn snapshot() -> io::Result<Vec<Process>> {
    #[cfg(target_os = "linux")]
    {
        snapshot_proc()
    }
    #[cfg(target_os = "macos")]
    {
        identity::snapshot_libproc()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process-tree scans are implemented for Linux and macOS only",
        ))
    }
}

#[cfg(target_os = "linux")]
fn snapshot_proc() -> io::Result<Vec<Process>> {
    let mut processes = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
            continue;
        };
        // A process that exits between the listing and the read is gone.
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        if let Some(process) = parse_proc_stat(pid, &stat) {
            processes.push(process);
        }
    }
    Ok(processes)
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

/// A supervised tree, named by every handle that can still reach it.
///
/// `roots` must be processes the caller has not reaped (its own children,
/// alive or zombie): a reaped pid can be reused by an unrelated process.
/// Groups and sessions need no such care, because a process-group or
/// session id is not reused while any process still carries it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub roots: Vec<u32>,
    pub groups: Vec<u32>,
    pub sessions: Vec<u32>,
}

impl Scope {
    /// The live members of the scope in `processes`, plus every live process
    /// in `seen` whose identity still matches, and their descendants. This
    /// process's own group and session are never part of a scope, whatever
    /// was named, and neither are this process and its ancestors.
    pub fn reach(&self, processes: &[Process], seen: &BTreeMap<u32, u64>) -> Vec<Process> {
        // SAFETY: both calls take plain integers and touch no memory.
        let (own_group, own_session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
        let own_group = u32::try_from(own_group).ok();
        let own_session = u32::try_from(own_session).ok();
        let groups: BTreeSet<u32> = (self.groups.iter().copied())
            .filter(|g| *g > 1 && Some(*g) != own_group)
            .collect();
        let sessions: BTreeSet<u32> = (self.sessions.iter().copied())
            .filter(|s| *s > 1 && Some(*s) != own_session)
            .collect();
        let mut children: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for process in processes {
            children.entry(process.ppid).or_default().push(process.pid);
        }
        let mut reached: BTreeSet<u32> = processes
            .iter()
            .filter(|p| {
                self.roots.contains(&p.pid)
                    || groups.contains(&p.pgid)
                    || p.sid.is_some_and(|sid| sessions.contains(&sid))
                    || seen.get(&p.pid) == Some(&p.start)
            })
            .map(|p| p.pid)
            .collect();
        let mut frontier: Vec<u32> = reached.iter().copied().collect();
        while let Some(pid) = frontier.pop() {
            for child in children.get(&pid).into_iter().flatten() {
                if reached.insert(*child) {
                    frontier.push(*child);
                }
            }
        }
        let protected = protected(processes);
        processes
            .iter()
            .filter(|p| reached.contains(&p.pid) && !p.zombie && !protected.contains(&p.pid))
            .copied()
            .collect()
    }

    /// The pids of the live members of the scope in `processes`.
    pub fn members_in(&self, processes: &[Process]) -> Vec<u32> {
        let members = self.reach(processes, &BTreeMap::new());
        members.iter().map(|p| p.pid).collect()
    }

    /// The live members now.
    pub fn members(&self) -> io::Result<Vec<u32>> {
        Ok(self.members_in(&snapshot()?))
    }

    /// Send `signal` to every live member once; returns the members found.
    pub fn signal(&self, signal: i32) -> io::Result<Vec<u32>> {
        Tracker::new(self.clone()).signal(signal)
    }

    /// Kill every member until none is left or `bound` runs out; returns
    /// the members still alive then (empty: the scope is confirmed empty).
    pub fn kill(&self, bound: std::time::Duration) -> io::Result<Vec<u32>> {
        let survivors = Tracker::new(self.clone()).kill(bound)?;
        Ok(survivors.iter().map(|p| p.pid).collect())
    }
}

/// Make the calling process the reaper of its orphaned descendants (Linux
/// `PR_SET_CHILD_SUBREAPER`), so that they stay its descendants, and in
/// every [`Scope`] rooted at it, after their own parent exits. Async-signal
/// safe: meant for `pre_exec`, where it survives the exec. A no-op where the
/// kernel has no such attribute.
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
