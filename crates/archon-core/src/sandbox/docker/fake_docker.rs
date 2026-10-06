//! A stand-in for the docker CLI, so no unit test depends on a host daemon.
//!
//! The pool's tests used to run the real `docker` binary, and hung with it the
//! day the daemon stopped answering. Each fake is its own temp dir holding a
//! `docker` and a `body.sh`: the call is recorded, then the body says what the
//! "daemon" does — answer, fail, or never answer.
//!
//! Every fake's `docker` is a symlink to one dispatcher script per test
//! process, run once before first use. Measured on macOS: the first exec of a
//! freshly written script took 0.8–5 s (the OS scans each new executable), which
//! is far longer than any test bound; through a symlink to an already-run
//! script the same call took 20 ms. The body is sourced, not executed, so it is
//! never scanned.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Short enough that a fake daemon that never answers is seen in seconds,
/// long enough that a fake that does answer is never tripped on a loaded
/// machine (each answering call measured at about 20 ms).
pub(super) const TEST_BOUND: Duration = Duration::from_secs(2);

/// A ceiling no bounded call should come near: the fakes that hang sleep for
/// 30 s, so finishing under this proves the bound fired.
pub(super) const WELL_UNDER_A_HANG: Duration = Duration::from_secs(10);

pub(super) struct FakeDocker {
    dir: tempfile::TempDir,
    binary: PathBuf,
}

impl FakeDocker {
    /// `body` is POSIX sh run after the call is recorded; `$1` is the docker
    /// subcommand and `$FAKE_DIR` is this fake's own directory.
    pub(super) fn new(body: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("body.sh"), format!("{body}\n")).expect("write body");
        let binary = dir.path().join("docker");
        std::os::unix::fs::symlink(dispatcher(), &binary).expect("link fake docker");
        Self { dir, binary }
    }

    pub(super) fn binary(&self) -> String {
        self.binary.display().to_string()
    }

    pub(super) fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// Every call made so far, one line of arguments each.
    pub(super) fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

/// The one script every fake runs: record the call beside the symlink it was
/// invoked through, then source that fake's body.
fn dispatcher() -> &'static Path {
    static DISPATCHER: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    &DISPATCHER
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("temp dir");
            let script = dir.path().join("fake-docker");
            std::fs::write(
                &script,
                "#!/bin/sh\nFAKE_DIR=$(dirname \"$0\")\n\
                 printf '%s\\n' \"$*\" >> \"$FAKE_DIR/calls.log\"\n\
                 . \"$FAKE_DIR/body.sh\"\n",
            )
            .expect("write dispatcher");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("make dispatcher executable");
            std::fs::write(dir.path().join("body.sh"), "exit 0\n").expect("warm-up body");
            // Pay the first-exec cost here, unbounded, rather than inside a test.
            let warmed = warm_up(&script);
            assert!(warmed.success(), "dispatcher warm-up failed: {warmed}");
            std::fs::remove_file(dir.path().join("calls.log")).ok();
            (dir, script)
        })
        .1
}

/// Run the dispatcher once. Retried on ETXTBSY: on Linux a just-written file
/// can still be open for writing in a child another test thread forked.
fn warm_up(script: &Path) -> std::process::ExitStatus {
    let mut attempts = 0;
    loop {
        match std::process::Command::new(script).status() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempts < 50 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result.expect("run dispatcher once"),
        }
    }
}
