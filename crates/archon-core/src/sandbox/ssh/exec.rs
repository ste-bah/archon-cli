use std::path::Path;

use archon_permissions::sandbox::{
    SandboxCommandRequest, SandboxCommandResult, SandboxTerminalCommand, SandboxTerminalRequest,
};

use super::SshConfig;

pub(super) fn ssh_command_args(
    config: &SshConfig,
    request: &SandboxCommandRequest,
) -> Result<Vec<String>, String> {
    let mut args = ssh_connection_args(config, "-T")?;
    args.push(remote_bash_command(config, request)?);
    Ok(args)
}

/// A terminal over the same connection `Bash` uses, with a TTY on it.
///
/// `-tt` forces TTY allocation even though the caller's stdin is a PTY the
/// local `ssh` cannot see as one, and `exec` replaces the login shell so the
/// terminal's process *is* the shell — otherwise closing it would leave the
/// remote shell behind under a wrapper.
pub(super) fn ssh_terminal_args(
    config: &SshConfig,
    request: &SandboxTerminalRequest,
) -> Result<SandboxTerminalCommand, String> {
    let (shell, program) = remote_shell(request.shell.as_deref())?;
    let workdir = terminal_workdir(config, request)?;
    let mut args = ssh_connection_args(config, "-tt")?;
    args.push(format!(
        "cd -- {} && exec {program} -i",
        shell_quote(&workdir)
    ));
    Ok(SandboxTerminalCommand {
        program: config.binary.clone(),
        args,
        shell,
        location: format!("{workdir} on {}", ssh_target(config)?),
    })
}

/// Everything up to and including the target: options, key, `user@host`.
///
/// Shared with [`ssh_command_args`] so a terminal cannot reach the host under
/// weaker options than a command does — `BatchMode=yes` in particular, which is
/// what stops a PTY session from turning into an interactive credential prompt
/// the model would be the one answering.
fn ssh_connection_args(config: &SshConfig, tty_flag: &str) -> Result<Vec<String>, String> {
    let mut args = vec![
        tty_flag.into(),
        "-p".into(),
        config.port.to_string(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        "ForwardAgent=no".into(),
        "-o".into(),
        "PermitLocalCommand=no".into(),
    ];
    if let Some(key_file) = config.key_file.as_deref().map(str::trim)
        && !key_file.is_empty()
    {
        args.extend(["-i".into(), key_file.into()]);
    }
    args.push(ssh_target(config)?);
    Ok(args)
}

fn remote_shell(shell: Option<&str>) -> Result<(String, String), String> {
    match shell {
        None | Some("bash") => Ok(("bash".into(), "/bin/bash".into())),
        Some("sh") => Ok(("sh".into(), "/bin/sh".into())),
        Some(other @ ("powershell" | "cmd")) => Err(format!(
            "the ssh sandbox reaches a POSIX host, which has no {other}; \
             ask for bash or sh"
        )),
        Some(other) => Err(format!(
            "unknown shell {other:?}; the ssh sandbox offers bash and sh"
        )),
    }
}

/// Where a terminal's shell starts.
///
/// In `remote` mode the workspace is a directory on the far side with no
/// relationship to any host path, so a caller-chosen `cwd` cannot be honoured
/// and must not be quietly dropped either — starting somewhere other than where
/// it asked is how an agent ends up editing the wrong tree.
fn terminal_workdir(
    config: &SshConfig,
    request: &SandboxTerminalRequest,
) -> Result<String, String> {
    if config.workspace_mode == "remote" && request.cwd != request.workspace {
        return Err(format!(
            "sandbox.ssh.workspace_mode is \"remote\", so the shell starts in \
             sandbox.ssh.remote_workdir; the host path {} names nothing there. \
             Open the terminal without a cwd and cd once it is up",
            request.cwd.display()
        ));
    }
    workspace_workdir(config, &request.cwd)
}

fn ssh_target(config: &SshConfig) -> Result<String, String> {
    let host = safe_ssh_token(config.host.as_deref().unwrap_or(""), "sandbox.ssh.host")?;
    let Some(user) = config
        .user
        .as_deref()
        .map(str::trim)
        .filter(|user| !user.is_empty())
    else {
        return Ok(host.to_string());
    };
    let user = safe_ssh_token(user, "sandbox.ssh.user")?;
    Ok(format!("{user}@{host}"))
}

fn remote_bash_command(
    config: &SshConfig,
    request: &SandboxCommandRequest,
) -> Result<String, String> {
    let workdir = workspace_workdir(config, &request.working_dir)?;
    Ok(format!(
        "cd -- {} && /bin/bash -lc {}",
        shell_quote(&workdir),
        shell_quote(&request.command)
    ))
}

/// The directory the far side works in. `host_dir` is used only in `mirror`
/// mode, where the remote tree is the same paths as the local one.
fn workspace_workdir(config: &SshConfig, host_dir: &Path) -> Result<String, String> {
    let workdir = if config.workspace_mode == "remote" {
        config
            .remote_workdir
            .as_deref()
            .ok_or_else(|| "sandbox.ssh.remote_workdir is required in remote mode".to_string())?
            .trim()
            .to_string()
    } else {
        host_dir.to_string_lossy().to_string()
    };
    if workdir.trim().is_empty() || workdir.contains('\0') {
        return Err("ssh sandbox remote workdir must not be empty or contain NUL".into());
    }
    Ok(workdir)
}

fn safe_ssh_token<'a>(value: &'a str, field: &str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.starts_with('-')
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(format!("{field} contains unsupported characters"));
    }
    Ok(value)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

pub(super) fn ssh_output_result(
    output: std::process::Output,
    max_output_bytes: usize,
) -> SandboxCommandResult {
    let exit_code = output.status.code();
    let combined = [output.stdout, output.stderr].concat();
    let truncated = combined.len() > max_output_bytes;
    let bytes = if truncated {
        &combined[..max_output_bytes]
    } else {
        &combined
    };
    let mut text = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        text.push_str(&format!("\n\nOutput truncated at {max_output_bytes} bytes"));
    }
    match exit_code {
        Some(0) => SandboxCommandResult {
            content: text,
            is_error: false,
            exit_code,
        },
        Some(exit_code) => SandboxCommandResult {
            content: format!("Exit code {exit_code}\n{text}"),
            is_error: true,
            exit_code: Some(exit_code),
        },
        None => SandboxCommandResult {
            content: format!("Process terminated without an exit code\n{text}"),
            is_error: true,
            exit_code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn request() -> SandboxCommandRequest {
        SandboxCommandRequest {
            command: "printf 'hello'".into(),
            working_dir: PathBuf::from("/workspace/local"),
            timeout_ms: 1000,
            max_output_bytes: 1024,
            env: vec![("SECRET_TOKEN".into(), "nope".into())],
        }
    }

    #[test]
    fn args_use_strict_remote_execution_without_env_forwarding() {
        let cfg = SshConfig {
            enabled: true,
            host: Some("sandbox.example".into()),
            user: Some("archon".into()),
            port: 2222,
            key_file: Some("/tmp/key".into()),
            remote_workdir: Some("/srv/workspace".into()),
            ..SshConfig::default()
        };

        let args = ssh_command_args(&cfg, &request()).unwrap();

        assert_eq!(args[0], "-T");
        assert!(args.contains(&"StrictHostKeyChecking=yes".to_string()));
        assert!(args.contains(&"ForwardAgent=no".to_string()));
        assert!(args.contains(&"/tmp/key".to_string()));
        assert!(args.contains(&"archon@sandbox.example".to_string()));
        assert!(args.last().unwrap().contains("cd -- '/srv/workspace'"));
        assert!(!args.iter().any(|arg| arg.contains("SECRET_TOKEN")));
    }

    #[test]
    fn remote_mode_requires_explicit_remote_workdir() {
        let cfg = SshConfig {
            enabled: true,
            host: Some("sandbox.example".into()),
            ..SshConfig::default()
        };

        let err = ssh_command_args(&cfg, &request()).unwrap_err();

        assert!(err.contains("remote_workdir"));
    }

    #[test]
    fn mirror_mode_uses_request_workdir() {
        let cfg = SshConfig {
            enabled: true,
            host: Some("sandbox.example".into()),
            workspace_mode: "mirror".into(),
            ..SshConfig::default()
        };

        let args = ssh_command_args(&cfg, &request()).unwrap();

        assert!(args.last().unwrap().contains("cd -- '/workspace/local'"));
    }

    fn terminal_request(cwd: &str) -> SandboxTerminalRequest {
        SandboxTerminalRequest {
            shell: None,
            workspace: PathBuf::from("/workspace/local"),
            cwd: PathBuf::from(cwd),
        }
    }

    fn remote_config() -> SshConfig {
        SshConfig {
            enabled: true,
            host: Some("sandbox.example".into()),
            user: Some("archon".into()),
            remote_workdir: Some("/srv/workspace".into()),
            ..SshConfig::default()
        }
    }

    /// The connection a terminal makes must be the connection a command makes,
    /// option for option — `BatchMode=yes` above all, since a PTY session
    /// without it would put a credential prompt in front of the model.
    #[test]
    fn a_terminal_reaches_the_host_under_the_same_options_a_command_does() {
        let command = ssh_terminal_args(&remote_config(), &terminal_request("/workspace/local"))
            .expect("remote terminal");

        assert_eq!(command.program, "ssh");
        assert_eq!(command.args[0], "-tt", "a PTY session needs a remote TTY");
        for option in [
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "ForwardAgent=no",
            "PermitLocalCommand=no",
        ] {
            assert!(
                command.args.contains(&option.to_string()),
                "missing {option}"
            );
        }
        assert!(command.args.contains(&"archon@sandbox.example".to_string()));
        assert_eq!(
            command.args.last().map(String::as_str),
            Some("cd -- '/srv/workspace' && exec /bin/bash -i")
        );
        assert_eq!(command.location, "/srv/workspace on archon@sandbox.example");
    }

    /// In `remote` mode a host path names nothing on the far side. Dropping it
    /// and starting at the remote root would put the shell somewhere the caller
    /// did not ask for and would never be told about.
    #[test]
    fn remote_mode_refuses_a_caller_chosen_directory_instead_of_ignoring_it() {
        let error = ssh_terminal_args(&remote_config(), &terminal_request("/workspace/local/src"))
            .expect_err("a host subdirectory has no remote meaning");

        assert!(error.contains("remote"), "{error}");
        assert!(error.contains("/workspace/local/src"), "{error}");
    }

    #[test]
    fn mirror_mode_starts_the_terminal_where_it_was_asked_to() {
        let config = SshConfig {
            workspace_mode: "mirror".into(),
            ..remote_config()
        };

        let command = ssh_terminal_args(&config, &terminal_request("/workspace/local/src"))
            .expect("mirror terminal");

        assert!(
            command
                .args
                .last()
                .expect("remote command")
                .contains("cd -- '/workspace/local/src'"),
            "{:?}",
            command.args
        );
    }

    #[test]
    fn a_windows_only_shell_is_refused_with_the_reason() {
        let mut request = terminal_request("/workspace/local");
        request.shell = Some("cmd".into());

        let error =
            ssh_terminal_args(&remote_config(), &request).expect_err("no cmd on a POSIX host");

        assert!(error.contains("POSIX"), "{error}");
        assert!(error.contains("bash"), "{error}");
    }

    #[test]
    fn rejects_option_like_targets() {
        let cfg = SshConfig {
            enabled: true,
            host: Some("-oProxyCommand=bad".into()),
            workspace_mode: "mirror".into(),
            ..SshConfig::default()
        };

        let err = ssh_command_args(&cfg, &request()).unwrap_err();

        assert!(err.contains("sandbox.ssh.host"));
    }
}
