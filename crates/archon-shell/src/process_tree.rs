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
//! On Linux a supervised child can call [`become_subreaper`] (in `pre_exec`),
//! so that orphans below it are reparented to it, and stay reachable, for as
//! long as it lives. macOS has no equivalent. A process that left the session
//! and lost its parent is reachable there only through what it holds open,
//! which [`holders`] finds.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[path = "process_tree_holders.rs"]
mod holders_impl;
pub use holders_impl::{Holder, holders};

/// One process-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub ppid: u32,
    pub pgid: u32,
    /// `None` when the session could not be read (the process exited
    /// between the listing and the query, or is not ours to inspect).
    pub sid: Option<u32>,
    /// Exited but not yet reaped: it runs no code and holds nothing open.
    pub zombie: bool,
}

/// Rounds of stop-and-rescan before a kill. Each round stops the members a
/// scan found, so only a member forked between a scan and its stop can be
/// new on the next one; a fork bomb outruns any bound, so this one is small.
const FREEZE_ROUNDS: usize = 8;
const KILL_POLL: Duration = Duration::from_millis(10);

/// The live process table.
pub fn snapshot() -> io::Result<Vec<Process>> {
    #[cfg(target_os = "linux")]
    {
        snapshot_proc()
    }
    #[cfg(not(target_os = "linux"))]
    {
        snapshot_ps()
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
/// closing parenthesis: state, ppid, pgrp, session.
pub fn parse_proc_stat(pid: u32, stat: &str) -> Option<Process> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    let ppid = fields.next()?.parse().ok()?;
    let pgid = fields.next()?.parse().ok()?;
    let sid = fields.next()?.parse().ok()?;
    Some(Process {
        pid,
        ppid,
        pgid,
        sid: Some(sid),
        zombie: matches!(state, "Z" | "X"),
    })
}

#[cfg(not(target_os = "linux"))]
fn snapshot_ps() -> io::Result<Vec<Process>> {
    // `ps -o sess` is not a session id on macOS, so the session is asked for
    // per process; getsid(2) answers for any process of the same user.
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,stat="])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            let zombie = fields.next().is_some_and(|state| state.starts_with('Z'));
            let pid_t = libc::pid_t::try_from(pid).ok()?;
            // SAFETY: getsid takes a plain integer and touches no memory.
            let sid = unsafe { libc::getsid(pid_t) };
            Some(Process {
                pid,
                ppid,
                pgid,
                sid: u32::try_from(sid).ok(),
                zombie,
            })
        })
        .collect())
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
    /// The live members of the scope in `processes`. This process's own
    /// group and session are never part of a scope, whatever was named, and
    /// neither are this process and its ancestors.
    pub fn members_in(&self, processes: &[Process]) -> Vec<u32> {
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
            .map(|p| p.pid)
            .collect()
    }

    /// The live members now.
    pub fn members(&self) -> io::Result<Vec<u32>> {
        Ok(self.members_in(&snapshot()?))
    }

    /// Send `signal` to every live member once; returns the members found.
    pub fn signal(&self, signal: i32) -> io::Result<Vec<u32>> {
        let members = self.members()?;
        for pid in &members {
            send(*pid, signal);
        }
        Ok(members)
    }

    /// Kill every member until none is left or `bound` runs out, and return
    /// the members still alive then (empty: the scope is confirmed empty).
    ///
    /// Members are stopped before they are killed, and the scope is scanned
    /// again after each stop: a member that forked between a scan and the
    /// kill would otherwise leave a child no later scan can tie to the scope
    /// once its parent is gone.
    pub fn kill(&self, bound: Duration) -> io::Result<Vec<u32>> {
        let deadline = Instant::now() + bound;
        loop {
            let mut members = self.members()?;
            if members.is_empty() {
                return Ok(members);
            }
            let mut stopped = BTreeSet::new();
            for _ in 0..FREEZE_ROUNDS {
                let fresh: Vec<u32> = members.into_iter().filter(|p| stopped.insert(*p)).collect();
                if fresh.is_empty() {
                    break;
                }
                for pid in fresh {
                    send(pid, libc::SIGSTOP);
                }
                members = self.members()?;
            }
            for pid in &stopped {
                send(*pid, libc::SIGKILL);
            }
            if Instant::now() >= deadline {
                return self.members();
            }
            std::thread::sleep(KILL_POLL);
        }
    }
}

fn send(pid: u32, signal: i32) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pid <= 1 {
        return;
    }
    // SAFETY: kill takes plain integers and touches no memory. A member that
    // exited since the scan answers ESRCH, which is the outcome wanted.
    unsafe {
        libc::kill(pid, signal);
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
