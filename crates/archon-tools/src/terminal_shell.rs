//! Which shell a terminal runs, and how it is launched (#189 Phase 6).
//!
//! Windows is a first-class case here rather than an afterthought: both
//! PowerShell and a POSIX shell have to be reachable on the primary
//! development platform, so the choice is a named argument and not an
//! `#[cfg]`.

use std::path::Path;

use archon_pty::CommandBuilder;

/// The shells a terminal may run, by the name the model asks for.
pub(crate) const SHELLS: &[&str] = &["bash", "sh", "powershell", "cmd"];

/// What is launched when the caller does not say.
///
/// Windows follows the platform rather than the POSIX habit: an agent asking
/// for "a terminal" on Windows means the shell the user has, and a silent
/// Git-bash would make every path and every command it writes wrong.
pub(crate) fn default_shell() -> &'static str {
    if cfg!(windows) { "powershell" } else { "bash" }
}

/// Resolve `shell` to a launchable command rooted at `cwd`.
pub(crate) fn build(shell: &str, cwd: &Path) -> Result<CommandBuilder, String> {
    let mut command = match shell {
        "bash" => CommandBuilder::new(archon_shell::resolve_bash()),
        "sh" => CommandBuilder::new(archon_shell::resolve_posix_shell()),
        "cmd" => windows_only(shell, || CommandBuilder::new("cmd.exe"))?,
        "powershell" => powershell()?,
        other => {
            return Err(format!(
                "unknown shell {other:?}; expected one of {}",
                SHELLS.join(", ")
            ));
        }
    };
    command.cwd(cwd);
    // Claimed rather than discovered: this side renders nothing, but a program
    // that finds `TERM` unset falls back to line-at-a-time behaviour or refuses
    // to run at all, and either is worse than escape sequences that get
    // stripped on the way out anyway.
    command.env("TERM", "xterm-256color");
    Ok(command)
}

fn powershell() -> Result<CommandBuilder, String> {
    // `pwsh` first: on a machine that has both, it is the one the user chose to
    // install. `-NoLogo` because the banner is a screenful of nothing.
    let program = which::which("pwsh")
        .or_else(|_| which::which("powershell"))
        .map_err(|_| "no PowerShell on PATH (looked for pwsh, then powershell)".to_string())?;
    let mut command = CommandBuilder::new(program);
    command.args(["-NoLogo"]);
    Ok(command)
}

fn windows_only(
    shell: &str,
    build: impl FnOnce() -> CommandBuilder,
) -> Result<CommandBuilder, String> {
    if cfg!(windows) {
        Ok(build())
    } else {
        Err(format!("the {shell} shell exists only on Windows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_shell_matches_the_platform() {
        if cfg!(windows) {
            assert_eq!(default_shell(), "powershell");
        } else {
            assert_eq!(default_shell(), "bash");
        }
        assert!(SHELLS.contains(&default_shell()));
    }

    #[test]
    fn an_unknown_shell_is_refused_by_name() {
        let error = build("fish", Path::new(".")).expect_err("fish is not offered");

        assert!(error.contains("fish"), "{error}");
        assert!(
            error.contains("bash"),
            "the message must list what is: {error}"
        );
    }

    /// A POSIX shell resolves on every platform — on Windows through the
    /// Git-for-Windows tree `archon-shell` already knows how to find.
    #[test]
    fn a_posix_shell_resolves_everywhere() {
        assert!(build("sh", Path::new(".")).is_ok());
    }

    #[test]
    fn cmd_is_refused_off_windows() {
        let built = build("cmd", Path::new("."));
        assert_eq!(built.is_ok(), cfg!(windows));
    }
}
