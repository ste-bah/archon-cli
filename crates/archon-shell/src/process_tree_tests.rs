use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use super::holders_impl::{
    STANDARD_LSOF, fdinfo_writes, first_path_fallback, holders_using, lsof_program_from,
    parse_lsof_fields,
};
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

/// An executable script at `dir/name`.
fn fake_program(dir: &Path, name: &str, script: &str) -> std::path::PathBuf {
    let program = dir.join(name);
    std::fs::create_dir_all(program.parent().unwrap()).unwrap();
    std::fs::write(&program, script).unwrap();
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    program
}

/// Round 2, rule 4: a probe that cannot finish (lsof stuck on an unavailable
/// filesystem) must end within a bound and say so, never hang the check.
/// Issue 321: the hanging `lsof` comes in through the lookup seam, not PATH.
#[test]
fn holders_end_within_a_bound_when_lsof_hangs() {
    if cfg!(target_os = "linux") {
        return; // Linux reads /proc; no lsof is involved.
    }
    let fake = tempfile::tempdir().unwrap();
    let lsof = fake_program(fake.path(), "lsof", "#!/bin/sh\nexec sleep 120\n");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found = holders_using(&[root.as_path()], HOLDER_PROBE_DEADLINE, || Ok(lsof));
        let _ = sender.send(found.map(|_| ()));
    });
    let result = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("holders did not return within 30 s while lsof hung");
    assert!(
        result.is_err(),
        "a probe that could not finish proves nothing"
    );
}

/// Issue 321: no lsof, or one that cannot run, is an error ("unknown"),
/// never an empty list of holders.
#[test]
fn holders_without_a_runnable_lsof_are_unknown_not_none() {
    if cfg!(target_os = "linux") {
        return; // Linux reads /proc; no lsof is involved.
    }
    let dir = tempfile::tempdir().unwrap();
    let missing = || {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no executable lsof",
        ))
    };
    let error = holders_using(&[dir.path()], HOLDER_PROBE_DEADLINE, missing).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound, "{error}");
    let plain = dir.path().join("plain-lsof");
    std::fs::write(&plain, "").unwrap();
    let found = holders_using(&[dir.path()], HOLDER_PROBE_DEADLINE, || Ok(plain));
    assert!(found.is_err(), "an lsof that cannot run proved {found:?}");
    let silent = fake_program(dir.path(), "silent/lsof", "#!/bin/sh\nexit 0\n");
    let found = holders_using(&[dir.path()], HOLDER_PROBE_DEADLINE, || Ok(silent));
    assert!(
        found.is_err(),
        "an lsof that listed nothing proved {found:?}"
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

/// Issue 321: a fixed system location wins over PATH, which is used only
/// when no standard location has an executable `lsof`.
#[test]
fn lsof_is_found_at_a_standard_location_then_on_path() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str, mode: u32| {
        let p = dir.path().join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    };
    let planted = make("toolchain/lsof", 0o755);
    let standard = make("sbin/lsof", 0o755);
    let not_executable = make("plain/lsof", 0o644);
    let (s, plain) = (standard.to_str().unwrap(), not_executable.to_str().unwrap());
    let missing = dir.path().join("missing/lsof");
    let missing_s = missing.to_str().unwrap();
    let pick = |std_: &[&str], path: Vec<std::path::PathBuf>| {
        lsof_program_from(std_, path.into_iter()).map_err(|e| e.kind())
    };
    assert_eq!(
        pick(&[missing_s, s], vec![planted.clone()]),
        Ok(standard.clone()),
        "a standard location wins over a planted PATH lsof"
    );
    assert_eq!(
        pick(&[s], vec![]),
        Ok(standard.clone()),
        "an empty or confined PATH still finds the standard location"
    );
    assert_eq!(
        pick(&[plain, s], vec![planted.clone()]),
        Ok(standard.clone()),
        "a standard file without an execute bit is skipped"
    );
    assert_eq!(
        pick(&[missing_s, plain], vec![missing.clone(), planted.clone()]),
        Ok(planted.clone()),
        "PATH is used when no standard location has an executable lsof"
    );
    let error =
        lsof_program_from(&[missing_s, plain], vec![missing.clone()].into_iter()).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains("no executable lsof"), "{error}");
    assert!(error.to_string().contains(missing_s), "{error}");
}

const RELATIVE_PATH_ENV: &str = "ARCHON_TEST_RELATIVE_PATH_LSOF";

/// Issue 321: a relative PATH entry resolves against the working directory,
/// so it is never used, even when an executable `lsof` sits right there.
/// Run in a child test process whose cwd holds `toolchain/lsof`.
#[test]
fn a_relative_path_lsof_is_never_used() {
    let cwd = tempfile::tempdir().unwrap();
    fake_program(cwd.path(), "toolchain/lsof", "#!/bin/sh\nexit 1\n");
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "process_tree::tests::relative_path_lsof_child",
            "--nocapture",
        ])
        .current_dir(cwd.path())
        .env(RELATIVE_PATH_ENV, "1")
        .status()
        .unwrap();
    assert!(status.success(), "the relative-PATH child failed: {status}");
}

#[test]
#[ignore = "child process of a_relative_path_lsof_is_never_used"]
fn relative_path_lsof_child() {
    if std::env::var_os(RELATIVE_PATH_ENV).is_none() {
        return;
    }
    let relative = std::path::PathBuf::from("toolchain/lsof");
    assert!(relative.is_file(), "the cwd does not hold toolchain/lsof");
    let missing = "/no/such/dir/lsof";
    let found = lsof_program_from(&[missing], vec![relative].into_iter());
    assert_eq!(
        found.map_err(|e| e.kind()),
        Err(std::io::ErrorKind::NotFound),
        "a relative PATH entry is never used"
    );
}

/// Issue 321: the PATH-fallback warning is logged once per process and path.
#[test]
fn a_path_fallback_warns_once_per_path() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a/lsof"), dir.path().join("b/lsof"));
    assert!(first_path_fallback(&a));
    assert!(!first_path_fallback(&a));
    assert!(first_path_fallback(&b));
}

/// Issue 321, the production list: on a host with `lsof` at a standard
/// location (`/usr/sbin/lsof` on macOS), a planted PATH `lsof` never wins.
#[test]
fn a_planted_path_lsof_never_beats_the_system_one() {
    let Some(system) = STANDARD_LSOF.iter().find(|p| Path::new(p).is_file()) else {
        return; // No system lsof here; the seam test above covers the order.
    };
    let dir = tempfile::tempdir().unwrap();
    let planted = fake_program(dir.path(), "lsof", "#!/bin/sh\nexit 1\n");
    let found = lsof_program_from(STANDARD_LSOF, vec![planted.clone()].into_iter()).unwrap();
    assert_ne!(found, planted);
    assert_eq!(found, Path::new(system));
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
