//! Locating a POSIX shell, including on Windows.
//!
//! Several subsystems run user-supplied or generated commands through `sh -c`:
//! workflow verification, worktree branch operations, and the slash streaming
//! path. Every one of them hardcoded `Command::new("sh")`, which fails on
//! Windows with `NotFound: program not found` — so those features were
//! unavailable there, and the tests covering them failed for the same reason.
//!
//! Windows does have a POSIX shell whenever Git is installed, which archon
//! already requires. The catch is narrower than "no shell": Git for Windows
//! ships `<git>\bin\sh.exe` and `<git>\usr\bin\sh.exe`, but its installer only
//! adds `<git>\cmd` to PATH — the directory holding `git.exe` and not
//! `sh.exe`. So a correctly installed Git still leaves `sh` unresolvable.

use std::ffi::OsString;

/// The program to invoke for `sh -c <command>`.
///
/// Plain `sh` wherever it resolves. On Windows, falls back to the `sh.exe` that
/// sits beside the `git.exe` already on PATH, and finally to bare `sh` so a
/// machine with no shell at all still produces the familiar launch error rather
/// than a confusing one.
pub(crate) fn posix_shell() -> OsString {
    #[cfg(windows)]
    {
        use std::path::PathBuf;
        use std::process::Command;

        if Command::new("sh").arg("-c").arg("exit 0").output().is_ok() {
            return "sh".into();
        }
        let Ok(output) = Command::new("where").arg("git").output() else {
            return "sh".into();
        };
        let Some(git) = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(PathBuf::from)
        else {
            return "sh".into();
        };
        // <git>\cmd\git.exe -> <git>\bin\sh.exe
        let candidate = git
            .parent()
            .and_then(|cmd_dir| cmd_dir.parent())
            .map(|root| root.join("bin").join("sh.exe"));
        match candidate {
            Some(path) if path.is_file() => path.into_os_string(),
            _ => "sh".into(),
        }
    }
    #[cfg(not(windows))]
    {
        "sh".into()
    }
}
