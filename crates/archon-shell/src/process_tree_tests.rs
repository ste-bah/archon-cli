use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use super::holders_impl::{fdinfo_writes, lsof_program_from, parse_lsof_fields};
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
fn a_scope_selects_its_groups_and_sessions_but_never_this_process() {
    let own = std::process::id();
    let table = vec![
        process(own, 1, 900, 900),
        process(100, 1, 100, 300),
        process(101, 1, 200, 300),
        process(102, 1, 100, 400),
        process(400, 1, 400, 400),
    ];
    let scope = Scope {
        groups: vec![100],
        sessions: vec![300],
    };
    assert_eq!(scope.members_in(&table), vec![100, 101, 102]);
}

#[test]
fn this_processs_own_group_and_session_are_never_a_scope() {
    // SAFETY: plain integer calls.
    let (group, session) = unsafe { (libc::getpgrp(), libc::getsid(0)) };
    let scope = Scope {
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
pub(super) fn group_leader(body: &str, dir: &Path) -> std::process::Child {
    std::process::Command::new("/bin/sh")
        .args(["-c", body])
        .current_dir(dir)
        .process_group(0)
        .spawn()
        .unwrap()
}

pub(super) fn wait_for(path: &Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "{path:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
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

#[path = "process_tree_tracker_tests.rs"]
mod tracker;

#[test]
fn lsof_is_found_on_path_then_at_a_standard_location() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str, mode: u32| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    };
    let on_path = make("path/lsof", 0o755);
    let standard = make("sbin/lsof", 0o755);
    let not_executable = make("plain/lsof", 0o644);
    let s = standard.to_str().unwrap();
    let pick =
        |path: Vec<std::path::PathBuf>, std_: &[&str]| lsof_program_from(path.into_iter(), std_);
    assert_eq!(pick(vec![on_path.clone()], &[s]), on_path, "PATH wins");
    assert_eq!(
        pick(vec![dir.path().join("missing/lsof")], &[s]),
        standard,
        "a confined PATH without lsof uses the standard location"
    );
    assert_eq!(
        pick(vec![not_executable], &[s]),
        standard,
        "a file without an execute bit is not a program"
    );
    assert_eq!(
        pick(vec![dir.path().join("path")], &[]),
        std::path::PathBuf::from("lsof"),
        "nothing found keeps a bare name so the spawn error names lsof"
    );
}

const CONFINED_PATH_ENV: &str = "ARCHON_TEST_CONFINED_PATH_HOLDERS";

/// Issues 270/311: a run-end guardian runs with its environment cleared and
/// PATH set to the policy's toolchain path (for example `/usr/bin:/bin`),
/// where an sbin-installed `lsof` is not on PATH. The probe must still find
/// a holder there. Run in a child test process with exactly that environment.
#[test]
fn holders_find_a_holder_under_a_confined_path() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "process_tree::tests::holders_child_under_a_confined_path",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(CONFINED_PATH_ENV, "1")
        .status()
        .unwrap();
    assert!(status.success(), "the confined-PATH child failed: {status}");
}

#[test]
#[ignore = "child process of holders_find_a_holder_under_a_confined_path"]
fn holders_child_under_a_confined_path() {
    if std::env::var_os(CONFINED_PATH_ENV).is_none() {
        return;
    }
    assert_eq!(std::env::var("PATH").unwrap(), "/usr/bin:/bin");
    let temp = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", ": > ready; exec sleep 30"])
        .current_dir(temp.path())
        .spawn()
        .unwrap();
    wait_for(&temp.path().join("ready"));
    let found = holders(&[temp.path()]);
    let _ = child.kill();
    let _ = child.wait();
    let found = found.expect("the probe ran under a confined PATH");
    assert!(found.iter().any(|h| h.pid == child.id()), "{found:?}");
}
