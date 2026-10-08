//! Issue 270 round 3: adoption never takes a stranger, identities hold across
//! scans, and an incomplete scan forgets nobody.
use std::time::{Duration, Instant};

use super::super::*;
use super::{group_leader, wait_for};

fn sleeper() -> std::process::Child {
    crate::spawn::command("sleep").arg("30").spawn().unwrap()
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
        generation: Table::next_generation(),
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
        generation: Table::next_generation(),
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
    let mut child = crate::spawn::command("/bin/sh")
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

#[test]
fn unreadable_pins_are_returned_as_possible_survivors() {
    let mut root = sleeper();
    let pinned = pin(&root);
    let mut tracker = Tracker::new(pinned, Vec::new(), Vec::new());
    let live = tracker.absorb(&Table {
        generation: Table::next_generation(),
        listed: [pinned.pid].into(),
        processes: Vec::new(),
    });
    let _ = root.kill();
    let _ = root.wait();
    assert_eq!(live, vec![pinned], "unreadable is unknown, never empty");
}

#[test]
fn a_timed_out_scan_with_no_pins_is_unknown_not_empty() {
    let mut tracker = Tracker::default();
    assert!(
        tracker.kill(Duration::ZERO).is_err(),
        "unknown descendants cannot mean confirmed empty"
    );
}

#[test]
fn an_older_snapshot_cannot_erase_a_newer_pin() {
    let mut root = sleeper();
    let mut child = sleeper();
    let (root_pin, pinned) = (pin(&root), pin(&child));
    let mut tracker = Tracker::new(root_pin, vec![root_pin.pid], Vec::new());
    let older = table(vec![entry(root_pin, 1, root_pin.pid)]);
    let newer = table(vec![
        entry(root_pin, 1, root_pin.pid),
        entry(pinned, 1, root_pin.pid),
    ]);
    tracker.absorb(&newer);
    tracker.leader_reaped(); // Now only the new pin reaches the escaped member.
    let returned = tracker.absorb(&older);
    let kept = tracker.has_seen(pinned.pid, pinned.start);
    for member in [&mut root, &mut child] {
        let _ = member.kill();
        let _ = member.wait();
    }
    assert!(
        kept && returned.contains(&pinned),
        "an out-of-order table forgot a pin"
    );
}

#[test]
fn adoption_honours_an_expired_deadline() {
    let mut tracker = Tracker::default();
    let expired = Instant::now() - Duration::from_millis(1);
    assert!(tracker.absorb_until(&table(Vec::new()), expired).is_err());
}

#[test]
fn pidfd_permission_failure_never_allows_numeric_signalling() {
    assert!(!super::super::identity::pidfd_failure_allows_fallback(
        libc::EPERM
    ));
    assert!(!super::super::identity::pidfd_failure_allows_fallback(
        libc::EMFILE
    ));
    assert!(super::super::identity::pidfd_failure_allows_fallback(
        libc::ENOSYS
    ));
}

#[test]
fn a_large_adoption_batch_cannot_outlive_its_budget() {
    let processes = (100_000..150_000)
        .map(|pid| Process {
            pid,
            ppid: 1,
            pgid: 1,
            sid: Some(1),
            start: 1,
            zombie: false,
        })
        .collect();
    let table = table(processes);
    let mut tracker = Tracker::default();
    let begin = Instant::now();
    let result = tracker.absorb_until(&table, begin + Duration::from_millis(1));
    assert!(
        result.is_err(),
        "the entire absorption must share the deadline"
    );
    assert!(begin.elapsed() < Duration::from_millis(150));
}

#[test]
fn an_unreadable_but_proven_exited_leader_does_not_stall() {
    let mut root = crate::spawn::command("true").spawn().unwrap();
    while !exited(root.id()).unwrap() {
        std::thread::sleep(Duration::from_millis(2));
    }
    let pinned = Pinned {
        pid: root.id(),
        start: 0,
    };
    let mut tracker = Tracker::new(pinned, vec![pinned.pid], Vec::new());
    let live = tracker.absorb(&Table {
        generation: Table::next_generation(),
        listed: [pinned.pid].into(),
        processes: Vec::new(),
    });
    root.wait().unwrap();
    assert!(
        live.is_empty(),
        "waitid proved the unreadable leader exited: {live:?}"
    );
}

#[test]
fn a_deadline_that_expires_mid_freeze_still_kills_every_stopped_member() {
    // Round 5: a member SIGSTOPped by a freeze round is neither running nor
    // dead. When the clock ran out before the SIGKILL, it was left stopped.
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("child");
    let body = format!(
        "sleep 30 & echo $! > '{ready}.tmp' && mv '{ready}.tmp' '{ready}'\nsleep 30",
        ready = ready.display()
    );
    let mut leader = group_leader(&body, temp.path());
    wait_for(&ready);
    let child: u32 = std::fs::read_to_string(&ready)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let child_start = start_of(child).expect("the backgrounded sleep runs");
    let leader_start = start_of(leader.id()).expect("the leader runs");
    let members = [leader.id() as i32, child as i32];
    // Whatever happens below, nothing this test started is left behind.
    struct Reap([i32; 2]);
    impl Drop for Reap {
        fn drop(&mut self) {
            for pid in self.0 {
                // SAFETY: these are the processes this test started.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                    libc::kill(pid, libc::SIGCONT);
                }
            }
        }
    }
    let _reap = Reap(members);
    let mut tracker = Tracker::new(pin(&leader), vec![leader.id()], Vec::new());
    let bound = Duration::from_secs(2);
    let expired = Instant::now() + bound + Duration::from_millis(50);
    let mut rounds = 0;
    // The first freeze round has stopped both members; the clock then runs
    // out before the rescan, so teardown ends on its deadline.
    let _ = tracker.kill_observed(bound, &mut || {
        rounds += 1;
        while Instant::now() < expired {
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    assert_eq!(rounds, 1, "the freeze ran one round before the deadline");
    let start = Instant::now();
    // Dead is gone or a zombie, never merely stopped (start_of reads
    // neither a zombie nor a reused pid as the pinned process).
    let leader_dead = loop {
        if start_of(leader.id()) != Some(leader_start) {
            break true;
        }
        if start.elapsed() > Duration::from_secs(5) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let child_dead = loop {
        if start_of(child) != Some(child_start) {
            break true;
        }
        if start.elapsed() > Duration::from_secs(5) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(_reap);
    let _ = leader.wait();
    assert!(
        leader_dead && child_dead,
        "a stopped member was left stopped: leader dead {leader_dead}, child dead {child_dead}"
    );
}

#[test]
fn a_stopped_child_has_not_exited() {
    // macOS waitid reports a stop although only WEXITED was asked for: a
    // leader frozen by teardown (or by job control) read as exited.
    let mut child = sleeper();
    // SAFETY: signals only the child this test started.
    unsafe { libc::kill(child.id() as i32, libc::SIGSTOP) };
    // Block until the stop has happened, so the probe below sees it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is valid output space; WSTOPPED|WNOWAIT only queries.
    let waited = unsafe {
        libc::waitid(
            libc::P_PID,
            libc::id_t::from(child.id()),
            &mut info,
            libc::WSTOPPED | libc::WNOWAIT,
        )
    };
    let stopped_exited = exited(child.id());
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(waited, 0, "the stop was never observed");
    assert!(
        !stopped_exited.unwrap(),
        "a stopped child was reported as exited"
    );
}

#[path = "process_tree_recording_r4_tests.rs"]
mod recording_r4_tests;
