//! TC-ARCH-02 (REQ-FOR-D1): grep lint for .await on agent work in input handler.
//!
//! Runs `scripts/lint/arch-lint.sh` and asserts exit 0 on the clean tree.
//! This validates Rule 1: no `.process_message().await` at handler scope
//! inside the INPUT_HANDLER region of src/main.rs.

use std::process::Command;

#[test]
fn arch_lint_passes_on_clean_tree() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lint_script = repo_root.join("scripts/lint/arch-lint.sh");

    assert!(
        lint_script.exists(),
        "TC-ARCH-02: scripts/lint/arch-lint.sh not found at {lint_script:?}"
    );

    let output = Command::new(bash_program())
        .arg(bash_path(&lint_script))
        .current_dir(repo_root)
        .output()
        .expect("failed to execute arch-lint.sh");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "TC-ARCH-02: arch-lint.sh exited with non-zero on clean tree.\n\
         stdout: {stdout}\n\
         stderr: {stderr}"
    );
}

#[ignore = "TDD test for AGS-106/107 INPUT_HANDLER markers in src/main.rs; tracked under #224 (CI cross-platform parity with P1.1 canary skip list)"]
#[test]
fn input_handler_markers_exist() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main_rs =
        std::fs::read_to_string(repo_root.join("src/main.rs")).expect("failed to read src/main.rs");

    assert!(
        main_rs.contains("BEGIN INPUT_HANDLER"),
        "TC-ARCH-02: src/main.rs missing BEGIN INPUT_HANDLER marker"
    );
    assert!(
        main_rs.contains("END INPUT_HANDLER"),
        "TC-ARCH-02: src/main.rs missing END INPUT_HANDLER marker"
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
