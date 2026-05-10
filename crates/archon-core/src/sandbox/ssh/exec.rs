use archon_permissions::sandbox::{SandboxCommandRequest, SandboxCommandResult};

use super::SshConfig;

pub(super) fn ssh_command_args(
    config: &SshConfig,
    request: &SandboxCommandRequest,
) -> Result<Vec<String>, String> {
    let mut args = vec![
        "-T".into(),
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
    if let Some(key_file) = config.key_file.as_deref().map(str::trim) {
        if !key_file.is_empty() {
            args.extend(["-i".into(), key_file.into()]);
        }
    }
    args.push(ssh_target(config)?);
    args.push(remote_bash_command(config, request)?);
    Ok(args)
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
    let workdir = remote_workdir(config, request)?;
    Ok(format!(
        "cd -- {} && /bin/bash -lc {}",
        shell_quote(&workdir),
        shell_quote(&request.command)
    ))
}

fn remote_workdir(config: &SshConfig, request: &SandboxCommandRequest) -> Result<String, String> {
    let workdir = if config.workspace_mode == "remote" {
        config
            .remote_workdir
            .as_deref()
            .ok_or_else(|| "sandbox.ssh.remote_workdir is required in remote mode".to_string())?
            .trim()
            .to_string()
    } else {
        request.working_dir.to_string_lossy().to_string()
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
    let exit_code = output.status.code().unwrap_or(-1);
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
    if exit_code == 0 {
        SandboxCommandResult {
            content: text,
            is_error: false,
        }
    } else {
        SandboxCommandResult {
            content: format!("Exit code {exit_code}\n{text}"),
            is_error: true,
        }
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
