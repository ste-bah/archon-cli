use std::path::{Component, Path};

use archon_permissions::sandbox::{SandboxCommandRequest, SandboxCommandResult};

use super::DockerConfig;
use super::fs::CONTAINER_WORKSPACE;

pub(super) fn docker_run_args(
    config: &DockerConfig,
    workspace_access: &str,
    request: &SandboxCommandRequest,
) -> Vec<String> {
    let mut args = docker_container_args(
        config,
        workspace_access,
        &request.working_dir,
        CONTAINER_WORKSPACE,
    );
    args.extend(allowed_env_args(&request.env, &config.env_allowlist));
    args.extend([
        config.image.clone(),
        "/bin/bash".into(),
        "-lc".into(),
        request.command.clone(),
    ]);
    args
}

/// A terminal's `docker run`, built from the same pieces as a command's.
///
/// The differences are exactly two: `-i -t`, so the container gets a TTY on the
/// PTY the caller opened; and a shell in place of a command, because the shell
/// is what stays. Everything else — the mount, the caps, the network mode — is
/// shared with [`docker_run_args`] on purpose, so a terminal cannot end up in a
/// more permissive container than `Bash` gets.
pub(super) fn docker_terminal_args(
    config: &DockerConfig,
    workspace_access: &str,
    workspace: &Path,
    container_workdir: &str,
    shell_program: &str,
) -> Vec<String> {
    let mut args = docker_container_args(config, workspace_access, workspace, container_workdir);
    args.extend(["--interactive".into(), "--tty".into()]);
    // Claimed rather than inherited: the docker CLI's own `TERM` does not reach
    // the container, and a shell that finds it unset drops to line-at-a-time
    // behaviour that the output buffer then has to read back.
    args.extend(["--env".into(), "TERM=xterm-256color".into()]);
    args.extend([config.image.clone(), shell_program.to_string()]);
    args
}

/// The container `Bash` and a terminal both get: same isolation, same mount.
/// Run as the user who owns the workspace.
///
/// `--cap-drop ALL` takes `CAP_DAC_OVERRIDE` with it, and without that the
/// container's root cannot write through a bind mount it does not own. On Linux
/// that is every ordinary checkout — the tree belongs to the developer, mode
/// 0755 — so `workspace_access = "rw"` produced "Permission denied" for `Bash`
/// and for terminals alike. Measured, not assumed: root in the container gets
/// EACCES on a 0755 directory owned by uid 1000, and the same container run as
/// that uid writes.
///
/// Running as the host user is also the stronger posture. It is not root in the
/// container, and files it creates in the workspace belong to the developer
/// rather than arriving owned by root.
///
/// Unix only. Windows has no uid to pass, and Docker Desktop's filesystem
/// translation gives the container write access to a bind mount regardless —
/// which is why this bug was invisible from a Windows host.
#[cfg(unix)]
fn host_identity_args() -> Vec<String> {
    // Safe: `getuid`/`getgid` cannot fail and touch no shared state.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    vec!["--user".into(), format!("{uid}:{gid}")]
}

#[cfg(not(unix))]
fn host_identity_args() -> Vec<String> {
    Vec::new()
}

fn docker_container_args(
    config: &DockerConfig,
    workspace_access: &str,
    working_dir: &Path,
    container_workdir: &str,
) -> Vec<String> {
    let mut args = vec!["run".into(), "--rm".into(), "--pull".into(), "never".into()];
    args.extend(["--security-opt".into(), "no-new-privileges".into()]);
    args.extend(["--cap-drop".into(), "ALL".into()]);
    args.extend(host_identity_args());
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
        working_dir,
        workspace_access,
        &config.writable_paths,
        container_workdir,
    ));
    args
}

/// The container path a host path names, for a shell that must start there.
///
/// Strict where `DockerFs::to_container` is lenient: that one translates
/// results the model will merely read, while this one chooses the directory a
/// live shell comes up in. A path outside the mount has no container form at
/// all, and starting the shell at the workspace root instead would silently
/// answer a different question than the one asked.
pub(super) fn container_workdir(workspace: &Path, cwd: &Path) -> Result<String, String> {
    let Ok(relative) = cwd.strip_prefix(workspace) else {
        return Err(format!(
            "{} is outside the sandbox workspace ({}), which is the only host \
             directory the container can see",
            cwd.display(),
            workspace.display()
        ));
    };
    let mut path = CONTAINER_WORKSPACE.to_string();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                path.push('/');
                path.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "{} leaves the sandbox workspace mount",
                    cwd.display()
                ));
            }
        }
    }
    Ok(path)
}

/// The shell to run in the container, by the name the model asked for.
///
/// `None` means the caller did not say, and the container's answer is bash
/// regardless of what the host would have chosen — the host default on Windows
/// is PowerShell, and asking a Linux image for it would refuse every terminal
/// opened without an explicit `shell` on the platform this is developed on.
pub(super) fn container_shell(shell: Option<&str>) -> Result<(String, String), String> {
    match shell {
        None | Some("bash") => Ok(("bash".into(), "/bin/bash".into())),
        Some("sh") => Ok(("sh".into(), "/bin/sh".into())),
        Some(other @ ("powershell" | "cmd")) => Err(format!(
            "the docker sandbox runs a Linux container, which has no {other}; \
             ask for bash or sh"
        )),
        Some(other) => Err(format!(
            "unknown shell {other:?}; the docker sandbox offers bash and sh"
        )),
    }
}

fn workspace_mount_args(
    working_dir: &Path,
    workspace_access: &str,
    writable_paths: &[String],
    container_workdir: &str,
) -> Vec<String> {
    let readonly = workspace_access != "rw";
    let mut args = vec![
        "--mount".into(),
        format!(
            "type=bind,src={},dst={CONTAINER_WORKSPACE}{}",
            working_dir.display(),
            if readonly { ",readonly" } else { "" }
        ),
        "--workdir".into(),
        container_workdir.to_string(),
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
        // Joined as a `/`-separated string rather than via `Path::join(..)
        // .display()`. The latter emits native separators, so on Windows the
        // mount spec came out as `src=/repo\target` -- a malformed bind source
        // for a Linux container, whatever the host.
        let source = format!(
            "{}/{}",
            working_dir
                .display()
                .to_string()
                .trim_end_matches(['/', '\\']),
            relative
        );
        args.extend([
            "--mount".into(),
            format!("type=bind,src={source},dst={CONTAINER_WORKSPACE}/{relative}"),
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
    let exit_code = status.as_ref().ok().and_then(|status| status.code());
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
    match exit_code {
        Some(0) => SandboxCommandResult {
            content: output,
            is_error: false,
            exit_code,
        },
        Some(exit_code) => SandboxCommandResult {
            content: format!("Exit code {exit_code}\n{output}"),
            is_error: true,
            exit_code: Some(exit_code),
        },
        None => SandboxCommandResult {
            content: format!("Process terminated without an exit code\n{output}"),
            is_error: true,
            exit_code: None,
        },
    }
}
