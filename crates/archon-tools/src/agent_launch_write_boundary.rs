use super::*;

/// Absolute directory values of the host-selected environment variables.
pub(super) fn env_dirs(env: &[(String, String)], extra: &[String]) -> Vec<PathBuf> {
    let mut keys: Vec<&str> = extra.iter().map(String::as_str).collect();
    for key in HOST_DIR_ENV_KEYS
        .iter()
        .copied()
        .chain(crate::build_cache_env::toolchain_cache_env_keys())
    {
        keys.push(key);
    }
    let mut dirs = Vec::new();
    for key in keys {
        if let Some((_, value)) = env.iter().rev().find(|(name, _)| name == key) {
            let path = PathBuf::from(value.trim());
            if path.is_absolute() {
                push_unique(&mut dirs, path);
            }
        }
    }
    dirs
}

/// Build any tool command under the same boundary as Bash.
pub(crate) fn command(
    program: impl AsRef<std::ffi::OsStr>,
    boundary: Option<&ShellBoundary>,
) -> (
    tokio::process::Command,
    archon_shell::read_boundary::LaunchReport,
) {
    match boundary.map(|boundary| &boundary.applied) {
        Some(Applied::SandboxExec(profile)) => {
            let mut command =
                archon_shell::spawn::tokio_command(archon_shell::write_boundary::SANDBOX_EXEC);
            command.arg("-p").arg(profile).arg("--").arg(program);
            tracing::info!(writes = "kernel-boundary", "agent tool launch");
            (command, "named-paths-denied".into())
        }
        #[cfg(target_os = "linux")]
        Some(Applied::Landlock(sandbox)) => {
            let (mut command, report) =
                archon_shell::execution_environment::tokio_command_with_report(program);
            sandbox.install_std(command.as_std_mut());
            tracing::info!(writes = "kernel-boundary", "agent tool launch");
            (command, report)
        }
        #[cfg(not(target_os = "linux"))]
        Some(Applied::Landlock(_)) => unreachable!("Linux-only boundary"),
        other => {
            tracing::warn!(
                writes = "not-contained",
                snapshot = other.is_some(),
                "agent tool launch: kernel write isolation unavailable or no workflow boundary"
            );
            archon_shell::execution_environment::tokio_command_with_report(program)
        }
    }
}

/// Persistent terminals require a kernel boundary; snapshots cannot settle while alive.
pub(crate) fn apply_pty(
    boundary: &ShellBoundary,
    command: &mut archon_pty::CommandBuilder,
) -> Result<(), String> {
    match &boundary.applied {
        Applied::SandboxExec(profile) => {
            // terminal_shell already installed the host's read wrapper. Replace its
            // profile rather than attempting to nest two sandbox-exec processes.
            let argv = command.get_argv_mut();
            if argv
                .first()
                .is_some_and(|arg| arg == archon_shell::write_boundary::SANDBOX_EXEC)
            {
                argv[2] = profile.into();
            } else {
                argv.splice(
                    0..0,
                    [
                        archon_shell::write_boundary::SANDBOX_EXEC.into(),
                        "-p".into(),
                        profile.into(),
                        "--".into(),
                    ],
                );
            }
        }
        Applied::Landlock(_) => command.protect_writes(
            boundary.boundary.protected.clone(),
            boundary.boundary.writable.clone(),
        ),
        Applied::HostSnapshot(_) => {
            return Err(archon_shell::write_boundary::refusal(
                "Persistent terminal",
                "a snapshot cannot confine a persistent terminal",
            ));
        }
    }
    Ok(())
}

/// The host pipeline's compilation, test and smoke launches share Bash's boundary.
/// Capture workflow task scopes before launching; no agent-authored file supplies paths.
pub struct PipelineBoundary {
    boundary: Option<ShellBoundary>,
    env: Vec<(String, String)>,
    reports: std::sync::Mutex<Vec<archon_shell::read_boundary::LaunchReport>>,
}

impl PipelineBoundary {
    pub fn current(working_dir: &std::path::Path) -> std::io::Result<Self> {
        use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
        let settings = WorkflowReadGuardSettings::default();
        let write_guard = WorkflowReadGuard::from_settings(&settings);
        let guard = if write_guard.isolated_write_branch() {
            write_guard
        } else {
            WorkflowReadGuard::shell_only(&settings)
        };
        let ctx = crate::tool::ToolContext {
            working_dir: working_dir.to_owned(),
            workflow_read_guard: Some(std::sync::Arc::new(guard)),
            ..Default::default()
        };
        let mut env = archon_shell::execution_environment::environment()
            .into_iter()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        let boundary = for_call(&ctx, &mut env, &[]).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, error.content)
        })?;
        Ok(Self {
            boundary,
            env,
            reports: Default::default(),
        })
    }

    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
        let (mut command, report) = command(program, self.boundary.as_ref());
        self.reports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(report);
        archon_shell::execution_environment::apply(command.as_std_mut());
        command.envs(self.env.iter().cloned());
        command
    }

    pub fn settle(&self) -> std::io::Result<()> {
        for report in self
            .reports
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            tracing::warn!("child seal outcome: {report}");
        }
        if let Some(ShellBoundary {
            applied: Applied::HostSnapshot(snapshot),
            ..
        }) = &self.boundary
        {
            snapshot.verify_restore().map_err(|violation| {
                std::io::Error::other(format!("pipeline write boundary violation: {violation:?}"))
            })?;
        }
        Ok(())
    }
}
