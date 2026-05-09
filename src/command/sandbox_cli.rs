use anyhow::Result;

use crate::cli_args::SandboxAction;

#[path = "sandbox_explain_tools.rs"]
mod sandbox_explain_tools;

pub(crate) fn handle_sandbox_command(
    action: Option<SandboxAction>,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let action = action.unwrap_or(SandboxAction::Status { verbose: false });
    let output = match action {
        SandboxAction::Status { verbose } => {
            let output = render_status(&config.sandbox, verbose)?;
            persist_sandbox_command_event(&config.sandbox, None, "status", "cli_status");
            output
        }
        SandboxAction::Explain {
            backend,
            tool,
            command,
        } => {
            let output = render_explain(
                &config.sandbox,
                backend.clone(),
                tool.as_deref(),
                command.as_deref(),
            )?;
            persist_sandbox_command_event(
                &config.sandbox,
                backend.as_deref(),
                "explain",
                "cli_explain",
            );
            output
        }
        SandboxAction::Doctor { backend } => {
            let output = crate::command::sandbox_doctor::render_sandbox_doctor(
                &doctor_args(backend.clone()),
                crate::command::sandbox_doctor::SandboxDoctorOverrides::default(),
            );
            persist_sandbox_command_event(
                &config.sandbox,
                backend.as_deref(),
                "doctor",
                "cli_doctor",
            );
            output
        }
        SandboxAction::Test { backend } => {
            let output = render_test(&config.sandbox, backend.clone())?;
            persist_sandbox_command_event(
                &config.sandbox,
                backend.as_deref(),
                "test_config_valid",
                "cli_test",
            );
            output
        }
        SandboxAction::Sessions {
            status,
            agent,
            limit,
            json,
        } => {
            let output = render_sessions(status.as_deref(), agent.as_deref(), limit, json)?;
            persist_sandbox_command_event(&config.sandbox, None, "sessions", "cli_sessions");
            output
        }
    };
    print!("{output}");
    Ok(())
}

fn render_status(config: &archon_core::sandbox::SandboxConfig, verbose: bool) -> Result<String> {
    let policy = config.policy().map_err(anyhow::Error::msg)?;
    let mut output = format!(
        "Sandbox status\nBackend: {}\nMode: {}\nScope: {}\nWorkspace access: {}\nIsolation: {}\n",
        policy.backend,
        policy.mode,
        policy.scope,
        policy.workspace_access,
        policy.describes_isolation()
    );
    if verbose {
        output.push_str(
            "Compatibility: /sandbox toggles the logical TUI gate only; normal permission rules still apply\n",
        );
        output.push_str(
            "Execution: docker, ssh, and openshell can route Bash when selected; no backend may fall back to host shell\n",
        );
        append_backend_verbose_status(&mut output, config);
    }
    Ok(output)
}

fn append_backend_verbose_status(
    output: &mut String,
    config: &archon_core::sandbox::SandboxConfig,
) {
    match config.backend.as_str() {
        "docker" => output.push_str(&format!(
            "Docker: enabled={} image={} network={} privileged={} mount_home={} mount_docker_socket={}\n",
            config.docker.enabled,
            config.docker.image,
            config.docker.network,
            config.docker.privileged,
            config.docker.mount_home,
            config.docker.mount_docker_socket
        )),
        "ssh" => output.push_str(&format!(
            "SSH: enabled={} workspace_mode={} remote_workdir_configured={} host_configured={} host_key_checking={} host_shell_fallback={}\n",
            config.ssh.enabled,
            config.ssh.workspace_mode,
            config
                .ssh
                .remote_workdir
                .as_deref()
                .is_some_and(|workdir| !workdir.trim().is_empty()),
            config.ssh.host.as_deref().is_some_and(|host| !host.trim().is_empty()),
            config.ssh.host_key_checking,
            config.ssh.host_shell_fallback
        )),
        "openshell" => output.push_str(&format!(
            "OpenShell: enabled={} workspace_mode={} gateway_configured={} provider_injection={} host_shell_fallback={}\nProvider routing: host-side; sandbox provider injection must stay disabled for Claude Code spoofing\n",
            config.openshell.enabled,
            config.openshell.workspace_mode,
            config
                .openshell
                .gateway
                .as_deref()
                .is_some_and(|gateway| !gateway.trim().is_empty()),
            config.openshell.provider_injection,
            config.openshell.host_shell_fallback
        )),
        _ => {}
    }
}

fn render_explain(
    config: &archon_core::sandbox::SandboxConfig,
    backend: Option<String>,
    tool: Option<&str>,
    command: Option<&str>,
) -> Result<String> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend) = backend {
        policy.backend = backend.parse().map_err(anyhow::Error::msg)?;
    }
    policy.validate().map_err(anyhow::Error::msg)?;
    let mut output = format!(
        "Sandbox explain\nBackend: {}\nIsolation: {}\nDecision flow: UnifiedToolPreflight -> PermissionChecker -> SandboxPolicyResolver -> SandboxBackend -> ToolDispatch\nPermissions: sandbox policy cannot bypass always_deny rules, permission modes, or dangerous-bypass guards\nExecution: docker, ssh, and openshell can route Bash when selected; direct host shell fallback stays forbidden\n",
        policy.backend,
        policy.describes_isolation()
    );
    append_explain_details(&mut output, config, &policy)?;
    sandbox_explain_tools::append_tool_explain(&mut output, &policy, tool, command);
    Ok(output)
}

fn append_explain_details(
    output: &mut String,
    config: &archon_core::sandbox::SandboxConfig,
    policy: &archon_core::sandbox::SandboxPolicy,
) -> Result<()> {
    match policy.backend {
        archon_core::sandbox::SandboxBackendKind::Docker => {
            config.docker.validate().map_err(anyhow::Error::msg)?;
            output.push_str(&format!(
                "Mount policy: {}\nWritable paths: {}\nNetwork policy: {}\nResource limits: memory={} cpu={}\nRedaction policy: only env_allowlist entries are forwarded; provider tokens, SSH agents, Git credentials, Docker socket, and broad home mounts are excluded by default\n",
                workspace_access_summary(&policy.workspace_access),
                list_or_none(&config.docker.writable_paths),
                config.docker.network,
                config.docker.memory_limit.as_deref().unwrap_or("unset"),
                config.docker.cpu_limit.as_deref().unwrap_or("unset")
            ));
        }
        archon_core::sandbox::SandboxBackendKind::Ssh => {
            config.ssh.validate().map_err(anyhow::Error::msg)?;
            output.push_str(&format!(
                "Transport policy: ssh remote execution; host_configured={} host_key_checking={} host_shell_fallback={}\nWorkspace policy: mode={} remote_workdir_configured={} remote workspace remains explicit\nRedaction policy: provider tokens, generated memory stores, SSH agents, Git credentials, and host home mounts are not forwarded by default\n",
                config.ssh.host.as_deref().is_some_and(|host| !host.trim().is_empty()),
                config.ssh.host_key_checking,
                config.ssh.host_shell_fallback,
                config.ssh.workspace_mode,
                config
                    .ssh
                    .remote_workdir
                    .as_deref()
                    .is_some_and(|workdir| !workdir.trim().is_empty())
            ));
        }
        archon_core::sandbox::SandboxBackendKind::OpenShell => {
            config.openshell.validate().map_err(anyhow::Error::msg)?;
            output.push_str(&format!(
                "Transport policy: openshell mediated execution; workspace_mode={} gateway_configured={}\nProvider routing: host-side; provider_injection={} so Anthropic Claude Code spoofing stays in Archon's provider runtime\nFallback policy: host_shell_fallback={} and direct host shell fallback is not allowed\nRedaction policy: provider credentials, token stores, generated memory databases, SSH agents, Git credentials, and arbitrary home mounts are not synced by default\n",
                config.openshell.workspace_mode,
                config
                    .openshell
                    .gateway
                    .as_deref()
                    .is_some_and(|gateway| !gateway.trim().is_empty()),
                config.openshell.provider_injection,
                config.openshell.host_shell_fallback
            ));
        }
        archon_core::sandbox::SandboxBackendKind::Logical => output.push_str(
            "Isolation policy: logical permission gate only; no process, network, or filesystem isolation is claimed\n",
        ),
        archon_core::sandbox::SandboxBackendKind::Disabled => output.push_str(
            "Isolation policy: sandbox backend disabled; normal permission checks still apply\n",
        ),
    }
    Ok(())
}

fn workspace_access_summary(workspace_access: &str) -> &'static str {
    match workspace_access {
        "rw" => "workspace mounted read-write",
        "scratch" => "workspace mounted read-only with ephemeral /scratch",
        _ => "workspace mounted read-only",
    }
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".into()
    } else {
        values.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn render_test(
    config: &archon_core::sandbox::SandboxConfig,
    backend: Option<String>,
) -> Result<String> {
    let mut policy = config.policy().map_err(anyhow::Error::msg)?;
    if let Some(backend) = backend {
        policy.backend = backend.parse().map_err(anyhow::Error::msg)?;
    }
    policy.validate().map_err(anyhow::Error::msg)?;
    Ok(format!(
        "Sandbox test\nBackend: {}\nConfig: valid\nExecution: detect-only; no untrusted command was run\n",
        policy.backend
    ))
}

fn doctor_args(backend: Option<String>) -> Vec<String> {
    match backend {
        Some(backend) => vec!["--backend".into(), backend],
        None => Vec::new(),
    }
}

fn render_sessions(
    status: Option<&str>,
    agent_filter: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<String> {
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    let mut sessions = if let Some(status) = status.filter(|value| !value.trim().is_empty()) {
        archon_learning::sandbox_sessions::list_sandbox_sessions_by_status(&db, status)?
    } else {
        archon_learning::sandbox_sessions::list_sandbox_sessions(&db)?
    };
    if let Some(agent) = agent_filter.filter(|value| !value.trim().is_empty()) {
        sessions.retain(|session| session.agent_type.as_deref() == Some(agent));
    }
    sessions.truncate(limit);

    if json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&sessions)?));
    }
    if sessions.is_empty() {
        return Ok("No sandbox sessions found.\n".into());
    }
    Ok(render_sessions_table(&sessions))
}

fn render_sessions_table(
    sessions: &[archon_learning::sandbox_sessions::SandboxSessionRecord],
) -> String {
    let mut output = String::from(
        "Sandbox sessions (Cozo)\n\nsession_id                           backend    status      agent              workspace  transport  provider_injection\n",
    );
    for session in sessions {
        output.push_str(&format!(
            "{:<36} {:<10} {:<11} {:<18} {:<10} {:<10} {}\n",
            session.sandbox_session_id,
            session.backend_kind,
            session.status,
            session.agent_type.as_deref().unwrap_or("-"),
            session.workspace_mode.as_deref().unwrap_or("-"),
            session.transport_kind.as_deref().unwrap_or("-"),
            yes_no(session.provider_injection_enabled)
        ));
    }
    output.push_str(
        "\nProvider credentials and generated memory stores are redacted by sandbox audit policy.\n",
    );
    output
}

fn learning_db_path() -> Result<std::path::PathBuf> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(parent.join("learning.db"))
}

fn open_learning_db(path: &std::path::Path) -> Result<cozo::DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    cozo::DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("open learning db: {e}"))
}

fn persist_sandbox_command_event(
    config: &archon_core::sandbox::SandboxConfig,
    backend_override: Option<&str>,
    decision: &str,
    reason_code: &str,
) {
    if let Err(error) = crate::runtime::sandbox_events::record_sandbox_cli_event(
        config,
        backend_override,
        decision,
        reason_code,
    ) {
        tracing::warn!(%error, decision, "sandbox audit event persistence failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_status_shows_policy_fields() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "docker".into(),
            mode: "all".into(),
            workspace_access: "rw".into(),
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_status(&config, true).unwrap();

        assert!(body.contains("Backend: docker"));
        assert!(body.contains("Mode: all"));
        assert!(body.contains("Workspace access: rw"));
        assert!(body.contains("normal permission rules still apply"));
        assert!(body.contains("Docker: enabled="));
        assert!(body.contains("mount_docker_socket=false"));
    }

    #[test]
    fn sandbox_status_verbose_shows_openshell_safety_knobs() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "openshell".into(),
            openshell: archon_core::sandbox::OpenShellConfig {
                enabled: true,
                workspace_mode: "mirror".into(),
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_status(&config, true).unwrap();

        assert!(body.contains("OpenShell: enabled=true"));
        assert!(body.contains("provider_injection=false"));
        assert!(body.contains("Provider routing: host-side"));
        assert!(body.contains("Claude Code spoofing"));
    }

    #[test]
    fn sandbox_explain_rejects_unknown_backend_override() {
        let config = archon_core::sandbox::SandboxConfig::default();

        let error = render_explain(&config, Some("host".into()), None, None).unwrap_err();

        assert!(error.to_string().contains("sandbox.backend"));
    }

    #[test]
    fn sandbox_explain_docker_shows_mount_network_and_redaction_policy() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "docker".into(),
            workspace_access: "scratch".into(),
            docker: archon_core::sandbox::DockerConfig {
                writable_paths: vec!["target".into()],
                network: "disabled".into(),
                ..archon_core::sandbox::DockerConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_explain(&config, None, None, None).unwrap();

        assert!(body.contains("workspace mounted read-only with ephemeral /scratch"));
        assert!(body.contains("Writable paths: target"));
        assert!(body.contains("Network policy: disabled"));
        assert!(body.contains("provider tokens"));
        assert!(body.contains("Docker socket"));
    }

    #[test]
    fn sandbox_explain_openshell_keeps_provider_routing_host_side() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "openshell".into(),
            openshell: archon_core::sandbox::OpenShellConfig {
                enabled: true,
                workspace_mode: "mirror".into(),
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_explain(&config, None, None, None).unwrap();

        assert!(body.contains("openshell mediated execution"));
        assert!(body.contains("provider_injection=false"));
        assert!(body.contains("Claude Code spoofing stays in Archon's provider runtime"));
        assert!(body.contains("host_shell_fallback=false"));
        assert!(body.contains("generated memory databases"));
    }

    #[test]
    fn sandbox_explain_can_show_tool_routing_without_execution() {
        let config = archon_core::sandbox::SandboxConfig {
            backend: "openshell".into(),
            openshell: archon_core::sandbox::OpenShellConfig {
                enabled: true,
                ..archon_core::sandbox::OpenShellConfig::default()
            },
            ..archon_core::sandbox::SandboxConfig::default()
        };

        let body = render_explain(&config, None, Some("Bash"), Some("cargo test")).unwrap();

        assert!(body.contains("Tool: Bash"));
        assert!(body.contains("Decision: route_to_sandbox"));
        assert!(body.contains("Command preview: cargo test"));
    }

    #[test]
    fn sandbox_test_is_detect_only() {
        let config = archon_core::sandbox::SandboxConfig::default();

        let body = render_test(&config, Some("openshell".into())).unwrap();

        assert!(body.contains("Backend: openshell"));
        assert!(body.contains("no untrusted command was run"));
    }

    #[test]
    fn sandbox_sessions_render_redacted_audit_rows() {
        let session = archon_learning::sandbox_sessions::SandboxSessionRecord::new(
            "sandbox-session-1",
            "openshell",
            "sandbox-profile-1",
            "configured",
            "2026-05-08T12:00:00Z",
        )
        .with_run_context(Some("run-1".into()), Some("reviewer".into()))
        .with_workspace(Some("mirror".into()), Some("local".into()))
        .with_transport(Some("openshell".into()), Some("gateway/[redacted]".into()));

        let body = render_sessions_table(&[session]);

        assert!(body.contains("sandbox-session-1"));
        assert!(body.contains("openshell"));
        assert!(body.contains("provider_injection"));
        assert!(body.contains("memory stores are redacted"));
    }
}
