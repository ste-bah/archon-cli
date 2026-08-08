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
        // Git's bash FIRST, deliberately. A bare `bash` on Windows usually
        // resolves to the WSL launcher in System32, which runs inside
        // the Linux filesystem and cannot see `F:/...` at all -- it reports
        // "No such file or directory" for a perfectly valid Windows path.
        //
        // `where git` returns a DIFFERENT layout depending on the calling
        // shell: `<root>\cmd\git.exe` from PowerShell/cmd, but
        // `<root>\mingw64\bin\git.exe` first from Git Bash. Deriving the
        // install root by stripping exactly two components only works for
        // the former; from Git Bash it produced
        // `<root>\mingw64\bin\bash.exe`, which does not exist, silently fell
        // back to the WSL launcher, and failed the suite. Walk every
        // ancestor of every candidate instead and take the first bash.exe
        // that actually exists.
        if let Ok(output) = Command::new("where").arg("git").output() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let git = std::path::Path::new(line.trim());
                if git.as_os_str().is_empty() {
                    continue;
                }
                if let Some(bash) = bash_near(git) {
                    return bash.into_os_string();
                }
            }
        }
        "bash".into()
    }
    #[cfg(not(windows))]
    {
        "bash".into()
    }
}

/// First `bash.exe` found in any ancestor of `git`, checking both layouts Git
/// for Windows ships (`<root>\bin` and `<root>\usr\bin`).
#[cfg(windows)]
fn bash_near(git: &std::path::Path) -> Option<std::path::PathBuf> {
    const RELATIVE: [&[&str]; 2] = [&["bin", "bash.exe"], &["usr", "bin", "bash.exe"]];
    for root in git.ancestors() {
        for parts in RELATIVE {
            let mut candidate = root.to_path_buf();
            for part in parts {
                candidate.push(part);
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A path bash will accept.
///
/// Git's bash consumes backslashes as escapes, so a native Windows path arrives
/// as `F:archon-localarchon-cli...` and the script is "not found". Forward
/// slashes survive intact and Windows accepts them too.
fn bash_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}
