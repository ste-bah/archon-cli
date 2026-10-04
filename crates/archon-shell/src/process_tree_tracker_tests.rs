//! Issue 270 round 3: adoption never takes a stranger, identities hold across
//! scans, and an incomplete scan forgets nobody.
use std::time::{Duration, Instant};

use super::super::*;
use super::{group_leader, wait_for};

fn sleeper() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap()
}

fn pin(child: &std::process::Child) -> Pinned {
    Pinned {
        pid: child.id(),
        start: start_of(child.id()).expect("a live child has a start time"),
    }
}

/// A table listing `processes`, as a complete scan would.
fn table(processes: Vec<Process>) -> Table {
    Table {
        listed: processes.iter().map(|p| p.pid).collect(),
        processes,
    }
}

fn entry(pinned: Pinned, ppid: u32, pgid: u32) -> Process {
    Process {
        pid: pinned.pid,
        ppid,
        pgid,
        sid: Some(pinned.pid),
        start: pinned.start,
        zombie: false,
    }
}

#[test]
fn a_pid_whose_identity_changed_is_never_signalled() {
    let mut child = sleeper();
    let pinned = pin(&child);
    let reused = Pinned {
        start: pinned.start + 1,
        ..pinned
    };
    assert!(
        !deliver(reused, libc::SIGKILL),
        "a stale identity was signalled"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none(), "the child was killed");
    assert!(deliver(pinned, libc::SIGKILL));
    child.wait().unwrap();
    assert!(
        !deliver(pinned, libc::SIGKILL),
        "a reaped process was signalled"
    );
}

#[test]
fn a_child_older_than_its_pinned_parent_is_never_adopted() {
    // A process that started before the member it names as its parent was
    // not started by it: its pid-numbered parent is a reuse.
    let mut older = sleeper();
    std::thread::sleep(Duration::from_millis(20));
    let mut root = sleeper();
    let (older_pin, root_pin) = (pin(&older), pin(&root));
    let mut tracker = Tracker::new(root_pin, Vec::new(), Vec::new());
    tracker.absorb(&table(vec![
        entry(root_pin, 1, root_pin.pid),
        entry(older_pin, root_pin.pid, older_pin.pid),
    ]));
    let adopted = tracker.has_seen(older_pin.pid, older_pin.start);
    for child in [&mut older, &mut root] {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(!adopted, "a process older than its parent was adopted");
}

#[test]
fn group_and_session_selectors_end_when_the_leader_is_reaped() {
    // Once the leader is reaped its group id can be reused: a process in a
    // group of that number is a stranger.
    let mut stranger = sleeper();
    let mut root = sleeper();
    let (stranger_pin, root_pin) = (pin(&stranger), pin(&root));
    let selected = || table(vec![entry(stranger_pin, 1, root_pin.pid)]);
    let mut live = Tracker::new(root_pin, vec![root_pin.pid], Vec::new());
    live.absorb(&selected());
    let mut reaped = Tracker::new(root_pin, vec![root_pin.pid], Vec::new());
    reaped.leader_reaped();
    reaped.absorb(&selected());
    let (with_leader, after_reap) = (
        live.has_seen(stranger_pin.pid, stranger_pin.start),
        reaped.has_seen(stranger_pin.pid, stranger_pin.start),
    );
    for child in [&mut stranger, &mut root] {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(
        with_leader,
        "the selector names group members while the leader is unreaped"
    );
    assert!(
        !after_reap,
        "a reaped leader's group number adopted a stranger"
    );
}

#[test]
fn an_incomplete_or_failed_scan_forgets_nobody() {
    let mut root = sleeper();
    let root_pin = pin(&root);
    let mut tracker = Tracker::new(root_pin, Vec::new(), Vec::new());
    assert!(tracker.has_seen(root_pin.pid, root_pin.start));
    // A scan out of time before it started is an error, not an empty table.
    let expired = Instant::now() - Duration::from_millis(1);
    assert!(tracker.refresh(expired).is_err());
    // A listed pid that could not be read is not taken for gone.
    tracker.absorb(&Table {
        processes: Vec::new(),
        listed: [root_pin.pid].into(),
    });
    let kept = tracker.has_seen(root_pin.pid, root_pin.start);
    // Only a complete listing without the pid forgets it.
    tracker.absorb(&table(Vec::new()));
    let forgotten = !tracker.has_seen(root_pin.pid, root_pin.start);
    let _ = root.kill();
    let _ = root.wait();
    assert!(kept && forgotten, "kept {kept}, forgotten {forgotten}");
}

#[test]
fn exited_reports_an_exit_without_reaping_the_child() {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 3"])
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !exited(child.id()).unwrap() {
        assert!(start.elapsed() < Duration::from_secs(10), "never exited");
        std::thread::sleep(Duration::from_millis(5));
    }
    // Still unreaped: its status is there to collect.
    assert_eq!(child.wait().unwrap().code(), Some(3));
}

#[test]
fn kill_reaches_members_that_left_the_group_and_the_session_and_whose_parent_exited() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let body = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); if (fork() == 0) {{ setpgid(0, 0); open(F, \">\", \"{}\"); close F; sleep 30; exit 0 }} sleep 30' </dev/null >/dev/null 2>&1 &\nuntil [ -e '{}' ]; do sleep 0.01; done\nsleep 0.3",
        ready.display(),
        ready.display()
    );
    let mut leader = group_leader(&body, temp.path());
    wait_for(&ready);
    let leader_pin = pin(&leader);
    let mut tracker = Tracker::new(leader_pin, vec![leader.id()], Vec::new());
    // A generation per scan: the perl leader, then its forked child.
    let deadline = || Instant::now() + Duration::from_secs(5);
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen = tracker.refresh(deadline()).unwrap();
    }
    assert!(seen.len() >= 3, "leader, perl and its child: {seen:?}");
    // The shell exits; only the pins tie the rest to the tree now.
    while !exited(leader.id()).unwrap() {
        std::thread::sleep(Duration::from_millis(10));
    }
    leader.wait().unwrap();
    tracker.leader_reaped();
    let escaped: Vec<Pinned> = seen
        .into_iter()
        .filter(|p| p.pid != leader_pin.pid)
        .collect();
    assert!(tracker.kill(Duration::from_secs(5)).unwrap().is_empty());
    let start = Instant::now();
    while escaped.iter().any(|p| start_of(p.pid) == Some(p.start)) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "a pinned member survived"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
