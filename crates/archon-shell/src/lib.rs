//! Locating a shell to run generated commands through, including on Windows.
//!
//! Several subsystems run user-supplied or generated commands through a shell:
//! hook execution, workflow verification, worktree branch operations, and the
//! slash streaming path. They used to answer "which shell?" twice, differently,
//! and one of the two answers was wrong on Windows:
//!
//! * `archon-core`'s hook executor asked `which::which("sh")`. Git for Windows
//!   ships `<git>\bin\sh.exe` and `<git>\usr\bin\sh.exe`, but its installer only
//!   adds `<git>\cmd` to PATH — the directory holding `git.exe` and not
//!   `sh.exe`. So `which` found nothing and hooks silently dropped to `cmd /C`
//!   on a machine that does have a POSIX shell, while workflow verification on
//!   that same machine ran under `sh`.
//! * The bin crate's `posix_shell` knew about the Git layout but returned only a
//!   program name, so it could not have expressed `cmd`'s `/C` even if it had
//!   wanted the fallback.
//!
//! This crate is the single answer, with one discovery pass and one precedence
//! order behind two views of the result, because the callers genuinely differ:
//!
//! * [`resolve_shell`] — "run this one command string", and `cmd /C` is an
//!   acceptable last resort. Hook execution.
//! * [`resolve_posix_shell`] — the caller supplies POSIX-only invocations
//!   (`-lc`, or a script piped to stdin) that `cmd` cannot honour, so it must
//!   never be handed `cmd`. Workflow verification and the slash path.
//!
//! It is deliberately a leaf with no `archon-*` dependencies: `archon-core`
//! depends on `archon-topology`, which depends on `archon-workflow`, so anything
//! both `archon-workflow` and `archon-core` need must sit below all three.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The program to launch and the flag that introduces the command string.
///
/// Callers must use both fields. `command_arg` is not always `-c`: on a Windows
/// machine with no POSIX shell at all, `program` is `cmd` and the flag is `/C`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellInvocation {
    pub program: PathBuf,
    pub command_arg: &'static str,
}

/// A shell for running one command string, falling back to `cmd /C` on a
/// Windows host with no POSIX shell anywhere.
///
/// Resolution touches the filesystem and reads PATH, and the workflow
/// verification path invokes a shell per command, so the answer is cached for
/// the life of the process. A shell appearing or vanishing mid-run is not a case
/// worth re-probing for.
pub fn resolve_shell() -> &'static ShellInvocation {
    &resolved().invocation
}

/// The POSIX shell for this process, never `cmd`.
///
/// For callers whose command strings are POSIX by construction — `sh -lc`, or a
/// generated script piped to stdin — where `cmd` would not fail cleanly but
/// would misinterpret the script. When no POSIX shell was found this is a bare
/// `sh`, which fails closed at spawn with the familiar "program not found"
/// rather than running the caller's script under the wrong interpreter.
pub fn resolve_posix_shell() -> &'static Path {
    &resolved().posix
}

/// Every shell this machine offers, gathered before any choice is made.
///
/// Discovery is split from selection so `best_posix_shell` stays pure and its
/// precedence is unit-testable on any platform, including the Windows-specific
/// branches from a Linux CI runner.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ShellCandidates {
    /// `sh` as PATH resolves it.
    path_sh: Option<PathBuf>,
    /// `bash` as PATH resolves it.
    path_bash: Option<PathBuf>,
    /// `sh.exe` found relative to the `git.exe` on PATH. Windows only.
    git_sh: Option<PathBuf>,
    /// `bash.exe` found relative to the `git.exe` on PATH. Windows only.
    git_bash: Option<PathBuf>,
}

struct ResolvedShells {
    invocation: ShellInvocation,
    posix: PathBuf,
}

fn resolved() -> &'static ResolvedShells {
    static SHELLS: LazyLock<ResolvedShells> = LazyLock::new(|| {
        let is_windows = cfg!(windows);
        select(is_windows, discover_candidates(is_windows))
    });
    &SHELLS
}

fn select(is_windows: bool, candidates: ShellCandidates) -> ResolvedShells {
    let posix = best_posix_shell(candidates);
    let invocation = match (&posix, is_windows) {
        (Some(program), _) => ShellInvocation {
            program: program.clone(),
            command_arg: "-c",
        },
        (None, true) => ShellInvocation {
            program: PathBuf::from("cmd"),
            command_arg: "/C",
        },
        (None, false) => ShellInvocation {
            program: PathBuf::from("sh"),
            command_arg: "-c",
        },
    };
    ResolvedShells {
        invocation,
        posix: posix.unwrap_or_else(|| PathBuf::from("sh")),
    }
}

/// Precedence: `sh` beats `bash`, and within each shell a PATH hit beats a
/// Git-relative one because PATH is what the user configured.
fn best_posix_shell(candidates: ShellCandidates) -> Option<PathBuf> {
    candidates
        .path_sh
        .or(candidates.git_sh)
        .or(candidates.path_bash)
        .or(candidates.git_bash)
}

fn discover_candidates(is_windows: bool) -> ShellCandidates {
    let (git_sh, git_bash) = if is_windows {
        (git_relative_shell("sh.exe"), git_relative_shell("bash.exe"))
    } else {
        (None, None)
    };
    ShellCandidates {
        path_sh: which::which("sh").ok(),
        path_bash: which::which("bash").ok(),
        git_sh,
        git_bash,
    }
}

/// Finds a shell beside the `git.exe` already on PATH.
///
/// Git for Windows puts `git.exe` in `<git>\cmd` (or `<git>\bin`) and its shells
/// in `<git>\bin` and `<git>\usr\bin`, so the grandparent of the resolved
/// `git.exe` is the install root either way.
fn git_relative_shell(exe: &str) -> Option<PathBuf> {
    let git = which::which("git").ok()?;
    let root = git.parent()?.parent()?;
    [
        root.join("bin").join(exe),
        root.join("usr").join("bin").join(exe),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
