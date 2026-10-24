//! Whether any process other than the observing one holds a path.
use super::*;

/// lsof includes cwd references and open files, including detached descendants.
/// No diagnostics and no listed process means no holder; any scan failure is unsafe.
/// The observing process's own handles (its site root, `SiteRoot::capture`)
/// are not a holder: only another process is.
/// The guardian runs this under the policy's toolchain PATH, which need not
/// name the directory `lsof` is in, so the shared lookup finds it.
#[cfg(unix)]
pub(super) fn target_idle(target: &Path) -> bool {
    lsof_says_idle(archon_shell::process_tree::lsof_program(), target)
}

/// [`target_idle`] with the `lsof` lookup's result given. No `lsof`, or one
/// that cannot run, proves nothing: the target counts as busy.
#[cfg(unix)]
pub(super) fn lsof_says_idle(program: std::io::Result<PathBuf>, target: &Path) -> bool {
    let program = match program {
        Ok(program) => program,
        Err(error) => {
            eprintln!(
                "acceptance cache idle check has no lsof: {error}; {} counts as busy",
                target.display()
            );
            return false;
        }
    };
    match archon_shell::spawn::command(&program)
        .args(["-nP", "-t", "+D"])
        .arg(target)
        .output()
    {
        Ok(out) => {
            let own = std::process::id().to_string();
            let listed = String::from_utf8_lossy(&out.stdout);
            let mut pids = listed.lines().map(str::trim).filter(|pid| !pid.is_empty());
            let others = pids.any(|pid| pid != own);
            // `+D` exits 1 when not every file under the path is open, even
            // when it lists some; any other status is a scan failure.
            out.stderr.is_empty() && !others && matches!(out.status.code(), Some(0 | 1))
        }
        Err(error) => {
            eprintln!(
                "acceptance cache idle check could not run {}: {error}; {} counts as busy",
                program.display(),
                target.display()
            );
            false
        }
    }
}

/// Off Unix there is no holder probe, so no target is proved idle (and a
/// shared cache is refused before any slot is checked; see `try_lock`).
#[cfg(not(unix))]
pub(super) fn target_idle(_target: &Path) -> bool {
    false
}
