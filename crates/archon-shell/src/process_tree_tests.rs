use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use super::holders_impl::{fdinfo_writes, parse_lsof_fields};
use super::*;

fn process(pid: u32, ppid: u32, pgid: u32, sid: u32) -> Process {
    Process {
        pid,
        ppid,
        pgid,
        sid: Some(sid),
        start: 1,
        zombie: false,
    }
}

#[test]
fn proc_stat_fields_are_read_after_the_last_parenthesis() {
    // Fields after the name: state, ppid, pgrp, session, then 15 more, then
    // starttime (field 22).
    let stat =
        "4242 (a (weird) name) S 17 4242 4000 0 -1 4194560 0 0 0 0 1 2 0 0 20 0 1 0 98765 1000 50";
    let mut expected = process(4242, 17, 4242, 4000);
    expected.start = 98765;
    assert_eq!(parse_proc_stat(4242, stat), Some(expected));
    let zombie = parse_proc_stat(7, "7 (z) Z 1 7 7 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 0 0").unwrap();
    assert!(zombie.zombie);
    assert_eq!(parse_proc_stat(7, "7 (truncated"), None);
    // A line without a start time is not an identity.
    assert_eq!(parse_proc_stat(7, "7 (short) S 1 7 7 0"), None);
}

#[test]
fn lsof_fields_pair_each_path_with_its_holder_and_access_mode() {
    let text = "p10\nfcwd\na \nn/a/b\nf3\naw\nn/a/c\nf4\nar\nn/a/d\np11\nf5\nau\nn/e\n";
    assert_eq!(
        parse_lsof_fields(text),
        vec![
            (10, "/a/b".into(), false),
            (10, "/a/c".into(), true),
            (10, "/a/d".into(), false),
            (11, "/e".into(), true)
        ]
    );
}

#[test]
fn fdinfo_access_mode_says_whether_a_file_is_written() {
    assert_eq!(
        fdinfo_writes("pos:\t0\nflags:\t0100000\nmnt_id:\t1\n"),
        Some(false)
    );
    assert_eq!(fdinfo_writes("pos:\t0\nflags:\t0100001\n"), Some(true));
    assert_eq!(fdinfo_writes("pos:\t0\nflags:\t02100002\n"), Some(true));
    assert_eq!(fdinfo_writes("pos:\t0\n"), None);
}

#[test]
fn scope_reaches_groups_sessions_and_descendants_but_never_this_process() {
    let own = std::process::id();
    let table = vec![
        process(own, 1, 900, 900),
        // A group member, its child that moved to its own group, and that
        // child's child that left the session too.
        process(100, own, 100, 900),
        process(101, 100, 101, 900),
        process(102, 101, 102, 102),
        // A member of the session in another group, with no tie by ancestry.
        process(200, 1, 200, 300),
        // Unrelated.
        process(400, 1, 400, 400),
    ];
    let scope = Scope {
        roots: Vec::new(),
        groups: vec![100],
        sessions: vec![300],
    };
    assert_eq!(scope.members_in(&table), vec![100, 101, 102, 200]);
    // Naming this process directly reaches nothing of its own.
    let self_scope = Scope {
        roots: vec![own],
        groups: Vec::new(),
        sessions: Vec::new(),
    };
    assert_eq!(self_scope.members_in(&table), vec![100, 101, 102]);
}

#[test]
fn this_processs_own_group_and_session_are_never_a_scope() {
    // SAFETY: plain integer calls.
    let (group, session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
    let scope = Scope {
        roots: Vec::new(),
        groups: vec![group as u32],
        sessions: vec![session as u32],
    };
    let members = scope.members().unwrap();
    assert!(
        members.is_empty(),
        "own group/session leaked into a scope: {members:?}"
    );
}

/// Spawns `sh -c body` as its own group leader; returns the child.
fn group_leader(body: &str, dir: &Path) -> std::process::Child {
    std::process::Command::new("/bin/sh")
        .args(["-c", body])
        .current_dir(dir)
        .process_group(0)
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "{path:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn kill_empties_a_tree_whose_members_left_the_group_and_the_session() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let body = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); if (fork() == 0) {{ setpgid(0, 0); open(F, \">\", \"{}\"); close F; sleep 30; exit 0 }} sleep 30' & sleep 30",
        ready.display()
    );
    let mut leader = group_leader(&body, temp.path());
    wait_for(&ready);
    let scope = Scope {
        roots: vec![leader.id()],
        groups: vec![leader.id()],
        sessions: Vec::new(),
    };
    assert!(scope.members().unwrap().len() >= 3, "the tree is visible");
    let survivors = scope.kill(Duration::from_secs(5)).unwrap();
    leader.wait().unwrap();
    assert!(survivors.is_empty(), "survivors: {survivors:?}");
    assert!(scope.members().unwrap().is_empty());
}

#[test]
fn holders_finds_a_detached_process_by_its_cwd_and_skips_this_one() {
    let temp = tempfile::tempdir().unwrap();
    let _own = std::fs::File::create(temp.path().join("held-here")).unwrap();
    assert_eq!(holders(&[temp.path()]).unwrap(), Vec::new());
    let ready = temp.path().join("ready");
    let mut child = group_leader(
        &format!("touch '{}'; exec sleep 30", ready.display()),
        temp.path(),
    );
    wait_for(&ready);
    let found = holders(&[temp.path(), Path::new("/no/such/root")]).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        found.iter().map(|h| h.pid).collect::<Vec<_>>(),
        vec![child.id()],
        "{found:?}"
    );
    assert!(holders(&[Path::new("/no/such/root")]).unwrap().is_empty());
}

const HANGING_LSOF_ENV: &str = "ARCHON_TEST_HANGING_LSOF";

/// Round 2, rule 4: a probe that cannot finish (lsof stuck on an unavailable
/// filesystem) must end within a bound and say so, never hang the check.
/// Run in a child test process, whose PATH puts a hanging `lsof` first.
#[test]
fn holders_end_within_a_bound_when_lsof_hangs() {
    if cfg!(target_os = "linux") {
        return; // Linux reads /proc; no lsof is involved.
    }
    let fake = tempfile::tempdir().unwrap();
    let lsof = fake.path().join("lsof");
    std::fs::write(&lsof, "#!/bin/sh\nexec sleep 120\n").unwrap();
    std::fs::set_permissions(&lsof, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        fake.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "process_tree::tests::holders_child_with_a_hanging_lsof",
            "--nocapture",
        ])
        .env("PATH", path)
        .env(HANGING_LSOF_ENV, "1")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
#[ignore = "child process of holders_end_within_a_bound_when_lsof_hangs"]
fn holders_child_with_a_hanging_lsof() {
    if std::env::var_os(HANGING_LSOF_ENV).is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(holders(&[root.as_path()]).map(|_| ()));
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("holders did not return within 30 s while lsof hung");
    assert!(
        result.is_err(),
        "a probe that could not finish proves nothing"
    );
}

#[test]
fn a_pid_whose_identity_changed_is_never_signalled() {
    // Round 2, rule 1: the same pid with another start time is another
    // process (the pid was reused). It must not receive the signal.
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let pid = child.id();
    let start = start_of(pid).expect("a live child has a start time");
    let reused = Pinned {
        pid,
        start: start + 1,
    };
    assert!(
        !deliver(reused, libc::SIGKILL),
        "a stale identity was signalled"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none(), "the child was killed");
    assert!(deliver(Pinned { pid, start }, libc::SIGKILL));
    child.wait().unwrap();
    // A reaped process has no identity left to match.
    assert!(!deliver(Pinned { pid, start }, libc::SIGKILL));
}

#[test]
fn a_tracker_kills_a_member_it_saw_after_its_parent_exited() {
    // Round 2, rule 2: the descendant leaves the group and the session, and
    // its parent exits. Only a scan made while the parent lived ties it to
    // the tree.
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let body = format!(
        "perl -MPOSIX -e 'POSIX::setsid(); open(F, \">\", \"{}\"); close F; sleep 30' </dev/null >/dev/null 2>&1 &\nuntil [ -e '{}' ]; do sleep 0.01; done\nsleep 0.3",
        ready.display(),
        ready.display()
    );
    let mut leader = group_leader(&body, temp.path());
    wait_for(&ready);
    let scope = Scope {
        roots: vec![leader.id()],
        groups: vec![leader.id()],
        sessions: Vec::new(),
    };
    let mut tracker = Tracker::new(scope.clone());
    let seen = tracker.refresh().unwrap();
    assert!(seen.len() >= 2, "the leader and the descendant: {seen:?}");
    leader.wait().unwrap();
    tracker.scope_mut().roots.clear();
    let mut without_roots = scope;
    without_roots.roots.clear();
    let unreachable = without_roots.members().unwrap();
    assert!(
        unreachable.is_empty(),
        "no scope reaches it any more: {unreachable:?}"
    );
    let daemon = seen
        .iter()
        // The one that leads its own session.
        .find(|p| p.pid != leader.id() && p.sid == Some(p.pid))
        .map(Process::pinned)
        .unwrap();
    assert_eq!(start_of(daemon.pid), Some(daemon.start), "still running");
    assert!(tracker.kill(Duration::from_secs(5)).unwrap().is_empty());
    let start = Instant::now();
    while start_of(daemon.pid) == Some(daemon.start) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "seen member survived"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn holders_say_which_holder_writes() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("held");
    std::fs::write(&file, "x").unwrap();
    let spawn = |mode: &str, mark: &str| {
        let ready = temp.path().join(mark);
        let child = std::process::Command::new("perl")
            .args([
                "-e",
                &format!(
                    "open(F, '{mode}', $ARGV[0]) or die; open(R, '>', $ARGV[1]); close R; sleep 30"
                ),
            ])
            .arg(&file)
            .arg(&ready)
            .current_dir("/")
            .spawn()
            .unwrap();
        wait_for(&ready);
        child
    };
    let mut reader = spawn("<", "reader-ready");
    let mut writer = spawn(">>", "writer-ready");
    let found = holders(&[temp.path()]).unwrap();
    for child in [&mut reader, &mut writer] {
        let _ = child.kill();
        let _ = child.wait();
    }
    let writes = |pid: u32| found.iter().find(|h| h.pid == pid).map(|h| h.writes);
    assert_eq!(writes(reader.id()), Some(false), "{found:?}");
    assert_eq!(writes(writer.id()), Some(true), "{found:?}");
}
