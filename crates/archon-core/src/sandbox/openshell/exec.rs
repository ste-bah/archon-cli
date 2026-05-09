use archon_permissions::sandbox::{SandboxCommandRequest, SandboxCommandResult};

use super::OpenShellConfig;

pub(super) fn openshell_create_args(
    config: &OpenShellConfig,
    request: &SandboxCommandRequest,
) -> Result<Vec<String>, String> {
    validate_request(request)?;
    let mut args = vec!["sandbox".into(), "create".into()];
    if config.gpu {
        args.push("--gpu".into());
    }
    args.push("--no-keep".into());
    if let Some(policy) = config.policy.as_deref().map(str::trim) {
        if policy.contains('\0') {
            return Err("sandbox.openshell.policy must not contain NUL".into());
        }
        if !policy.is_empty() {
            args.extend(["--policy".into(), policy.into()]);
        }
    }
    args.extend([
        "--".into(),
        "/bin/bash".into(),
        "-lc".into(),
        sandbox_bash_command(config, request)?,
    ]);
    Ok(args)
}

fn validate_request(request: &SandboxCommandRequest) -> Result<(), String> {
    if request.command.contains('\0') {
        return Err("openshell sandbox command must not contain NUL".into());
    }
    if request.working_dir.to_string_lossy().contains('\0') {
        return Err("openshell sandbox workdir must not contain NUL".into());
    }
    Ok(())
}

fn sandbox_bash_command(
    config: &OpenShellConfig,
    request: &SandboxCommandRequest,
) -> Result<String, String> {
    let workdir = openshell_workdir(config, request)?;
    Ok(format!(
        "cd -- {} && {}",
        shell_quote(&workdir),
        request.command
    ))
}

fn openshell_workdir(
    config: &OpenShellConfig,
    request: &SandboxCommandRequest,
) -> Result<String, String> {
    let workdir = if config.workspace_mode == "remote" {
        "/sandbox".into()
    } else {
        request.working_dir.to_string_lossy().to_string()
    };
    if workdir.trim().is_empty() || workdir.contains('\0') {
        return Err("openshell sandbox workdir must not be empty or contain NUL".into());
    }
    Ok(workdir)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r#"'\''"#))
}

pub(super) fn openshell_output_result(
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
            env: vec![("ANTHROPIC_API_KEY".into(), "nope".into())],
        }
    }

    #[test]
    fn args_create_ephemeral_sandbox_without_provider_flags_or_env() {
        let cfg = OpenShellConfig {
            enabled: true,
            policy: Some("./policy.yaml".into()),
            providers: vec!["my-claude".into()],
            ..OpenShellConfig::default()
        };

        let args = openshell_create_args(&cfg, &request()).unwrap();

        assert_eq!(args[0], "sandbox");
        assert_eq!(args[1], "create");
        assert!(args.contains(&"--no-keep".to_string()));
        assert!(args.contains(&"--policy".to_string()));
        assert!(args.contains(&"./policy.yaml".to_string()));
        assert!(args.contains(&"/bin/bash".to_string()));
        assert!(args.last().unwrap().contains("cd -- '/workspace/local'"));
        assert!(!args.iter().any(|arg| arg == "--provider"));
        assert!(!args.iter().any(|arg| arg.contains("ANTHROPIC_API_KEY")));
        assert!(!args.iter().any(|arg| arg.contains("my-claude")));
    }

    #[test]
    fn gpu_flag_is_explicit_when_configured() {
        let cfg = OpenShellConfig {
            enabled: true,
            gpu: true,
            ..OpenShellConfig::default()
        };

        let args = openshell_create_args(&cfg, &request()).unwrap();

        assert!(args.contains(&"--gpu".to_string()));
    }

    #[test]
    fn remote_mode_uses_openshell_sandbox_workspace() {
        let cfg = OpenShellConfig {
            enabled: true,
            workspace_mode: "remote".into(),
            gateway: Some("team-gateway".into()),
            ..OpenShellConfig::default()
        };

        let args = openshell_create_args(&cfg, &request()).unwrap();

        assert!(args.last().unwrap().contains("cd -- '/sandbox'"));
    }

    #[test]
    fn rejects_nul_in_command() {
        let mut req = request();
        req.command = "echo hi\0echo bad".into();

        let err = openshell_create_args(&OpenShellConfig::default(), &req).unwrap_err();

        assert!(err.contains("NUL"));
    }
}
