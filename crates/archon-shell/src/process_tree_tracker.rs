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

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

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
    #[cfg(target_os = "linux")]
    numeric: bool,
}

impl Member {
    /// Keep failed pidfd acquisitions as unknown, non-signallable pins.
    fn adopt(pinned: Pinned) -> Self {
        #[cfg(target_os = "linux")]
        {
            let opened = super::identity::pidfd::open(pinned);
            let numeric = matches!(&opened, Ok(None));
            Self {
                start: pinned.start,
                pidfd: opened.ok().flatten(),
                numeric,
            }
        }
        #[cfg(not(target_os = "linux"))]
        Self {
            start: pinned.start,
        }
    }

    fn signal(&self, pid: u32, signal: i32) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(fd) = &self.pidfd {
            return super::identity::pidfd::send(fd, signal);
        }
        #[cfg(target_os = "linux")]
        if !self.numeric {
            return false;
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

/// Reaping can invalidate selectors without acquiring the tracker mutex.
#[derive(Debug, Clone, Default)]
pub struct ReapToken(Arc<AtomicBool>);
impl ReapToken {
    pub fn mark_reaped(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "process tree operation ran out of time",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Tracker {
    /// The leader, while it is unreaped.
    root: Option<Pinned>,
    root_exited: bool,
    reaped: ReapToken,
    generation: Option<u64>,
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
            root_exited: false,
            reaped: ReapToken::default(),
            generation: None,
            groups,
            sessions,
            members: BTreeMap::new(),
        };
        tracker.members.insert(root.pid, Member::adopt(root));
        tracker
    }

    /// A tracker for a child that is no longer known to be unreaped: no root
    /// and no selectors, only the members already pinned.
    pub fn leader_reaped(&self) {
        self.reaped.mark_reaped();
    }

    pub fn reap_token(&self) -> ReapToken {
        self.reaped.clone()
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

    fn pinned_until(&self, deadline: Instant) -> io::Result<Vec<Pinned>> {
        let mut pins = Vec::new();
        for (pid, member) in &self.members {
            check_deadline(deadline)?;
            pins.push(Pinned {
                pid: *pid,
                start: member.start,
            });
        }
        Ok(pins)
    }

    /// Absorb one complete scan: forget members the listing proves gone,
    /// adopt new members by the rules in the module docs, and return the
    /// live members.
    pub fn absorb(&mut self, table: &Table) -> Vec<Pinned> {
        self.absorb_until(table, Instant::now() + super::SNAPSHOT_DEADLINE)
            .unwrap_or_else(|_| self.pinned())
    }

    /// Absorption, adoption and identity rechecks share the scan's deadline.
    /// Older snapshots cannot erase pins adopted by a newer snapshot.
    pub fn absorb_until(&mut self, table: &Table, deadline: Instant) -> io::Result<Vec<Pinned>> {
        check_deadline(deadline)?;
        if self.generation.is_some_and(|last| table.generation <= last) {
            return self.pinned_until(deadline);
        }
        // Record the generation even if adoption runs out of time: an older
        // table must never erase the pins this scan did manage to adopt.
        self.generation = Some(table.generation);
        let mut read = BTreeMap::new();
        let mut parents = BTreeMap::new();
        for process in &table.processes {
            check_deadline(deadline)?;
            read.insert(process.pid, process);
            parents.insert(process.pid, process.ppid);
        }
        let mut protected = BTreeSet::from([0, 1]);
        let mut current = std::process::id();
        while protected.insert(current) {
            check_deadline(deadline)?;
            match parents.get(&current) {
                Some(parent) => current = *parent,
                None => break,
            }
        }
        let mut gone = Vec::new();
        for (pid, member) in &self.members {
            check_deadline(deadline)?;
            if !table.listed.contains(pid) || read.get(pid).is_some_and(|p| p.start != member.start)
            {
                gone.push(*pid);
            }
        }
        for pid in gone {
            check_deadline(deadline)?;
            self.members.remove(&pid);
        }
        // One ancestry generation per scan: adopting a child cannot make its
        // parent an eligible identity recheck for another child in this table.
        let mut adopted = Vec::new();
        for process in &table.processes {
            check_deadline(deadline)?;
            if process.zombie
                || protected.contains(&process.pid)
                || self.has_seen(process.pid, process.start)
            {
                continue;
            }
            let selected = self.root.is_some()
                && !self.reaped.0.load(Ordering::SeqCst)
                && (self.groups.contains(&process.pgid)
                    || process.sid.is_some_and(|sid| self.sessions.contains(&sid)));
            let parent = self
                .members
                .get(&process.ppid)
                .map(|m| m.start)
                .filter(|start| process.start >= *start)
                .filter(|start| start_of(process.ppid) == Some(*start));
            check_deadline(deadline)?;
            if selected || parent.is_some() {
                adopted.push(process.pinned());
            }
        }
        for pinned in adopted {
            check_deadline(deadline)?;
            self.members.insert(pinned.pid, Member::adopt(pinned));
        }
        check_deadline(deadline)?;
        if !self.reaped.0.load(Ordering::SeqCst) && !self.root_exited {
            self.root_exited = self
                .root
                .is_some_and(|root| super::exited(root.pid).unwrap_or(false));
        }
        let mut live = Vec::new();
        for (pid, member) in &self.members {
            check_deadline(deadline)?;
            // A listed but unreadable pinned member may still be alive.
            let exited_leader = self.root_exited
                && !self.reaped.0.load(Ordering::SeqCst)
                && self.root.is_some_and(|root| root.pid == *pid);
            if !exited_leader && read.get(pid).is_none_or(|p| !p.zombie) {
                live.push(Pinned {
                    pid: *pid,
                    start: member.start,
                });
            }
        }
        Ok(live)
    }

    /// Scan once within `deadline` and absorb it; a failed scan changes
    /// nothing.
    pub fn refresh(&mut self, deadline: Instant) -> io::Result<Vec<Pinned>> {
        let table = snapshot_until(deadline)?;
        self.absorb_until(&table, deadline)
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
            check_deadline(deadline)?;
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
        self.kill_observed(bound, &mut || {})
    }

    /// [`Self::kill`], calling `after_stop` once each freeze round has sent
    /// its stops (a test seam: it lets a test run the clock out there).
    pub(super) fn kill_observed(
        &mut self,
        bound: Duration,
        after_stop: &mut dyn FnMut(),
    ) -> io::Result<Vec<Pinned>> {
        let deadline = Instant::now() + bound;
        loop {
            let members = self.refresh(deadline)?;
            if members.is_empty() || Instant::now() >= deadline {
                return Ok(members);
            }
            let mut stopped = BTreeSet::new();
            let frozen = self.freeze(members, &mut stopped, deadline, after_stop);
            // Whatever ended the freeze, the deadline included: a stopped
            // member is neither running nor dead, so every one is killed
            // before anything returns. The set is finite and each kill is
            // one signal, so this needs no deadline of its own.
            for pinned in &stopped {
                self.send(*pinned, libc::SIGKILL);
            }
            frozen?;
            std::thread::sleep(KILL_POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }

    /// Stop-and-rescan rounds: stop every member not stopped yet, recording
    /// it in `stopped` as soon as the stop is sent, then rescan. Ends when a
    /// rescan finds nobody new, a rescan fails, or the deadline passes.
    fn freeze(
        &mut self,
        mut members: Vec<Pinned>,
        stopped: &mut BTreeSet<Pinned>,
        deadline: Instant,
        after_stop: &mut dyn FnMut(),
    ) -> io::Result<()> {
        for _ in 0..FREEZE_ROUNDS {
            let mut fresh = Vec::new();
            for pinned in members {
                check_deadline(deadline)?;
                if !stopped.contains(&pinned) {
                    fresh.push(pinned);
                }
            }
            if fresh.is_empty() || Instant::now() >= deadline {
                return Ok(());
            }
            for pinned in fresh {
                check_deadline(deadline)?;
                self.send(pinned, libc::SIGSTOP);
                stopped.insert(pinned);
            }
            after_stop();
            match self.refresh(deadline) {
                Ok(next) => members = next,
                Err(_) => return Ok(()),
            }
        }
        Ok(())
    }
}
