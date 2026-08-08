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

// ---------------------------------------------------------------------------
// #118: locating Git's shell regardless of where git.exe sits
// ---------------------------------------------------------------------------

/// Build a fake Git for Windows tree with shells under `<root>\bin` and
/// `<root>\usr\bin`, plus a `git.exe` at each depth PATH may report.
fn fake_git_install() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let root = dir.path();
    for sub in [
        root.join("bin"),
        root.join("usr").join("bin"),
        root.join("cmd"),
        root.join("mingw64").join("bin"),
    ] {
        std::fs::create_dir_all(&sub).expect("create dir");
    }
    std::fs::write(root.join("bin").join("sh.exe"), b"").expect("write sh");
    std::fs::write(root.join("usr").join("bin").join("bash.exe"), b"").expect("write bash");
    std::fs::write(root.join("cmd").join("git.exe"), b"").expect("write git");
    std::fs::write(root.join("mingw64").join("bin").join("git.exe"), b"").expect("write git");
    dir
}

/// `<root>\cmd\git.exe` — what PATH resolves to from PowerShell or cmd.
#[test]
fn finds_shell_from_cmd_layout() {
    let dir = fake_git_install();
    let git = dir.path().join("cmd").join("git.exe");
    assert_eq!(
        super::shell_near_git(&git, "sh.exe"),
        Some(dir.path().join("bin").join("sh.exe"))
    );
}

/// `<root>\mingw64\bin\git.exe` — what PATH resolves to inside Git Bash. The
/// old fixed-grandparent lookup derived `<root>\mingw64` here, found nothing,
/// and silently fell through to the WSL launcher.
#[test]
fn finds_shell_from_mingw64_layout() {
    let dir = fake_git_install();
    let git = dir.path().join("mingw64").join("bin").join("git.exe");
    assert_eq!(
        super::shell_near_git(&git, "sh.exe"),
        Some(dir.path().join("bin").join("sh.exe"))
    );
}

/// The `usr\bin` layout must resolve from either depth too.
#[test]
fn finds_usr_bin_shell_from_both_layouts() {
    let dir = fake_git_install();
    let expected = dir.path().join("usr").join("bin").join("bash.exe");
    for git in [
        dir.path().join("cmd").join("git.exe"),
        dir.path().join("mingw64").join("bin").join("git.exe"),
    ] {
        assert_eq!(
            super::shell_near_git(&git, "bash.exe"),
            Some(expected.clone())
        );
    }
}

/// No shell anywhere above `git.exe` must stay `None` rather than inventing a
/// path — the caller drops the candidate and selection continues.
#[test]
fn absent_shell_is_none() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    std::fs::create_dir_all(dir.path().join("cmd")).expect("create dir");
    let git = dir.path().join("cmd").join("git.exe");
    std::fs::write(&git, b"").expect("write git");
    assert_eq!(super::shell_near_git(&git, "sh.exe"), None);
}

/// The Windows build must never select the WSL launcher: it runs in a separate
/// filesystem namespace, so it does not fail on a Windows path — it silently
/// runs against paths it cannot see (#118).
///
/// Windows-only because the assertions are about Windows path *parsing*: on
/// Unix a backslash is an ordinary character, so `Path::new(r"C:\...\bash.exe")`
/// has no parent and its whole string is the file name. The production code is
/// gated on `is_windows` and so is unaffected — only the test needs the gate.
#[cfg(windows)]
#[test]
fn wsl_launcher_is_recognised() {
    for path in [
        r"C:\Windows\System32\bash.exe",
        r"C:\Windows\system32\BASH.EXE",
        r"C:\Windows\Sysnative\bash.exe",
        r"C:\Program Files\WindowsApps\bash.exe",
    ] {
        assert!(
            super::is_wsl_launcher(std::path::Path::new(path)),
            "{path} should be rejected as the WSL launcher"
        );
    }
}

/// Windows-only for the same path-parsing reason as above: on Unix these
/// backslash paths would pass for the wrong reason, which is worse than not
/// running.
#[cfg(windows)]
#[test]
fn real_shells_are_not_mistaken_for_wsl() {
    for path in [
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        r"C:\Windows\System32\cmd.exe",
        "/bin/bash",
        "/usr/bin/sh",
    ] {
        assert!(
            !super::is_wsl_launcher(std::path::Path::new(path)),
            "{path} must remain usable"
        );
    }
}
