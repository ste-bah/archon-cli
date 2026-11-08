use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn fixture_name() -> String {
    format!("{}::fixture", module_path!().split_once("::").unwrap().1)
}

fn fixture_command(mode: &str, root: &Path) -> Command {
    let mut command = archon_shell::spawn::command(std::env::current_exe().unwrap());
    command
        .args(["--exact", &fixture_name()])
        .env("BASH_OWNER_EXIT_FIXTURE", mode)
        .env("BASH_OWNER_EXIT_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_for(mut ready: impl FnMut() -> bool, message: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "{message}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// The fake owner must exit without waiting for its wrapper.
#[allow(clippy::zombie_processes)]
#[test]
fn fixture() {
    let Ok(mode) = std::env::var("BASH_OWNER_EXIT_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("BASH_OWNER_EXIT_ROOT").unwrap());
    if mode == "detached" || mode == "orphan" {
        if mode == "detached" {
            // SAFETY: setsid operates only on this fixture process.
            assert!(unsafe { libc::setsid() } > 0);
        }
        std::fs::write(root.join("descendant.pid"), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(120));
        panic!("descendant fixture outlived cleanup");
    }
    assert!(mode == "owner" || mode == "orphan-owner");
    let exe = std::env::current_exe().unwrap();
    let descendant_mode = if mode == "owner" {
        "detached"
    } else {
        "orphan"
    };
    let launch = format!(
        "BASH_OWNER_EXIT_FIXTURE={descendant_mode} '{}' --exact {} &",
        exe.display().to_string().replace('\'', "'\"'\"'"),
        fixture_name(),
    );
    let probe = if mode == "owner" {
        format!("{launch} wait")
    } else {
        format!("({launch}); sleep 120")
    };
    let bash = std::env::var_os("BASH_OWNER_EXIT_SHELL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| super::BASH_PROGRAM.to_path_buf());
    let mut command = archon_shell::execution_environment::command(&bash).0;
    command
        .args([
            "--noprofile",
            "--norc",
            "-c",
            super::UNIX_DESCENDANT_CLEANUP,
        ])
        .arg("owner-exit-guard")
        .arg(&bash)
        .arg(probe)
        .env("BASH_OWNER_EXIT_ROOT", &root)
        // Bash 3.2 must ignore an inherited BASHPID when identifying the watcher.
        .env("BASHPID", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and changes only the child session.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    std::fs::write(root.join("wrapper.pid"), child.id().to_string()).unwrap();
    wait_for(
        || {
            let Some(pid) = std::fs::read_to_string(root.join("descendant.pid"))
                .ok()
                .and_then(|pid| pid.parse::<u32>().ok())
            else {
                return false;
            };
            if mode == "owner" {
                return pid > 0;
            }
            // Ensure the double-fork child has left the wrapper's tree while
            // remaining in its process group before the fake owner exits.
            archon_shell::process_tree::snapshot().is_ok_and(|table| {
                table.processes.iter().any(|process| {
                    process.pid == pid && process.ppid == 1 && process.pgid == child.id()
                })
            })
        },
        "descendant never became ready",
    );
    // Returning exits the fake owner's test process without killing the wrapper.
}

struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        for name in ["wrapper.pid", "descendant.pid"] {
            if let Ok(pid) = std::fs::read_to_string(self.0.join(name))
                && let Ok(pid) = pid.parse::<i32>()
                && pid > 0
            {
                // SAFETY: these ids belong to the fixture's processes.
                unsafe { libc::kill(pid, libc::SIGKILL) };
                if name == "wrapper.pid" {
                    unsafe { libc::kill(-pid, libc::SIGKILL) };
                }
            }
        }
    }
}

#[test]
fn owner_exit_kills_setsid_descendant() {
    check_owner_exit(super::BASH_PROGRAM.as_path(), "owner");
    #[cfg(target_os = "macos")]
    check_owner_exit(Path::new("/bin/bash"), "owner");
}

#[test]
fn owner_exit_kills_orphaned_group_member() {
    check_owner_exit(super::BASH_PROGRAM.as_path(), "orphan-owner");
    #[cfg(target_os = "macos")]
    check_owner_exit(Path::new("/bin/bash"), "orphan-owner");
}

fn check_owner_exit(bash: &Path, mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let _cleanup = Cleanup(root.path());
    let mut owner = fixture_command(mode, root.path())
        .env("BASH_OWNER_EXIT_SHELL", bash)
        .spawn()
        .unwrap();
    wait_for(
        || owner.try_wait().unwrap().is_some(),
        "fake owner did not exit",
    );
    assert!(owner.wait().unwrap().success(), "fake owner fixture failed");
    let pid = std::fs::read_to_string(root.path().join("descendant.pid"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    wait_for(
        || {
            // Zombies have exited even if the new parent has not reaped them.
            archon_shell::process_tree::identity_of(pid).is_ok_and(|identity| identity.is_none())
        },
        "descendant survived fake owner exit",
    );
}
