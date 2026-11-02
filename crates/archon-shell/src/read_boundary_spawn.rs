//! Preserve launch errors when macOS execs a target through sandbox-exec.
use std::process::Command;

/// Check after the caller has set the final environment and working directory.
/// A missing wrapper target must remain a spawn error, not a command exit code.
/// This is a diagnostic preflight; the kernel boundary still judges the exec.
pub fn prepare_spawn(command: &Command) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if command.get_program() == crate::write_boundary::SANDBOX_EXEC {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};
        let mut args = command.get_args();
        if args.next() != Some(OsStr::new("-p")) || args.next().is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "sandbox profile missing before executable preflight",
            ));
        }
        let target = args
            .next()
            .filter(|arg| *arg != "--")
            .or_else(|| args.next())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "sandbox target missing")
            })?;
        let cwd =
            std::env::current_dir()?.join(command.get_current_dir().unwrap_or(Path::new(".")));
        // Boundary callers install a complete environment. When PATH is absent,
        // execvp uses the system default, rather than the operator's PATH.
        let path = command
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .unwrap_or(OsStr::new("/usr/bin:/bin"));
        let candidates: Vec<PathBuf> = if target.as_bytes().contains(&b'/') {
            vec![cwd.join(target)]
        } else {
            std::env::split_paths(path)
                .map(|dir| cwd.join(dir).join(target))
                .collect()
        };
        let mut code = libc::ENOENT;
        for candidate in candidates {
            let name = std::ffi::CString::new(candidate.as_os_str().as_bytes()).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "executable path contains NUL",
                )
            })?;
            // SAFETY: name is a live NUL-terminated path; access retains no pointer.
            if unsafe { libc::access(name.as_ptr(), libc::X_OK) } == 0 {
                if candidate.is_file() {
                    return Ok(());
                }
                code = libc::EACCES;
            } else if std::io::Error::last_os_error().raw_os_error() == Some(libc::EACCES) {
                code = libc::EACCES;
            }
        }
        return Err(std::io::Error::from_raw_os_error(code));
    }
    let _ = command;
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn preflight_uses_final_child_path_and_working_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        let program = dir.path().join("bin/probe");
        std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
        command.args(["-p", "(version 1)(allow default)", "--", "probe"]);
        command.current_dir(dir.path());
        crate::spawn::replace_environment(&mut command, [("PATH", "bin")]);
        prepare_spawn(&command).unwrap();
        command.env("PATH", "missing");
        assert_eq!(
            prepare_spawn(&command).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        command.env("PATH", "bin");
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            prepare_spawn(&command).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
