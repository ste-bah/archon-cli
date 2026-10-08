//! A library launcher that inherits this process's environment gets the
//! same jobserver policy as a command built here.
//!
//! The parent environment is supplied only to an isolated child test, so the
//! test process itself never mutates its environment.
use std::process::Command;

const CHILD: &str = "spawn::env_tests::launcher_child";

fn with_inherited_flags(flags: [&str; 3]) {
    let mut pipe = [-1; 2];
    // SAFETY: pipe writes two descriptors into the array supplied here.
    assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
    use std::os::fd::FromRawFd;
    // SAFETY: both descriptors were just created and are owned only here.
    let _read = unsafe { std::fs::File::from_raw_fd(pipe[0]) };
    // SAFETY: as above.
    let _write = unsafe { std::fs::File::from_raw_fd(pipe[1]) };
    let pair = format!("{},{}", pipe[0], pipe[1]);
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--ignored", "--exact", CHILD, "--nocapture"]);
    for (name, value) in ["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS"]
        .into_iter()
        .zip(flags)
    {
        command.env(name, value.replace("PAIR", &pair));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_library_launcher_drops_an_inherited_jobserver_auth_pair() {
    with_inherited_flags(["--jobserver-auth=PAIR -j2", "--silent", ""]);
}

#[test]
fn a_library_launcher_drops_inherited_legacy_jobserver_fds() {
    with_inherited_flags(["--silent", "--jobserver-fds=PAIR -j2", ""]);
}

#[test]
fn a_library_launcher_keeps_fifo_jobservers_and_assignments() {
    with_inherited_flags([
        "--silent --jobserver-auth PAIR",
        "",
        "--jobserver-auth=fifo:/tmp/archon-jobserver --jobserver-fds PAIR -- X=keep",
    ]);
}

#[test]
#[ignore = "the parent environment is supplied only to an isolated child"]
fn launcher_child() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("seen");
    // Built as a library builds its launchers: a plain std command that
    // inherits this process's environment.
    let mut launcher = Command::new("/bin/sh");
    launcher.args([
        "-c",
        "printf '%s\\n' \"$MAKEFLAGS\" \"$MFLAGS\" \"$GNUMAKEFLAGS\" > \"$1\"",
        "sh",
    ]);
    launcher.arg(&out);
    super::run_first_launcher(vec![launcher]).unwrap();
    let seen = std::fs::read_to_string(&out).unwrap();
    assert!(!seen.contains("--jobserver-fds"), "stale fds: {seen}");
    for line in seen
        .lines()
        .filter(|line| line.contains("--jobserver-auth"))
    {
        assert!(
            line.contains("--jobserver-auth=fifo:"),
            "stale auth: {seen}"
        );
    }
    let inherited = |name| std::env::var(name).unwrap_or_default();
    assert!(
        seen.contains("--silent"),
        "other flags must survive: {seen}"
    );
    if inherited("GNUMAKEFLAGS").contains("fifo:") {
        assert!(seen.contains("--jobserver-auth=fifo:/tmp/archon-jobserver"));
        assert!(seen.contains("-- X=keep"), "{seen}");
    }
}
