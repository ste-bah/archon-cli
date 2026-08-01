//! TC-ARCH-05: Agent event emission must await bounded capacity.

use std::process::Command;

#[test]
fn agent_event_send_is_awaited() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let events = std::fs::read_to_string(repo_root.join("crates/archon-core/src/agent/events.rs"))
        .expect("read Agent events source");

    assert!(
        events.contains("self.event_tx.send(timestamped).await"),
        "Agent event send must await bounded channel capacity"
    );
}

#[test]
fn arch_lint_passes() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(bash_program())
        .arg(bash_path(&repo_root.join("scripts/lint/arch-lint.sh")))
        .current_dir(repo_root)
        .output()
        .expect("execute arch-lint.sh");

    assert!(
        output.status.success(),
        "arch-lint.sh failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Locate a bash that actually exists.
///
/// Git for Windows ships `bash.exe` but its installer only adds `<git>\cmd` to
/// PATH -- the directory with `git.exe` and not `bash.exe` -- so a bare
/// `Command::new("bash")` fails with "program not found" on an otherwise
/// correctly set up machine.
fn bash_program() -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::path::PathBuf;
        // Git's bash FIRST, deliberately. A bare `bash` on Windows usually
        // resolves to the WSL launcher in System32, which runs inside
        // the Linux filesystem and cannot see `F:/...` at all -- it reports
        // "No such file or directory" for a perfectly valid Windows path.
        if let Ok(output) = Command::new("where").arg("git").output()
            && let Some(git) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(PathBuf::from)
        {
            let candidate = git
                .parent()
                .and_then(|cmd_dir| cmd_dir.parent())
                .map(|root| root.join("bin").join("bash.exe"));
            if let Some(path) = candidate
                && path.is_file()
            {
                return path.into_os_string();
            }
        }
        "bash".into()
    }
    #[cfg(not(windows))]
    {
        "bash".into()
    }
}

/// A path bash will accept.
///
/// Git's bash consumes backslashes as escapes, so a native Windows path arrives
/// as `F:archon-localarchon-cli...` and the script is "not found". Forward
/// slashes survive intact and Windows accepts them too.
fn bash_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}
