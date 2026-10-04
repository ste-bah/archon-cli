//! The members of one supervised tree, pinned and remembered across scans.
//!
//! Adoption is conservative, because adopting a stranger means killing it:
//!
//! - The leader is pinned when the tracker is made. Its process group and
//!   session ids are used as selectors only while the leader is unreaped:
//!   an unreaped leader (alive or a zombie) holds its pid, so no new group or
//!   session can take that id. Once the leader is reaped the selectors go.
//! - Any other process is adopted only as the child of a member pinned by an
//!   earlier scan (or the leader), only if it did not start before that
//!   parent, and only if the parent is still the same process after the
//!   child was read, so a child of a stranger that reused the parent's pid
//!   in the middle of a scan is never taken for ours.
//! - A scan that fails, runs out of time, or cannot list the whole table
//!   changes nothing: a member is forgotten only when a complete listing no
//!   longer has its pid, or has it with another start time.
//!
//! Each member keeps the identity it was adopted with (on Linux, its pidfd)
//! and every signal goes to that identity only.

use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, Instant};

use super::Process;
use super::identity::{Pinned, Table, snapshot_until, start_of};

/// Rounds of stop-and-rescan before a kill. Each round adopts one more
/// generation and stops it, so only a member forked between a scan and its
/// stop can be new on the next one.
const FREEZE_ROUNDS: usize = 8;
const KILL_POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
struct Member {
    start: u64,
    #[cfg(target_os = "linux")]
    pidfd: Option<std::os::fd::OwnedFd>,
}

impl Member {
    fn adopt(pinned: Pinned) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let pidfd = super::identity::pidfd::open(pinned);
            // A kernel without pidfds still adopts, with verify-then-kill.
            if pidfd.is_none() && start_of(pinned.pid) != Some(pinned.start) {
                return None;
            }
            Some(Self {
                start: pinned.start,
                pidfd,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            (start_of(pinned.pid) == Some(pinned.start)).then_some(Self {
                start: pinned.start,
            })
        }
    }

    fn signal(&self, pid: u32, signal: i32) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(fd) = &self.pidfd {
            return super::identity::pidfd::send(fd, signal);
        }
        super::identity::deliver(
            Pinned {
                pid,
                start: self.start,
            },
            signal,
        )
    }
}

#[derive(Debug, Default)]
pub struct Tracker {
    /// The leader, while it is unreaped.
    root: Option<Pinned>,
    groups: Vec<u32>,
    sessions: Vec<u32>,
    members: BTreeMap<u32, Member>,
}

impl Tracker {
    /// A tracker for the tree led by `root`, whose own group and session
    /// ids (`groups`, `sessions`) name members while `root` is unreaped.
    /// `root` must be this process's unreaped child.
    pub fn new(root: Pinned, groups: Vec<u32>, sessions: Vec<u32>) -> Self {
        let mut tracker = Self {
            root: Some(root),
            groups,
            sessions,
            members: BTreeMap::new(),
        };
        if let Some(member) = Member::adopt(root) {
            tracker.members.insert(root.pid, member);
        }
        tracker
    }

    /// A tracker for a child that is no longer known to be unreaped: no root
    /// and no selectors, only the members already pinned.
    pub fn leader_reaped(&mut self) {
        self.root = None;
        self.groups.clear();
        self.sessions.clear();
    }

    /// Whether `pid` with `start` is a member this tracker pinned.
    pub fn has_seen(&self, pid: u32, start: u64) -> bool {
        self.members.get(&pid).is_some_and(|m| m.start == start)
    }

    /// Every pinned member, for a record of survivors.
    pub fn pinned(&self) -> Vec<Pinned> {
        (self.members.iter())
            .map(|(pid, m)| Pinned {
                pid: *pid,
                start: m.start,
            })
            .collect()
    }

    /// Absorb one complete scan: forget members the listing proves gone,
    /// adopt new members by the rules in the module docs, and return the
    /// live members.
    pub fn absorb(&mut self, table: &Table) -> Vec<Pinned> {
        let read: BTreeMap<u32, &Process> = table.processes.iter().map(|p| (p.pid, p)).collect();
        self.members.retain(|pid, member| {
            table.listed.contains(pid) && read.get(pid).is_none_or(|p| p.start == member.start)
        });
        let protected = super::protected(&table.processes);
        let selectors = self.root.is_some();
        let mut adopted: Vec<Pinned> = Vec::new();
        for process in &table.processes {
            if process.zombie
                || protected.contains(&process.pid)
                || self.has_seen(process.pid, process.start)
            {
                continue;
            }
            let selected = selectors
                && (self.groups.contains(&process.pgid)
                    || process.sid.is_some_and(|sid| self.sessions.contains(&sid)));
            let parent = self
                .members
                .get(&process.ppid)
                .map(|m| m.start)
                .filter(|start| process.start >= *start)
                // The parent read now is the parent the child was read with.
                .filter(|start| start_of(process.ppid) == Some(*start));
            if selected || parent.is_some() {
                adopted.push(process.pinned());
            }
        }
        for pinned in adopted {
            if let Some(member) = Member::adopt(pinned) {
                self.members.insert(pinned.pid, member);
            }
        }
        self.live(table)
    }

    /// The members a table shows alive (not zombies).
    fn live(&self, table: &Table) -> Vec<Pinned> {
        (table.processes.iter())
            .filter(|p| !p.zombie && self.has_seen(p.pid, p.start))
            .map(Process::pinned)
            .collect()
    }

    /// Scan once within `deadline` and absorb it; a failed scan changes
    /// nothing.
    pub fn refresh(&mut self, deadline: Instant) -> io::Result<Vec<Pinned>> {
        let table = snapshot_until(deadline)?;
        Ok(self.absorb(&table))
    }

    fn send(&self, pinned: Pinned, signal: i32) -> bool {
        self.members
            .get(&pinned.pid)
            .is_some_and(|m| m.start == pinned.start && m.signal(pinned.pid, signal))
    }

    /// Send `signal` to every live member once; returns the members found.
    pub fn signal(&mut self, signal: i32, deadline: Instant) -> io::Result<Vec<Pinned>> {
        let members = self.refresh(deadline)?;
        for member in &members {
            self.send(*member, signal);
        }
        Ok(members)
    }

    /// Kill every member until none is left or `bound` runs out, and return
    /// the members still alive then (empty: the tree is confirmed empty).
    /// The bound is passed into every scan and checked between freeze
    /// rounds. A scan that cannot finish is an error: what is alive is then
    /// unknown, and the caller must not take the tree for empty.
    pub fn kill(&mut self, bound: Duration) -> io::Result<Vec<Pinned>> {
        let deadline = Instant::now() + bound;
        loop {
            let mut members = match self.refresh(deadline) {
                Ok(members) => members,
                // Out of time mid-scan: every member not proven gone may
                // still be alive.
                Err(error) if error.kind() == io::ErrorKind::TimedOut => return Ok(self.pinned()),
                Err(error) => return Err(error),
            };
            if members.is_empty() || Instant::now() >= deadline {
                return Ok(members);
            }
            let mut stopped: Vec<Pinned> = Vec::new();
            for _ in 0..FREEZE_ROUNDS {
                let fresh: Vec<Pinned> = members
                    .into_iter()
                    .filter(|p| !stopped.contains(p))
                    .collect();
                if fresh.is_empty() || Instant::now() >= deadline {
                    break;
                }
                for pinned in fresh {
                    self.send(pinned, libc::SIGSTOP);
                    stopped.push(pinned);
                }
                match self.refresh(deadline) {
                    Ok(next) => members = next,
                    Err(_) => break,
                }
            }
            for pinned in &stopped {
                self.send(*pinned, libc::SIGKILL);
            }
            std::thread::sleep(KILL_POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}
