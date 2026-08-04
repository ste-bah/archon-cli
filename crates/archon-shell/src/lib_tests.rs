use super::{ShellCandidates, ShellInvocation, select};
use std::path::PathBuf;

fn path(value: &str) -> Option<PathBuf> {
    Some(PathBuf::from(value))
}

fn select_shell(is_windows: bool, candidates: ShellCandidates) -> ShellInvocation {
    select(is_windows, candidates).invocation
}

/// Carried over from `archon-core`'s `hooks::shell` when the two resolvers
/// merged: a discovered POSIX `sh` must beat the `cmd` fallback on Windows.
#[test]
fn windows_prefers_discovered_posix_shell() {
    let shell = select_shell(
        true,
        ShellCandidates {
            path_sh: path(r"C:\Program Files\Git\bin\sh.exe"),
            path_bash: path(r"C:\Program Files\Git\bin\bash.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from(r"C:\Program Files\Git\bin\sh.exe"),
            command_arg: "-c",
        }
    );
}

#[test]
fn windows_uses_bash_when_sh_is_unavailable() {
    let shell = select_shell(
        true,
        ShellCandidates {
            path_bash: path(r"C:\Program Files\Git\bin\bash.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            command_arg: "-c",
        }
    );
}

/// The defect this crate exists to fix: Git for Windows adds only `<git>\cmd`
/// to PATH, so `which` sees no shell while `<git>\usr\bin\sh.exe` is right
/// there. Before the merge this machine ran hooks under `cmd /C`.
#[test]
fn windows_uses_git_relative_sh_when_path_has_no_shell() {
    let shell = select_shell(
        true,
        ShellCandidates {
            git_sh: path(r"C:\Program Files\Git\usr\bin\sh.exe"),
            git_bash: path(r"C:\Program Files\Git\bin\bash.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from(r"C:\Program Files\Git\usr\bin\sh.exe"),
            command_arg: "-c",
        }
    );
}

/// PATH is what the user configured, so it outranks a Git-relative guess even
/// when both resolve.
#[test]
fn path_sh_outranks_git_relative_sh() {
    let shell = select_shell(
        true,
        ShellCandidates {
            path_sh: path(r"C:\msys64\usr\bin\sh.exe"),
            git_sh: path(r"C:\Program Files\Git\bin\sh.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(shell.program, PathBuf::from(r"C:\msys64\usr\bin\sh.exe"));
}

/// A Git-relative `sh` is still a POSIX `sh`, so it outranks any `bash`.
#[test]
fn git_relative_sh_outranks_path_bash() {
    let shell = select_shell(
        true,
        ShellCandidates {
            path_bash: path(r"C:\msys64\usr\bin\bash.exe"),
            git_sh: path(r"C:\Program Files\Git\bin\sh.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(
        shell.program,
        PathBuf::from(r"C:\Program Files\Git\bin\sh.exe")
    );
}

#[test]
fn git_relative_bash_is_the_last_posix_resort() {
    let shell = select_shell(
        true,
        ShellCandidates {
            git_bash: path(r"C:\Program Files\Git\bin\bash.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"),
            command_arg: "-c",
        }
    );
}

#[test]
fn plain_windows_falls_back_to_cmd() {
    let shell = select_shell(true, ShellCandidates::default());

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from("cmd"),
            command_arg: "/C",
        }
    );
}

#[test]
fn unix_falls_back_to_sh() {
    let shell = select_shell(false, ShellCandidates::default());

    assert_eq!(
        shell,
        ShellInvocation {
            program: PathBuf::from("sh"),
            command_arg: "-c",
        }
    );
}

/// `cmd` cannot honour `-lc` or a POSIX script on stdin, so the POSIX view must
/// hand back a bare `sh` that fails at spawn rather than an interpreter that
/// would run the script wrong.
#[test]
fn posix_view_never_yields_cmd_on_a_shell_less_windows_host() {
    let shells = select(true, ShellCandidates::default());

    assert_eq!(shells.invocation.program, PathBuf::from("cmd"));
    assert_eq!(shells.posix, PathBuf::from("sh"));
}

/// Both views must name the same shell whenever one exists — the disagreement
/// between the two old resolvers is exactly what this crate removes.
#[test]
fn both_views_agree_whenever_a_posix_shell_exists() {
    let shells = select(
        true,
        ShellCandidates {
            git_sh: path(r"C:\Program Files\Git\bin\sh.exe"),
            ..ShellCandidates::default()
        },
    );

    assert_eq!(shells.invocation.program, shells.posix);
    assert_eq!(shells.invocation.command_arg, "-c");
}
