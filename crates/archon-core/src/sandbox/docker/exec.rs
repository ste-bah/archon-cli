use std::path::{Component, Path};

use archon_permissions::sandbox::{SandboxCommandRequest, SandboxCommandResult};

use super::DockerConfig;

pub(super) fn docker_run_args(
    config: &DockerConfig,
    workspace_access: &str,
    request: &SandboxCommandRequest,
) -> Vec<String> {
    let mut args = vec!["run".into(), "--rm".into(), "--pull".into(), "never".into()];
    args.extend(["--security-opt".into(), "no-new-privileges".into()]);
    args.extend(["--cap-drop".into(), "ALL".into()]);
    args.extend(["--pids-limit".into(), "256".into()]);
    args.extend(["--tmpfs".into(), "/tmp:rw,nosuid,size=256m".into()]);
    args.extend([
        "--network".into(),
        docker_network_mode(&config.network).into(),
    ]);
    if let Some(memory) = &config.memory_limit {
        args.extend(["--memory".into(), memory.clone()]);
    }
    if let Some(cpus) = &config.cpu_limit {
        args.extend(["--cpus".into(), cpus.clone()]);
    }
    args.extend(workspace_mount_args(
        &request.working_dir,
        workspace_access,
        &config.writable_paths,
    ));
    args.extend(allowed_env_args(&request.env, &config.env_allowlist));
    args.extend([
        config.image.clone(),
        "/bin/bash".into(),
        "-lc".into(),
        request.command.clone(),
    ]);
    args
}

fn workspace_mount_args(
    working_dir: &Path,
    workspace_access: &str,
    writable_paths: &[String],
) -> Vec<String> {
    let readonly = workspace_access != "rw";
    let mut args = vec![
        "--mount".into(),
        format!(
            "type=bind,src={},dst=/workspace{}",
            working_dir.display(),
            if readonly { ",readonly" } else { "" }
        ),
        "--workdir".into(),
        "/workspace".into(),
    ];
    if workspace_access == "scratch" {
        args.extend(["--tmpfs".into(), "/scratch:rw,nosuid,size=512m".into()]);
        args.extend(["--env".into(), "ARCHON_SANDBOX_SCRATCH=/scratch".into()]);
    }
    if readonly {
        args.extend(writable_path_mount_args(working_dir, writable_paths));
    }
    args
}

fn writable_path_mount_args(working_dir: &Path, writable_paths: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    for path in writable_paths {
        let Ok(relative) = normal_writable_path(path) else {
            continue;
        };
        let source = working_dir.join(&relative);
        args.extend([
            "--mount".into(),
            format!(
                "type=bind,src={},dst=/workspace/{}",
                source.display(),
                relative
            ),
        ]);
    }
    args
}

pub(super) fn validate_workspace_access(workspace_access: &str) -> Result<(), String> {
    match workspace_access {
        "ro" | "rw" | "scratch" => Ok(()),
        other => Err(format!(
            "sandbox.workspace_access must be ro, rw, or scratch, got \"{other}\""
        )),
    }
}

pub(super) fn normal_writable_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("sandbox.docker.writable_paths entries must not be empty".into());
    }
    if trimmed.contains(',') || trimmed.contains('\0') {
        return Err(format!(
            "sandbox.docker.writable_paths entry \"{trimmed}\" contains an unsupported character"
        ));
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(format!(
            "sandbox.docker.writable_paths entry \"{trimmed}\" must be relative to the workspace"
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "sandbox.docker.writable_paths entry \"{trimmed}\" must not escape the workspace"
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(format!(
            "sandbox.docker.writable_paths entry \"{trimmed}\" must name a subpath"
        ));
    }
    Ok(parts.join("/"))
}

fn allowed_env_args(env: &[(String, String)], allowlist: &[String]) -> Vec<String> {
    let mut args = Vec::new();
    for name in allowlist {
        if sensitive_env_name(name) {
            continue;
        }
        if let Some((_, value)) = env.iter().find(|(key, _)| key == name) {
            args.extend(["--env".into(), format!("{name}={value}")]);
        }
    }
    args
}

fn sensitive_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["TOKEN", "SECRET", "KEY", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|needle| upper.contains(needle))
}

fn docker_network_mode(network: &str) -> &'static str {
    match network {
        "enabled" => "bridge",
        "limited" | "disabled" => "none",
        _ => "none",
    }
}

pub(super) fn docker_output_result(
    stdout_buf: Vec<u8>,
    stderr_buf: Vec<u8>,
    status: std::io::Result<std::process::ExitStatus>,
    max_output_bytes: usize,
) -> SandboxCommandResult {
    let exit_code = status.as_ref().ok().and_then(|s| s.code()).unwrap_or(-1);
    let combined = [stdout_buf, stderr_buf].concat();
    let truncated = combined.len() > max_output_bytes;
    let bytes = if truncated {
        &combined[..max_output_bytes]
    } else {
        &combined
    };
    let mut output = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        output.push_str(&format!("\n\nOutput truncated at {max_output_bytes} bytes"));
    }
    if exit_code == 0 {
        SandboxCommandResult {
            content: output,
            is_error: false,
        }
    } else {
        SandboxCommandResult {
            content: format!("Exit code {exit_code}\n{output}"),
            is_error: true,
        }
    }
}
