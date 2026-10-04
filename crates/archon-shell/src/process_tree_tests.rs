use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use super::holders_impl::parse_lsof_fields;
use super::*;

fn process(pid: u32, ppid: u32, pgid: u32, sid: u32) -> Process {
    Process {
        pid,
        ppid,
        pgid,
        sid: Some(sid),
        zombie: false,
    }
}

#[test]
fn proc_stat_fields_are_read_after_the_last_parenthesis() {
    let stat = "4242 (a (weird) name) S 17 4242 4000 0 -1 4194560 0 0";
    assert_eq!(
        parse_proc_stat(4242, stat),
        Some(process(4242, 17, 4242, 4000))
    );
    let zombie = parse_proc_stat(7, "7 (z) Z 1 7 7 0").unwrap();
    assert!(zombie.zombie);
    assert_eq!(parse_proc_stat(7, "7 (truncated"), None);
}

#[test]
fn lsof_fields_pair_each_path_with_the_process_that_holds_it() {
    let text = "p10\nfcwd\nn/a/b\nftxt\nn/bin/x\np11\nf3\nn/c\n";
    assert_eq!(
        parse_lsof_fields(text),
        vec![
            (10, "/a/b".into()),
            (10, "/bin/x".into()),
            (11, "/c".into())
        ]
    );
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
