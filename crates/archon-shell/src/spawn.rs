//! The one way this process builds a child process (Issue 340).
//!
//! Every child inherits only its stdin, stdout and stderr. A descriptor is
//! inherited across `exec` unless it is close-on-exec, and where std has no
//! `pipe2` (macOS) it creates every pipe with `pipe()` and only then sets
//! `FD_CLOEXEC`. std's `posix_spawn` path does not set
//! `POSIX_SPAWN_CLOEXEC_DEFAULT` either. So a child that any thread starts in
//! that window keeps another thread's new pipe for good, and the reader of
//! that pipe never sees end of file while the child lives. Issue 334 closed
//! this for supervised host commands only; any other spawner (a git call, the
//! Bash tool, a detached daemon) could still hold a host command's output
//! open and make its clean teardown read as a stall.
//!
//! The rule is therefore process-wide and on the inheriting side: every
//! command built here marks each descriptor above stdio close-on-exec in its
//! `pre_exec` hook ([`crate::process_tree::inherit_only_stdio`]), whatever
//! thread created the descriptor and when. A `pre_exec` hook makes std fork
//! and exec instead of `posix_spawn` on macOS; the sweep runs in the child.
//!
//! A site that must pass a particular descriptor to its child adds its own
//! `pre_exec` hook that `dup2`s it into place: hooks run in the order they
//! are added, so that one runs after this sweep and its descriptor survives.
//! No site in this workspace does so today.
//!
//! The workspace lint (`spawn_lint_tests.rs`) fails when non-test code calls
//! `Command::new` outside this file.

use std::ffi::OsStr;
use std::process::Command;

/// A `std` command for `program` whose child inherits only its stdio.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    stdio_only(&mut command);
    command
}

/// A `tokio` command for `program` whose child inherits only its stdio.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    tokio::process::Command::from(command(program))
}

/// Applies the rule to a command that a library built (for example the
/// commands `open::commands` returns), so its child inherits only its stdio.
///
/// The descriptor ceiling is read here, before the fork, because nothing
/// that reads it is async-signal safe. If it cannot be read, the spawn fails
/// with that error rather than run a child that may inherit anything.
pub fn stdio_only(command: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let ceiling = crate::process_tree::descriptor_ceiling()
            .map_err(|error| error.raw_os_error().unwrap_or(libc::EINVAL));
        // SAFETY: the hook calls only fcntl (and close_range on Linux), which
        // are async-signal safe, and builds an io::Error from a plain code
        // without allocating.
        unsafe {
            command.pre_exec(move || match ceiling {
                Ok(ceiling) => crate::process_tree::inherit_only_stdio(ceiling),
                Err(code) => Err(std::io::Error::from_raw_os_error(code)),
            });
        }
    }
    command
}

/// Runs the first of `commands` (a library's launchers, such as
/// `open::commands`) that starts, with no stdio and inheriting nothing else,
/// and reports its exit: the contract of `open::that`, under this rule.
pub fn run_first_launcher(commands: Vec<Command>) -> std::io::Result<()> {
    let mut last_error = None;
    for mut launcher in commands {
        let status = stdio_only(&mut launcher)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                return Err(std::io::Error::other(format!(
                    "launcher {launcher:?} failed with {status}"
                )));
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("no launcher to run")))
}

#[cfg(all(test, unix))]
#[path = "spawn_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_lint_tests.rs"]
mod lint_tests;
